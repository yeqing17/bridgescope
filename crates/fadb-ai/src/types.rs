//! Chat request and response types shared by every provider backend.

use fadb_domain::DeviceSerial;

/// Conversation role for a single message.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// One message in a chat transcript.
///
/// Content is owned UTF-8 text; multimodal input (images) is gated behind
/// [`crate::ProviderCapabilities`] and carried separately on the request.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    #[must_use]
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(ChatRole::User, content)
    }

    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(ChatRole::Assistant, content)
    }
}

/// An incremental text fragment produced by a streaming backend.
///
/// Reserved for the planned streaming surface; the current single-shot
/// [`crate::AiProvider::complete`] returns a full [`ChatResponse`] instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatDelta {
    pub text: String,
}

/// Why generation stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatFinishReason {
    Stop,
    Length,
    Error,
    Other,
}

/// Device context explicitly granted to the provider for one request.
///
/// Kept minimal in this reserved surface; richer excerpts (shell transcripts,
/// screenshots) are added alongside the matching capability flag. The serial is
/// the only identifier carried — never tokens, paths, or file contents.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceContext {
    pub serial: Option<DeviceSerial>,
    pub overview_summary: Option<String>,
}

/// A completion request bound to an explicit context authorization.
#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub device: DeviceContext,
    pub authorization: crate::ContextAuthorization,
}

impl ChatRequest {
    #[must_use]
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            device: DeviceContext::default(),
            authorization: crate::ContextAuthorization::default(),
        }
    }

    #[must_use]
    pub fn with_device(mut self, device: DeviceContext) -> Self {
        self.device = device;
        self
    }

    #[must_use]
    pub fn authorized_by(mut self, authorization: crate::ContextAuthorization) -> Self {
        self.authorization = authorization;
        self
    }

    /// Returns whether the request has any user content to send.
    #[must_use]
    pub fn has_user_content(&self) -> bool {
        self.messages
            .iter()
            .any(|message| message.role == ChatRole::User && !message.content.trim().is_empty())
    }
}

/// A completed response from a provider.
#[derive(Clone, Debug)]
pub struct ChatResponse {
    pub message: ChatMessage,
    pub finish_reason: ChatFinishReason,
    pub model: String,
}
