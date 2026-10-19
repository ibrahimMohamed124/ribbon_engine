## Summary

<!-- What does this change do, and why? -->

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --locked --workspace --all-targets -- -D warnings`
- [ ] `cargo test --locked --workspace` (and with `--no-default-features`)
- [ ] `python tools/check_deps.py` (if a manifest changed, `tools/allowed_deps.toml` is updated)

## Screenshots (if applicable)

<!-- Include before/after images for visual changes. -->
