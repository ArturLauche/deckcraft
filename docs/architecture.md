# DeckCraft architecture

> **Last reviewed:** 2026-10-19 · **Last updated:** 2026-10-19 · **Change:** minor (live collaboration over a shared folder or a self-hosted TCP relay, with presence, cursors, role enforcement and snapshot undo; previous: video export, the format registry, share links and settings sync) · **Target:** Microsoft PowerPoint for Mac 16.113 (Microsoft 365)

How DeckCraft is built today. The binding rules (layering, everything is a command, thin UI, never
break wasm, never crash) are in [AGENTS.md](../AGENTS.md) §3–§5; this document describes what
exists. 216k lines of Rust in 24 crates, 3 apps and `xtask`; 1,812 `#[test]` functions
(`cargo xtask stats` counts them per crate).

## Layers

`cargo xtask layers` enforces that a crate depends only on lower layers (the table in
`xtask/src/layers.rs`, which is authoritative; AGENTS.md §4 gives the simplified picture), and that
nothing below L6 uses egui, eframe, winit or rfd.

| Layer | Crates | What they do |
|---|---|---|
| L0 | `geom`, `color`, `metafile`; standalone `bitstream`, `matroska`, `ogg`, `opus` | EMU/pt units, transforms, ~180 preset shape generators re-derived from ECMA-376, adjust handles, hit testing; RGB/HSL and DrawingML colour transforms; MKV/WebM and Ogg demuxers, the Opus decoder; EMF/WMF read into a neutral vector model (`metafile`) |
| L1 | `model`, `fonts`; `h264`, `hevc`, `vp9`, `av1`, `isobmff` | Presentation, masters, layouts, slides, shapes, text, themes, tables, charts, transitions, animations, placeholder inheritance (`resolve`); font database (craft-fonts + system) with metric-compatible substitutes for Office fonts (Carlito for Calibri, Caladea for Cambria, Liberation for Arial/Times); video decoders and the MP4/MOV demuxer |
| L2 | `text`, `anim`, `format`, `media` | Shaping (HarfRust), UAX #9 bidi, UAX #14 line breaking, autofit, caret hit testing; per-shape animation state over time, the show state machine, transition frame layouts, Morph matching; `.deckcraft` (zip + JSON + media) and outline text; media probing, decoding, mixer and clock |
| L3 | `render`, `pptx` | `vello_cpu` raster of slides, masters and layouts (fills, outlines, effects, pictures, tables, charts, Morph frames); PPTX import/export from ECMA-376 |
| L4 | `pdf` | App-owned export layout and rendering through the shared `craft-pdf` writer |
| L5 | `engine` | Sessions, documents, selection, undo history on shared slide snapshots, the command registry (`cmd/*`), pointer tools, keys, AutoRecover, the panic guard (`guard.rs`), the format registry (`formats.rs`), video timelines (`video_export.rs`), share links and access (`sharing.rs`, `cmd/share.rs`), settings sync (`sync.rs`) and live collaboration over a folder or TCP relay (`collab.rs`, `collab/relay.rs`, `cmd/collab.rs`) |
| L6 | `ui-egui`, `mcp` | The PowerPoint-style interface (ribbon with 10 tabs + contextual tabs, canvas, thumbnails, panes, 41 dialogs, show and presenter windows, the JSON control channel); the MCP server (JSON-RPC 2.0 over stdio), headless or connected to a running app |
| apps | `deckcraft` (desktop: eframe/wgpu, cpal audio, logging), `deckcraft-cli` (run, render, convert, mcp, app), `deckcraft-web` (trunk; WebGPU with WebGL2 fallback), `xtask` | Exempt from layering |

The media codecs and containers are clean-room pure Rust, 59k lines, shared in spirit with
FilmCraft.

## Data model

