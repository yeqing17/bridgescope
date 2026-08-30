use std::collections::{HashMap, HashSet};

use eframe::egui::{self, RichText};
use fadb_domain::{
    ApplicationAction, ApplicationDetails, ApplicationIconData, ApplicationRecord, BackendCommand,
    BackendEvent, DeviceTarget, OperationId, PackageName,
};

use crate::i18n::{Language, text};
use crate::theme;

/// Which subset of the package list is shown. Third-party is the default so
/// the panel opens on the apps the user actually installed.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum ApplicationFilter {
    #[default]
    ThirdParty,
    System,
    All,
}

/// One in-flight package action, matched back by request id when the
/// completion event arrives.
#[derive(Clone, Copy, Debug)]
struct PendingAction {
    request_id: OperationId,
    action: ApplicationAction,
}

#[derive(Default)]
pub struct ApplicationsPanelState {
    pub target: Option<DeviceTarget>,
    pub applications: Vec<ApplicationRecord>,
    pub loading: bool,
    pub filter: ApplicationFilter,
    pub search: String,
    pub selected: Option<PackageName>,
    pub details: Option<ApplicationDetails>,
    pub details_loading: bool,
    pending: Option<PendingAction>,
    /// An APK install in flight, matched back by request id.
    install: Option<OperationId>,
    /// A transient success notice; cleared when the refreshed list arrives.
    pub install_notice: bool,
    /// A destructive action awaiting confirmation (clear data, uninstall).
    pub confirm: Option<ApplicationAction>,
    /// Decoded launcher icons received from the transport, keyed by package.
    pub icons: HashMap<PackageName, ApplicationIconData>,
    /// Packages whose icon request is in flight; keeps the per-frame
    /// request pass from re-issuing commands while one is being served.
    icon_pending: HashSet<PackageName>,
    /// GPU textures lazily uploaded from [`Self::icons`] once per frame need.
    icon_textures: HashMap<PackageName, egui::TextureHandle>,
}

impl ApplicationsPanelState {
    pub fn reset_for(&mut self, target: Option<DeviceTarget>) {
        if self.target != target {
            self.target = target;
            self.applications.clear();
            self.selected = None;
            self.details = None;
            self.details_loading = false;
            self.pending = None;
            self.install = None;
            self.install_notice = false;
            self.confirm = None;
            self.loading = false;
            self.icons.clear();
            self.icon_pending.clear();
            self.icon_textures.clear();
        }
    }

    /// Applies one backend event; mutating actions that change the package
    /// list answer with a reload command.
    pub fn handle_event(&mut self, event: &BackendEvent) -> Vec<BackendCommand> {
        let mut commands = Vec::new();
        match event {
            BackendEvent::ApplicationsLoading(target) if self.target.as_ref() == Some(target) => {
                self.loading = true;
            }
            BackendEvent::ApplicationsLoaded(snapshot)
                if self.target.as_ref() == Some(&snapshot.target) =>
            {
                self.loading = false;
                self.install_notice = false;
                self.applications.clone_from(&snapshot.applications);
                // The selection may have vanished (uninstalled, or its record
                // no longer matches); drop it so the details pane follows.
                if let Some(selected) = &self.selected
                    && !self.applications.iter().any(|app| &app.package == selected)
                {
                    self.selected = None;
                    self.details = None;
                    self.details_loading = false;
                }
                // Icon requests are issued by the frame pass for exactly the
                // visible packages — see `request_visible_icons`.
            }
            BackendEvent::ApplicationsFailed { target, .. }
                if self.target.as_ref() == Some(target) =>
            {
                self.loading = false;
            }
            BackendEvent::ApplicationIconLoaded {
                target,
                package,
                icon,
            } if self.target.as_ref() == Some(target) => {
                self.icons.insert(package.clone(), icon.clone());
                self.icon_pending.remove(package);
            }
            BackendEvent::ApplicationDetailsLoading { package, .. }
                if self.selected.as_ref() == Some(package) =>
            {
                self.details_loading = true;
            }
            BackendEvent::ApplicationDetailsLoaded { details, .. }
                if self.selected.as_ref() == Some(&details.package) =>
            {
                self.details_loading = false;
                self.details = Some(details.clone());
            }
            BackendEvent::ApplicationDetailsFailed { package, .. }
                if self.selected.as_ref() == Some(package) =>
            {
                self.details_loading = false;
                self.details = None;
            }
            BackendEvent::ApplicationActionCompleted {
                request_id, action, ..
            } => {
                if self
                    .pending
                    .is_some_and(|pending| pending.request_id == *request_id)
                {
                    self.pending = None;
                    if action.mutates_listing()
                        && let Some(target) = self.target.clone()
                    {
                        self.loading = true;
                        commands.push(BackendCommand::LoadApplications(target));
                    }
                }
            }
            BackendEvent::ApplicationActionFailed { request_id, .. } => {
                if self
                    .pending
                    .is_some_and(|pending| pending.request_id == *request_id)
                {
                    self.pending = None;
                }
            }
            BackendEvent::ApkInstallFinished { request_id, target }
                if self.install.as_ref() == Some(request_id)
                    && self.target.as_ref() == Some(target) =>
            {
                self.install = None;
                self.install_notice = true;
                // The freshly installed package should show up right away.
                if let Some(current) = self.target.clone() {
                    self.loading = true;
                    commands.push(BackendCommand::LoadApplications(current));
                }
            }
            BackendEvent::ApkInstallFailed { request_id, .. }
                if self.install.as_ref() == Some(request_id) =>
            {
                self.install = None;
            }
            _ => {}
        }
        commands
    }

