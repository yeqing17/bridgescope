//! Context authorization boundary.
//!
//! Fadb never forwards device or session data to an AI provider unless
//! the user has explicitly granted it for the current request. The grant is a
//! capability-scoped bitmask checked in Rust by every provider, so a UI toggle
//! that drifts out of sync cannot leak data on its own.

use crate::ProviderCapabilities;

/// A single capability the user has approved for one request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextGrant {
    /// Allow the Fadb system prompt.
    SystemPrompt,
    /// Allow the selected device's serial and overview.
    DeviceOverview,
    /// Allow a bounded shell transcript excerpt.
    ShellTranscript,
    /// Allow a captured screenshot image.
    ScreenshotImage,
}

impl ContextGrant {
    /// Returns the capability flag this grant corresponds to.
    #[must_use]
    pub const fn capability(self) -> CapabilityFlag {
        match self {
            Self::SystemPrompt => CapabilityFlag::SystemPrompt,
            Self::DeviceOverview => CapabilityFlag::DeviceOverview,
            Self::ShellTranscript => CapabilityFlag::ShellTranscript,
            Self::ScreenshotImage => CapabilityFlag::ScreenshotImage,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CapabilityFlags(u8);

impl CapabilityFlags {
    const SYSTEM_PROMPT: u8 = 1 << 0;
    const DEVICE_OVERVIEW: u8 = 1 << 1;
    const SHELL_TRANSCRIPT: u8 = 1 << 2;
    const SCREENSHOT_IMAGE: u8 = 1 << 3;

    fn set(&mut self, flag: u8) {
        self.0 |= flag;
    }

    const fn contains(self, flag: u8) -> bool {
        self.0 & flag == flag
    }
}

/// Capability identifiers shared between grants and provider declarations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityFlag {
    SystemPrompt,
    DeviceOverview,
    ShellTranscript,
    ScreenshotImage,
}

/// The set of capabilities the user has approved for one request.
///
/// Built only via [`ContextAuthorization::grant`]; defaults to nothing allowed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContextAuthorization {
    flags: CapabilityFlags,
}

impl ContextAuthorization {
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Add a grant. The matching provider capability must still be declared
    /// before the data is sent (see [`Self::permits`]).
    #[must_use]
    pub fn grant(mut self, grant: ContextGrant) -> Self {
        match grant.capability() {
            CapabilityFlag::SystemPrompt => self.flags.set(CapabilityFlags::SYSTEM_PROMPT),
            CapabilityFlag::DeviceOverview => self.flags.set(CapabilityFlags::DEVICE_OVERVIEW),
            CapabilityFlag::ShellTranscript => self.flags.set(CapabilityFlags::SHELL_TRANSCRIPT),
            CapabilityFlag::ScreenshotImage => self.flags.set(CapabilityFlags::SCREENSHOT_IMAGE),
        }
        self
    }

    /// True only when the user granted the capability AND the provider declares
    /// support for it. This is the single gate every provider must call before
    /// materializing device or session data into a prompt.
    #[must_use]
    pub fn permits(&self, grant: ContextGrant, capabilities: &ProviderCapabilities) -> bool {
        let user_granted = match grant.capability() {
            CapabilityFlag::SystemPrompt => self.flags.contains(CapabilityFlags::SYSTEM_PROMPT),
            CapabilityFlag::DeviceOverview => self.flags.contains(CapabilityFlags::DEVICE_OVERVIEW),
            CapabilityFlag::ShellTranscript => {
                self.flags.contains(CapabilityFlags::SHELL_TRANSCRIPT)
            }
            CapabilityFlag::ScreenshotImage => {
                self.flags.contains(CapabilityFlags::SCREENSHOT_IMAGE)
            }
        };
        user_granted
            && match grant {
                ContextGrant::SystemPrompt => capabilities.system_prompt,
                ContextGrant::DeviceOverview => capabilities.device_overview,
                ContextGrant::ShellTranscript => capabilities.shell_transcript,
                ContextGrant::ScreenshotImage => capabilities.screenshot_image,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_capabilities() -> ProviderCapabilities {
        ProviderCapabilities {
            system_prompt: true,
            device_overview: true,
            shell_transcript: true,
            screenshot_image: true,
        }
    }

    #[test]
    fn defaults_allow_nothing() {
        let auth = ContextAuthorization::none();
        assert!(!auth.permits(ContextGrant::SystemPrompt, &full_capabilities()));
        assert!(!auth.permits(ContextGrant::DeviceOverview, &full_capabilities()));
    }

    #[test]
    fn grant_is_checked_against_provider_capability() {
        let auth = ContextAuthorization::none().grant(ContextGrant::DeviceOverview);
        let caps = full_capabilities();
        assert!(auth.permits(ContextGrant::DeviceOverview, &caps));
        assert!(!auth.permits(ContextGrant::ShellTranscript, &caps));

        let caps_without_overview = ProviderCapabilities {
            system_prompt: true,
            device_overview: false,
            shell_transcript: true,
            screenshot_image: true,
        };
        assert!(!auth.permits(ContextGrant::DeviceOverview, &caps_without_overview));
    }

    #[test]
    fn grants_are_independent() {
        let auth = ContextAuthorization::none()
            .grant(ContextGrant::SystemPrompt)
            .grant(ContextGrant::ShellTranscript);
        let caps = full_capabilities();
        assert!(auth.permits(ContextGrant::SystemPrompt, &caps));
        assert!(auth.permits(ContextGrant::ShellTranscript, &caps));
        assert!(!auth.permits(ContextGrant::DeviceOverview, &caps));
        assert!(!auth.permits(ContextGrant::ScreenshotImage, &caps));
    }
}