A `Presentation` holds a theme set, slide masters with their layouts, slides, sections, custom
shows, comments, media and document properties. Shapes are a tree (`ShapeKind`: preset or custom
geometry, picture, table, chart, group, connector, ink, media, SmartArt-as-group, and `Opaque` for
anything read from PPTX that DeckCraft doesn't model: its XML is kept and written back). Text
bodies hold paragraphs of runs with DrawingML properties; formatting resolves slide → layout →
master → theme at render time. Transitions live on slides; animations are the flat ordered list the
Animation Pane shows, rebuilt into PowerPoint's timing tree on save.

## Commands and frontends

Every user-visible action is a `CommandSpec` (id, label, ribbon/menu place, shortcut, params doc,
`enabled`, `run`) in `crates/engine/src/cmd/*`. The UI, `deckcraft-cli run`, the JSON control
channel (`deckcraft --control PORT`, [control-protocol.md](control-protocol.md)) and MCP
([mcp.md](mcp.md)) all dispatch through `Session::execute`, so they behave identically.
Programmatic calls never open dialogs. Pointer gestures go through `Session::pointer`, keys through
`Session::key`.

## Sharing, access and settings sync

A share is a capability token in a sidecar beside the deck (`talk.deckcraft` → `talk.share.json`),
never inside the presentation. Each link carries a permission (`view`, `comment`, `edit`), an
optional expiry and a revocation flag; `share.open` loads the deck and sets the session's `Access`.
`Session::execute` rejects mutating commands for restricted sessions, and `Session::pointer`
rejects every press, drag and double-click, because pointer gestures edit the document directly.
Commenters may run `comment.*`; viewers may navigate, show, print and export. `file.saveAs` to a
different path is allowed and makes the copy a full-access document, so "save a copy to edit" works
without letting a viewer overwrite the shared file.

Settings sync is opt-in and local-first. `sync::Snapshot` holds only the categories the user
selected, with a `modified` time per category. A merge is last-writer-wins by category, ties broken
by device id, and refuses a remote whose revision is older than the last one seen. Transports are a
local folder, S3-compatible storage (signed through `curl --aws-sigv4`) and Google Drive (OAuth
refresh token through `curl`); the engine owns the merge and the UI worker owns the thread and the
progress state. Credentials live in an owner-only config file (`0600` on Unix) and never in a
snapshot. Encryption is opt-in: the snapshot is encrypted with AES-256-GCM under an Argon2id key
from a passphrase, in an envelope with a random salt and nonce; a plain remote is re-uploaded
encrypted when the user turns it on, and a wrong passphrase or altered ciphertext is an explicit
error. The passphrase lives in the same owner-only config so automatic sync can run.

## Live collaboration

`collab.rs` is a journal protocol over a small `Store` abstraction. Every peer appends commands to
`journal-<peer>.jsonl`, writes presence and its pointer to `peer-<peer>.json`, and reads the other
peers' files; `snapshot.json` is the full document at a known revision. `collab.start` writes the
first snapshot; `collab.join` adopts the latest one and tails the journals. The folder store is the
offline default: a shared drive, a Syncthing folder or a network share is enough. The relay store
speaks the same operations over newline-delimited JSON to `deckcraft-cli relay`, which keeps one
room directory per token and bounds request and file sizes. The UI polls every 150 ms in both
cases.

Each journal entry carries a sequence, timestamp, peer id, role and the normal command id and
params. Peers apply entries through `Session::execute_remote`, which skips the journal echo and the
local undo stack; the same command code, panic guard and access checks run as for a local edit.
Entries are ordered by `(time, peer, seq)`. A snapshot records the highest sequence it contains for
each peer, so a peer whose clock is behind still gets new entries applied. `collab.start` / `join`
take an editor, commenter or viewer role; the role is enforced before a local command and again before a remote entry is
applied. Undo, redo and repeat publish a new snapshot instead of an inverse command, so every peer
converges on the actor's document and starts a fresh history. A peer whose command fails writes a
recovery request; the next editor publishes a snapshot and the others reload it. Presence files
carry the cursor's slide and normalized position; the canvas draws remote pointers in the peer's
colour.

The limits are explicit: two edits to the same text at the same instant are ordered by timestamp
(the later command wins), not merged by a CRDT. A folder's filesystem permissions are its outer
security boundary; a relay instead requires the room token on every request, but speaks plain TCP,
so a public deployment needs TLS at a reverse proxy and a per-user identity is still open.
`docs/gaps.md` tracks those remaining pieces.

## Video export

`video_export::plan` turns a deck and its options into a timeline of frames: slide durations from
`secondsPerSlide` or rehearsed `advance_after_ms`, transitions from `anim::transition_frame`, and
build animations from `anim::Timeline`. The same `render::render_slide` output feeds each frame, so
the video matches the editor. Frames are converted to opaque RGBA and piped to `ffmpeg`; MP4 uses
H.264/AAC, WebM uses VP9/Opus. Autoplay media audio is mixed from the media decoder. The UI runs the
job on a worker thread with progress and Cancel and encodes a snapshot of the deck, so editing
while it runs cannot change the output. There is no bundled encoder: a missing `ffmpeg` is a clear
error.

## Rendering and the show

The renderer draws a slide to an RGBA pixmap on the CPU; the UI uploads it as an egui texture.
Transitions are layers of textured quads over the old and new slide images, or over their
backgrounds and contents rendered apart (Dynamic Content), from `anim::transition_frame`: 3-D
effects project cards through a perspective camera and `anim::layer_mesh` subdivides them so
textures stay straight; soft edges are per-corner opacities. The show renders the slides once per
transition and only composites cached textures each frame; `render::composite_transition` is the
CPU twin used by `deckcraft-cli render-transition` and the tests. The show window and the presenter view are egui viewports; animation state per shape
comes from `anim::timeline` each frame. Animations ▸ Preview plays that same state on the editor
canvas for the current slide, then draws the slide at rest. PDF export renders each slide at print resolution and
overlays an invisible real-text layer with embedded font subsets. Print shares that writer, with
margins, scaling, custom header/footer, page numbers and optional fit-to-paper; the desktop app
opens the PDF or, on Unix with CUPS, sends it directly with `lp` (the path is a process argument,
never a shell string).

## File I/O

- **PPTX** (`crates/pptx`): lenient reader (unknown parts skipped, malformed parts logged and
  ignored, sizes and depths bounded, zip bombs capped), writer that PowerPoint opens without
  repair. Unknown elements round-trip through `Opaque` and `raw_ext` fields.
- **Native** (`crates/format`): zip with `mimetype`, `presentation.json` and `media/<id>`; readers
  ignore unknown fields; `.slidecraft` files from before the rename still open.
- **Format registry** (`crates/engine/src/formats.rs`): the one list behind Save As, Export, the
  CLI and MCP. It separates editable formats (`.deckcraft`, `.pptx`, `.potx`, `.ppsx`) from copies
  (PDF, PNG, JPEG, WebP, GIF, MP4, WebM, TXT, MD), names each one's losses and options, and marks
  desktop-only writers. Save As rejects a copy format; Export writes one. Paths are normalized to
  the chosen format's extension and every write goes through `crate::io`, which enforces the
  automation roots and writes through a temporary sibling.
- **Video** (`crates/engine/src/video_export.rs`): a timeline of rendered frames piped to the
  system `ffmpeg`; the UI worker is `crates/ui-egui/src/videojob.rs`.
- Formats and their gaps: [file-format-parity.md](file-format-parity.md).

## Quality gates

`cargo xtask ci`: rustfmt check, clippy `-D warnings`, tests, layering, asset attribution
(`cargo xtask assets`), wasm32 check. CI runs it on Ubuntu for every PR and push to `main`;
macOS is built and tested by the release workflow only. Releases (pushes to `release`) build signed
macOS, Windows, Linux, FreeBSD and web artifacts.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-19 | minor | Live collaboration runs on a `Store` abstraction: shared folder or self-hosted TCP relay with room tokens and persistence. 333 commands, 216k lines, 1,812 tests |
| 2026-10-19 | minor | Optional AES-256-GCM/Argon2id encryption for settings sync, with plaintext migration |
| 2026-10-19 | minor | Shared-folder live collaboration: ordered journals, role enforcement, presence/cursors, snapshot undo and recovery. 333 commands, 213k lines, 1,796 tests |
| 2026-10-19 | minor | Format registry, video export worker, share links with engine-level access enforcement, and opt-in settings sync (folder/S3/Drive). 328 commands, 212k lines, 1,786 tests |
| 2026-10-14 | minor | Community PR wave folded: one L2/L5/L6 row each; command registry 320 (`cmd!` registrations in `cmd/*`), 41 dialogs (titled in `dialogs.rs`), 202k lines, 24 crates (`metafile` added at L0), 1,641 tests. The per-PR rows below give the counts each PR saw on its own branch |
| 2026-10-14 | minor | Size and Position dialog (`shape.sizePosition`): position, size, rotation, scale and lock aspect in one undo step. 227 commands, 28 dialogs |
| 2026-10-14 | trivial | Command registry: 228 (`comment.next`, `comment.prev`) |
| 2026-10-14 | trivial | Photo Album dialog and `insert.photoAlbum` (227 commands, 28 dialogs) |
| 2026-10-14 | trivial | Edit Photo Album (`insert.photoAlbumEdit`) reopens the Photo Album dialog and re-flows those slides (228 commands, 28 dialogs) |
| 2026-10-14 | minor | Create New Theme Colors (`design.colors` with no params); command count stays 226 |
| 2026-10-14 | minor | Create New Theme Fonts (`design.fonts` with no params); command count stays 226 |
| 2026-10-14 | minor | Insert › Date and Time dialog (`insert.dateTime` with no keys). 28 dialogs; command count stays 226 |
| 2026-10-14 | minor | Command registry is 227 (`design.effects`). Dialogs stay 27. The test-count line stays 504 |
| 2026-10-14 | minor | Command registry counted at 236, including `master.preserve` and `master.placeholders`. Dialogs stay 27. The test-count line stays 504. The menu measurement and the d556ff9 snapshot still count 226 |
| 2026-10-13 | trivial | Animation preview draws the current slide's timeline on the editor canvas |
| 2026-10-14 | minor | Paragraph and Tabs dialogs; command registry 229; 29 dialogs |
| 2026-10-14 | minor | Font dialog; command registry 227; 28 dialogs |
| 2026-10-13 | minor | Transitions: perspective meshes, background/content parts, CPU compositor |
| 2026-10-10 | major | Created from the code on main (d556ff9) and AGENTS.md §4 |

The PDF writer dependency enables only `write`. Krilla handles serialization and simple-text
shaping for notes and footers; slide layout, rasterization, font selection, text mapping,
links and outlines remain app-owned. PDF embedding and all Hayro production dependencies
are disabled. Native builds enable parallel compression on the same pinned Krilla backend;
WASM keeps it disabled. Generated-PDF assertions use a std-only parser as a development dependency.
