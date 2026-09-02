use std::collections::VecDeque;

use eframe::egui::{self, Color32, Pos2, RichText, Stroke, Vec2};
use fadb_domain::{BackendCommand, BackendEvent, DeviceTarget, PerformanceSnapshot};

use crate::i18n::{Language, text};

const MAX_SAMPLES: usize = 60;
/// One sample per second, so the 60-sample window covers the last minute and
/// the chart's time labels can be hard-coded.
const SAMPLE_WINDOW_LABELS: [(&str, f32); 3] = [("-60s", 0.0), ("-30s", 0.5), ("0s", 1.0)];

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    cpu: Option<f32>,
    memory_used_mib: Option<f32>,
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
                let memory_used_mib = metrics
                    .memory_total_kib
                    .zip(metrics.memory_available_kib)
                    .map(|(total, available)| (total.saturating_sub(available)) as f32 / 1024.0);
                self.samples.push_back(Sample {
                    cpu: metrics.cpu_usage_percent,
                    memory_used_mib,
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

#[allow(clippy::too_many_lines)]
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

    let samples: Vec<Sample> = state.samples.iter().copied().collect();
    let cpu_values: Vec<Option<f32>> = samples.iter().map(|sample| sample.cpu).collect();
    let memory_values: Vec<Option<f32>> = samples
        .iter()
        .map(|sample| sample.memory_used_mib)
        .collect();

    let cpu_color = Color32::from_rgb(248, 113, 113);
    let memory_color = Color32::from_rgb(96, 165, 250);

    chart_title(
        ui,
        text(language, "cpu"),
        format_percent(metrics.cpu_usage_percent),
        cpu_color,
    );
    area_chart(ui, cpu_color, &cpu_values, 100.0, "100%", "50%", 140.0);

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let memory_total_mib = latest
        .metrics
        .memory_total_kib
        .map_or(100.0, |kib| kib as f32 / 1024.0);
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let memory_used_mib = latest
        .metrics
        .memory_total_kib
        .zip(latest.metrics.memory_available_kib)
        .map(|(total, available)| (total.saturating_sub(available)) as f32 / 1024.0);
    let memory_label = memory_used_mib.map_or_else(
        || "-".to_owned(),
        |used| format!("{:.0}% · {:.0} MB", used / memory_total_mib * 100.0, used),
    );
    chart_title(ui, text(language, "memory"), memory_label, memory_color);
    area_chart(
        ui,
        memory_color,
        &memory_values,
        memory_total_mib.max(1.0),
        &format!("{memory_total_mib:.0} MB"),
        &format!("{:.0} MB", memory_total_mib / 2.0),
        140.0,
    );
    commands
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    ui.strong(label);
    ui.label(value);
}

/// The chart header: metric name on the left, current value (in the series
/// color) on the right, mirroring the summary grid above.
fn chart_title(ui: &mut egui::Ui, name: &str, value: String, color: Color32) {
    ui.horizontal(|ui| {
        ui.strong(RichText::new(name).size(14.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).strong().color(color));
        });
    });
}

/// An AYA-style area chart: filled series over a 0..`y_max` scale, labeled
/// gridlines, and time labels along the top edge (one sample per second).
#[allow(clippy::cast_precision_loss)]
fn area_chart(
    ui: &mut egui::Ui,
    color: Color32,
    values: &[Option<f32>],
    y_max: f32,
    y_top_label: &str,
    y_mid_label: &str,
    height: f32,
) {
    let desired = Vec2::new(ui.available_width().max(360.0), height);
    let (response, painter) = ui.allocate_painter(desired, egui::Sense::hover());
    let rect = response.rect;
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, Color32::from_gray(75)),
        egui::StrokeKind::Inside,
    );
    for (fraction, label) in [
        (0.0, Some(y_top_label)),
        (0.5, Some(y_mid_label)),
        (1.0, None),
    ] {
        let y = rect.top() + rect.height() * fraction;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(45)),
        );
        if let Some(label) = label {
            painter.text(
                Pos2::new(rect.left() + 6.0, y + 2.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::monospace(11.0),
                Color32::from_gray(150),
            );
        }
    }
    for (label, fraction) in SAMPLE_WINDOW_LABELS {
        let y = rect.top() + 16.0;
        let x = rect.left() + 6.0 + (rect.width() - 12.0) * fraction;
        painter.text(
            Pos2::new(x, y),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::monospace(11.0),
            Color32::from_gray(150),
        );
    }

    // Contiguous runs of present values, each rendered as a filled polygon
    // (triangle mesh down to the chart floor) plus an anti-aliased outline.
    let mut run: Vec<Pos2> = Vec::new();
    let mut runs: Vec<Vec<Pos2>> = Vec::new();
    for (index, value) in values.iter().copied().enumerate() {
        let Some(value) = value else {
            if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
            continue;
        };
        let x = if values.len() <= 1 {
            rect.left()
        } else {
            rect.left() + rect.width() * index as f32 / (values.len() - 1) as f32
        };
        let y = rect.bottom() - rect.height() * (value / y_max).clamp(0.0, 1.0);
        run.push(egui::pos2(x, y));
    }
    if !run.is_empty() {
        runs.push(run);
    }

    // Straight-alpha translucent fill over a dark background; premultiplied
    // colors render washed-out near-white here.
    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 45);
    for run in &runs {
        if run.len() < 2 {
            continue;
        }
        let mut mesh = egui::Mesh::default();
        // Vertices: the series points first, then one floor point under each,
        // so segment `i` spans vertices i, i+1, run_len+i, run_len+i+1.
        for point in run {
            mesh.colored_vertex(*point, fill);
        }
        for point in run {
            mesh.colored_vertex(egui::pos2(point.x, rect.bottom()), fill);
        }
        let line_count = u32::try_from(run.len()).expect("a chart run never exceeds u32 vertices");
        for index in 0..line_count - 1 {
            let left = index;
            let right = index + 1;
            let left_floor = line_count + index;
            let right_floor = line_count + right;
            mesh.add_triangle(left, right, right_floor);
            mesh.add_triangle(left, right_floor, left_floor);
        }
        painter.add(egui::Shape::mesh(mesh));
        painter.add(egui::Shape::line(run.clone(), Stroke::new(2.0, color)));
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
