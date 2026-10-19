//! Presentation video export: the slide show's own timeline, rendered frame by frame and encoded
//! to MP4 (H.264/AAC) or WebM (VP9/Opus) through an optional ffmpeg backend.
//!
//! The frames come from the same renderer, transitions (`deckcraft_render::render_transition`) and
//! animation state (`deckcraft_anim::Timeline`) the slide show uses, so a video is not a second,
//! drifting picture of the deck. Slide timings come from the slide's advance time (what Rehearse
//! Timings records), falling back to `secondsPerSlide`.
//!
//! ffmpeg is an optional external backend: DeckCraft does not ship an encoder, and the browser
//! build cannot start one. When it is missing the command says so, names the package to install
//! and suggests the animated GIF or PDF export instead. No part of the editor needs ffmpeg.
//!
//! Limits are hostile-input limits: frame size, rate, count, slide duration and audio length are
//! bounded, and every number from the command is checked before a frame is allocated.

use std::sync::atomic::AtomicBool;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::Ordering;

use deckcraft_anim::Timeline;
use deckcraft_model::{Presentation, ShapeKind};
use serde_json::Value;

use crate::cmd::bad;
use crate::{EngineError, Result};

const CMD: &str = "file.export";

/// Longest side of a video frame.
pub const MAX_SIDE: u32 = 3840;
pub const MIN_SIDE: u32 = 64;
pub const DEFAULT_WIDTH: u32 = 1920;
pub const DEFAULT_FPS: u32 = 30;
pub const MIN_FPS: u32 = 1;
pub const MAX_FPS: u32 = 120;
pub const DEFAULT_SECONDS: f64 = 5.0;
pub const MIN_SECONDS: f64 = 0.1;
pub const MAX_SECONDS: f64 = 600.0;
/// Most frames one export may render (about 1 h 50 min at 30 fps).
pub const MAX_FRAMES: u64 = 200_000;
/// Longest video one export may make, in seconds (2 h).
pub const MAX_DURATION: f64 = 7_200.0;
/// Audio mix rate and channels.
const AUDIO_RATE: u32 = 48_000;
const AUDIO_CHANNELS: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    Mp4,
    WebM,
}

impl VideoFormat {
    pub fn id(self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "mp4",
            VideoFormat::WebM => "webm",
        }
    }
    pub fn extension(self) -> &'static str {
        self.id()
    }
}

/// What to export.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoOptions {
    pub format: VideoFormat,
    pub width: u32,
    /// `None`: the slide's aspect ratio decides the height.
    pub height: Option<u32>,
    pub fps: u32,
    /// How long a slide stays after its animation when it has no recorded advance time.
    pub seconds_per_slide: f64,
    /// Slide indices in play order (default: all visible slides).
    pub slides: Option<Vec<usize>>,
    pub include_hidden: bool,
    /// Use the slide's recorded advance time (`transition.advance_after_ms`).
    pub use_timings: bool,
    pub transitions: bool,
    pub animations: bool,
    /// Mix the audio of autoplay media into the video.
    pub audio: bool,
    /// 0 (best) to 100 (smallest).
    pub quality: u8,
}

impl Default for VideoOptions {
    fn default() -> Self {
        VideoOptions {
            format: VideoFormat::Mp4,
            width: DEFAULT_WIDTH,
            height: None,
            fps: DEFAULT_FPS,
            seconds_per_slide: DEFAULT_SECONDS,
            slides: None,
            include_hidden: false,
            use_timings: true,
            transitions: true,
            animations: true,
            audio: true,
            quality: 60,
        }
    }
}

fn given<'a>(p: &'a Value, key: &str) -> Option<&'a Value> {
    p.get(key).filter(|v| !v.is_null())
}

fn side(value: &Value, key: &str) -> Result<u32> {
    let n = value.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad(CMD, format!("`{key}` must be a whole number of pixels")))?;
    if (MIN_SIDE..=MAX_SIDE).contains(&n) { Ok(n) } else { Err(bad(CMD, format!("`{key}` must be from {MIN_SIDE} to {MAX_SIDE} pixels"))) }
}

fn flag(p: &Value, key: &str, default: bool) -> Result<bool> {
    match given(p, key) {
        None => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(bad(CMD, format!("`{key}` must be true or false"))),
    }
}