    /// The record of the selected package, if it is still listed.
    fn selected_record(&self) -> Option<&ApplicationRecord> {
        let selected = self.selected.as_ref()?;
        self.applications
            .iter()
            .find(|app| &app.package == selected)
    }

    /// The packages matching the current filter and search, in list order.
    fn visible_applications(&self) -> Vec<ApplicationRecord> {
        let query = self.search.trim().to_lowercase();
        self.applications
            .iter()
            .filter(|app| match self.filter {
                ApplicationFilter::ThirdParty => !app.system,
                ApplicationFilter::System => app.system,
                ApplicationFilter::All => true,
            })
            .filter(|app| query.is_empty() || app.package.as_str().to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    /// Per-frame icon request pass covering exactly the visible packages —
    /// hundreds of system apps never get requested from the default
    /// third-party view, and switching filters fetches only the newly shown
    /// ones. The pending set keeps this from re-issuing while requests are
    /// in flight (packages whose icon proves unfetchable never get an
    /// event, and staying pending also spares them a retry storm).
    fn request_visible_icons(&mut self, commands: &mut Vec<BackendCommand>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let missing: Vec<PackageName> = self
            .visible_applications()
            .into_iter()
            .map(|app| app.package)
            .filter(|package| {
                !self.icons.contains_key(package) && !self.icon_pending.contains(package)
            })
            .collect();
        if missing.is_empty() {
            return;
        }
        for package in &missing {
            self.icon_pending.insert(package.clone());
        }
        commands.push(BackendCommand::LoadApplicationIcons {
            target,
            packages: missing,
        });
    }
}

#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut ApplicationsPanelState,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    let palette = theme::palette(ui.visuals().dark_mode);

    ui.horizontal(|ui| {
        ui.heading(text(language, "applications"));
        if state.loading || state.install.is_some() {
            ui.spinner();
        }
        if state.install.is_some() {
            ui.label(text(language, "applications_install_running"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(
                    state.target.is_some() && state.install.is_none(),
                    egui::Button::new(text(language, "applications_install")),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("APK", &["apk"])
                    .pick_file()
                && let Some(target) = state.target.clone()
            {
                let request_id = OperationId::new();
                state.install = Some(request_id);
                state.install_notice = false;
                commands.push(BackendCommand::InstallApk {
                    request_id,
                    target,
                    apk_path: path,
                });
            }
        });
    });
    ui.small(text(language, "applications_hint"));
    if state.install_notice {
        ui.label(
            RichText::new(text(language, "applications_install_ok"))
                .color(egui::Color32::from_rgb(74, 222, 128)),
        );
    }
    ui.add_space(6.0);

    if state.target.is_none() {
        ui.label(text(language, "select_device"));
        return commands;
    }

    let visible = state.visible_applications();
    // Fetch icons for exactly what the grid shows, right before painting it.
    state.request_visible_icons(&mut commands);

    // Details pane on the right first, so the toolbar and app grid below
    // span only the list's column.
    egui::SidePanel::right("applications-details")
        .resizable(true)
        .default_width(330.0)
        .width_range(280.0..=460.0)
        .frame(
            egui::Frame::new()
                .fill(palette.ai_bubble)
                .stroke(egui::Stroke::new(1.0, palette.bubble_stroke))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(10)),
        )
        .show_inside(ui, |ui| {
            details_pane(ui, language, &palette, state, &mut commands);
        });

    toolbar(ui, language, state, &mut commands);

    egui::ScrollArea::vertical()
        .id_salt("applications-grid")
        .auto_shrink(false)
        .show(ui, |ui| {
            if visible.is_empty() {
                ui.weak(if state.loading {
                    text(language, "loading")
                } else {
                    text(language, "no_applications")
                });
                return;
            }
            // Upload textures lazily: one texture per package, once.
            for app in &visible {
                if state.icon_textures.contains_key(&app.package) {
                    continue;
                }
                if let Some(icon) = state.icons.get(&app.package) {
                    let width = usize::try_from(icon.width).unwrap_or(0);
                    let height = usize::try_from(icon.height).unwrap_or(0);
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied([width, height], &icon.rgba);
                    let texture = ui.ctx().load_texture(
                        format!("app-icon-{}", app.package),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    state.icon_textures.insert(app.package.clone(), texture);
                }
            }
            let columns = grid_columns(ui.available_width());
            egui::Grid::new("applications-grid")
                .num_columns(columns)
                .spacing([6.0, 10.0])
                .show(ui, |ui| {
                    for (index, app) in visible.iter().enumerate() {
                        app_tile(ui, language, &palette, state, app, &mut commands);
                        if (index + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
        });

    confirmation_dialog(ui, language, &palette, state, &mut commands);
    commands
}

/// Refresh button, filter segmented control, and the search field.
fn toolbar(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut ApplicationsPanelState,
    commands: &mut Vec<BackendCommand>,
) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                state.target.is_some() && !state.loading,
                egui::Button::new(text(language, "refresh")),
            )
            .clicked()
            && let Some(target) = state.target.clone()
        {
            state.loading = true;
            commands.push(BackendCommand::LoadApplications(target));
        }
        for (filter, key) in [
            (ApplicationFilter::ThirdParty, "app_filter_third"),
            (ApplicationFilter::System, "app_filter_system"),
            (ApplicationFilter::All, "app_filter_all"),
        ] {
            if ui
                .add(egui::Button::new(text(language, key)).selected(state.filter == filter))
                .clicked()
            {
                state.filter = filter;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .desired_width(190.0)
                    .hint_text(text(language, "apps_search_hint")),
            );
        });
    });
    ui.add_space(4.0);
}

const TILE_SIZE: egui::Vec2 = egui::Vec2::new(92.0, 92.0);
const ICON_SIZE: f32 = 52.0;

/// Columns the grid affords at the current width, keeping tiles comfortable
/// on narrow windows.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn grid_columns(available: f32) -> usize {
    ((available / (TILE_SIZE.x + 8.0)).floor() as usize).clamp(2, 8)
}

/// One launcher-style tile: the app icon (or a generated monogram until the
/// icon arrives), the short package name, and a right-click quick menu.
fn app_tile(
    ui: &mut egui::Ui,
    language: Language,
    palette: &theme::Palette,
    state: &mut ApplicationsPanelState,
    app: &ApplicationRecord,
    commands: &mut Vec<BackendCommand>,
) {
    let (cell_rect, _) = ui.allocate_exact_size(TILE_SIZE, egui::Sense::hover());
    let response = ui.interact(
        cell_rect,
        egui::Id::new(("application-tile", app.package.as_str())),
        egui::Sense::click(),
    );
    let selected = state.selected.as_ref() == Some(&app.package);
    let painter = ui.painter();
    let fill = if selected {
        Some(ui.visuals().widgets.active.weak_bg_fill)
    } else if response.hovered() {
        Some(palette.chip_fill)
    } else {
        None
    };
    if let Some(fill) = fill {
        painter.rect_filled(cell_rect, 10.0, fill);
    }

    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(
            cell_rect.center().x,
            cell_rect.top() + 5.0 + ICON_SIZE / 2.0,
        ),
        egui::Vec2::splat(ICON_SIZE),
    );
    if let Some(texture) = state.icon_textures.get(&app.package) {
        painter.rect_filled(icon_rect, 12.0, palette.chip_fill);
        let tint = if app.disabled {
            // Frozen apps render desaturated instead of vanishing.
            egui::Color32::from_rgba_unmultiplied(150, 150, 158, 255)
        } else {
            egui::Color32::WHITE
        };
        painter.image(
            texture.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    } else {
        painter.rect_filled(icon_rect, 12.0, tile_color(app.package.as_str()));
        painter.text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            tile_initial(app.package.as_str()),
            egui::FontId::proportional(22.0),
            egui::Color32::WHITE,
        );
    }
    if app.disabled {
        painter.text(
            egui::pos2(icon_rect.right() - 3.0, icon_rect.bottom() - 3.0),
            egui::Align2::RIGHT_BOTTOM,
            "❄",
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgb(160, 165, 175),
        );
    }
    painter.text(
        egui::pos2(cell_rect.center().x, icon_rect.bottom() + 7.0),
        egui::Align2::CENTER_TOP,
        elide(short_name(app.package.as_str()), 14),
        egui::FontId::proportional(11.0),
        if selected {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        },
    );

