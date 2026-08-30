use std::collections::VecDeque;

use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use fadb_domain::{BackendCommand, BackendEvent, DeviceTarget, PerformanceSnapshot};

use crate::i18n::{Language, text};

const MAX_SAMPLES: usize = 60;

#[derive(Clone, Copy, Debug)]
struct Sample {
    cpu: Option<f32>,
    memory: Option<f32>,
    battery: Option<f32>,
}

#[derive(Default)]
pub struct PerformancePanelState {
    pub target: Option<DeviceTarget>,
    pub latest: Option<PerformanceSnapshot>,
    pub loading: bool,
    samples: VecDeque<Sample>,
}

impl PerformancePanelState {
    pub fn reset_for(&mut self, target: Option<DeviceTarget>) {
        if self.target != target {
            self.target = target;
            self.latest = None;
            self.samples.clear();
        }
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::PerformanceLoading(target) if self.target.as_ref() == Some(target) => {
                self.loading = true;
            }
            BackendEvent::PerformanceLoaded(snapshot)
                if self.target.as_ref() == Some(&snapshot.target) =>
            {
                self.loading = false;
                self.latest = Some(snapshot.clone());
                let metrics = &snapshot.metrics;
                let total = metrics.memory_total_kib.unwrap_or_default();
                let available = metrics.memory_available_kib.unwrap_or_default();
                let memory = (total > 0).then(|| {
                    ((total.saturating_sub(available) as f64 / total as f64) * 100.0) as f32
                });
                self.samples.push_back(Sample {
                    cpu: metrics.cpu_usage_percent,
                    memory,
                    battery: metrics.battery_percent.map(f32::from),
                });
                while self.samples.len() > MAX_SAMPLES {
                    self.samples.pop_front();
                }
            }
            BackendEvent::PerformanceFailed { target, .. }
                if self.target.as_ref() == Some(target) =>
            {
                self.loading = false;
            }
            _ => {}
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut PerformancePanelState,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    ui.heading(text(language, "performance"));
    ui.horizontal(|ui| {
        ui.label(text(language, "performance_live_hint"));
        if state.loading {
            ui.spinner();
        }
        if ui
            .add_enabled(
                state.target.is_some() && !state.loading,
                egui::Button::new(text(language, "refresh")),
            )
            .clicked()
            && let Some(target) = state.target.clone()
        {
            state.loading = true;
            commands.push(BackendCommand::LoadPerformance(target));
        }
    });
    ui.add_space(8.0);

    let Some(latest) = &state.latest else {
        ui.label(if state.target.is_some() {
            text(language, "performance_waiting")
        } else {
            text(language, "select_device")
        });
        return commands;
    };
    let metrics = &latest.metrics;
    egui::Grid::new("performance-summary")
        .num_columns(4)
        .spacing([22.0, 10.0])
        .striped(true)
        .show(ui, |ui| {
            metric(
                ui,
                text(language, "cpu"),
                format_percent(metrics.cpu_usage_percent),
            );
            metric(ui, text(language, "memory"), format_memory(metrics));
            metric(
                ui,
                text(language, "load_1m"),
                format_value(metrics.load_average_1m),
            );
            metric(
                ui,
                text(language, "battery"),
                format_percent(metrics.battery_percent.map(f32::from)),
            );
        });
    ui.add_space(14.0);
    ui.strong(text(language, "performance_history"));
    draw_chart(ui, language, &state.samples);
    commands
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    ui.strong(label);
    ui.label(value);
}

#[allow(clippy::cast_precision_loss)]
fn draw_chart(ui: &mut egui::Ui, language: Language, samples: &VecDeque<Sample>) {
    let desired = Vec2::new(ui.available_width().max(360.0), 220.0);
    let (response, painter) = ui.allocate_painter(desired, egui::Sense::hover());
    let rect = response.rect;
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, Color32::from_gray(75)),
        egui::StrokeKind::Inside,
    );
    for step in 0..=4 {
        let y = rect.top() + rect.height() * step as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(45)),
        );
        painter.text(
            Pos2::new(rect.left() + 6.0, y + 2.0),
            egui::Align2::LEFT_TOP,
            format!("{}%", 100 - step * 25),
            egui::FontId::monospace(11.0),
            Color32::from_gray(150),
        );
    }
    let series = [
        (text(language, "cpu"), Color32::from_rgb(248, 113, 113), 0),
        (text(language, "memory"), Color32::from_rgb(96, 165, 250), 1),
        (
            text(language, "battery"),
            Color32::from_rgb(74, 222, 128),
            3,
        ),
    ];
    for (label, color, index) in series {
        let points = samples
            .iter()
            .enumerate()
            .filter_map(|(position, sample)| {
                let value = match index {
                    0 => sample.cpu,
                    1 => sample.memory,
                    _ => sample.battery,
                }?;
                let x = if samples.len() <= 1 {
                    rect.left()
                } else {
                    rect.left() + rect.width() * position as f32 / (samples.len() - 1) as f32
                };
                Some(Pos2::new(
                    x,
                    rect.bottom() - rect.height() * (value / 100.0).clamp(0.0, 1.0),
                ))
            })
            .collect::<Vec<_>>();
        for pair in points.windows(2) {
            painter.line_segment([pair[0], pair[1]], Stroke::new(2.0, color));
        }
        painter.text(
            Pos2::new(
                rect.right() - 8.0 - label.len() as f32 * 7.0,
                rect.top() + 8.0 + index as f32 * 16.0,
            ),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(12.0),
            color,
        );
    }
}

fn format_percent(value: Option<f32>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| format!("{value:.1}%"))
}

fn format_value(value: Option<f32>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| format!("{value:.2}"))
}

fn format_memory(metrics: &fadb_domain::PerformanceMetrics) -> String {
    let (Some(total), Some(available)) = (metrics.memory_total_kib, metrics.memory_available_kib)
    else {
        return "-".to_owned();
    };
    format!(
        "{} / {} MiB",
        total.saturating_sub(available) / 1024,
        total / 1024
    )
}
