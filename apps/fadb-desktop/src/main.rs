#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod i18n;
mod panels;
mod platform;
mod quick_commands;
mod runtime;
mod theme;
mod wireless;

use app::FadbApp;
use eframe::egui;
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("fadb=info".parse().expect("valid directive")),
        )
        .with_target(false)
        .init();

    let native_options = eframe::NativeOptions {
        // Undecorated: the app draws its own title bar (drag to move, window
        // controls in the top bar) and edge seams for resizing.
        viewport: egui::ViewportBuilder::default()
            .with_title("Fadb")
            .with_icon(app_icon())
            .with_decorations(false)
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Fadb",
        native_options,
        Box::new(|creation_context| Ok(Box::new(FadbApp::new(creation_context)))),
    )
}

/// Window/taskbar icon, embedded at compile time from the logo set in
/// `assets/` (taskbar, Alt+Tab; the exe file icon comes from `build.rs`).
fn app_icon() -> egui::IconData {
    let png = include_bytes!("../assets/icon-256.png");
    let img = image::load_from_memory(png)
        .expect("embedded icon is a valid PNG")
        .to_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        width,
        height,
        rgba: img.into_raw(),
    }
}