    // Clicking (or opening the quick menu) selects the app so the details
    // pane follows along.
    if (response.clicked() || response.context_menu_opened())
        && state.selected.as_ref() != Some(&app.package)
    {
        state.selected = Some(app.package.clone());
        state.details = None;
        state.details_loading = false;
        if let Some(target) = state.target.clone() {
            commands.push(BackendCommand::LoadApplicationDetails {
                request_id: OperationId::new(),
                target,
                package: app.package.clone(),
            });
        }
    }
    response
        .on_hover_text(app.package.as_str())
        .context_menu(|menu| {
            quick_menu(menu, language, palette, state, app, commands);
        });
}

/// Right-click actions mirroring the details pane; destructive ones still
/// route through the confirmation dialog.
fn quick_menu(
    menu: &mut egui::Ui,
    language: Language,
    palette: &theme::Palette,
    state: &mut ApplicationsPanelState,
    app: &ApplicationRecord,
    commands: &mut Vec<BackendCommand>,
) {
    let busy = state.pending.is_some();
    let Some(target) = state.target.clone() else {
        return;
    };
    if menu
        .add_enabled(!busy, egui::Button::new(text(language, "app_open")))
        .clicked()
    {
        push_action(
            state,
            commands,
            &target,
            &app.package,
            ApplicationAction::Launch,
        );
    }
    if menu
        .add_enabled(!busy, egui::Button::new(text(language, "app_force_stop")))
        .clicked()
    {
        push_action(
            state,
            commands,
            &target,
            &app.package,
            ApplicationAction::ForceStop,
        );
    }
    if menu.button(text(language, "app_clear_data")).clicked() {
        state.confirm = Some(ApplicationAction::ClearData);
    }
    let freeze_label = if app.disabled {
        text(language, "app_unfreeze")
    } else {
        text(language, "app_freeze")
    };
    if menu
        .add_enabled(!busy, egui::Button::new(freeze_label))
        .clicked()
    {
        let action = if app.disabled {
            ApplicationAction::Unfreeze
        } else {
            ApplicationAction::Freeze
        };
        push_action(state, commands, &target, &app.package, action);
    }
    menu.separator();
    if menu
        .button(RichText::new(text(language, "app_uninstall")).color(palette.danger_stroke))
        .clicked()
    {
        state.confirm = Some(ApplicationAction::Uninstall);
    }
}

