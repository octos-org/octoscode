## Summary

<!-- Explain the problem and the outcome of this change. -->

## Validation

<!-- List the exact commands and any manual checks you ran. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all-targets`
- [ ] `cargo test --doc`

## Review checklist

- [ ] The change is focused and linked to an issue when appropriate.
- [ ] Tests cover the changed behavior, or this PR explains why none are needed.
- [ ] Behavior changes update the corresponding `specs/` contract.
- [ ] User-facing changes update documentation and localized text where needed.
- [ ] Protocol, compatibility, migration, and security impact are described.
- [ ] Logs, fixtures, screenshots, and commits contain no secrets or private data.

<!-- Add screenshots or terminal captures for visible TUI changes. -->
