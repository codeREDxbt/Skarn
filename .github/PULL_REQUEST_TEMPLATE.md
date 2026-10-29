## What

<!-- What does this PR do? One paragraph. -->

## Scope

<!-- Which area does this touch: editor, workspace, terminal, git, LSP, AI,
     workflows, installer, UI? -->

## Gate

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] frontend `npm run typecheck && npm test && npm run build`

## Design

- [ ] Colors, fonts, spacing, and motion come from the design tokens in
      `crates/selora-ui/src/theme.rs` and `frontend/src/styles/tokens.css`
      (no hardcoded visual values)
- [ ] No secrets in files or logs (the API key lives in the OS keyring only)

## Notes for the reviewer

<!-- Risks, follow-ups, and anything intentionally left out of scope. -->