/// Stable pastel from the package name, so fallback tiles look deliberate.
#[allow(clippy::cast_precision_loss)]
fn tile_color(package: &str) -> egui::Color32 {
    let hash = package.bytes().fold(2_166_136_261_u32, |acc, byte| {
        acc.wrapping_mul(16_777_619).wrapping_add(u32::from(byte))
    });
    let hue = f32::from(u16::try_from(hash % 3600).unwrap_or(0)) / 3600.0;
    egui::Color32::from(egui::ecolor::Hsva::new(hue, 0.42, 0.55, 1.0))
}

/// The last dot-segment's initial: `org.mozilla.firefox` -> `F`.
fn tile_initial(package: &str) -> String {
    let initial = package
        .rsplit('.')
        .next()
        .and_then(|segment| segment.chars().next());
    initial.map_or_else(|| "?".to_owned(), |char| char.to_uppercase().collect())
}

/// The short display name: the last dot-segment of the package.
fn short_name(package: &str) -> &str {
    package.rsplit('.').next().unwrap_or(package)
}

/// Painter text cannot truncate, so elide long segments manually.
fn elide(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_owned()
    } else {
        let kept: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

/// The right-hand pane: app info plus the action buttons.
fn details_pane(
    ui: &mut egui::Ui,
    language: Language,
    palette: &theme::Palette,
    state: &mut ApplicationsPanelState,
    commands: &mut Vec<BackendCommand>,
) {
    let Some(selected) = state.selected.clone() else {
        ui.add_space(32.0);
        ui.vertical_centered(|ui| {
            ui.weak(text(language, "app_details_hint"));
        });
        return;
    };

    ui.heading(text(language, "app_info"));
    // Not selectable: rows and the pane already carry the value; selectable
    // text would fight pane resizing.
    ui.add(
        egui::Label::new(RichText::new(selected.as_str()).strong())
            .selectable(false)
            .truncate(),
    );
    ui.add_space(4.0);

    if state.details_loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak(text(language, "loading"));
        });
    } else if let Some(details) = &state.details {
        details_fields(ui, language, details);
    } else {
        ui.weak(text(language, "app_details_unavailable"));
    }

    ui.add_space(8.0);
    ui.separator();
    action_buttons(ui, language, palette, state, &selected, commands);

    if let Some(pending) = state.pending {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak(format!(
                "{} · {}",
                text(language, "app_action_running"),
                action_label(language, pending.action)
            ));
        });
    }
}

