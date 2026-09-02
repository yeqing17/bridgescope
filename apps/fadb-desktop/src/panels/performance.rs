use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{self, Color32, RichText, Stroke};
use egui_plot::{AxisHints, GridMark, Line, Plot, PlotPoint};
use fadb_domain::{BackendCommand, BackendEvent, DeviceTarget, PerformanceSnapshot};

use crate::i18n::{Language, text};

/// The chart's fixed x window, one minute, oldest sample on the left.
const WINDOW_SECONDS: f64 = 60.0;
/// Cap on retained samples: two per second across the one-minute window.
const MAX_SAMPLES: usize = 120;

#[derive(Clone, Copy, Debug)]
struct Sample {
    /// When the sample arrived; the chart's x position is its age.
    at: Instant,
    cpu: Option<f32>,
    memory_used_mib: Option<f32>,
}

#[derive(Default)]
pub struct PerformancePanelState {
    pub target: Option<DeviceTarget>,
    pub latest: Option<PerformanceSnapshot>,
    pub loading: bool,
    /// Whether continuous sampling is paused; `刷新` still takes one-shot
    /// samples while paused.
    pub paused: bool,
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
                    at: Instant::now(),
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
        // Sampling toggle instead of a spinner: the busy state is visible in
        // the chart itself (the line grows), so a spinner only added noise.
        let toggle_label = text(language, if state.paused { "start" } else { "stop" });
        if ui
            .add_enabled(state.target.is_some(), egui::Button::new(toggle_label))
            .clicked()
        {
            state.paused = !state.paused;
        }
        // The button stays enabled while an auto sample is in flight: with
        // sampling at up to 2 Hz, gating `enabled` on `loading` made it
        // flicker twice a second. A click during a load is simply ignored.
        if ui
            .add_enabled(
                state.target.is_some(),
                egui::Button::new(text(language, "refresh")),
            )
            .clicked()
            && !state.loading
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

    // The x position of a sample is its real age in seconds: the newest sits
    // at the right edge and the line flows leftward as samples age, so a
    // sparse start shows a short line at the right, never a stretched one.
    let now = Instant::now();
    let in_window = |sample: &Sample| now.duration_since(sample.at).as_secs_f64() <= WINDOW_SECONDS;
    let cpu_values: Vec<(f64, Option<f32>)> = state
        .samples
        .iter()
        .filter(|sample| in_window(sample))
        .map(|sample| (-(now.duration_since(sample.at).as_secs_f64()), sample.cpu))
        .collect();
    let memory_values: Vec<(f64, Option<f32>)> = state
        .samples
        .iter()
        .filter(|sample| in_window(sample))
        .map(|sample| {
            (
                -(now.duration_since(sample.at).as_secs_f64()),
                sample.memory_used_mib,
            )
        })
        .collect();

    let cpu_color = Color32::from_rgb(248, 113, 113);
    let memory_color = Color32::from_rgb(96, 165, 250);

    chart_title(
        ui,
        text(language, "cpu"),
        format_percent(metrics.cpu_usage_percent),
        cpu_color,
    );
    area_chart(
        ui,
        "cpu-plot",
        cpu_color,
        &cpu_values,
        100.0,
        150.0,
        &SeriesFormat {
            unit: "%",
            value_decimals: 1,
        },
    );

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
        "memory-plot",
        memory_color,
        &memory_values,
        f64::from(memory_total_mib.max(1.0)),
        150.0,
        &SeriesFormat {
            unit: " MB",
            value_decimals: 0,
        },
    );
    commands
}

