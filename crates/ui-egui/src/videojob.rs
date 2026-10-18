//! A video export running on a worker thread: the editor stays usable, the Export page shows the
//! frame count, and Cancel stops ffmpeg and removes the partial file.
//!
//! The job owns its copy of the presentation, so editing while it runs never changes the frames
//! already planned: the video is the deck as it was when Create Video was pressed, as PowerPoint's
//! export is. Rendering and encoding are the engine's [`deckcraft_engine::video_export`], not a
//! second implementation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use deckcraft_engine::video_export::{self, VideoOptions};
use deckcraft_model::Presentation;

/// What the worker last reported.
#[derive(Clone, Debug)]
pub enum State {
    Running { frame: u64, frames: u64 },
    Done(Box<Finished>),
    Failed(String),
    Cancelled,
}

/// A finished video, as the window reports it.
#[derive(Clone, Debug)]
pub struct Finished {
    pub path: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames: u64,
    pub duration: f64,
    pub audio_warnings: Vec<String>,
}

impl Finished {
    pub fn summary(&self) -> String {
        let seconds = self.duration.max(0.0);
        let minutes = (seconds / 60.0).floor() as u64;
        let rest = seconds - minutes as f64 * 60.0;
        let length = if minutes > 0 { format!("{minutes} min {rest:.0} s") } else { format!("{rest:.1} s") };
        format!("Wrote {} ({}×{}, {} fps, {length})", self.path, self.width, self.height, self.fps)
    }
}

/// A running (or finished) export.
pub struct Job {
    pub path: String,
    pub started: f64,
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
    /// The result has been put in the status bar; don't repeat it every frame.
    announced: bool,
}

impl Job {
    /// Start encoding `doc` to `path` in the background. `started` is egui's clock (seconds).
    pub fn start(doc: Arc<Presentation>, opts: VideoOptions, path: String, started: f64) -> Job {
        let state = Arc::new(Mutex::new(State::Running { frame: 0, frames: 0 }));
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job { path: path.clone(), started, state: state.clone(), cancel: cancel.clone(), announced: false };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let worker_state = state;
            let worker_cancel = cancel;
            std::thread::spawn(move || {
                let mut last_frame = 0;
                let result = video_export::encode_to(&doc, &opts, &path, &worker_cancel, &mut |p| {
                    // One lock per frame is cheap next to rendering it; the window reads the same
                    // state once a frame and never blocks on a long operation.
                    if p.frame != last_frame {
                        last_frame = p.frame;
                        if let Ok(mut s) = worker_state.lock() {
                            *s = State::Running { frame: p.frame, frames: p.frames };
                        }
                    }
                });
                let next = match result {
                    _ if worker_cancel.load(Ordering::Relaxed) => State::Cancelled,
                    Ok(out) => State::Done(Box::new(Finished {
                        path: out.path,
                        bytes: out.bytes,
                        width: out.width,
                        height: out.height,
                        fps: out.fps,
                        frames: out.frames,
                        duration: out.duration,
                        audio_warnings: out.audio_warnings,
                    })),
                    Err(e) => State::Failed(e.to_string()),
                };
                if let Ok(mut s) = worker_state.lock() {
                    *s = next;
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let Ok(mut s) = state.lock() {
                *s = State::Failed("Video export needs the desktop app: a browser can't run ffmpeg.".into());
            }
        }
        job
    }

    pub fn state(&self) -> State {
        match self.state.lock() {
            Ok(s) => s.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Stop at the next frame and remove the partial file.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state(), State::Running { .. })
    }
}

/// The progress window. Running: a bar, the frame count, Cancel. Finished: the result and Close.
/// Returns true when the window should stay open (the caller keeps the job).
pub fn show(app: &mut crate::SlideApp, ctx: &egui::Context) -> bool {
    let Some(job) = app.video_job.as_mut() else { return false };
    let state = job.state();
    let mut keep = true;
    match state {
        State::Running { frame, frames } => {
            let t = crate::theme::Tokens::get(ctx);
            let mut cancel = false;
            egui::Window::new("Creating video").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(
                ctx,
                |ui| {
                    ui.set_min_width(360.0);
                    ui.label(egui::RichText::new(&job.path).font(crate::theme::font(12.5)).color(t.text_dim));
                    ui.add_space(8.0);
                    let fraction = if frames == 0 { 0.0 } else { (frame as f32 / frames as f32).clamp(0.0, 1.0) };
                    ui.add(egui::ProgressBar::new(fraction).show_percentage().desired_width(340.0));
                    ui.label(if frames == 0 { "Preparing the first frame…".to_string() } else { format!("Frame {frame} of {frames}") });
                    ui.add_space(8.0);
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                },
            );
            if cancel {
                job.cancel();
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        State::Done(done) => {
            if !job.announced {
                job.announced = true;
                app.set_status(done.summary());
                for warning in &done.audio_warnings {
                    app.notify(warning.clone());
                }
            }
            let mut close = false;
            egui::Window::new("Video ready").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(
                ctx,
                |ui| {
                    ui.set_min_width(360.0);
                    ui.label(done.summary());
                    if !done.audio_warnings.is_empty() {
                        ui.add_space(6.0);
                        for warning in &done.audio_warnings {
                            ui.label(egui::RichText::new(warning).font(crate::theme::font(11.5)));
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Show in folder").clicked() {
                            app.open_url(&format!("file:///{}", done.path.replace('\\', "/").trim_start_matches('/')));
                        }
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                },
            );
            if close {
                keep = false;
            }
        }
        State::Failed(error) => {
            if !job.announced {
                job.announced = true;
                app.set_status(error.clone());
            }
            let mut close = false;
            egui::Window::new("Video export failed").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(
                ctx,
                |ui| {
                    ui.set_min_width(360.0);
                    ui.label(error);
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                },
            );
            if close {
                keep = false;
            }
        }
        State::Cancelled => {
            if !job.announced {
                job.announced = true;
                app.set_status("Video export cancelled");
            }
            let mut close = false;
            egui::Window::new("Video export cancelled")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("No file was kept.");
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            if close {
                keep = false;
            }
        }
    }
    if !keep {
        app.video_job = None;
    }
    true
}