fn details_fields(ui: &mut egui::Ui, language: Language, details: &ApplicationDetails) {
    let version = match (details.version_name.as_deref(), details.version_code) {
        (Some(name), Some(code)) => format!("{name} ({code})"),
        (Some(name), None) => name.to_owned(),
        (None, Some(code)) => code.to_string(),
        (None, None) => "-".to_owned(),
    };
    egui::Grid::new("app-details-grid")
        .num_columns(2)
        .spacing([12.0, 5.0])
        .show(ui, |ui| {
            field_row(ui, language, "app_version", &version);
            field_row(
                ui,
                language,
                "app_target_sdk",
                &details
                    .target_sdk
                    .map_or("-".to_owned(), |sdk| sdk.to_string()),
            );
            field_row(
                ui,
                language,
                "app_min_sdk",
                &details
                    .min_sdk
                    .map_or("-".to_owned(), |sdk| sdk.to_string()),
            );
            field_row(
                ui,
                language,
                "app_installer",
                details.installer.as_deref().unwrap_or("-"),
            );
            field_row(
                ui,
                language,
                "app_first_install",
                details.first_install_time.as_deref().unwrap_or("-"),
            );
            field_row(
                ui,
                language,
                "app_last_update",
                details.last_update_time.as_deref().unwrap_or("-"),
            );
            field_row(
                ui,
                language,
                "app_apk_path",
                details.apk_path.as_deref().unwrap_or("-"),
            );
        });

    ui.add_space(8.0);
    ui.strong(text(language, "app_permissions"));
    if details.permissions.is_empty() {
        ui.weak(text(language, "app_permissions_none"));
    } else {
        egui::ScrollArea::vertical()
            .id_salt("app-permissions")
            .max_height(110.0)
            .show(ui, |ui| {
                for permission in &details.permissions {
                    ui.small(permission);
                }
            });
    }
}

fn field_row(ui: &mut egui::Ui, language: Language, key: &str, value: &str) {
    ui.weak(text(language, key));
    // Long values (APK paths) truncate instead of stretching the grid.
    ui.add(egui::Label::new(value).truncate());
    ui.end_row();
}

