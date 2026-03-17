//! GUI front-end — launched with `rustwave-cli -gui`.
//!
//! Drag any file onto the window:
//!   • WAV  → decoded, output saved with the ORIGINAL filename next to the binary
//!   • Other → encoded to `<stem>_encoded.wav` next to the binary

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, Vec2};

// ─── State machine ───────────────────────────────────────────────────────────

enum State {
    Idle,
    Processing {
        filename: String,
        action: &'static str,
        progress: Arc<AtomicU32>,
        rx: mpsc::Receiver<Result<PathBuf, String>>,
    },
    Done {
        action: &'static str,
        output: PathBuf,
    },
    Failed(String),
}

// ─── App ─────────────────────────────────────────────────────────────────────

pub struct AfskGui {
    state: State,
}

impl AfskGui {
    pub const fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self { state: State::Idle }
    }

    fn start_processing(&mut self, path: PathBuf, ctx: egui::Context) {
        let is_wav = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"));

        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let action: &'static str = if is_wav { "Decoding" } else { "Encoding" };
        let progress = Arc::new(AtomicU32::new(0));
        let (tx, rx) = mpsc::channel::<Result<PathBuf, String>>();

        let binary_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));

        {
            let progress = Arc::clone(&progress);
            let thread_filename = filename.clone();
            thread::spawn(move || {
                let prog = Arc::clone(&progress);
                let ctx2 = ctx.clone();
                let on_progress = move |v: f32| {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    prog.store((v.clamp(0.0, 1.0) * 1_000_000.0) as u32, Ordering::Relaxed);
                    ctx2.request_repaint_after(Duration::from_millis(16));
                };

                let outcome: Result<PathBuf, String> = if is_wav {
                    crate::wav::read(&path)
                        .and_then(|samples| crate::decoder::decode_progress(&samples, on_progress))
                        .and_then(|decoded| {
                            let out = binary_dir.join(&decoded.filename);
                            std::fs::write(&out, &decoded.data)
                                .map(|()| out)
                                .map_err(|e| e.to_string())
                        })
                } else {
                    std::fs::read(&path)
                        .map_err(|e| e.to_string())
                        .map(|data| {
                            let framed = crate::framer::frame(&data, &thread_filename);
                            crate::encoder::encode_progress(&framed, on_progress)
                        })
                        .and_then(|samples| {
                            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                            let out = binary_dir.join(format!("{stem}_encoded.wav"));
                            crate::wav::write(&out, &samples).map(|()| out)
                        })
                };

                progress.store(1_000_000, Ordering::Relaxed);
                let _ = tx.send(outcome);
                ctx.request_repaint();
            });
        }

        self.state = State::Processing {
            filename,
            action,
            progress,
            rx,
        };
    }

    /// Poll the worker channel and advance state if the worker has finished.
    fn poll_worker(&mut self) {
        let finished: Option<(&'static str, Result<PathBuf, String>)> =
            if let State::Processing { rx, action, .. } = &self.state {
                rx.try_recv().ok().map(|result| (*action, result))
            } else {
                None
            };

        if let Some((action, result)) = finished {
            self.state = match result {
                Ok(output) => State::Done { action, output },
                Err(e) => State::Failed(e),
            };
        }
    }

    /// Draw the drop zone and its current contents.
    fn draw_zone(
        &self,
        ui: &mut egui::Ui,
        zone_size: Vec2,
        hovering: bool,
        accent: Color32,
        bg_panel: Color32,
        dim_text: Color32,
    ) {
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(zone_size, egui::Sense::hover());

            let fill = if hovering {
                Color32::from_rgba_premultiplied(100, 145, 235, 15)
            } else {
                bg_panel
            };
            let border = if hovering {
                accent
            } else {
                Color32::from_rgb(50, 55, 75)
            };

            ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
            dashed_border(ui.painter(), rect, Stroke::new(1.5, border));

            match &self.state {
                State::Idle => draw_idle(ui.painter(), rect, hovering, accent, dim_text),
                State::Processing {
                    filename,
                    action,
                    progress,
                    ..
                } => {
                    #[allow(clippy::cast_precision_loss)]
                    // u32 progress value; precision loss is fine
                    let v = progress.load(Ordering::Relaxed) as f32 / 1_000_000.0;
                    draw_processing(ui.painter(), rect, action, filename, v, accent, dim_text);
                }
                State::Done { action, output } => {
                    draw_result(
                        ui.painter(),
                        rect,
                        true,
                        &format!("{action} complete"),
                        &output.to_string_lossy(),
                        accent,
                        dim_text,
                    );
                }
                State::Failed(err) => {
                    draw_result(
                        ui.painter(),
                        rect,
                        false,
                        "Error",
                        err,
                        Color32::from_rgb(220, 85, 85),
                        dim_text,
                    );
                }
            }
        });
    }
}

