//! The file formats DeckCraft reads and writes, with what each one keeps and loses.
//!
//! The registry is the single source of truth behind Save As, Export, the CLI and MCP: a format's
//! id, extension, whether it stays editable, its limitations and the options it takes. The UI
//! must not invent a format that is not here, and a format listed here must have a writer in
//! [`crate::cmd::file::save_bytes`] or [`crate::cmd::file::export`].
//!
//! Everything is our own description; nothing here names or describes another product's file
//! format beyond the public standards (ECMA-376, ISO 32000, ISO 14496).

use serde::Serialize;

/// Whether opening the output again keeps the presentation editable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FormatKind {
    /// A presentation file that opens with its slides, shapes and text still editable.
    Editable,
    /// A copy for people or players: not meant to be edited again.
    Export,
}

/// Where a format can be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Availability {
    /// Every build, including the browser.
    Everywhere,
    /// Desktop builds: the writer needs a native tool or the file system.
    Desktop,
}

/// One writable format.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatSpec {
    /// The id commands take (`file.saveAs`, `file.export`).
    pub id: &'static str,
    /// The name a person sees.
    pub label: &'static str,
    /// The extension without the dot.
    pub extension: &'static str,
    pub kind: FormatKind,
    /// One sentence: what the format is for.
    pub description: &'static str,
    /// What survives a round trip, in a person's words.
    pub keeps: &'static str,
    /// What the format cannot keep. Empty when nothing is lost.
    pub limitations: &'static [&'static str],
    /// The option names `file.export` accepts for this format (besides `path`).
    pub options: &'static [&'static str],
    pub availability: Availability,
}

