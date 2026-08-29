//! OpenAI-compatible chat-completions provider.
//!
//! Speaks the ubiquitous `POST {base}/chat/completions` shape used by OpenAI,
//! DeepSeek, Zhipu GLM, Moonshot, Ollama (`/v1`), LM Studio and most hosted or
//! self-hosted gateways. Authentication is resolved from the configured
//! [`AuthTokenSource`] at request time; the token is never logged and never
//! echoed into error details.

use serde::{Deserialize, Serialize};

use crate::{
    AiError, AiProvider, AiProviderConfig, AuthTokenSource, ChatFinishReason, ChatMessage,
    ChatRequest, ChatResponse, ChatRole, ProviderCapabilities,
};
use async_trait::async_trait;

pub const KIND: &str = "openai-compatible";

/// Capabilities assumed for any OpenAI-compatible text backend.
#[must_use]
pub fn openai_compatible_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        system_prompt: true,
        device_overview: true,
        shell_transcript: true,
        screenshot_image: false,
    }
}

/// An [`AiProvider`] backed by any OpenAI-compatible HTTP endpoint.
pub struct OpenAiCompatibleProvider {
    config: AiProviderConfig,
    capabilities: ProviderCapabilities,
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    /// Validate the configuration and construct the HTTP client.
    ///
    /// Fails fast on invalid configuration; network reachability is only
    /// probed on the first [`AiProvider::complete`] call.
    pub fn new(config: AiProviderConfig) -> Result<Self, AiError> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .timeout(config.timeout())
            .build()
            .map_err(|error| AiError::InvalidConfig(format!("HTTP client: {error}")))?;
        Ok(Self {
            capabilities: openai_compatible_capabilities(),
            client,
            config,
        })
    }

    fn url(&self) -> String {
        chat_completions_url(&self.config.endpoint)
    }
}

#[async_trait]
impl AiProvider for OpenAiCompatibleProvider {
    fn kind(&self) -> &str {
        KIND
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError> {
        if !request.has_user_content() {
            return Err(AiError::EmptyRequest);
        }
        let token = resolve_token(&self.config.auth).await?;
        let body = ChatCompletionsBody {
            model: &self.config.model,
            messages: request
                .messages
                .iter()
                .map(|message| WireMessage {
                    role: wire_role(message.role),
                    content: &message.content,
                })
                .collect(),
        };
        let response = self
            .client
            .post(self.url())
            .bearer_auth(token)
            .json(&body)
            .timeout(self.config.timeout())
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return Err(AiError::TimedOut),
            Err(error) => {
                let kind = error_kind(&error);
                tracing::warn!(kind, "AI request failed");
                return Err(AiError::Backend(format!("request failed: {kind}")));
            }
        };
        let status = response.status();
        if !status.is_success() {
            // Deliberately do not include the body: provider error payloads can
            // echo parts of the request.
            return Err(AiError::Backend(format!("provider returned {status}")));
        }
        let payload = response
            .json::<ChatCompletionsResponse>()
            .await
            .map_err(|error| {
                AiError::Backend(format!("invalid response: {}", error_kind(&error)))
            })?;
        finish(payload, &self.config.model)
    }
}

/// Extract the assistant turn from a chat-completions response.
fn finish(
    payload: ChatCompletionsResponse,
    configured_model: &str,
) -> Result<ChatResponse, AiError> {
    let choice = payload
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| AiError::Backend("response contained no choices".to_owned()))?;
    Ok(ChatResponse {
        message: ChatMessage::assistant(choice.message.content),
        finish_reason: wire_finish_reason(choice.finish_reason.as_deref()),
        model: payload.model.unwrap_or_else(|| configured_model.to_owned()),
    })
}

/// Resolve the `{base}/chat/completions` URL for an endpoint.
///
/// A base that already ends with `/chat/completions` is used verbatim so URLs
/// copied straight from provider docs keep working; trailing slashes are
/// tolerated.
#[must_use]
pub fn chat_completions_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

async fn resolve_token(source: &AuthTokenSource) -> Result<String, AiError> {
    match source {
        AuthTokenSource::Environment { variable } => std::env::var(variable).map_err(|_| {
            AiError::InvalidConfig(format!("environment variable {variable} is not set"))
        }),
        AuthTokenSource::File { path } => tokio::fs::read_to_string(path)
            .await
            .map(|value| value.trim().to_owned())
            .map_err(|error| AiError::InvalidConfig(format!("auth file unreadable: {error}"))),
        AuthTokenSource::Inline { value } => Ok(value.clone()),
    }
}