// ─── eframe::App ─────────────────────────────────────────────────────────────

impl eframe::App for AfskGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();

        // Extract only the path to avoid cloning the entire Vec<DroppedFile> every frame.
        let dropped_path: Option<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
        if let Some(p) = dropped_path {
            if !matches!(self.state, State::Processing { .. }) {
                self.start_processing(p, ctx.clone());
            }
        }

        if matches!(self.state, State::Processing { .. }) {
            ctx.request_repaint_after(Duration::from_millis(16));
        }

        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let bg_dark = Color32::from_rgb(15, 15, 20);
        let bg_panel = Color32::from_rgb(22, 22, 30);
        let accent = Color32::from_rgb(100, 145, 235);
        let dim_text = Color32::from_rgb(100, 105, 130);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(bg_dark))
            .show(ctx, |ui| {
                let avail = ui.available_size();
                ui.add_space(18.0);
                ui.vertical_centered(|ui| {
                    #[allow(clippy::arithmetic_side_effects)]
                    // egui Vec2 addition; no panic risk
                    ui.painter().text(
                        ui.next_widget_position() + Vec2::new(avail.x / 2.0, 0.0),
                        egui::Align2::CENTER_TOP,
                        "RustWave",
                        FontId::proportional(20.0),
                        Color32::from_rgb(170, 175, 200),
                    );
                    ui.add_space(24.0);
                });

                let zone_size = Vec2::new((avail.x - 40.0).max(240.0), (avail.y - 90.0).max(160.0));
                self.draw_zone(ui, zone_size, hovering, accent, bg_panel, dim_text);
            });
    }
}

// ─── Drawing helpers ─────────────────────────────────────────────────────────

fn draw_idle(painter: &egui::Painter, zone: Rect, hovering: bool, accent: Color32, dim: Color32) {
    let cx = zone.center().x;
    let cy = zone.center().y;

    let heading = if hovering {
        "Release to process"
    } else {
        "Drop a file here"
    };
    let heading_color = if hovering {
        accent
    } else {
        Color32::from_rgb(210, 215, 230)
    };

    #[allow(clippy::arithmetic_side_effects)]
    {
        painter.text(
            Pos2::new(cx, cy - 28.0),
            egui::Align2::CENTER_CENTER,
            heading,
            FontId::proportional(18.0),
            heading_color,
        );
        painter.text(
            Pos2::new(cx, cy + 8.0),
            egui::Align2::CENTER_CENTER,
            "WAV  →  restores original file + extension",
            FontId::proportional(12.5),
            dim,
        );
        painter.text(
            Pos2::new(cx, cy + 25.0),
            egui::Align2::CENTER_CENTER,
            "Other  →  encode to .wav",
            FontId::proportional(12.5),
            dim,
        );
        painter.text(
            Pos2::new(cx, zone.max.y - 18.0),
            egui::Align2::CENTER_CENTER,
            "Output saved next to the binary",
            FontId::proportional(11.0),
            Color32::from_rgb(60, 65, 85),
        );
    }
}

#[allow(clippy::arithmetic_side_effects)]
fn draw_processing(
    painter: &egui::Painter,
    zone: Rect,
    action: &str,
    filename: &str,
    progress: f32,
    accent: Color32,
    dim: Color32,
) {
    let cx = zone.center().x;
    let cy = zone.center().y;

    painter.text(
        Pos2::new(cx, cy - 40.0),
        egui::Align2::CENTER_CENTER,
        action,
        FontId::proportional(15.0),
        accent,
    );
    painter.text(
        Pos2::new(cx, cy - 20.0),
        egui::Align2::CENTER_CENTER,
        filename,
        FontId::proportional(12.5),
        Color32::from_rgb(180, 185, 205),
    );

    let bar_w = (zone.width() - 70.0).max(80.0);
    let bar_rect = Rect::from_center_size(Pos2::new(cx, cy + 8.0), Vec2::new(bar_w, 10.0));
    painter.rect_filled(
        bar_rect,
        CornerRadius::same(5),
        Color32::from_rgb(35, 37, 52),
    );

    let filled_w = (bar_rect.width() * progress.clamp(0.0, 1.0)).max(0.0);
    if filled_w >= 1.0 {
        let filled = Rect::from_min_size(bar_rect.min, Vec2::new(filled_w, bar_rect.height()));
        painter.rect_filled(filled, CornerRadius::same(5), accent);
    }

    painter.text(
        Pos2::new(cx, cy + 28.0),
        egui::Align2::CENTER_CENTER,
        format!("{:.0}%", progress * 100.0),
        FontId::proportional(12.5),
        dim,
    );
}

