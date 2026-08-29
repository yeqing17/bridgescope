//! The wireless-debugging sections of the device-manager window: pairing
//! with a code, switching the selected device to TCP mode, and connecting to
//! mDNS-discovered services.

use std::time::{Duration, Instant};

use bridgescope_domain::{
    AdbEndpoint, BackendCommand, BackendEvent, BridgeError, DeviceSerial, MdnsService, OperationId,
};
use eframe::egui::{self, RichText};

use crate::i18n::{Language, error_text, text};

/// Backoff between automatic mDNS discoveries while the window is open.
const MDNS_RETRY: Duration = Duration::from_secs(8);

/// The TCP port `tcpip` mode listens on by default.
const TCPIP_PORT: u16 = 5555;

#[derive(Default)]
pub struct WirelessState {
    pair_host: String,
    pair_port: String,
    pair_code: String,
    pairing: Option<OperationId>,
    /// The tcpip switch in flight and the device it targets.
    tcpip: Option<(OperationId, DeviceSerial)>,
    notice: Option<&'static str>,
    error: Option<BridgeError>,
    mdns: Vec<MdnsService>,
    mdns_loading: bool,
    last_mdns_attempt: Option<Instant>,
}

impl WirelessState {
    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::PairFinished { .. } => {
                self.pairing = None;
                self.notice = Some("wireless_pair_ok");
                self.error = None;
            }
            BackendEvent::PairFailed { error, .. } => {
                self.pairing = None;
                self.error = Some(error.clone());
            }
            BackendEvent::TcpIpEnabled { serial, .. } => {
                if self
                    .tcpip
                    .as_ref()
                    .is_some_and(|(_, pending)| pending == serial)
                {
                    self.tcpip = None;
                }
                self.notice = Some("wireless_tcpip_ok");
                self.error = None;
            }
            BackendEvent::TcpIpFailed { error, serial, .. } => {
                if self
                    .tcpip
                    .as_ref()
                    .is_some_and(|(_, pending)| pending == serial)
                {
                    self.tcpip = None;
                }
                self.error = Some(error.clone());
            }
            BackendEvent::MdnsServicesLoaded { services } => {
                self.mdns_loading = false;
                self.mdns.clone_from(services);
            }
            BackendEvent::MdnsFailed { error } => {
                self.mdns_loading = false;
                self.error = Some(error.clone());
            }
            _ => {}
        }
    }
}

/// Renders the wireless sections. Returns commands plus, when the user asked
/// to connect to a discovered service, the endpoint to connect to.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut WirelessState,
    selected: Option<&DeviceSerial>,
) -> (Vec<BackendCommand>, Option<AdbEndpoint>) {
    let mut commands = Vec::new();
    let mut connect_to = None;
    ui.separator();
    ui.heading(text(language, "wireless"));
    ui.label(text(language, "wireless_hint"));
    ui.add_space(2.0);

    if let Some(error) = &state.error {
        ui.label(
            RichText::new(error_text(language, error))
                .color(egui::Color32::from_rgb(248, 113, 113)),
        );
    }
    if let Some(key) = state.notice {
        ui.label(RichText::new(text(language, key)).color(egui::Color32::from_rgb(74, 222, 128)));
    }

    // Pairing: an input row like the plain connect row above.
    ui.horizontal(|ui| {
        ui.label(text(language, "wireless_pair"));
        ui.add(
            egui::TextEdit::singleline(&mut state.pair_host)
                .desired_width(160.0)
                .hint_text(text(language, "ip_host")),
        );
        ui.add(
            egui::TextEdit::singleline(&mut state.pair_port)
                .desired_width(70.0)
                .hint_text("41234"),
        );
        ui.label(text(language, "wireless_code"));
        ui.add(
            egui::TextEdit::singleline(&mut state.pair_code)
                .desired_width(80.0)
                .hint_text("123456"),
        );
        let pairing = state.pairing.is_some();
        if ui
            .add_enabled(
                !pairing,
                egui::Button::new(text(language, "wireless_pair_go")),
            )
            .clicked()
        {
            match pair_inputs(state) {
                Ok((host, port, code)) => {
                    state.notice = Some("wireless_pairing");
                    commands.push(BackendCommand::PairDevice {
                        request_id: OperationId::new(),
                        host,
                        port,
                        code,
                    });
                }
                Err(error) => state.error = Some(error),
            }
        }
        if pairing {
            ui.spinner();
        }
    });

    // TCP mode: switches the currently selected USB device to network adb.
    ui.horizontal(|ui| {
        let busy = state
            .tcpip
            .as_ref()
            .is_some_and(|(_, pending)| Some(pending) == selected);
        if ui
            .add_enabled(
                selected.is_some() && !busy,
                egui::Button::new(text(language, "wireless_tcpip")),
            )
            .on_hover_text(text(language, "wireless_tcpip_hint"))
            .on_disabled_hover_text(text(language, "wireless_tcpip_need_device"))
            .clicked()
            && let Some(serial) = selected
        {
            state.notice = Some("wireless_tcpip_running");
            commands.push(BackendCommand::EnableTcpIp {
                request_id: OperationId::new(),
                serial: serial.clone(),
                port: TCPIP_PORT,
            });
        }
        if busy {
            ui.spinner();
        }
        ui.label(text(language, "wireless_tcpip_hint"));
    });

    // Discovery: one-shot button, then the service list with connect actions.
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !state.mdns_loading,
                egui::Button::new(text(language, "wireless_mdns")),
            )
            .clicked()
        {
            state.mdns_loading = true;
            commands.push(BackendCommand::ListMdnsServices);
        }
        if state.mdns_loading {
            ui.spinner();
        }
        ui.label(text(language, "wireless_mdns_hint"));
    });
    if state.mdns.is_empty() && !state.mdns_loading {
        ui.label(text(language, "wireless_none"));
    } else {
        for service in state.mdns.clone() {
            ui.horizontal(|ui| {
                ui.label(format!("{} · {}", service.name, service.service_type));
                ui.label(service.address.clone());
                if let Some(endpoint) = endpoint_from_address(&service.address)
                    && ui
                        .add_enabled(
                            state.pairing.is_none(),
                            egui::Button::new(text(language, "wireless_connect")),
                        )
                        .clicked()
                {
                    connect_to = Some(endpoint);
                }
            });
        }
    }

    (commands, connect_to)
}

