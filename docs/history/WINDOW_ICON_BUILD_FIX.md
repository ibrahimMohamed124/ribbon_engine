# Windows icon build fix

The retired `voxelworks-platform` compatibility library did not produce a Windows executable.
It therefore must not invoke the shared PE icon resource helper, which emits
`cargo:rustc-link-arg-bins` and is intended for executable packages only.

The library's runtime window-icon behavior remains separate. Executable packages keep
using `build_support/windows/icon.rs` to attempt to embed the `.ico` into their own PE
resources.
