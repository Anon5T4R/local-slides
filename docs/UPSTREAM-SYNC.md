# Sincronização com o upstream (storytold/deckcraft)

## Objetivo
Manter o `local-slides` próximo do `storytold/deckcraft` sem perder a independência do rebranding:
nenhum merge cego, nenhuma dependência de servidor da org deles, nenhum `git+` no lock. Tudo que
entrar no main deste repo passa por revisão primeiro.

**Baseline:** deckcraft `v0.15.1` = `9bac60ff` (o commit importado na Fase A). Formato nativo
`.deckcraft` mantido byte-a-byte, inclusive o MIMETYPE `application/vnd.storyteller.deckcraft+zip`.

## Princípios
1. **O main é nosso.** O upstream é uma fonte de mudanças candidatas, nunca um destino automático.
2. **Revisão humana obrigatória.** Nada entra no `main` sem passar pela revisão de arquitetura.
3. **Mapeamento de renames é explícito.** Todo caminho que difere fica em `docs/UPSTREAM-MAP.toml`.
4. **Puxamos apenas o que é nosso.** Não puxamos branding, `docs/brand/`, links `getartcraft.com`,
   Discord, CI da org `storytold`, nem o packaging deles.
5. **Contribuímos de volta.** Bug compartilhado vira Issue/PR no `storytold/deckcraft` (em inglês),
   registrado em `docs/UPSTREAM-CONTRIBUTIONS.md` se/quando existir.
6. **Independência de conteúdo.** Feature que baixa conteúdo do servidor deles não entra como está:
   ou o conteúdo é embutido/`assets/`, ou o fetch tem checksum fixo, ou a feature fica de fora.
7. **Vendoring por rev.** `libs/craft-libs` fica no rev `c5a1cbf`, com `vendor/krilla`; `libs/egui`
   `6d518fc`; `libs/winit` `a517b38`. Um sync que precise de outro rev de craft-libs é decisão
   própria (lição §43: árvore no rev da maioria + crates pinados sobrepostos).

## O que puxar vs o que não puxar

| Categoria | Puxar? | Motivo / exemplo |
|-----------|--------|------------------|
| Correção de segurança (parser PPTX, mídia, links) | ✅ Sim | `crates/pptx/`, `crates/media/` |
| Correção de corrupção / perda de dados | ✅ Sim | `crates/format/`, `crates/engine/src/io.rs` |
| Crash / travamento | ✅ Sim | engine, UI, render |
| Performance crítica | ✅ Sim | layout, rasterização, export |
| Bug de paridade com PowerPoint | ✅ Sim | `crates/pptx/`, `crates/engine/src/cmd/` |
| Feature de UI / ribbon específica | ⚠️ Caso a caso | avaliar com o rebrand |
| Mudança de nomes / bundle id / marca | ❌ Não | é nosso rebrand; conflito garantido |
| Dependências novas / atualizações | ⚠️ Caso a caso | só via `local-deps/WORKFLOW.md` |
| CI / workflows da org storytold | ❌ Não | nosso CI é próprio (`.github/workflows/`) |
| Packaging | ❌ Não | `packaging/` é do molde da suíte |

## Fluxo de sincronização
1. `git remote add upstream https://github.com/storytold/deckcraft.git` (uma vez) e `git fetch upstream`.
2. `git checkout -b sync/upstream-<YYYYMMDD>` (nunca direto no `main`).
3. Listar os commits novos desde `9bac60ff` (ou o último sync registrado) e aplicar por arquivo,
   conforme `docs/UPSTREAM-MAP.toml`.
4. Testar: `cargo build --locked`, `cargo test --locked --no-fail-fast -- --test-threads=1`,
   `cargo clippy --locked --all-targets -- -D warnings`, `cargo xtask assets` e `cargo xtask layers`.
5. Revisão de arquitetura; merge no `main` só se tudo verde.
6. Registrar aqui: data, faixa de commits, o que entrou, o que ficou de fora e por quê.

