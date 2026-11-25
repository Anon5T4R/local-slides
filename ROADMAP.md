# ROADMAP — Local Slides

Fork of deckcraft (deckcraft v0.15.1), rebranded for the Local suite. Pure Rust + egui, desktop.
The native format is the upstream's `.deckcraft` (zip + JSON, kept byte for byte, with the MIME
`application/vnd.storyteller.deckcraft+zip`); PPTX comes in and goes out. How it is built:
`docs/architecture.md`; what comes from the upstream: `docs/UPSTREAM-SYNC.md`.

**Stage: alpha.** It opens, edits, presents and saves real-world decks; what is missing below is
the path to "ready for real work".

## What is missing (by area, in no order of difficulty)

- **PPTX fidelity.** The round-trip corpus is only decks we authored ourselves: checking against
  PowerPoint renderings and third-party decks is missing; media options still get lost in a
  round-trip.
- **Stability.** The app does not open on some Windows GPUs (Vulkan enumeration).
- **Images.** SVG does not render (EMF/WMF do); pasting an image on Windows fails in some cases.
- **Animations.** The panel covers part of PowerPoint's effects (timeline and Play From already
  exist).
- **Charts.** The data can be edited on the grid (Edit Data); editing the chart elements (title,
  axes, legend) is missing.
- **Formatting.** The Format Shape pane is leaner than PowerPoint's; fonts only in the Regular,
  Bold and Italic faces.
- **Printing.** It does not send to a printer. File > Print makes a PDF with the print options and
  opens it in the system's PDF viewer.
- **Deck video.** It exports animated GIF, not MP4/MOV.
- **Presenting.** The presenter view does not position itself on the second screen.
- **Performance.** The slides rasterize on the CPU, with no performance baseline.
- **Localization.** English, Portuguese (Brazil) and Spanish; the catalogs still have untranslated
  strings (`cargo xtask i18n-report`).

## Formats

| Format | Read | Write |
| --- | --- | --- |
| `.pptx` | yes | yes |
| `.potx` / `.ppsx` | yes (as a presentation) | yes, with the proper content type |
| `.deckcraft` (native; also `.slidecraft`) | yes | yes, losslessly |
| `.thmx` (Office theme) | colors, fonts and effects only (Design › Browse for Themes…) | no |
| Macro PPTX (`.pptm`/`.potm`/`.ppsm`), `.ppt` 97–2003, `.odp` | no | no |
| Outline `.txt`/`.rtf`/`.md` | yes (titles become slides, items become bullets) | `.txt`/`.rtf`/`.md` |
| PDF | — | yes (slides, notes, handouts); the slide art goes in rasterized |
| PNG/JPEG per slide | — | yes (with optional transparent background) |
| Animated GIF of the deck | — | yes (transitions and animations do not enter) |
| TIFF/BMP/GIF per slide, MP4/MOV | — | no |

## References

`docs/architecture.md` · `docs/control-protocol.md` · `docs/mcp.md` · `i18n/README.md` ·
`docs/UPSTREAM-SYNC.md`