impl VideoOptions {
    /// Read `file.export` parameters. Absent values take their defaults; a wrong type or an
    /// out-of-range number is an error naming the key.
    pub fn from_params(p: &Value) -> Result<Self> {
        let mut o = VideoOptions::default();
        if let Some(v) = given(p, "format") {
            let s = v.as_str().ok_or_else(|| bad(CMD, "`format` must be \"mp4\" or \"webm\""))?;
            o.format = match s.to_ascii_lowercase().as_str() {
                "mp4" | "m4v" => VideoFormat::Mp4,
                "webm" => VideoFormat::WebM,
                other => return Err(bad(CMD, format!("unknown video format `{other}` (mp4 or webm)"))),
            };
        }
        if let Some(v) = given(p, "width") {
            o.width = match v {
                Value::String(s) if s.eq_ignore_ascii_case("720p") => 1280,
                Value::String(s) if s.eq_ignore_ascii_case("1080p") => 1920,
                Value::String(s) if s.eq_ignore_ascii_case("4k") || s.eq_ignore_ascii_case("2160p") => 3840,
                _ => side(v, "width")?,
            };
        }
        if let Some(v) = given(p, "height") {
            o.height = Some(side(v, "height")?);
        }
        if let Some(v) = given(p, "fps") {
            let n = v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad(CMD, "`fps` must be a whole number"))?;
            if !(MIN_FPS..=MAX_FPS).contains(&n) {
                return Err(bad(CMD, format!("`fps` must be from {MIN_FPS} to {MAX_FPS}")));
            }
            o.fps = n;
        }
        if let Some(v) = given(p, "secondsPerSlide") {
            let s = v.as_f64().ok_or_else(|| bad(CMD, "`secondsPerSlide` must be a number"))?;
            if !(MIN_SECONDS..=MAX_SECONDS).contains(&s) {
                return Err(bad(CMD, format!("`secondsPerSlide` must be from {MIN_SECONDS} to {MAX_SECONDS}")));
            }
            o.seconds_per_slide = s;
        }
        if let Some(v) = given(p, "slides") {
            let a = v.as_array().ok_or_else(|| bad(CMD, "`slides` must be a list of slide indices"))?;
            if a.is_empty() {
                return Err(bad(CMD, "`slides` is empty"));
            }
            let list = a
                .iter()
                .map(|x| x.as_u64().and_then(|i| usize::try_from(i).ok()))
                .collect::<Option<Vec<usize>>>()
                .ok_or_else(|| bad(CMD, "`slides` must be a list of slide indices (0, 1, 2…)"))?;
            o.slides = Some(list);
        }
        o.include_hidden = flag(p, "includeHidden", o.include_hidden)?;
        o.use_timings = flag(p, "useTimings", o.use_timings)?;
        o.transitions = flag(p, "transitions", o.transitions)?;
        o.animations = flag(p, "animations", o.animations)?;
        o.audio = flag(p, "audio", o.audio)?;
        if let Some(v) = given(p, "quality") {
            let n = v.as_u64().ok_or_else(|| bad(CMD, "`quality` must be from 0 to 100"))?;
            o.quality = u8::try_from(n.min(100)).unwrap_or(100);
        }
        Ok(o)
    }

    /// The frame size: the requested width and the slide's aspect ratio, or the requested height,
    /// with both sides even (H.264 encoders reject odd sizes) and the longest side capped.
    pub fn frame_size(&self, doc: &Presentation) -> (u32, u32) {
        let sw = if doc.slide_size.width.is_finite() && doc.slide_size.width > 0.0 { doc.slide_size.width } else { 960.0 };
        let sh = if doc.slide_size.height.is_finite() && doc.slide_size.height > 0.0 { doc.slide_size.height } else { 540.0 };
        let aspect = sh / sw;
        let even = |v: f64| {
            let n = if v.is_finite() { v.round().clamp(f64::from(MIN_SIDE), f64::from(MAX_SIDE)) } else { 540.0 } as u32;
            (n & !1).max(MIN_SIDE)
        };
        let w = even(f64::from(self.width.clamp(MIN_SIDE, MAX_SIDE)));
        let h = match self.height {
            Some(h) => even(f64::from(h.clamp(MIN_SIDE, MAX_SIDE))),
            None => even(f64::from(w) * aspect),
        };
        (w, h)
    }

    /// The slides in play order: the chosen ones (or all), without hidden slides unless asked.
    pub fn slides(&self, doc: &Presentation) -> Result<Vec<usize>> {
        let n = doc.slides.len();
        let chosen: Vec<usize> = match &self.slides {
            Some(list) => {
                if let Some(i) = list.iter().find(|i| **i >= n) {
                    return Err(bad(CMD, format!("no slide {i} (the presentation has {n})")));
                }
                list.clone()
            }
            None => (0..n).collect(),
        };
        let out: Vec<usize> = chosen.into_iter().filter(|i| self.include_hidden || doc.slides.get(*i).is_some_and(|s| !s.hidden)).collect();
        if out.is_empty() {
            return Err(bad(CMD, "no slides to export (they are all hidden, or the presentation is empty)"));
        }
        Ok(out)
    }

    /// CRF for ffmpeg's x264/VP9 encoders: 18 (best) to 34 (smallest).
    pub fn crf(&self) -> u8 {
        34u8.saturating_sub(self.quality.min(100) / 6)
    }
}

/// One slide's place on the video clock.
#[derive(Clone, Debug)]
pub struct SlideSegment {
    pub index: usize,
    /// Time from the start of the video.
    pub start: f64,
    /// The transition into this slide (0 when there is none).
    pub transition: f64,
    /// The build animations' total time (0 when there are none or they are off).
    pub animation: f64,
    /// How long the finished slide stays before the next one.
    pub hold: f64,
    /// The slide's animation timeline.
    pub timeline: Timeline,
}

impl SlideSegment {
    pub fn total(&self) -> f64 {
        self.transition + self.animation + self.hold
    }
}

/// The video clock: the slides in order and how long each one takes.
#[derive(Clone, Debug)]
pub struct VideoPlan {
    pub segments: Vec<SlideSegment>,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    /// Total length in seconds.
    pub duration: f64,
    /// Total frames, rounded up.
    pub frames: u64,
}