## Automação
Pode: `git fetch`, listar commits, tentar `git apply` nos arquivos `strategy = "apply"`, rodar os
gates nos crates afetados, gerar relatório.
Não pode: resolver `Cargo.toml`, `packaging/`, `.github/`; adicionar/atualizar dependência; decidir
se uma feature entra; fazer merge; abrir PR no upstream.

## Registro de syncs

| Data | Upstream | O que entrou | O que ficou de fora |
|------|----------|--------------|---------------------|

## Divergências estruturais da camada do app (a camada do app SEMPRE vence)

### Modelo de janelas: uma janela nativa por apresentação (desde a branch `port-multiwindow`)
O upstream `deckcraft` usa uma janela única com múltiplas apresentações na sessão
(`session.set_active` + `window.next` para ciclar). O Local Slides usa **uma janela nativa por
apresentação**, no molde do Local Word (`crates/ui-egui/src/windows.rs`, driver
`craft_window::Viewports`; dep `craft-window` do craft-libs vendored). `View › New Window` abre
outra janela editável do mesmo documento (o visor read-only `doc_window.rs` do upstream foi
removido). No wasm continua uma janela só.

**Regra de sync:** commits upstream que tocarem `crates/ui-egui/src/app.rs`,
`crates/ui-egui/src/windows.rs`, `backstage/`, `menus.rs`, `app/files.rs`, `control.rs` ou
`doc_window.rs` conflitam por construção — reconciliar mantendo a camada de janelas do app
(mesma regra do rebrand: a camada do app vence). Mudança de comportamento que o upstream fizer
na *engine* (`Session`, selection per-doc, undo, recovery) entra limpa. Também divergente no
craft-libs vendored: `crates/window/src/viewports.rs` carrega o patch do tile fantasma
(porte do `715a06d5` do Local Word — `monitor_bounds` pelo centro) que o rev `c5a1cbf` não tem;
nunca reverter.
| 2026-11-05 | `9bac60ff` (v0.15.1) | baseline: árvore inteira importada (Fase A) | `.github/`, `sonar-project.properties`, `.coderabbit.yaml`, `omastore.toml`, `docs/brand/`, packaging de cada SO reescrito no molde da suíte |

## Decision Ledger

| Data | Decisão | Justificativa |
|------|---------|---------------|
| 2026-11-05 | Sync é manual-revisado, nunca automático no main | Protege o rebrand de regressões silenciosas |
| 2026-11-05 | Formato `.deckcraft` fica byte-a-byte (extensão, MIMETYPE, manifesto) | Compatibilidade com arquivos e ferramentas do upstream |
| 2026-11-05 | `docs/UPSTREAM-MAP.toml` é o contrato do que pode ser aplicado por script | Evita que nomes de crate/bundle id do upstream entrem no repo |
| 2026-11-05 | Packaging e CI não vêm do upstream | O molde da suíte (WiX per-machine/per-user, workflows próprios) é a referência |

## craft-update vendored: fork permanente da suíte (fora do sync)

`libs/craft-libs/crates/update` está **fora do escopo** deste sync com o upstream. É um crate nosso, hospedado no `craft-libs` da suíte Local, e um **fork permanente consciente** do `craft-update` upstream — unificado nos 10 apps em 2026-11-13, com um diff de milhares de linhas contra `storytold/craft-libs/crates/update` (MSI context/migrate, pins do launcher, `offers_update`, `desktop_register`, catálogo único de apps). Nenhum diff de sync encaixa nele.

Bugfix e feature vão **direto no vendored de cada app** (e no `craft-libs` quando o fix vale para todos); contribuição ao upstream só via **Issue/PR manual**, em inglês, quando o problema também existir lá — mesma política do `UPSTREAM-CONTRIBUTIONS.md` do local-office. O restante do `craft-libs` segue o fluxo normal deste documento.