#[allow(clippy::arithmetic_side_effects)]
fn draw_result(
    painter: &egui::Painter,
    zone: Rect,
    success: bool,
    label: &str,
    detail: &str,
    color: Color32,
    dim: Color32,
) {
    let cx = zone.center().x;
    let cy = zone.center().y;

    painter.text(
        Pos2::new(cx, cy - 32.0),
        egui::Align2::CENTER_CENTER,
        if success { "✓" } else { "✗" },
        FontId::proportional(30.0),
        color,
    );
    painter.text(
        Pos2::new(cx, cy + 6.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(15.0),
        color,
    );

    // Truncate to last ~max_chars characters, safely respecting UTF-8 boundaries.
    let max_chars = 55usize;
    let char_count = detail.chars().count();
    let display = if char_count > max_chars {
        let skip = char_count - (max_chars - 1);
        let byte_offset = detail.char_indices().nth(skip).map_or(0, |(i, _)| i);
        format!("…{}", &detail[byte_offset..])
    } else {
        detail.to_owned()
    };
    painter.text(
        Pos2::new(cx, cy + 28.0),
        egui::Align2::CENTER_CENTER,
        display,
        FontId::monospace(11.0),
        Color32::from_rgb(155, 160, 185),
    );

    painter.text(
        Pos2::new(cx, zone.max.y - 18.0),
        egui::Align2::CENTER_CENTER,
        "Drop another file to continue",
        FontId::proportional(11.0),
        dim,
    );
}

#[allow(
    clippy::arithmetic_side_effects,  // float/Vec2 arithmetic in egui types; no panic risk
    clippy::cast_possible_truncation, // f32.ceil() → usize: always positive
    clippy::cast_sign_loss,           // f32.ceil() → usize: always positive
    clippy::cast_precision_loss,      // usize i → f32: acceptable for pixel coordinates
)]
fn dashed_border(painter: &egui::Painter, rect: Rect, stroke: Stroke) {
    let dash = 8.0_f32;
    let gap = 5.0_f32;
    let step = dash + gap;
    let r = 10.0_f32;

    let seg = |from: Pos2, to: Pos2| {
        let delta = to - from;
        let len = delta.length();
        if len < 1.0 {
            return;
        }
        let dir = delta / len;
        let steps = (len / step).ceil() as usize;
        for i in 0..steps {
            let t = i as f32 * step;
            let a = from + dir * t;
            let b = from + dir * (t + dash).min(len);
            painter.line_segment([a, b], stroke);
        }
    };

    seg(
        Pos2::new(rect.min.x + r, rect.min.y),
        Pos2::new(rect.max.x - r, rect.min.y),
    );
    seg(
        Pos2::new(rect.min.x + r, rect.max.y),
        Pos2::new(rect.max.x - r, rect.max.y),
    );
    seg(
        Pos2::new(rect.min.x, rect.min.y + r),
        Pos2::new(rect.min.x, rect.max.y - r),
    );
    seg(
        Pos2::new(rect.max.x, rect.min.y + r),
        Pos2::new(rect.max.x, rect.max.y - r),
    );
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run() -> eframe::Result<()> {
    // Spawn the /broadcast/* API server on a background OS thread.
    std::thread::spawn(|| {
        let addr: std::net::SocketAddr = std::env::var("RUSTWAVE_BIND")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], 7071)));

        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("failed to build Tokio runtime for GUI API server: {e}");
                return;
            }
        };

        rt.block_on(async move {
            let state = crate::api::state::AppState::new(false);
            let router = crate::api::gui_router(state);
            if let Err(e) = crate::api::run_server(router, addr).await {
                tracing::error!("GUI API server error: {e}");
            }
        });
    });

    tracing::info!("GUI mode: /broadcast/* API started on 127.0.0.1:7071");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 340.0])
            .with_min_inner_size([360.0, 260.0])
            .with_title("RustWave")
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "RustWave",
        options,
        Box::new(|cc| Ok(Box::new(AfskGui::new(cc)) as Box<dyn eframe::App>)),
    )
}