impl VideoPlan {
    /// The segment and local time at `t` seconds.
    pub fn at(&self, t: f64) -> Option<(&SlideSegment, f64)> {
        let t = if t.is_finite() { t.max(0.0) } else { 0.0 };
        // Segments are short; a linear walk is clearer than a binary search over floats.
        for (i, s) in self.segments.iter().enumerate() {
            let end = s.start + s.total();
            if t < end || i + 1 == self.segments.len() {
                return Some((s, (t - s.start).clamp(0.0, s.total())));
            }
        }
        self.segments.last().map(|s| (s, s.total()))
    }

    /// The animation clock at local time `local` in `seg`: `(step, t)` for [`Timeline::state`].
    pub fn animation_clock(seg: &SlideSegment, local: f64) -> (usize, f64) {
        let t = local - seg.transition;
        if t < 0.0 {
            return (0, 0.0);
        }
        if t >= seg.animation {
            return (seg.timeline.steps(), 0.0);
        }
        seg.timeline.locate(t)
    }
}

/// Build the clock for `doc` and `opts`.
pub fn plan(doc: &Presentation, opts: &VideoOptions) -> Result<VideoPlan> {
    let slides = opts.slides(doc)?;
    let (width, height) = opts.frame_size(doc);
    let no_animation = doc.show.without_animation;
    let use_timings = opts.use_timings && doc.show.use_timings;
    let mut segments = Vec::with_capacity(slides.len());
    let mut start = 0.0f64;
    for index in slides {
        let Some(slide) = doc.slides.get(index) else { continue };
        let timeline = Timeline::new(slide, doc.slide_size.width, doc.slide_size.height);
        let transition = slide
            .transition
            .as_ref()
            .filter(|t| opts.transitions && !no_animation && t.kind != "none")
            .map(|t| f64::from(t.duration_ms.min(86_400_000)) / 1000.0)
            .unwrap_or(0.0)
            .clamp(0.0, MAX_SECONDS);
        let animation = if opts.animations && !no_animation { timeline.total_duration().clamp(0.0, MAX_SECONDS) } else { 0.0 };
        let recorded = slide.transition.as_ref().and_then(|t| t.advance_after_ms).map(|ms| f64::from(ms.min(86_400_000)) / 1000.0);
        let hold = match (use_timings, recorded) {
            (true, Some(advance)) => (advance - transition - animation).max(0.0),
            _ => opts.seconds_per_slide,
        };
        let total = (transition + animation + hold).max(1.0 / f64::from(opts.fps.max(1)));
        segments.push(SlideSegment { index, start, transition, animation, hold, timeline });
        start += total;
        if start > MAX_DURATION {
            return Err(bad(CMD, format!("the video would be longer than {MAX_DURATION} seconds; choose fewer slides or shorter timings")));
        }
    }
    if segments.is_empty() {
        return Err(bad(CMD, "no slides to export"));
    }
    let duration = segments.iter().map(SlideSegment::total).sum::<f64>();
    let frames = (duration * f64::from(opts.fps.max(1))).ceil().max(1.0);
    if frames > MAX_FRAMES as f64 {
        return Err(bad(CMD, format!("the video would need more than {MAX_FRAMES} frames; choose a lower frame rate or shorter timings")));
    }
    Ok(VideoPlan { segments, fps: opts.fps, width, height, duration, frames: frames as u64 })
}

/// What an export wrote.
#[derive(Clone, Debug)]
pub struct VideoOutput {
    pub path: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames: u64,
    pub duration: f64,
    pub format: VideoFormat,
    /// Media whose audio couldn't be decoded (the video was still written).
    pub audio_warnings: Vec<String>,
}

/// Progress of a running export.
#[derive(Clone, Debug)]
pub struct Progress {
    pub frame: u64,
    pub frames: u64,
    pub stage: &'static str,
}

