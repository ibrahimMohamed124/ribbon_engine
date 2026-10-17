# Decisions

Only decisions needed to resolve details left unspecified by the staged refactor plan are recorded here.

| Phase | Decision | Rejected alternative | Reason |
| --- | --- | --- | --- |
| 0 | Use the existing screenshot example's SHA-256 of each emitted PNG as the style/view checksum. | Add a new image-hash implementation to production code. | The example already emits deterministic baseline images, so hashing files is the smallest measurement method. |
| 0 | Measure GPU baseline with a temporary, untracked harness using the existing Realistic game scene, offscreen renderer, and readback. Report the readback-including time separately from window presentation. | Infer GPU performance from the interactive preview or compare different scenes. | The existing benchmark is CPU-only; this gives a repeatable GPU measurement on the available adapter without adding a lasting product feature. |
| 1 | Dependency policy lists direct dependencies including normal, dev, build, and optional entries. | Ignore dev/optional edges. | Those edges still affect workspace tooling and feature-enabled builds, and the plan disallows unapproved crate references. |
| 1 | Keep example directories as documentation placeholders until Phase 13 creates executable packages. | Add nonfunctional example packages to the workspace now. | They have no specified implementation until Phase 13 and placeholder packages would add noise to every intermediate workspace build. |
| 4 | Represent app frame/fixed time with `std::time::Duration`; when the eight-step cap is reached, discard excess whole steps but retain the fractional remainder. | Use floating-point seconds and clear the full accumulator after a capped frame. | Duration avoids avoidable timing drift, and retaining the remainder follows the specified “drop excess” behavior while preserving sub-step progress. |
| 5 | Apply queued OS input before `VirtualInput` events in each `First` stage; a focus-loss event releases all held keys and mouse buttons. | Merge sources nondeterministically or preserve held input after losing window focus. | A fixed source order keeps scripted runs reproducible, and clearing held input matches the existing keyboard-state behavior on focus loss. |
