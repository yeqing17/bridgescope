//! A scripted in-process provider used by tests and the fake backend.
//!
//! It performs no network I/O and never touches credentials. The response is
//! derived deterministically from the request so behavior is reproducible.

use crate::{
    AiError, AiProvider, ChatFinishReason, ChatMessage, ChatRequest, ChatResponse, ContextGrant,
    ProviderCapabilities,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

const KIND: &str = "fake";
const MODEL: &str = "fadb-fake-1";

/// Capabilities the fake backend advertises by default.
pub fn fake_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        system_prompt: true,
        device_overview: true,
        shell_transcript: false,
        screenshot_image: false,
    }
}

/// A deterministic in-process provider for tests and offline development.
pub struct FakeAiProvider {
    capabilities: ProviderCapabilities,
    /// Last request seen, retained for assertions. Holds no secrets beyond the
    /// prompt text the test itself supplied.
    last_request: Arc<Mutex<Option<ChatRequest>>>,
}

impl FakeAiProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            capabilities: fake_capabilities(),
            last_request: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the last request forwarded to the provider, if any.
    pub async fn last_request(&self) -> Option<ChatRequest> {
        self.last_request.lock().await.clone()
    }
}

impl Default for FakeAiProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AiProvider for FakeAiProvider {
    fn kind(&self) -> &str {
        KIND
    }

    fn model(&self) -> &str {
        MODEL
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError> {
        if !request.has_user_content() {
            return Err(AiError::EmptyRequest);
        }
        // Enforce the same authorization gate a real provider must, so tests
        // exercise the boundary rather than a stub that ignores it.
        if !request
            .authorization
            .permits(ContextGrant::DeviceOverview, &self.capabilities)
            && request.device.overview_summary.is_some()
        {
            return Err(AiError::Unauthorized(
                "device overview requested without a matching grant".to_owned(),
            ));
        }

        let user_text = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == crate::ChatRole::User)
            .map(|message| message.content.clone())
            .unwrap_or_default();
        let device_suffix = request
            .device
            .overview_summary
            .as_deref()
            .map(|_| " (device context acknowledged)")
            .unwrap_or_default();
        let reply = format!("Fake assistant reply to: {user_text}{device_suffix}");

        *self.last_request.lock().await = Some(request.clone());

        Ok(ChatResponse {
            message: ChatMessage::assistant(reply),
            finish_reason: ChatFinishReason::Stop,
            model: MODEL.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatRole, ContextAuthorization};

    fn user_request(text: &str) -> ChatRequest {
        ChatRequest::new(vec![ChatMessage::user(text)])
    }

    #[tokio::test]
    async fn rejects_empty_request() {
        let provider = FakeAiProvider::new();
        let request = ChatRequest::new(Vec::new());
        assert_eq!(
            provider.complete(&request).await.unwrap_err(),
            AiError::EmptyRequest
        );
    }

    #[tokio::test]
    async fn echoes_request_deterministically() {
        let provider = FakeAiProvider::new();
        let response = provider
            .complete(&user_request("hello"))
            .await
            .expect("fake completion");
        assert_eq!(response.finish_reason, ChatFinishReason::Stop);
        assert!(response.message.content.contains("hello"));
        let captured = provider.last_request().await.expect("request captured");
        assert_eq!(captured.messages[0].role, ChatRole::User);
    }

    #[tokio::test]
    async fn blocks_device_overview_without_grant() {
        let provider = FakeAiProvider::new();
        let request = user_request("summarize the device").with_device(crate::DeviceContext {
            overview_summary: Some("Pixel".to_owned()),
            ..Default::default()
        });
        let err = provider.complete(&request).await.unwrap_err();
        assert!(matches!(err, AiError::Unauthorized(_)), "{err:?}");
    }

    #[tokio::test]
    async fn allows_device_overview_with_grant() {
        let provider = FakeAiProvider::new();
        let request = user_request("summarize the device")
            .with_device(crate::DeviceContext {
                overview_summary: Some("Pixel".to_owned()),
                ..Default::default()
            })
            .authorized_by(ContextAuthorization::none().grant(ContextGrant::DeviceOverview));
        let response = provider.complete(&request).await.expect("granted");
        assert!(
            response
                .message
                .content
                .contains("device context acknowledged")
        );
        // The fake deliberately does NOT echo the overview text, so a leaked
        // value never round-trips through the mock.
    }
}
