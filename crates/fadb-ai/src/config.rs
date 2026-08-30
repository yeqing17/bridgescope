//! AI provider configuration.
//!
//! Configuration is provider-neutral and never stores raw credentials inline.
//! Authentication is referenced indirectly via [`AuthTokenSource`], so a key
//! can live in an environment variable or a file the operator controls without
//! ever being serialized into Fadb settings or logs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Maximum length accepted for an endpoint URL.
pub const MAX_ENDPOINT_BYTES: usize = 2 * 1024;
/// Maximum length accepted for an inline auth-token fallback (testing only).
pub const MAX_AUTH_TOKEN_BYTES: usize = 4 * 1024;

/// Where a provider should read its authentication token from.
///
/// Fadb resolves the token at request time and never persists the value.
/// An inline literal is accepted only for development/test configurations and
/// is rejected when it exceeds [`MAX_AUTH_TOKEN_BYTES`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthTokenSource {
    /// Read the token from the named environment variable.
    Environment { variable: String },
    /// Read the token from a file on disk (operator-managed).
    File { path: PathBuf },
    /// Inline literal. Test/development only — never commit this to settings.
    Inline { value: String },
}

/// Provider-neutral description of one chat backend.
///
/// `kind` selects the transport shape (e.g. `"openai-compatible"`,
/// `"anthropic-messages"`); a concrete client is constructed from this config
/// by the application once a real provider crate is added.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AiProviderConfig {
    pub kind: String,
    pub endpoint: String,
    pub model: String,
    pub auth: AuthTokenSource,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

fn default_timeout_seconds() -> u64 {
    30
}

impl AiProviderConfig {
    /// Validates the config without touching the network.
    ///
    /// Returns an error if required fields are empty, bounds are exceeded, or
    /// the auth source is structurally invalid. Does NOT verify that a token is
    /// present — that is resolved lazily so configuration can be loaded before
    /// credentials exist.
    pub fn validate(&self) -> Result<(), crate::AiError> {
        if self.kind.trim().is_empty() {
            return Err(crate::AiError::InvalidConfig(
                "provider kind is empty".to_owned(),
            ));
        }
        if self.model.trim().is_empty() {
            return Err(crate::AiError::InvalidConfig(
                "provider model is empty".to_owned(),
            ));
        }
        if self.endpoint.trim().is_empty() {
            return Err(crate::AiError::InvalidConfig(
                "provider endpoint is empty".to_owned(),
            ));
        }
        if self.endpoint.len() > MAX_ENDPOINT_BYTES {
            return Err(crate::AiError::InvalidConfig(format!(
                "provider endpoint exceeds {MAX_ENDPOINT_BYTES} bytes"
            )));
        }
        match &self.auth {
            AuthTokenSource::Environment { variable } if variable.trim().is_empty() => Err(
                crate::AiError::InvalidConfig("auth environment variable is empty".to_owned()),
            ),
            AuthTokenSource::File { path } if path.as_os_str().is_empty() => Err(
                crate::AiError::InvalidConfig("auth file path is empty".to_owned()),
            ),
            AuthTokenSource::Inline { value } if value.len() > MAX_AUTH_TOKEN_BYTES => {
                Err(crate::AiError::InvalidConfig(format!(
                    "inline auth token exceeds {MAX_AUTH_TOKEN_BYTES} bytes"
                )))
            }
            _ => Ok(()),
        }
    }

    #[must_use]
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout_seconds.max(1))
    }
}

/// Top-level AI configuration: whether AI is enabled and which provider to use.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    pub enabled: bool,
    pub provider: Option<AiProviderConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_provider() -> AiProviderConfig {
        AiProviderConfig {
            kind: "openai-compatible".to_owned(),
            endpoint: "https://example.invalid/v1".to_owned(),
            model: "demo-model".to_owned(),
            auth: AuthTokenSource::Environment {
                variable: "DEMO_KEY".to_owned(),
            },
            timeout_seconds: 30,
        }
    }

    #[test]
    fn valid_config_passes() {
        assert!(valid_provider().validate().is_ok());
    }

    #[test]
    fn rejects_empty_kind() {
        let mut config = valid_provider();
        config.kind = "  ".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_empty_env_variable() {
        let mut config = valid_provider();
        config.auth = AuthTokenSource::Environment {
            variable: String::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_oversized_inline_token() {
        let mut config = valid_provider();
        config.auth = AuthTokenSource::Inline {
            value: "x".repeat(MAX_AUTH_TOKEN_BYTES + 1),
        };
        assert!(config.validate().is_err());
    }
}
