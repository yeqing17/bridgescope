#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod i18n;
mod panels;
mod platform;
mod quick_commands;
mod runtime;
mod theme;
mod wireless;

use app::BridgeScopeApp;
use eframe::egui;
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("bridgescope=info".parse().expect("valid directive")),
        )
        .with_target(false)
        .init();

    let native_options = eframe::NativeOptions {
        // Undecorated: the app draws its own title bar (drag to move, window
        // controls in the top bar) and edge seams for resizing.
        viewport: egui::ViewportBuilder::default()
            .with_title("BridgeScope")
            .with_decorations(false)
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "BridgeScope",
        native_options,
        Box::new(|creation_context| Ok(Box::new(BridgeScopeApp::new(creation_context)))),
    )
}