fn wire_role(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

fn wire_finish_reason(reason: Option<&str>) -> ChatFinishReason {
    match reason {
        Some("stop") | None => ChatFinishReason::Stop,
        Some("length") => ChatFinishReason::Length,
        Some(_) => ChatFinishReason::Other,
    }
}

/// A short, non-leaking description of a reqwest error.
fn error_kind(error: &reqwest::Error) -> &'static str {
    if error.is_connect() {
        "connection failed"
    } else if error.is_decode() || error.is_body() {
        "malformed payload"
    } else if error.is_request() {
        "request could not be sent"
    } else {
        "transport error"
    }
}

#[derive(Serialize)]
struct ChatCompletionsBody<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
}

#[derive(Serialize, Deserialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionsResponse {
    #[serde(default)]
    choices: Vec<WireChoice>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireChoiceMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextGrant;

    fn provider_config(endpoint: &str) -> AiProviderConfig {
        AiProviderConfig {
            kind: KIND.to_owned(),
            endpoint: endpoint.to_owned(),
            model: "demo-model".to_owned(),
            auth: AuthTokenSource::Inline {
                value: "test-key".to_owned(),
            },
            timeout_seconds: 30,
        }
    }

    #[test]
    fn url_joins_base_and_full_endpoints() {
        assert_eq!(
            chat_completions_url("https://api.example.com/v1"),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.example.com/v1/"),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.example.com/v1/chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn new_rejects_invalid_config() {
        let mut config = provider_config("https://api.example.com/v1");
        config.endpoint = "   ".to_owned();
        assert!(OpenAiCompatibleProvider::new(config).is_err());
    }

    #[test]
    fn provider_reports_kind_model_and_capabilities() {
        let provider = OpenAiCompatibleProvider::new(provider_config("https://api.example.com/v1"))
            .expect("valid config");
        assert_eq!(provider.kind(), KIND);
        assert_eq!(provider.model(), "demo-model");
        assert!(provider.capabilities().system_prompt);
        assert!(!provider.capabilities().screenshot_image);
    }

    #[test]
    fn request_body_serializes_openai_wire_format() {
        let body = ChatCompletionsBody {
            model: "demo-model",
            messages: vec![
                WireMessage {
                    role: wire_role(ChatRole::System),
                    content: "be brief",
                },
                WireMessage {
                    role: wire_role(ChatRole::User),
                    content: "hello",
                },
            ],
        };
        let json = serde_json::to_value(&body).expect("serializable");
        assert_eq!(json["model"], "demo-model");
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][1]["content"], "hello");
    }

    #[test]
    fn response_parsing_maps_finish_reasons() {
        let payload: ChatCompletionsResponse = serde_json::from_str(
            r#"{"model":"demo-model","choices":[{"message":{"role":"assistant","content":"hi"},
               "finish_reason":"length"}]}"#,
        )
        .expect("valid wire response");
        let response = finish(payload, "demo-model").expect("parsed");
        assert_eq!(response.message.content, "hi");
        assert_eq!(response.finish_reason, ChatFinishReason::Length);
        assert_eq!(response.model, "demo-model");
    }

    #[test]
    fn response_without_choices_is_an_error() {
        let payload: ChatCompletionsResponse =
            serde_json::from_str(r#"{"model":"m","choices":[]}"#).expect("valid");
        assert!(finish(payload, "m").is_err());
    }

    #[test]
    fn missing_finish_reason_defaults_to_stop() {
        assert_eq!(wire_finish_reason(None), ChatFinishReason::Stop);
        assert_eq!(wire_finish_reason(Some("stop")), ChatFinishReason::Stop);
        assert_eq!(
            wire_finish_reason(Some("content_filter")),
            ChatFinishReason::Other
        );
    }

    #[tokio::test]
    async fn empty_requests_are_rejected_without_network() {
        let provider = OpenAiCompatibleProvider::new(provider_config("https://api.example.com/v1"))
            .expect("valid config");
        let request = ChatRequest::new(Vec::new())
            .authorized_by(crate::ContextAuthorization::none().grant(ContextGrant::SystemPrompt));
        assert_eq!(
            provider.complete(&request).await.unwrap_err(),
            AiError::EmptyRequest
        );
    }
}
