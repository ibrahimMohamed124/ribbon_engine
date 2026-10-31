# ATHAR dead-code Clippy fix

This is a one-line-attribute source patch. The non-staged `spawn_village` helper is currently called only by unit tests; production transition code stages village visuals and adds colliders at commit time. The helper should therefore be compiled only for tests rather than silenced with `#[allow(dead_code)]`.

From the repository root:

```powershell
py -3 apply_athar_spawn_village_clippy_fix.py --project-root .
```

The script is idempotent and creates `games/athar/src/scripts/village.rs.before_spawn_village_clippy_fix.bak` before modifying the file. It refuses to edit if the expected declaration is not found exactly once.

Then run:

```powershell
cargo fmt --all -- --check
cargo clippy -p athar --all-targets -- -D warnings
cargo test -p athar
cargo test --workspace --no-fail-fast
```