fn action_buttons(
    ui: &mut egui::Ui,
    language: Language,
    palette: &theme::Palette,
    state: &mut ApplicationsPanelState,
    selected: &PackageName,
    commands: &mut Vec<BackendCommand>,
) {
    let busy = state.pending.is_some();
    let frozen = state
        .selected_record()
        .is_some_and(|record| record.disabled);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!busy, egui::Button::new(text(language, "app_open")))
            .clicked()
            && let Some(target) = state.target.clone()
        {
            push_action(
                state,
                commands,
                &target,
                selected,
                ApplicationAction::Launch,
            );
        }
        if ui
            .add_enabled(!busy, egui::Button::new(text(language, "app_force_stop")))
            .clicked()
            && let Some(target) = state.target.clone()
        {
            push_action(
                state,
                commands,
                &target,
                selected,
                ApplicationAction::ForceStop,
            );
        }
        if ui
            .add_enabled(!busy, egui::Button::new(text(language, "app_clear_data")))
            .clicked()
        {
            state.confirm = Some(ApplicationAction::ClearData);
        }
        let freeze_label = if frozen {
            text(language, "app_unfreeze")
        } else {
            text(language, "app_freeze")
        };
        if ui
            .add_enabled(!busy, egui::Button::new(freeze_label))
            .clicked()
            && let Some(target) = state.target.clone()
        {
            let action = if frozen {
                ApplicationAction::Unfreeze
            } else {
                ApplicationAction::Freeze
            };
            push_action(state, commands, &target, selected, action);
        }
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(
                    RichText::new(text(language, "app_uninstall")).color(palette.danger_stroke),
                ),
            )
            .clicked()
        {
            state.confirm = Some(ApplicationAction::Uninstall);
        }
    });
}

/// Confirmation modal for destructive actions.
fn confirmation_dialog(
    ui: &mut egui::Ui,
    language: Language,
    palette: &theme::Palette,
    state: &mut ApplicationsPanelState,
    commands: &mut Vec<BackendCommand>,
) {
    let Some(action) = state.confirm else {
        return;
    };
    let (title, body) = match action {
        ApplicationAction::ClearData => (
            text(language, "app_confirm_clear_title"),
            text(language, "app_confirm_clear_body"),
        ),
        ApplicationAction::Uninstall => (
            text(language, "app_confirm_uninstall_title"),
            text(language, "app_confirm_uninstall_body"),
        ),
        // Launch/ForceStop/Freeze/Unfreeze run without confirmation.
        _ => return,
    };
    let mut open = true;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ui.ctx(), |ui| {
            ui.label(body);
            if let Some(package) = &state.selected {
                ui.strong(package.as_str());
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let confirm = egui::Button::new(
                    RichText::new(text(language, "confirm"))
                        .strong()
                        .color(palette.danger_stroke),
                )
                .fill(palette.danger_fill)
                .corner_radius(8.0);
                if ui.add(confirm).clicked()
                    && let (Some(target), Some(package)) =
                        (state.target.clone(), state.selected.clone())
                {
                    push_action(state, commands, &target, &package, action);
                    state.confirm = None;
                }
                if ui.button(text(language, "cancel")).clicked() {
                    state.confirm = None;
                }
            });
        });
    if !open {
        state.confirm = None;
    }
}

fn push_action(
    state: &mut ApplicationsPanelState,
    commands: &mut Vec<BackendCommand>,
    target: &DeviceTarget,
    package: &PackageName,
    action: ApplicationAction,
) {
    let request_id = OperationId::new();
    commands.push(BackendCommand::RunApplicationAction {
        request_id,
        action,
        target: target.clone(),
        package: package.clone(),
    });
    state.pending = Some(PendingAction { request_id, action });
}