/// Every format DeckCraft writes, in the order Save As and Export list them: editable formats
/// first, then copies.
pub const FORMATS: &[FormatSpec] = &[
    FormatSpec {
        id: "deckcraft",
        label: "DeckCraft Presentation",
        extension: "deckcraft",
        kind: FormatKind::Editable,
        description: "DeckCraft's own format: a zip with the whole presentation and its media.",
        keeps: "Everything: slides, layouts, masters, themes, notes, comments, media and animation.",
        limitations: &[],
        options: &[],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "pptx",
        label: "PowerPoint Presentation",
        extension: "pptx",
        kind: FormatKind::Editable,
        description: "The Office Open XML presentation format, for exchanging editable decks.",
        keeps: "Slides, shapes, text, tables, charts, pictures, media, comments, transitions and animations.",
        limitations: &[
            "A few effects and unmodelled objects are kept as their original XML and may look different.",
            "Fonts the deck embeds are written only when the face can be embedded; others fall back.",
        ],
        options: &[],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "potx",
        label: "PowerPoint Template",
        extension: "potx",
        kind: FormatKind::Editable,
        description: "A presentation template: the masters, layouts and theme a new deck starts from.",
        keeps: "Masters, layouts, theme, and the sample slides in the file.",
        limitations: &["Some apps treat a template as a starting point and do not save back into it."],
        options: &[],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "ppsx",
        label: "PowerPoint Slide Show",
        extension: "ppsx",
        kind: FormatKind::Editable,
        description: "A presentation that opens straight into the slide show.",
        keeps: "The whole presentation and its show settings.",
        limitations: &["Opening it starts the show; edit it from the presentation app's editor."],
        options: &[],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "pdf",
        label: "PDF Document",
        extension: "pdf",
        kind: FormatKind::Export,
        description: "A fixed-layout copy that opens anywhere, with selectable text.",
        keeps: "Slide layout, pictures, fonts, links and bookmarks; notes and handouts layouts.",
        limitations: &["Not editable as a presentation.", "Slide artwork is rasterised at the chosen resolution."],
        options: &["layout", "perPage", "dpi", "slides", "includeHidden", "textLayer", "frame", "paper", "orientation", "color"],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "png",
        label: "PNG Picture",
        extension: "png",
        kind: FormatKind::Export,
        description: "Lossless pictures of one slide or all of them, optionally transparent.",
        keeps: "Every pixel of the slide at the chosen scale.",
        limitations: &["One picture per slide; no notes, links or animation."],
        options: &["slide", "all", "scale", "transparentBackground"],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "jpeg",
        label: "JPEG Picture",
        extension: "jpg",
        kind: FormatKind::Export,
        description: "Small, widely compatible pictures of one slide or all of them.",
        keeps: "The slide's appearance at the chosen scale.",
        limitations: &["Lossy compression.", "No transparency.", "One picture per slide."],
        options: &["slide", "all", "scale"],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "webp",
        label: "WebP Picture",
        extension: "webp",
        kind: FormatKind::Export,
        description: "A compact picture with transparency, for the web.",
        keeps: "The slide's appearance, including a transparent background.",
        limitations: &["One picture per slide; no notes, links or animation."],
        options: &["slide", "all", "scale", "transparentBackground"],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "gif",
        label: "Animated GIF",
        extension: "gif",
        kind: FormatKind::Export,
        description: "A looping animation with one frame per slide.",
        keeps: "Each slide as it looks when its builds have finished.",
        limitations: &["256 colours per frame.", "Transitions and build animations are not animated."],
        options: &["width", "secondsPerSlide", "loop", "slides", "includeHidden"],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "mp4",
        label: "MP4 Video",
        extension: "mp4",
        kind: FormatKind::Export,
        description: "H.264 video of the presentation, with transitions, animations and audio.",
        keeps: "Slide timings, transitions, build animations and the audio of autoplay media.",
        limitations: &[
            "Needs the desktop app and an ffmpeg build with libx264 (or the platform encoder).",
            "Embedded video pictures are not composited into the video; their audio is mixed.",
        ],
        options: &[
            "width",
            "height",
            "fps",
            "secondsPerSlide",
            "slides",
            "includeHidden",
            "useTimings",
            "transitions",
            "animations",
            "audio",
            "quality",
        ],
        availability: Availability::Desktop,
    },
    FormatSpec {
        id: "webm",
        label: "WebM Video",
        extension: "webm",
        kind: FormatKind::Export,
        description: "VP9/Opus video for the web, with the same timeline as MP4.",
        keeps: "Slide timings, transitions, build animations and the audio of autoplay media.",
        limitations: &["Needs the desktop app and an ffmpeg build with libvpx-vp9 and libopus.", "Embedded video pictures are not composited."],
        options: &[
            "width",
            "height",
            "fps",
            "secondsPerSlide",
            "slides",
            "includeHidden",
            "useTimings",
            "transitions",
            "animations",
            "audio",
            "quality",
        ],
        availability: Availability::Desktop,
    },
    FormatSpec {
        id: "outline",
        label: "Outline",
        extension: "txt",
        kind: FormatKind::Export,
        description: "The slide titles and bullet text as plain text, one slide after another.",
        keeps: "Title and body text, indentation and notes headings.",
        limitations: &["No formatting, pictures, tables or layout."],
        options: &[],
        availability: Availability::Everywhere,
    },
    FormatSpec {
        id: "markdown",
        label: "Markdown Outline",
        extension: "md",
        kind: FormatKind::Export,
        description: "The presentation as a Markdown document that reads well in any editor.",
        keeps: "Titles, bullets, notes and slide breaks.",
        limitations: &["No formatting, pictures, tables or layout."],
        options: &[],
        availability: Availability::Everywhere,
    },
];

/// The format with `id` (case-insensitive).
pub fn by_id(id: &str) -> Option<&'static FormatSpec> {
    FORMATS.iter().find(|f| f.id.eq_ignore_ascii_case(id.trim()))
}

/// The format whose extension is `ext` (case-insensitive, with or without the dot).
pub fn by_extension(ext: &str) -> Option<&'static FormatSpec> {
    let ext = ext.trim().trim_start_matches('.');
    FORMATS.iter().find(|f| f.extension.eq_ignore_ascii_case(ext) || (f.id == "jpeg" && ext.eq_ignore_ascii_case("jpeg")))
}

/// The format a path names by its extension.
pub fn for_path(path: &str) -> Option<&'static FormatSpec> {
    std::path::Path::new(path).extension().and_then(|e| e.to_str()).and_then(by_extension)
}

