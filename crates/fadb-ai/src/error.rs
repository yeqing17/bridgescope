//! Error type for the AI boundary.
//!
//! Detail strings must never echo request bodies, tokens, or device data — they
//! describe only the failure category and the provider-facing reason.

use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AiError {
    #[error("AI request contained no user content")]
    EmptyRequest,
    #[error("AI provider was not configured")]
    NotConfigured,
    #[error("AI provider configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("AI context grant denied: {0}")]
    Unauthorized(String),
    #[error("AI backend call failed: {0}")]
    Backend(String),
    #[error("AI request timed out")]
    TimedOut,
}