/// Validates the pairing inputs into host/port/code.
fn pair_inputs(state: &WirelessState) -> Result<(String, u16, String), BridgeError> {
    let host = state.pair_host.trim();
    let code = state.pair_code.trim();
    let port = state
        .pair_port
        .trim()
        .parse::<u16>()
        .map_err(|_| BridgeError::invalid_input("wireless.pair_invalid"))?;
    if host.is_empty()
        || !(6..=8).contains(&code.len())
        || !code.chars().all(|c| c.is_ascii_digit())
    {
        return Err(BridgeError::invalid_input("wireless.pair_invalid"));
    }
    Ok((host.to_owned(), port, code.to_owned()))
}

/// Parses an mDNS `host:port` address into an endpoint.
fn endpoint_from_address(address: &str) -> Option<AdbEndpoint> {
    let (host, port) = address.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    AdbEndpoint::new(host, port).ok()
}

/// Called by the app each frame the device-manager window is open: discovers
/// services once shortly after opening, retrying with a backoff while empty.
pub fn auto(state: &mut WirelessState) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    let empty_and_idle = state.mdns.is_empty() && !state.mdns_loading;
    let recently_attempted = state
        .last_mdns_attempt
        .is_some_and(|last| last.elapsed() < MDNS_RETRY);
    if empty_and_idle && !recently_attempted {
        state.last_mdns_attempt = Some(Instant::now());
        state.mdns_loading = true;
        commands.push(BackendCommand::ListMdnsServices);
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_inputs_validate_host_port_and_code() {
        let mut state = WirelessState::default();
        assert!(pair_inputs(&state).is_err(), "empty inputs must fail");
        state.pair_host = "192.168.1.20".to_owned();
        state.pair_port = "not-a-port".to_owned();
        state.pair_code = "123456".to_owned();
        assert!(pair_inputs(&state).is_err(), "bad port must fail");
        state.pair_port = "41234".to_owned();
        state.pair_code = "12345".to_owned();
        assert!(pair_inputs(&state).is_err(), "short code must fail");
        state.pair_code = "123456".to_owned();
        let (host, port, code) = pair_inputs(&state).expect("valid inputs");
        assert_eq!(
            (host.as_str(), port, code.as_str()),
            ("192.168.1.20", 41234, "123456")
        );
    }

    #[test]
    fn events_clear_the_pending_markers() {
        let mut state = WirelessState::default();
        let request_id = OperationId::new();
        state.pairing = Some(request_id);
        state.handle_event(&BackendEvent::PairFailed {
            request_id,
            error: BridgeError::invalid_input("wireless.pair_invalid"),
        });
        assert_eq!(state.pairing, None);
        assert!(state.error.is_some());

        let request_id = OperationId::new();
        let serial = DeviceSerial::new("1A2B3C4D5E").expect("serial");
        state.tcpip = Some((request_id, serial.clone()));
        state.handle_event(&BackendEvent::TcpIpEnabled {
            request_id,
            serial: serial.clone(),
        });
        assert_eq!(state.tcpip, None);
        assert_eq!(state.notice, Some("wireless_tcpip_ok"));
    }

    #[test]
    fn auto_discovers_once_with_backoff() {
        let mut state = WirelessState::default();
        let commands = auto(&mut state);
        assert!(matches!(
            commands.as_slice(),
            [BackendCommand::ListMdnsServices]
        ));
        state.handle_event(&BackendEvent::MdnsServicesLoaded {
            services: Vec::new(),
        });
        // Still empty, but inside the backoff window: no repeat discovery.
        assert!(auto(&mut state).is_empty());
        assert!(!state.mdns_loading);
    }

    #[test]
    fn mdns_addresses_parse_into_endpoints() {
        let endpoint = endpoint_from_address("192.168.1.20:5555").expect("valid address");
        assert_eq!(endpoint.host(), "192.168.1.20");
        assert_eq!(endpoint.port(), 5555);
        assert!(endpoint_from_address("no-port").is_none());
        assert!(endpoint_from_address("host:99999").is_none());
    }
}
