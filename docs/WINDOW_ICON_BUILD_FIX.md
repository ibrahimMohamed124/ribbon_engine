# Windows icon build fix

`engine/crates/platform` produces a library, not a Windows executable. It should not
invoke the shared PE icon resource build helper because that helper emits
`cargo:rustc-link-arg-bins`, which Cargo rejects for packages without a binary target.

The library's runtime window-icon behavior remains separate. Executable packages keep
using `build_support/windows/icon.rs` to attempt to embed the `.ico` into their own PE
resources.
