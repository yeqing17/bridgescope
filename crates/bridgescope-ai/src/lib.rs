//! Provider-neutral AI assistant interface for BridgeScope.
//!
//! This crate is an intentionally small, vendor-agnostic boundary between the
//! desktop UI and any chat-completion backend (a hosted API, a local model, or
//! a scripted test backend). It defines the request/response shape, the context
//! authorization that gates which BridgeScope state may be sent to a provider,
//! and a single [`AiProvider`] trait that concrete backends implement.
//!
//! ## Status
//!
//! [`FakeAiProvider`] covers tests and offline development; any
//! OpenAI-compatible endpoint (OpenAI, DeepSeek, Zhipu GLM, Ollama `/v1`, …)
//! is served by [`OpenAiCompatibleProvider`], which the desktop app constructs
//! from settings. Until a backend is configured the UI surfaces an explicit
//! "AI not configured" state rather than silently calling a default endpoint.

mod authorization;
mod config;
mod error;
mod fake;
mod openai;
mod types;

use async_trait::async_trait;

pub use authorization::{ContextAuthorization, ContextGrant};
pub use config::{
    AiConfig, AiProviderConfig, AuthTokenSource, MAX_AUTH_TOKEN_BYTES, MAX_ENDPOINT_BYTES,
};
pub use error::AiError;
pub use fake::FakeAiProvider;
pub use openai::{KIND as OPENAI_COMPATIBLE_KIND, OpenAiCompatibleProvider};
pub use types::{
    ChatDelta, ChatFinishReason, ChatMessage, ChatRequest, ChatResponse, ChatRole, DeviceContext,
};

/// A provider-neutral chat backend.
///
/// Implementations must be cheap to clone (typically an `Arc` handle to shared
/// HTTP state) and safe to share across the backend Tokio runtime.
///
/// The current surface is single-shot completion. Streaming chunked output is
/// the planned next milestone; it will be added as a separate method so existing
/// backends keep working.
#[async_trait]
pub trait AiProvider: Send + Sync {
    /// Stable identifier for the backend kind (e.g. `"openai-compatible"`,
    /// `"anthropic-messages"`). Used only for diagnostics and UI labels.
    fn kind(&self) -> &str;

    /// Display name of the configured model.
    fn model(&self) -> &str;

    /// Returns the backend's claimed capabilities. Concrete providers should
    /// report only capabilities they actually support; the runtime never
    /// forwards a [`ContextGrant`] the provider has not declared.
    fn capabilities(&self) -> &ProviderCapabilities;

    /// Run a single completion against the configured model.
    ///
    /// Implementations MUST:
    /// - honour the request's [`ContextAuthorization`]: only fields covered by
    ///   an active [`ContextGrant`] may appear in the materialized prompt;
    /// - reject empty requests with [`AiError::EmptyRequest`];
    /// - enforce a bounded timeout and map network/protocol failures into
    ///   [`AiError::Backend`] without leaking request bodies into error detail;
    /// - never persist request content beyond the call.
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError>;
}

/// Capabilities a backend declares so the runtime can scope context grants.
///
/// BridgeScope refuses to send device or session data to a provider that has
/// not opted into the corresponding capability. This keeps the context
/// authorization boundary enforced in Rust, independent of any UI checkbox.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProviderCapabilities {
    /// Provider accepts a system prompt describing BridgeScope itself.
    pub system_prompt: bool,
    /// Provider accepts the currently selected device's serial and overview.
    pub device_overview: bool,
    /// Provider accepts a bounded excerpt of the live shell transcript.
    pub shell_transcript: bool,
    /// Provider accepts a captured screenshot image.
    pub screenshot_image: bool,
}