/// Render and encode the presentation to `path`. `cancel` is checked between frames; when it is
/// set the child process is stopped and the partial file removed.
#[cfg(not(target_arch = "wasm32"))]
pub fn encode_to(
    doc: &Presentation,
    opts: &VideoOptions,
    path: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<VideoOutput> {
    let plan = plan(doc, opts)?;
    let ffmpeg = ffmpeg_path().ok_or_else(missing_ffmpeg)?;
    if opts.format == VideoFormat::WebM && !ffmpeg_has_encoder(&ffmpeg, "libvpx-vp9") {
        return Err(EngineError::Other("WebM export needs an ffmpeg build with libvpx-vp9. Install a full ffmpeg, or export MP4 instead.".into()));
    }
    if opts.format == VideoFormat::Mp4 && !ffmpeg_has_encoder(&ffmpeg, "libx264") {
        return Err(EngineError::Other(
            "MP4 export needs an ffmpeg build with libx264. Install a full ffmpeg, or export WebM or an animated GIF instead.".into(),
        ));
    }
    crate::io::check_write_path(path).map_err(EngineError::Other)?;
    let audio = if opts.audio { mix_audio(doc, &plan)? } else { None };
    let mut cmd = std::process::Command::new(&ffmpeg);
    cmd.arg("-hide_banner").arg("-loglevel").arg("error").arg("-y");
    cmd.arg("-f").arg("rawvideo").arg("-pix_fmt").arg("rgba");
    cmd.arg("-s").arg(format!("{}x{}", plan.width, plan.height));
    cmd.arg("-r").arg(plan.fps.to_string());
    cmd.arg("-i").arg("pipe:0");
    if let Some(a) = &audio {
        cmd.arg("-f").arg("s16le").arg("-ar").arg(AUDIO_RATE.to_string()).arg("-ac").arg(AUDIO_CHANNELS.to_string());
        cmd.arg("-i").arg(&a.path);
    }
    match opts.format {
        VideoFormat::Mp4 => {
            cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", &opts.crf().to_string(), "-pix_fmt", "yuv420p"]);
            if audio.is_some() {
                cmd.args(["-c:a", "aac", "-b:a", "192k"]);
            }
            cmd.args(["-movflags", "+faststart"]);
        }
        VideoFormat::WebM => {
            cmd.args(["-c:v", "libvpx-vp9", "-crf", &opts.crf().to_string(), "-b:v", "0"]);
            if audio.is_some() {
                cmd.args(["-c:a", "libopus", "-b:a", "128k"]);
            }
        }
    }
    cmd.arg(path);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| EngineError::Other(format!("can't start ffmpeg ({}): {e}", ffmpeg.display())))?;
    let mut stdin = child.stdin.take().ok_or_else(|| EngineError::Other("ffmpeg has no input pipe".into()))?;
    let stderr = child.stderr.take();
    let reader = stderr.map(|mut s| {
        std::thread::spawn(move || {
            use std::io::Read as _;
            let mut text = String::new();
            let _ = s.read_to_string(&mut text);
            text
        })
    });
    let mut write_err: Option<String> = None;
    let total = plan.frames.max(1);
    for f in 0..total {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            crate::io::remove_file(std::path::Path::new(path));
            return Err(EngineError::Other("video export cancelled".into()));
        }
        let t = f as f64 / f64::from(plan.fps.max(1));
        let image = render_frame(doc, &plan, t);
        let rgba = opaque_rgba(&image, plan.width, plan.height);
        use std::io::Write as _;
        if let Err(e) = stdin.write_all(&rgba) {
            write_err = Some(e.to_string());
            break;
        }
        progress(Progress { frame: f + 1, frames: total, stage: "rendering" });
    }
    drop(stdin);
    let status = child.wait();
    let err_text = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    if let Some(a) = &audio {
        crate::io::remove_file(&a.path);
    }
    if let Some(e) = write_err {
        crate::io::remove_file(std::path::Path::new(path));
        return Err(EngineError::Other(format!("ffmpeg stopped reading frames: {e}{}", tail(&err_text))));
    }
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            crate::io::remove_file(std::path::Path::new(path));
            return Err(EngineError::Other(format!("ffmpeg failed ({s}){}", tail(&err_text))));
        }
        Err(e) => {
            crate::io::remove_file(std::path::Path::new(path));
            return Err(EngineError::Other(format!("ffmpeg could not be waited for: {e}{}", tail(&err_text))));
        }
    }
    let bytes = crate::io::file_len(path).unwrap_or(0);
    if bytes == 0 {
        crate::io::remove_file(std::path::Path::new(path));
        return Err(EngineError::Other(format!("ffmpeg wrote an empty file{}", tail(&err_text))));
    }
    progress(Progress { frame: total, frames: total, stage: "encoding" });
    Ok(VideoOutput {
        path: path.to_string(),
        bytes,
        width: plan.width,
        height: plan.height,
        fps: plan.fps,
        frames: total,
        duration: plan.duration,
        format: opts.format,
        audio_warnings: audio.map(|a| a.warnings).unwrap_or_default(),
    })
}

#[cfg(target_arch = "wasm32")]
pub fn encode_to(
    _doc: &Presentation,
    _opts: &VideoOptions,
    _path: &str,
    _cancel: &AtomicBool,
    _progress: &mut dyn FnMut(Progress),
) -> Result<VideoOutput> {
    Err(EngineError::Other("video export needs the desktop app: a browser can't run ffmpeg".into()))
}

fn tail(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        let mut lines: Vec<&str> = text.lines().rev().take(4).collect();
        lines.reverse();
        format!(": {}", lines.join(" | "))
    }
}

fn missing_ffmpeg() -> EngineError {
    EngineError::Other(
        "Video export needs ffmpeg with libx264 (MP4) or libvpx-vp9 (WebM). Install ffmpeg and put it on PATH, \
         set DECKCRAFT_FFMPEG to its path, or export an animated GIF or PDF instead."
            .into(),
    )
}

/// The ffmpeg to run: `DECKCRAFT_FFMPEG` when set, else `ffmpeg` from PATH. `None` when it isn't
/// there or doesn't run.
#[cfg(not(target_arch = "wasm32"))]
pub fn ffmpeg_path() -> Option<std::path::PathBuf> {
    let candidate = std::env::var_os("DECKCRAFT_FFMPEG").map(std::path::PathBuf::from).unwrap_or_else(|| "ffmpeg".into());
    let ok = std::process::Command::new(&candidate)
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(candidate)
}

