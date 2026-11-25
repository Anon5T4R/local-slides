# AGENTS.md — Local Slides

Fork independente do deckcraft (DeckCraft, ArtCraft), rebrandado para a suíte Local do João.
Rust puro + egui. Sem conta, sem telemetria, sem dependência de servidores de terceiros.

## Regras
- **Dependências:** toda dep que entra/sai/atualiza exige `Cargo.lock` commitado +
  linha no inventário do repo `Styx-Studios/local-deps` (regra completa em
  `local-deps/WORKFLOW.md`). PR sem isso não mergea.
- **Sem fontes git:** nenhuma dependência pode apontar para `git+` (o CI valida).
  Tudo vem do crates.io (pinado pelo lock) ou de `libs/`/`vendor/` (vendored nos
  revs exatos que o upstream pina).
- **Commits:** português, sem Co-Authored-By.
- **Release:** tag `vX.Y.Z` → o Actions builda Windows + Linux e publica. Mínimo local.
- **Disco:** no workspace do João vale a política de `target/` do dev-notes (`MEMORY.md` §4 — limpa o que
  está frio ou feito, mantém durante iteração ativa); fora dele, `cargo clean` ao terminar. Builds pesados são do CI.
- **Lints:** crate nova do workspace leva `[lints] workspace = true` no `Cargo.toml`; a raiz do crate de produção declara os atributos de lint exigidos antes dos módulos (doc `//!` pode vir antes).

## Estrutura
- `crates/` — engine e módulos (`localslides-*`), em camadas L0–L7 (`cargo xtask layers`)
- `apps/localslides` — app desktop + control channel; `apps/localslides-cli` — CLI/MCP
- `libs/` — forks vendored: `craft-libs` (c5a1cbf, com `vendor/krilla`), `egui` (6d518fc),
  `winit` (a517b38)
- `packaging/` — Windows (WiX/MSI per-machine e per-user), Linux (AppImage/deb/rpm/tar FHS com `install.sh`), Arch (PKGBUILD),
  macOS, FreeBSD
- `scripts/check-mirror.sh` — confere as crates e os assets (`craft-fonts`) espelhados do `local-libs` contra a fonte (CI e `xtask ci`)
- `xtask/` — tarefas de build/CI (`cargo xtask ci`, `fmt`, `layers`, `bench`, `assets`)

## Agent-native (MCP)
`crates/mcp` (servidor stdio + backends headless/`Remote` do control channel, porta com token) e
`localslides-cli mcp` (headless ou `--connect PORT|HOST:PORT`, loopback only). A crate compartilhada
da suíte é **`craft-mcp`** (fonte `local-libs`, espelho em `libs/craft-libs/crates/mcp`), usada aqui
no `--connect`; `run --script` (alias de `--cmd-file`) aceita JSON Lines ou um array de passos;
`commands --search TEXT` filtra o catálogo. As flags `--automation-read-root/write-root` são
repetíveis (multi-root, como no deckcraft). Docs: `docs/mcp.md`.

## Build
```
cargo build --locked          # debug
cargo test --locked           # testes (no Windows: -- --test-threads=1, testes de UI); as fontes vendored entram sempre (.cargo/config.toml), igual ao CI
cargo xtask ci                # o gate local: fmt, pdf-deps, clippy, testes, assets, camadas, cargo-deny, espelhos do local-libs
packaging/windows/package.ps1 # MSI + portable (Windows)
packaging/linux/package.sh    # AppImage/deb/rpm/tar (Linux)
```

Regras técnicas do código (never-crash, camadas, tudo é comando, política de assets):
`docs/architecture.md`; a sincronização com o upstream (o que entra e o que fica de fora):
`docs/UPSTREAM-SYNC.md`. Estado e fila: `ROADMAP.md`.


## Upstream e a pasta `C:\Dev\storytold`

Menções a `storytold/*` em código, comentários, docs e atribuições (NOTICE/ATTRIBUTION) são referência histórica e obrigação de licença. **Nada** é lido de `C:\Dev\storytold` em build, teste ou release: tudo vem do próprio repo (crates.io pinado pelo lock + `libs/`/`vendor/` vendored + `assets/craft-fonts` vendored). Essa pasta existe somente como referência local para a skill `sync-upstream`.
