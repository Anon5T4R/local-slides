# Local Slides architecture

> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-18 · **Change:** minor (native chart payloads stored in bounded ZIP members; previous: minor, community PR wave 2026-10-14: 320 engine commands, 41 dialogs, the `metafile` crate, 1,641 tests; duplicated L2/L5/L6 rows and headers folded; previous: minor, transition frames) · **Target:** Microsoft PowerPoint for Mac 16.113 (Microsoft 365)

How Local Slides is built today. The binding rules (layering, everything is a command, thin UI, never
break wasm, never crash) are in [AGENTS.md](../AGENTS.md) §3–§5; this document describes what
exists. 202k lines of Rust in 24 crates, 3 apps and `xtask`; 1,641 `#[test]` functions
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
| L5 | `engine` | Sessions, documents, selection, undo history on shared slide snapshots, the command registry (320 commands in `cmd/*`), pointer tools, keys, AutoRecover, the panic guard (`guard.rs`) |
| L6 | `ui-egui`, `mcp` | The PowerPoint-style interface (ribbon with 10 tabs + contextual tabs, canvas, thumbnails, panes, 41 dialogs, show and presenter windows, the JSON control channel); the MCP server (JSON-RPC 2.0 over stdio), headless or connected to a running app |
| apps | `localslides` (desktop: eframe/wgpu, cpal audio, logging), `localslides-cli` (run, render, convert, mcp, app), `localslides-web` (trunk; WebGPU with WebGL2 fallback), `xtask` | Exempt from layering |

The media codecs and containers are clean-room pure Rust, 59k lines, shared in spirit with
FilmCraft.

## Data model

A `Presentation` holds a theme set, slide masters with their layouts, slides, sections, custom
shows, comments, media and document properties. Shapes are a tree (`ShapeKind`: preset or custom
geometry, picture, table, chart, group, connector, ink, media, SmartArt-as-group, and `Opaque` for
anything read from PPTX that Local Slides doesn't model: its XML is kept and written back). Text
bodies hold paragraphs of runs with DrawingML properties; formatting resolves slide → layout →
master → theme at render time. Transitions live on slides; animations are the flat ordered list the
Animation Pane shows, rebuilt into PowerPoint's timing tree on save.

## Commands and frontends

Every user-visible action is a `CommandSpec` (id, label, ribbon/menu place, shortcut, params doc,
`enabled`, `run`) in `crates/engine/src/cmd/*`. The UI, `localslides-cli run`, the JSON control
channel (`localslides --control PORT`, [control-protocol.md](control-protocol.md)) and MCP
([mcp.md](mcp.md)) all dispatch through `Session::execute`, so they behave identically.
Programmatic calls never open dialogs. Pointer gestures go through `Session::pointer`, keys through
`Session::key`.

## Rendering and the show

The renderer draws a slide to an RGBA pixmap on the CPU; the UI uploads it as an egui texture.
Transitions are layers of textured quads over the old and new slide images, or over their
backgrounds and contents rendered apart (Dynamic Content), from `anim::transition_frame`: 3-D
effects project cards through a perspective camera and `anim::layer_mesh` subdivides them so
textures stay straight; soft edges are per-corner opacities. The show renders the slides once per
transition and only composites cached textures each frame; `render::composite_transition` is the
CPU twin used by `localslides-cli render-transition` and the tests. The show window and the presenter view are egui viewports; animation state per shape
comes from `anim::timeline` each frame. Animations ▸ Preview plays that same state on the editor
canvas for the current slide, then draws the slide at rest. PDF export renders each slide at print resolution and
overlays an invisible real-text layer with embedded font subsets.

## File I/O

- **PPTX** (`crates/pptx`): lenient reader (unknown parts skipped, malformed parts logged and
  ignored, sizes and depths bounded, zip bombs capped), writer that PowerPoint opens without
  repair. Unknown elements round-trip through `Opaque` and `raw_ext` fields.
- **Native** (`crates/format`): zip with `mimetype`, `presentation.json`, `media/<id>` and
  `chart-parts/<index>`. Chart relationships and payload references stay in the manifest; binary
  chart parts use bounded ZIP reads and share the media byte/item budgets. Earlier inline chart
  bytes still load and migrate on save; `.slidecraft` files from before the rename still open.

## Quality gates

`cargo xtask ci`: rustfmt check, clippy `-D warnings`, tests, layering, asset attribution
(`cargo xtask assets`), wasm32 check. CI runs it on Ubuntu for every PR and push to `main`;
macOS is built and tested by the release workflow only. Releases publish signed Windows and Linux
artifacts; the other packagers in `packaging/` are run by hand.
The PDF writer dependency enables only `write`. Krilla handles serialization and simple-text
shaping for notes and footers; slide layout, rasterization, font selection, text mapping,
links and outlines remain app-owned. PDF embedding and all Hayro production dependencies
are disabled. Native builds enable parallel compression on the same pinned Krilla backend;
WASM keeps it disabled. Generated-PDF assertions use a std-only parser as a development dependency.