/// Whether `ffmpeg` lists `encoder` in `-encoders`.
#[cfg(not(target_arch = "wasm32"))]
pub fn ffmpeg_has_encoder(ffmpeg: &std::path::Path, encoder: &str) -> bool {
    let out = std::process::Command::new(ffmpeg).args(["-hide_banner", "-encoders"]).stdin(std::process::Stdio::null()).output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).contains(encoder),
        _ => false,
    }
}

/// Render the frame at `t` seconds: a transition, an animation frame, or a finished slide.
pub fn render_frame(doc: &Presentation, plan: &VideoPlan, t: f64) -> deckcraft_render::Image {
    let Some((seg, local)) = plan.at(t) else { return deckcraft_render::Image::default() };
    let scale = f64::from(plan.width) / doc.slide_size.width.max(1.0);
    let size = Some((plan.width, plan.height));
    let threads = if cfg!(target_arch = "wasm32") { 0 } else { 4 };
    if seg.transition > 0.0 && local < seg.transition {
        let p = (local / seg.transition).clamp(0.0, 1.0);
        let (step, at) = VideoPlan::animation_clock(seg, 0.0);
        let new_state = |id| state_for(&seg.timeline, id, step, at);
        let new_opts = deckcraft_render::RenderOpts { scale, size, state: Some(&new_state), threads, ..Default::default() };
        let Some(slide) = doc.slides.get(seg.index) else { return deckcraft_render::Image::default() };
        let kind = slide.transition.as_ref().map(|x| x.kind.as_str()).unwrap_or("none");
        if kind == "morph" {
            return deckcraft_render::render_transition(doc, seg.index, p, &new_opts);
        }
        let new = deckcraft_render::render_slide(doc, seg.index, &new_opts);
        let prev = seg.index.checked_sub(1).filter(|i| doc.slides.get(*i).is_some());
        let old = prev.map(|i| {
            let tl = Timeline::new(&doc.slides[i], doc.slide_size.width, doc.slide_size.height);
            let f = |id| state_for(&tl, id, tl.steps(), 0.0);
            deckcraft_render::render_slide(doc, i, &deckcraft_render::RenderOpts { scale, size, state: Some(&f), threads, ..Default::default() })
        });
        let parts = prev.and_then(|_| {
            deckcraft_anim::uses_slide_parts(kind, slide.transition.as_ref().map(|x| x.option.as_str()).unwrap_or("")).then(|| {
                deckcraft_render::SlideParts::render(
                    doc,
                    prev,
                    seg.index,
                    &deckcraft_render::RenderOpts { scale, size, threads, ..Default::default() },
                )
            })
        });
        let aspect = doc.slide_size.width / doc.slide_size.height.max(1.0);
        let layers = deckcraft_anim::transition_frame(kind, slide.transition.as_ref().map(|x| x.option.as_str()).unwrap_or(""), p, aspect);
        return deckcraft_render::composite_transition(old.as_ref(), &new, parts.as_ref(), &layers);
    }
    let (step, at) = VideoPlan::animation_clock(seg, local);
    let state = |id| state_for(&seg.timeline, id, step, at);
    deckcraft_render::render_slide(doc, seg.index, &deckcraft_render::RenderOpts { scale, size, state: Some(&state), threads, ..Default::default() })
}

fn state_for(tl: &Timeline, id: deckcraft_model::ShapeId, step: usize, t: f64) -> deckcraft_render::ShapeState {
    let a = tl.state(id, None, step, t);
    deckcraft_render::ShapeState {
        visible: a.visible,
        opacity: a.opacity,
        offset: deckcraft_geom::Vec2::new(a.offset_x, a.offset_y),
        scale: (a.scale_x, a.scale_y),
        rotate: a.rotate,
        clip: a.clip,
        tint: a.tint,
        paras: tl
            .paragraph_targets(id)
            .into_iter()
            .map(|p| {
                let a = tl.state(id, Some(p), step, t);
                (p, deckcraft_render::ParaState { visible: a.visible, opacity: a.opacity, offset: deckcraft_geom::Vec2::new(a.offset_x, a.offset_y) })
            })
            .collect(),
    }
}

/// Premultiplied RGBA rendered over white, as the straight opaque RGBA an encoder takes.
fn opaque_rgba(img: &deckcraft_render::Image, w: u32, h: u32) -> Vec<u8> {
    let len = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if img.width != w || img.height != h || img.pixels.len() != len {
        log::error!("video export: a slide rendered at {}×{} instead of {w}×{h}; writing a white frame", img.width, img.height);
        return vec![255; len];
    }
    let mut px = img.pixels.clone();
    for p in px.as_chunks_mut::<4>().0 {
        let back = 255u16.saturating_sub(u16::from(p[3]));
        p[0] = (u16::from(p[0]) + back).min(255) as u8;
        p[1] = (u16::from(p[1]) + back).min(255) as u8;
        p[2] = (u16::from(p[2]) + back).min(255) as u8;
        p[3] = 255;
    }
    px
}

/// One audio cue: a media clip starting at `start` seconds into the video.
struct AudioCue<'a> {
    start: f64,
    clip: &'a deckcraft_model::MediaClip,
    media: &'a deckcraft_model::MediaItem,
    /// How long the cue may play (the slide segment's remaining time).
    until: f64,
}

struct MixedAudio {
    path: std::path::PathBuf,
    warnings: Vec<String>,
}