fn action_label(language: Language, action: ApplicationAction) -> &'static str {
    match action {
        ApplicationAction::Launch => text(language, "app_open"),
        ApplicationAction::ForceStop => text(language, "app_force_stop"),
        ApplicationAction::ClearData => text(language, "app_clear_data"),
        ApplicationAction::Freeze => text(language, "app_freeze"),
        ApplicationAction::Unfreeze => text(language, "app_unfreeze"),
        ApplicationAction::Uninstall => text(language, "app_uninstall"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fadb_domain::ApplicationSnapshot;

    fn empty_details(package: PackageName) -> ApplicationDetails {
        ApplicationDetails {
            package,
            version_name: None,
            version_code: None,
            min_sdk: None,
            target_sdk: None,
            first_install_time: None,
            last_update_time: None,
            installer: None,
            apk_path: None,
            permissions: Vec::new(),
        }
    }

    fn device_target() -> DeviceTarget {
        DeviceTarget::new(
            fadb_domain::DeviceSerial::new("emulator-5554").expect("valid serial"),
            1,
        )
    }

    fn record(package: &str) -> ApplicationRecord {
        ApplicationRecord {
            package: PackageName::new(package).expect("valid package"),
            system: false,
            disabled: false,
        }
    }

    fn snapshot(device: &DeviceTarget, apps: Vec<ApplicationRecord>) -> ApplicationSnapshot {
        ApplicationSnapshot {
            target: device.clone(),
            applications: apps,
        }
    }

    #[test]
    fn reloading_replaces_snapshot_and_drops_lost_selection() {
        let mut state = ApplicationsPanelState::default();
        let device = device_target();
        state.reset_for(Some(device.clone()));
        state.selected = Some(PackageName::new("com.example.gone").expect("valid package"));
        state.details = Some(empty_details(
            PackageName::new("com.example.gone").expect("valid package"),
        ));

        let loaded = snapshot(&device, vec![record("com.example.here")]);
        let commands = state.handle_event(&BackendEvent::ApplicationsLoaded(loaded));

        assert!(!state.loading);
        assert!(state.selected.is_none());
        assert!(state.details.is_none());
        assert!(commands.is_empty());
    }

    #[test]
    fn icon_requests_cover_visible_packages_exactly_once() {
        let mut state = ApplicationsPanelState::default();
        let device = device_target();
        state.reset_for(Some(device.clone()));
        let package = PackageName::new("com.example.app").expect("valid package");
        let system = PackageName::new("com.android.system").expect("valid package");
        state.handle_event(&BackendEvent::ApplicationsLoaded(snapshot(
            &device,
            vec![record("com.example.app"), {
                let mut app = record("com.android.system");
                app.system = true;
                app
            }],
        )));

        // Default third-party view: the system app is never requested.
        let mut commands = Vec::new();
        state.request_visible_icons(&mut commands);
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            BackendCommand::LoadApplicationIcons { packages, .. }
                if packages.len() == 1 && packages[0] == package
        ));

        // While the request is in flight, the per-frame pass is a no-op.
        let mut commands = Vec::new();
        state.request_visible_icons(&mut commands);
        assert!(commands.is_empty());

        // The icon lands, clearing the in-flight marker.
        state.handle_event(&BackendEvent::ApplicationIconLoaded {
            target: device.clone(),
            package: package.clone(),
            icon: ApplicationIconData {
                width: 48,
                height: 48,
                rgba: vec![0; 48 * 48 * 4],
            },
        });
        assert!(state.icons.contains_key(&package));
        let mut commands = Vec::new();
        state.request_visible_icons(&mut commands);
        assert!(commands.is_empty());

        // Switching to the system filter fetches only the system app.
        state.filter = ApplicationFilter::System;
        let mut commands = Vec::new();
        state.request_visible_icons(&mut commands);
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            BackendCommand::LoadApplicationIcons { packages, .. }
                if packages.len() == 1 && packages[0] == system
        ));
    }

    #[test]
    fn icons_from_other_targets_are_ignored() {
        let mut state = ApplicationsPanelState::default();
        state.reset_for(Some(device_target()));
        let package = PackageName::new("com.example.app").expect("valid package");

        let other = DeviceTarget::new(
            fadb_domain::DeviceSerial::new("other").expect("valid serial"),
            1,
        );
        state.handle_event(&BackendEvent::ApplicationIconLoaded {
            target: other,
            package: package.clone(),
            icon: ApplicationIconData {
                width: 1,
                height: 1,
                rgba: vec![0; 4],
            },
        });
        assert!(!state.icons.contains_key(&package));
    }

    #[test]
    fn completing_mutating_action_requests_a_reload() {
        let mut state = ApplicationsPanelState::default();
        let device = device_target();
        state.reset_for(Some(device.clone()));
        let request_id = OperationId::new();
        state.pending = Some(PendingAction {
            request_id,
            action: ApplicationAction::Uninstall,
        });

        let commands = state.handle_event(&BackendEvent::ApplicationActionCompleted {
            request_id,
            action: ApplicationAction::Uninstall,
            target: device,
            package: PackageName::new("com.example.app").expect("valid package"),
        });

        assert_eq!(commands.len(), 1);
        assert!(matches!(commands[0], BackendCommand::LoadApplications(_)));
        assert!(state.pending.is_none());
    }

    #[test]
    fn completing_launching_action_skips_the_reload() {
        let mut state = ApplicationsPanelState::default();
        let device = device_target();
        state.reset_for(Some(device.clone()));
        let request_id = OperationId::new();
        state.pending = Some(PendingAction {
            request_id,
            action: ApplicationAction::Launch,
        });

        let commands = state.handle_event(&BackendEvent::ApplicationActionCompleted {
            request_id,
            action: ApplicationAction::Launch,
            target: device,
            package: PackageName::new("com.example.app").expect("valid package"),
        });

        assert!(commands.is_empty());
        assert!(state.pending.is_none());
    }

    #[test]
    fn finishing_an_install_sets_the_notice_and_reloads() {
        let mut state = ApplicationsPanelState::default();
        let device = device_target();
        state.reset_for(Some(device.clone()));
        let request_id = OperationId::new();
        state.install = Some(request_id);

        let commands = state.handle_event(&BackendEvent::ApkInstallFinished {
            request_id,
            target: device,
        });

        assert_eq!(commands.len(), 1);
        assert!(matches!(commands[0], BackendCommand::LoadApplications(_)));
        assert!(state.install.is_none());
        assert!(state.install_notice);
        // The refreshed listing clears the notice again.
        state.handle_event(&BackendEvent::ApplicationsLoading(
            state.target.clone().expect("target"),
        ));
        let loaded = BackendEvent::ApplicationsLoaded(fadb_domain::ApplicationSnapshot {
            target: state.target.clone().expect("target"),
            applications: Vec::new(),
        });
        state.handle_event(&loaded);
        assert!(!state.install_notice);
    }

    #[test]
    fn failing_an_install_clears_the_pending_marker() {
        let mut state = ApplicationsPanelState::default();
        let device = device_target();
        state.reset_for(Some(device.clone()));
        let request_id = OperationId::new();
        state.install = Some(request_id);

        state.handle_event(&BackendEvent::ApkInstallFailed {
            request_id,
            target: device,
            error: fadb_domain::BridgeError::invalid_input("test"),
        });

        assert!(state.install.is_none());
        assert!(!state.install_notice);
    }

    #[test]
    fn events_for_other_requests_are_ignored() {
        let mut state = ApplicationsPanelState::default();
        let device = device_target();
        state.reset_for(Some(device.clone()));
        let request_id = OperationId::new();
        state.pending = Some(PendingAction {
            request_id,
            action: ApplicationAction::Launch,
        });

        let commands = state.handle_event(&BackendEvent::ApplicationActionFailed {
            request_id: OperationId::new(),
            action: ApplicationAction::Uninstall,
            target: device,
            package: PackageName::new("com.example.other").expect("valid package"),
            error: fadb_domain::BridgeError::invalid_input("test"),
        });

        assert!(commands.is_empty());
        assert!(state.pending.is_some(), "the tracked action stays pending");
    }

    #[test]
    fn reset_clears_transient_selection_state() {
        let mut state = ApplicationsPanelState::default();
        let device = device_target();
        state.reset_for(Some(device));
        state.selected = Some(PackageName::new("com.example.app").expect("valid package"));
        state.confirm = Some(ApplicationAction::Uninstall);
        state.icons.insert(
            PackageName::new("com.example.app").expect("valid package"),
            ApplicationIconData {
                width: 1,
                height: 1,
                rgba: vec![0; 4],
            },
        );
        state.reset_for(None);
        assert!(state.selected.is_none());
        assert!(state.confirm.is_none());
        assert!(state.applications.is_empty());
        assert!(state.icons.is_empty());
    }

    #[test]
    fn tile_naming_derives_initials_and_short_names() {
        assert_eq!(short_name("org.mozilla.firefox"), "firefox");
        assert_eq!(short_name("chrome"), "chrome");
        assert_eq!(tile_initial("org.mozilla.firefox"), "F");
        assert_eq!(tile_initial("com.fadb.demo"), "D");
        assert_eq!(elide("averyveryverylongname", 10), "averyvery…");
        assert_eq!(elide("short", 10), "short");
    }

    #[test]
    fn every_action_has_a_distinct_label() {
        let actions = [
            ApplicationAction::Launch,
            ApplicationAction::ForceStop,
            ApplicationAction::ClearData,
            ApplicationAction::Freeze,
            ApplicationAction::Unfreeze,
            ApplicationAction::Uninstall,
        ];
        for language in [Language::Chinese, Language::English] {
            let mut labels: Vec<&str> = actions
                .iter()
                .map(|action| action_label(language, *action))
                .collect();
            labels.sort_unstable();
            let total = labels.len();
            labels.dedup();
            assert_eq!(
                labels.len(),
                total,
                "labels must not collide in {language:?}"
            );
        }
    }
}