/// How a series' numbers render on the axis and in the hover readout.
struct SeriesFormat {
    /// Appended to every number: `%` or ` MB`.
    unit: &'static str,
    /// Digits after the decimal point in the hover readout; axis ticks are
    /// always whole numbers.
    value_decimals: usize,
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

/// An area chart over the last-minute sample window, drawn with
/// `egui_plot`: a vertical gradient fill fading toward the floor, a solid
/// series line, labeled axes and a crosshair that reads out the sample
/// under the pointer. Fixed viewport — no zoom or pan, it is a monitor,
/// not an explorer.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn area_chart(
    ui: &mut egui::Ui,
    id: &str,
    color: Color32,
    values: &[(f64, Option<f32>)],
    y_max: f64,
    height: f32,
    format: &SeriesFormat,
) {
    // Contiguous runs of present values, so gaps (samples the backend never
    // delivered) break the line instead of dropping to the floor. Points are
    // `(age_seconds, value)`; ages are negative, newest at the right edge.
    let mut run: Vec<[f64; 2]> = Vec::new();
    let mut runs: Vec<Vec<[f64; 2]>> = Vec::new();
    for (x, value) in values.iter().copied() {
        if let Some(value) = value {
            run.push([x, f64::from(value)]);
        } else if !run.is_empty() {
            runs.push(std::mem::take(&mut run));
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }

    let rgb = [color.r(), color.g(), color.b()];
    Plot::new(egui::Id::new(id))
        .height(height)
        .width(ui.available_width().max(360.0))
        .show_background(false)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .include_x(-WINDOW_SECONDS)
        .include_x(0.0)
        .include_y(0.0)
        .include_y(y_max)
        // Custom y axis: egui_plot fades tick labels whose pixel spacing is
        // under the axis' `label_spacing` minimum, which on these short
        // charts rendered every middle label nearly invisible. A single
        // "nice" step (see `nice_marks`) plus wide-open label spacing keeps
        // every label at full text color.
        .custom_y_axes(vec![AxisHints::new_y()
            .formatter(move |mark, _range| format!("{:.0}{}", mark.value, format.unit))
            .label_spacing(egui::Rangef::new(1.0, 5.0))
            .min_thickness(44.0)])
        .y_grid_spacer(move |input| nice_marks(input.bounds.0, input.bounds.1))
        .x_axis_formatter(|mark, _range| format!("{}s", mark.value.round() as i64))
        .label_formatter(move |_name, point: &PlotPoint| {
            format!(
                "{:.*}{} · {}s",
                format.value_decimals,
                point.y,
                format.unit,
                point.x.round() as i64
            )
        })
        .show(ui, |plot_ui| {
            for run in &runs {
                // Fill alpha scales with height (the gradient callback's alpha
                // is multiplied by `fill_alpha`), so the callback emits an
                // opaque color at the top of the scale and nothing at the
                // floor; the stroke is a separate, solid line because a
                // gradient callback would otherwise recolor it too.
                let gradient = Arc::new(move |point: PlotPoint| {
                    let t = (point.y / y_max).clamp(0.0, 1.0) as f32;
                    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (t * 255.0) as u8)
                });
                let area = Line::new("", run.clone())
                    .stroke(Stroke::NONE)
                    .fill(0.0)
                    .fill_alpha(0.45)
                    .gradient_color(gradient, true);
                plot_ui.line(area);
                plot_ui.line(Line::new("", run.clone()).color(color).width(2.0));
            }
        });
}

/// Axis marks at one "nice" step (1/2/5 × 10ⁿ, aiming for ~5 ticks), all
/// carrying the same `step_size` so egui_plot renders them — labels and
/// gridlines alike — at a uniform strength.
#[allow(
    clippy::cast_possible_truncation, // tick indices are small whole numbers
    clippy::cast_precision_loss // f64 mantissa covers these tiny integers
)]
fn nice_marks(min: f64, max: f64) -> Vec<GridMark> {
    let span = max - min;
    let raw = span / 4.0;
    let scale = 10f64.powf(raw.log10().floor());
    let multiple = raw / scale;
    let step = scale
        * if multiple < 1.5 {
            1.0
        } else if multiple < 3.5 {
            2.0
        } else if multiple < 7.5 {
            5.0
        } else {
            10.0
        };
    let first = (min / step).ceil() as i64;
    let last = (max / step).floor() as i64;
    (first..=last)
        .map(|index| GridMark {
            value: index as f64 * step,
            step_size: step,
        })
        .collect()
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