fn push_cue<'a>(cues: &mut Vec<AudioCue<'a>>, doc: &'a Presentation, shape: &'a deckcraft_model::Shape, start: f64, until: f64) {
    let ShapeKind::Media(clip) = &shape.kind else { return };
    let Some(media) = doc.media(clip.media) else { return };
    if !(media.content_type.starts_with("audio/") || media.content_type.starts_with("video/")) {
        return;
    }
    if media.data.is_empty() || start >= until {
        return;
    }
    cues.push(AudioCue { start, clip, media, until });
}

/// Mix autoplay media audio (and media animations' Play cues) into a temporary 48 kHz stereo
/// PCM file. `None` when the deck has no audio.
#[cfg(not(target_arch = "wasm32"))]
fn mix_audio(doc: &Presentation, plan: &VideoPlan) -> Result<Option<MixedAudio>> {
    use std::io::Write as _;

    let mut cues: Vec<AudioCue<'_>> = Vec::new();
    for seg in &plan.segments {
        let Some(slide) = doc.slides.get(seg.index) else { continue };
        let seg_start = seg.start + seg.transition;
        let seg_end = seg.start + seg.total();
        deckcraft_model::walk(&slide.shapes, &mut |shape, _| {
            if let ShapeKind::Media(clip) = &shape.kind
                && clip.autoplay
            {
                push_cue(&mut cues, doc, shape, seg_start, seg_end);
            }
        });
        // Media animations that play a clip at a step start.
        for e in seg.timeline.effects() {
            if e.class != deckcraft_model::AnimClass::Media {
                continue;
            }
            let Some(anim) = slide.animations.get(e.anim) else { continue };
            if !anim.effect.eq_ignore_ascii_case("play") {
                continue;
            }
            let Some(shape) = slide.shape(e.shape) else { continue };
            let step_start: f64 = (0..e.step).map(|i| seg.timeline.step_duration(i)).sum();
            let start = seg_start + step_start + e.start;
            push_cue(&mut cues, doc, shape, start, seg_end);
        }
    }
    if cues.is_empty() {
        return Ok(None);
    }
    let total = plan.duration.max(0.0);
    let frames = (total * f64::from(AUDIO_RATE)).ceil().max(1.0) as usize;
    // A 2 h stereo buffer is 1.4 GB; cap the mixed audio at 30 minutes of *content* but keep the
    // video length: silence after that is written as zeroes in chunks.
    let max_mix = (AUDIO_RATE as usize) * 2 * 60 * 30;
    let (path, file) = crate::io::create_temp("deckcraft-video-audio").map_err(EngineError::Other)?;
    let mut out = std::io::BufWriter::new(file);
    let mut warnings: Vec<String> = vec![];
    let mut written = 0usize;
    let mut buf = vec![0f32; (AUDIO_RATE as usize * 2).min(4096)];
    while written < frames {
        let chunk = buf.len() / 2;
        buf.fill(0.0);
        let t0 = written as f64 / f64::from(AUDIO_RATE);
        if written < max_mix {
            for cue in &cues {
                let Ok(pcm) = deckcraft_media::audio::decode(&cue.media.data) else {
                    let name = if cue.media.name.is_empty() { "a media file".to_string() } else { cue.media.name.clone() };
                    if !warnings.iter().any(|w| w.contains(&name)) {
                        warnings.push(format!("{name}: its audio couldn't be decoded and was left out"));
                    }
                    continue;
                };
                mix_cue(&mut buf, chunk, t0, cue, &pcm);
            }
        }
        let mut bytes = Vec::with_capacity(chunk * 4);
        for s in &buf[..chunk * 2] {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        out.write_all(&bytes).map_err(|e| EngineError::Other(format!("can't write the video's audio: {e}")))?;
        written += chunk;
    }
    out.flush().map_err(|e| EngineError::Other(format!("can't write the video's audio: {e}")))?;
    Ok(Some(MixedAudio { path, warnings }))
}

/// Mix one cue into `buf` for the output frames `[written, written + chunk)`.
#[cfg(not(target_arch = "wasm32"))]
fn mix_cue(buf: &mut [f32], chunk: usize, t0: f64, cue: &AudioCue<'_>, pcm: &deckcraft_media::Pcm) {
    let clip = cue.clip;
    let trim = f64::from(clip.trim_start_ms) / 1000.0;
    let trim_end = if clip.trim_end_ms > 0 { f64::from(clip.trim_end_ms) / 1000.0 } else { pcm.duration() };
    let source_len = (trim_end - trim).max(0.0);
    if source_len <= 0.0 || pcm.rate == 0 {
        return;
    }
    let fade_in = f64::from(clip.fade_in_ms) / 1000.0;
    let fade_out = f64::from(clip.fade_out_ms) / 1000.0;
    let volume = clip.volume.clamp(0.0, 1.0) as f32;
    let channels = pcm.channels.max(1) as usize;
    for i in 0..chunk {
        let t = t0 + i as f64 / f64::from(AUDIO_RATE);
        if t < cue.start || t >= cue.until {
            continue;
        }
        let into = t - cue.start;
        let source_t = if clip.loop_play { into % source_len } else { into };
        if source_t >= source_len {
            continue;
        }
        let at = trim + source_t;
        let pos = at * f64::from(pcm.rate);
        let i0 = pos.floor().max(0.0) as usize;
        let frac = (pos - i0 as f64) as f32;
        let sample = |ch: usize| {
            let a = pcm.at(i0, ch);
            let b = pcm.at(i0 + 1, ch);
            a + (b - a) * frac
        };
        let (l, r) = if channels >= 2 {
            (sample(0), sample(1))
        } else {
            let m = sample(0);
            (m, m)
        };
        let mut gain = volume;
        if fade_in > 0.0 && into < fade_in {
            gain *= (into / fade_in) as f32;
        }
        if fade_out > 0.0 {
            let left = (cue.until - t).max(0.0);
            if left < fade_out {
                gain *= (left / fade_out) as f32;
            }
        }
        if let Some(slot) = buf.get_mut(i * 2) {
            *slot += l * gain;
        }
        if let Some(slot) = buf.get_mut(i * 2 + 1) {
            *slot += r * gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckcraft_model::defaults;
    use serde_json::json;

    fn deck() -> Presentation {
        let mut doc = defaults::new_presentation(None);
        doc.slides.clear();
        let layout = defaults::layout_of_kind(&doc, deckcraft_model::LayoutType::TitleAndContent).unwrap_or_default();
        if let Some(slide) = defaults::new_slide(&mut doc, layout) {
            doc.slides.push(std::sync::Arc::new(slide));
        }
        doc
    }

    #[test]
    fn options_are_read_and_hostile_values_rejected() {
        let o = VideoOptions::from_params(&json!({"width": "1080p", "fps": 60, "secondsPerSlide": 2.5, "quality": 90})).unwrap();
        assert_eq!((o.width, o.fps, o.seconds_per_slide, o.quality), (1920, 60, 2.5, 90));
        assert!(VideoOptions::from_params(&json!({"width": 12})).is_err());
        assert!(VideoOptions::from_params(&json!({"fps": 0})).is_err());
        assert!(VideoOptions::from_params(&json!({"fps": 121})).is_err());
        assert!(VideoOptions::from_params(&json!({"secondsPerSlide": 0.0})).is_err());
        assert!(VideoOptions::from_params(&json!({"secondsPerSlide": 601.0})).is_err());
        assert!(VideoOptions::from_params(&json!({"audio": "yes"})).is_err());
        assert!(VideoOptions::from_params(&json!({"format": "avi"})).is_err());
        assert!(VideoOptions::from_params(&json!({"slides": []})).is_err());
        assert_eq!(VideoOptions::from_params(&json!({"quality": 999})).unwrap().quality, 100);
    }

    #[test]
    fn frame_sizes_are_even_capped_and_keep_the_aspect_ratio() {
        let doc = deck();
        let o = VideoOptions { width: 1920, ..Default::default() };
        assert_eq!(o.frame_size(&doc), (1920, 1080));
        let o = VideoOptions { width: 1280, height: Some(721), ..Default::default() };
        let (w, h) = o.frame_size(&doc);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert!(w <= MAX_SIDE && h <= MAX_SIDE);
        let o = VideoOptions { width: 9999, ..Default::default() };
        assert_eq!(o.frame_size(&doc).0, 3840);
    }

    #[test]
    fn plan_uses_recorded_advance_times_and_animation_durations() {
        let mut doc = deck();
        let slide = doc.slides[0].as_ref().clone();
        let mut slide = slide;
        slide.transition =
            Some(deckcraft_model::anim::Transition { kind: "fade".into(), duration_ms: 500, advance_after_ms: Some(3000), ..Default::default() });
        doc.slides[0] = std::sync::Arc::new(slide);
        let o = VideoOptions { fps: 10, seconds_per_slide: 9.0, ..Default::default() };
        let plan = plan(&doc, &o).unwrap();
        let seg = &plan.segments[0];
        assert_eq!(seg.transition, 0.5);
        assert_eq!(seg.hold, 2.5);
        assert!((seg.total() - 3.0).abs() < 1e-9);
        assert!((plan.duration - 3.0).abs() < 1e-9);
        assert_eq!(plan.frames, 30);
    }

    #[test]
    fn plan_honours_slide_order_hidden_slides_and_the_animation_switch() {
        let mut doc = deck();
        let layout = defaults::layout_of_kind(&doc, deckcraft_model::LayoutType::TitleOnly).unwrap_or_default();
        let mut hidden = defaults::new_slide(&mut doc, layout).unwrap();
        hidden.hidden = true;
        doc.slides.push(std::sync::Arc::new(hidden));
        let o = VideoOptions { fps: 10, ..Default::default() };
        assert_eq!(plan(&doc, &o).unwrap().segments.len(), 1);
        let o = VideoOptions { fps: 10, include_hidden: true, ..Default::default() };
        assert_eq!(plan(&doc, &o).unwrap().segments.len(), 2);
        let o = VideoOptions { fps: 10, slides: Some(vec![1, 0]), include_hidden: true, ..Default::default() };
        let p = plan(&doc, &o).unwrap();
        assert_eq!(p.segments.iter().map(|s| s.index).collect::<Vec<_>>(), vec![1, 0]);
        let o = VideoOptions { fps: 10, slides: Some(vec![9]), ..Default::default() };
        assert!(plan(&doc, &o).is_err());
    }

    #[test]
    fn animation_clock_reaches_the_finished_slide_at_the_end() {
        let mut doc = deck();
        let layout = defaults::layout_of_kind(&doc, deckcraft_model::LayoutType::TitleAndContent).unwrap_or_default();
        let mut slide = defaults::new_slide(&mut doc, layout).unwrap();
        if let Some(sh) = slide.shapes.first_mut() {
            slide.animations.push(deckcraft_model::Animation {
                shape: sh.id,
                effect: "fade".into(),
                class: deckcraft_model::AnimClass::Entrance,
                duration_ms: 1000,
                ..Default::default()
            });
        }
        doc.slides[0] = std::sync::Arc::new(slide);
        let o = VideoOptions { fps: 10, seconds_per_slide: 0.5, ..Default::default() };
        let p = plan(&doc, &o).unwrap();
        let seg = &p.segments[0];
        assert!(seg.animation >= 1.0, "{}", seg.animation);
        let (step, _) = VideoPlan::animation_clock(seg, seg.transition + seg.animation + 0.01);
        assert_eq!(step, seg.timeline.steps());
        let (step, _) = VideoPlan::animation_clock(seg, 0.0);
        assert_eq!(step, 0);
    }

    #[test]
    fn rendering_frames_works_without_ffmpeg() {
        let doc = deck();
        let o = VideoOptions { width: 320, height: Some(180), fps: 5, ..Default::default() };
        let p = plan(&doc, &o).unwrap();
        let img = render_frame(&doc, &p, 0.0);
        assert_eq!((img.width, img.height), (320, 180));
        let rgba = opaque_rgba(&img, 320, 180);
        assert_eq!(rgba.len(), 320 * 180 * 4);
        assert!(rgba.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn a_missing_ffmpeg_is_explained_not_a_panic() {
        // The test host may have ffmpeg; only the message contract is checked when it doesn't.
        if ffmpeg_path().is_none() {
            let e = missing_ffmpeg().to_string();
            assert!(e.contains("ffmpeg") && e.contains("GIF") && e.contains("PDF"));
        }
    }

    fn two_slides() -> Presentation {
        let mut doc = deck();
        let layout = defaults::layout_of_kind(&doc, deckcraft_model::LayoutType::TitleOnly).unwrap_or_default();
        if let Some(slide) = defaults::new_slide(&mut doc, layout) {
            doc.slides.push(std::sync::Arc::new(slide));
        }
        doc
    }

    /// A real encode, skipped (with a note) on hosts without ffmpeg or the encoder. It proves the
    /// frame pipe, the encoder arguments and the written container, not just the plan.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ffmpeg_writes_a_playable_mp4_and_webm() {
        let Some(ffmpeg) = ffmpeg_path() else {
            eprintln!("skipping the video encode test: ffmpeg is not on PATH");
            return;
        };
        let dir = std::env::temp_dir().join(format!("deckcraft-video-encode-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let doc = two_slides();
        let base = VideoOptions {
            width: 320,
            height: Some(180),
            fps: 5,
            seconds_per_slide: 0.4,
            transitions: false,
            animations: false,
            audio: false,
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        // MP4's first box is `ftyp`; WebM is EBML, whose magic is at byte 0.
        for (format, encoder, at, magic) in
            [(VideoFormat::Mp4, "libx264", 4, &b"ftyp"[..]), (VideoFormat::WebM, "libvpx-vp9", 0, &[0x1A, 0x45, 0xDF, 0xA3][..])]
        {
            if !ffmpeg_has_encoder(&ffmpeg, encoder) {
                eprintln!("skipping {format:?}: this ffmpeg has no {encoder}");
                continue;
            }
            let opts = VideoOptions { format, ..base.clone() };
            let path = dir.join(format!("out.{}", format.extension()));
            let path = path.to_str().unwrap();
            let mut seen = vec![];
            let out = encode_to(&doc, &opts, path, &cancel, &mut |p| seen.push((p.frame, p.stage))).unwrap();
            assert_eq!((out.width, out.height, out.fps, out.format), (320, 180, 5, format));
            assert_eq!(out.frames, 4, "two slides at 0.4 s and 5 fps");
            assert!(out.duration >= 0.79 && out.duration <= 0.81, "{}", out.duration);
            assert!(seen.first().is_some_and(|(f, s)| *f == 1 && *s == "rendering"));
            assert!(seen.last().is_some_and(|(f, s)| *f == out.frames && *s == "encoding"));
            let bytes = std::fs::read(path).unwrap();
            assert_eq!(out.bytes as usize, bytes.len());
            assert!(bytes.len() > 100, "{format:?} wrote {} bytes", bytes.len());
            assert_eq!(&bytes[at..at + magic.len()], magic, "{format:?} does not look like its container");
        }
        // A cancel before the first frame leaves no partial file behind.
        if ffmpeg_has_encoder(&ffmpeg, "libx264") {
            let path = dir.join("cancelled.mp4");
            let path = path.to_str().unwrap();
            let cancel = AtomicBool::new(true);
            let err = encode_to(&doc, &base, path, &cancel, &mut |_| {}).unwrap_err().to_string();
            assert!(err.contains("cancelled"), "{err}");
            assert!(!std::path::Path::new(path).exists(), "a cancelled export left a file");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
