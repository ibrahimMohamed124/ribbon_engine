## Context

Profiling primitives were duplicated across renderer modules.

## Decision

Introduce a std-only `ribbon_profiling` leaf for generic counters, timing samples, and stable key/value formatting.

## Rejected alternatives

A dependency on wgpu or renderer crates would make profiling unusable by non-rendering systems.

## Consequences

The duration-distribution type and CSV serialization implementation now live in `ribbon_profiling`-backed renderer modules, with `ribbon_render::profiling::RenderDurationSummary` retained as a public alias. The wgpu-aware recorder remains in `ribbon_render` because it consumes renderer-specific diagnostics.
