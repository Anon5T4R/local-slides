# Local Slides — working notes for Claude

Read [AGENTS.md](AGENTS.md) first: its rules bind you and win over anything here.

New workspace crates must include `[lints] workspace = true` in `Cargo.toml` to inherit
`[workspace.lints]`. Production crate roots must declare the required crate-level lint attributes
before modules or other crate items; crate-level `//!` documentation may precede them.

## Start every session
1. Read `plan/STATUS.md`, then the current milestone in `plan/execution-plan.md`.
2. Run `cargo xtask ci` before you change anything, so you know the tree is green.

## Habits
- Every user-visible action is a command with tests (`crates/engine/src/cmd/*`).
- Look at UI changes: `localslides --sample --control 7990` + `ui.screenshot`, or
  `localslides-cli render`.
- Commit after each landed arc, em português e sem `Co-Authored-By`.
- Keep `ROADMAP.md`, the progress docs in `docs/` (AGENTS.md §7) and `plan/STATUS.md` current.
- Don't ask the user questions; record genuinely-theirs decisions in `plan/STATUS.md`.

## CI workflows
Our CI is our own (`.github/workflows/ci.yml` and `release.yml`); it never calls workflows from
another organisation. A feature or fix PR does not change CI: if a change can't be built or tested,
it gets its own PR.