/// The suggested file name for `title` in `format`: the extension is the format's, never the
/// user's guess, and characters a file system dislikes are replaced.
pub fn suggested_name(title: &str, format: &FormatSpec) -> String {
    let mut stem: String = title
        .trim()
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .take(120)
        .collect();
    if stem.trim().is_empty() {
        stem = "Presentation".into();
    }
    format!("{stem}.{}", format.extension)
}

/// The formats whose label, description or extension contains `query` (empty matches all).
pub fn matching(query: &str) -> Vec<&'static FormatSpec> {
    let q = query.trim().to_lowercase();
    FORMATS
        .iter()
        .filter(|f| {
            q.is_empty()
                || f.label.to_lowercase().contains(&q)
                || f.id.contains(&q)
                || f.extension.contains(&q)
                || f.description.to_lowercase().contains(&q)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_is_findable_by_id_and_extension_and_has_a_unique_id() {
        let mut ids = std::collections::HashSet::new();
        for f in FORMATS {
            assert!(ids.insert(f.id), "duplicate format id {}", f.id);
            assert_eq!(by_id(f.id).map(|x| x.id), Some(f.id));
            assert_eq!(by_extension(f.extension).map(|x| x.id), Some(f.id));
            assert!(!f.label.is_empty() && !f.description.is_empty());
            assert!(!f.extension.starts_with('.'), "{} has a dotted extension", f.id);
        }
        assert_eq!(by_id("JPEG").map(|f| f.id), Some("jpeg"));
        assert_eq!(by_extension(".jpeg").map(|f| f.id), Some("jpeg"));
        assert_eq!(for_path("talk.PPTX").map(|f| f.id), Some("pptx"));
        assert!(for_path("talk.odp").is_none());
    }

    #[test]
    fn editable_formats_are_listed_before_copies_and_each_has_its_own_kind() {
        let first_export = FORMATS.iter().position(|f| f.kind == FormatKind::Export).unwrap_or(FORMATS.len());
        assert!(FORMATS[..first_export].iter().all(|f| f.kind == FormatKind::Editable));
        assert!(FORMATS[first_export..].iter().all(|f| f.kind == FormatKind::Export));
        for id in ["deckcraft", "pptx", "potx", "ppsx"] {
            assert_eq!(by_id(id).map(|f| f.kind), Some(FormatKind::Editable), "{id}");
        }
        for id in ["pdf", "png", "jpeg", "webp", "gif", "mp4", "webm", "outline", "markdown"] {
            assert_eq!(by_id(id).map(|f| f.kind), Some(FormatKind::Export), "{id}");
        }
    }

    #[test]
    fn suggested_names_replace_hostile_characters_and_always_use_the_format_extension() {
        assert_eq!(suggested_name("Quarterly report", by_id("pptx").unwrap()), "Quarterly report.pptx");
        assert_eq!(suggested_name("a/b:c", by_id("pdf").unwrap()), "a_b_c.pdf");
        assert_eq!(suggested_name("   ", by_id("png").unwrap()), "Presentation.png");
        assert_eq!(suggested_name("x.odp", by_id("deckcraft").unwrap()), "x.odp.deckcraft");
        assert!(suggested_name(&"a".repeat(500), by_id("mp4").unwrap()).chars().count() <= 130);
    }

    #[test]
    fn search_finds_formats_by_name_kind_and_extension() {
        assert_eq!(matching("video").iter().map(|f| f.id).collect::<Vec<_>>(), vec!["mp4", "webm"]);
        assert!(matching("gif").iter().any(|f| f.id == "gif"));
        assert!(matching("").len() == FORMATS.len());
        assert!(matching("no such format").is_empty());
    }

    #[test]
    fn video_is_marked_desktop_only_and_says_it_needs_ffmpeg() {
        for id in ["mp4", "webm"] {
            let f = by_id(id).unwrap();
            assert_eq!(f.availability, Availability::Desktop);
            assert!(f.limitations.iter().any(|l| l.contains("ffmpeg")));
        }
    }
}
