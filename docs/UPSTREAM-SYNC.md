# Upstream sync

The Local suite's port/rebrand: `main` is ours, the upstream is a source of candidate changes and
nothing enters without review. General rules in `docs/planos/licoes-de-porta.md` (dev-notes); the
map of paths the script can apply lives in `docs/UPSTREAM-MAP.toml`.

## Base

- Upstream: `storytold/deckcraft`, commit **9bac60ff** (`v0.15.1`, "Release 0.15.1") — the tree was
  copied from the commit (Phase A, 2026-11-05), with no history.
- Fork: `Styx-Studios/Local-Slides` (repo `local-slides`), own version `0.3.x` (a new numbering avoids
  a false auto-update to the upstream).
- Native `.deckcraft` format kept byte for byte, including the MIMETYPE
  `application/vnd.storyteller.deckcraft+zip`.
- Nothing is read from `C:\Dev\storytold` in build, test or release; the folder is only a local
  reference for the `sync-upstream` skill.

## Vendored (outside the normal sync)

| Path | Origin | Rev |
|---|---|---|
| `libs/craft-libs` | `storytold/craft-libs` | `c5a1cbf` (with `vendor/krilla`) |
| `libs/craft-libs/crates/update` | **`Styx-Studios/local-libs`** (the suite's single source) | mirror from `sync.ps1` |
| `libs/local-ai` | `underworld/local-ai` (the suite's BYOK) | single source |
| `libs/egui` | `storytold/egui` | `6d518fc` |
| `libs/winit` | `storytold/winit` | `a517b38` |
| `libs/rustls-rustcrypto` | the suite's fork (100% Rust TLS provider) | `0.0.2-alpha` |
| `assets/craft-fonts` | `storytold/craft-fonts` (build input `CRAFT_FONTS_DIR`) | vendored (no rev pinned in the app) |

`craft-update` is a **conscious permanent fork** and stays outside the sync: bugfixes go straight
into the source (`local-libs`) and are replicated with `sync.ps1 -Mode push` (the copy here is a
mirror). Never edit `libs/craft-libs/crates/update` by hand.

## Rebrand divergences (what is ours)

- Identity: crates `localslides-*`, binaries `localslides`/`localslides-cli`, app id
  `com.local.localslides` (desktop entry), Arch package `ai.storyteller.localslides`, MSI with its
  own `UpgradeCode`/`Manufacturer`, configuration folder `LocalSlides`.
- Own CI: no `craft-common-actions`, no sonarcloud; release by **tag** `vX.Y.Z` (the upstream uses
  the `release` branch + draft).
- `libs/` and `assets/craft-fonts/` vendored (the upstream fetches both by git in CI).
- Community and project links point only to `Styx-Studios/Local-Slides`
  (`crates/engine/src/links.rs`).
- **Windows:** one native window per presentation since the `port-multiwindow` branch
  (`crates/ui-egui/src/windows.rs`, driver `craft_window::Viewports`; the upstream's `doc_window.rs`
  viewport was removed). The upstream uses a single window with
  `session.set_active` + `window.next`. **Sync rule:** commits that touch
  `crates/ui-egui/src/app.rs`, `windows.rs`, `backstage/`, `menus.rs`, `app/files.rs`, `control.rs`
  or `doc_window.rs` conflict by construction — reconcile keeping the app's window layer (the
  app's layer wins). Upstream behavior changes in the *engine* (`Session`, per-doc selection, undo,
  recovery) come in clean. In the vendored `craft-libs`,
  `crates/window/src/viewports.rs` carries the ghost-tile patch (ported from Local Word's
  `715a06d5` — `monitor_bounds` by the center) that rev `c5a1cbf` does not have; never revert it.
- About: a single Close button, no Cancel (the upstream had both).

## Sync log

The base stays `9bac60ff`: only chosen points enter (skill `sync-upstream`), not a bump.

| Date | Target (deckcraft) | Taken | Relevant discards |
|---|---|---|---|
| 2026-11-05 | `9bac60ff` (v0.15.1) | baseline: whole tree imported (Phase A) | `.github/`, `sonar-project.properties`, `.coderabbit.yaml`, `omastore.toml`, `docs/brand/`, each OS's packaging rewritten in the suite's mold |
| 2026-11-21 | the day's `origin/main` | `49447f46` (#2083) → `88f01f8` (text effects from `defRPr`/`defaultTextStyle` enter the render filters); `e2ca2c41` (#1942) → `cb115d7` (`p:video`/`p:audio` from the timing: deferred autoplay on read, options recorded and round-tripped) | `44b133a3` and `67c1d789` (PPTX text overflow) were already in the base, record only; the `docs/file-format-parity.md` hunk of #1942 does not apply (the doc does not exist in this repo) |
