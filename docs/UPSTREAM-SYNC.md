# Sincronização com o upstream

Porta/rebrand da suíte Local: o `main` é nosso, o upstream é fonte de mudanças candidatas e nada
entra sem revisão. Regras gerais em `docs/planos/licoes-de-porta.md` (dev-notes); o mapa de paths
que o script pode aplicar fica em `docs/UPSTREAM-MAP.toml`.

## Base

- Upstream: `storytold/deckcraft`, commit **9bac60ff** (`v0.15.1`, "Release 0.15.1") — a árvore foi
  copiada do commit (Fase A, 2026-11-05), sem histórico.
- Fork: `Anon5T4R/Local-Slides` (repo `local-slides`), versão própria `0.3.x` (numeração nova evita
  auto-update falso para o upstream).
- Formato nativo `.deckcraft` mantido byte-a-byte, inclusive o MIMETYPE
  `application/vnd.storyteller.deckcraft+zip`.
- Nada é lido de `C:\Dev\storytold` em build, teste ou release; a pasta é só referência local para
  a skill `sync-upstream`.

## Vendorizado (fora do sync normal)

| Caminho | Origem | Rev |
|---|---|---|
| `libs/craft-libs` | `storytold/craft-libs` | `c5a1cbf` (com `vendor/krilla`) |
| `libs/craft-libs/crates/update` | **`Anon5T4R/local-libs`** (fonte única da suíte) | espelho de `sync.ps1` |
| `libs/local-ai` | `underworld/local-ai` (BYOK da suíte) | fonte única |
| `libs/egui` | `storytold/egui` | `6d518fc` |
| `libs/winit` | `storytold/winit` | `a517b38` |
| `libs/rustls-rustcrypto` | fork da suíte (provider TLS 100% Rust) | `0.0.2-alpha` |
| `assets/craft-fonts` | `storytold/craft-fonts` (build input `CRAFT_FONTS_DIR`) | vendored (sem rev pinado no app) |

`craft-update` é **fork permanente consciente** e fica fora do sync: bugfix vai direto na fonte
(`local-libs`) e é replicado com `sync.ps1 -Mode push` (a cópia aqui é espelho). Nunca edite
`libs/craft-libs/crates/update` à mão.

## Divergências do rebrand (o que é nosso)

- Identidade: crates `localslides-*`, binários `localslides`/`localslides-cli`, app id
  `com.local.localslides` (desktop entry), pacote Arch `ai.storyteller.localslides`, MSI com
  `UpgradeCode`/`Manufacturer` próprios, pasta de configuração `LocalSlides`.
- CI próprio: sem `craft-common-actions`, sem sonarcloud; release por **tag** `vX.Y.Z` (o upstream
  usa a branch `release` + draft).
- `libs/` e `assets/craft-fonts/` vendorizados (o upstream busca os dois por git no CI).
- Links de comunidade e do projeto apontam só para `Anon5T4R/local-slides`
  (`crates/engine/src/links.rs`).
- **Janelas:** uma janela nativa por apresentação desde a branch `port-multiwindow`
  (`crates/ui-egui/src/windows.rs`, driver `craft_window::Viewports`; o visor `doc_window.rs` do
  upstream foi removido). O upstream usa janela única com
  `session.set_active` + `window.next`. **Regra de sync:** commits que toquem
  `crates/ui-egui/src/app.rs`, `windows.rs`, `backstage/`, `menus.rs`, `app/files.rs`, `control.rs`
  ou `doc_window.rs` conflitam por construção — reconciliar mantendo a camada de janelas do app (a
  camada do app vence). Mudança de comportamento do upstream na *engine* (`Session`, selection
  per-doc, undo, recovery) entra limpa. No `craft-libs` vendored, `crates/window/src/viewports.rs`
  carrega o patch do tile fantasma (porte do `715a06d5` do Local Word — `monitor_bounds` pelo
  centro) que o rev `c5a1cbf` não tem; nunca reverter.
- About: um só botão Close, sem Cancel (o upstream tinha os dois).

## Registro de sync

A base continua `9bac60ff`: só pontos escolhidos entram (skill `sync-upstream`), não um bump.

| Data | Alvo (deckcraft) | Pegos | Descartados relevantes |
|---|---|---|---|
| 2026-11-05 | `9bac60ff` (v0.15.1) | baseline: árvore inteira importada (Fase A) | `.github/`, `sonar-project.properties`, `.coderabbit.yaml`, `omastore.toml`, `docs/brand/`, packaging de cada SO reescrito no molde da suíte |
| 2026-11-21 | `origin/main` do dia | `49447f46` (#2083) → `88f01f8` (efeitos de texto do `defRPr`/`defaultTextStyle` entram nos filtros do render); `e2ca2c41` (#1942) → `cb115d7` (`p:video`/`p:audio` do timing: autoplay deferido na leitura, opções gravadas e round-trip) | `44b133a3` e `67c1d789` (overflow de texto PPTX) já estavam na base, só registro; o hunk de `docs/file-format-parity.md` do #1942 não se aplica (o doc não existe neste repo) |
