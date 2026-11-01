## Context

The generic profiling crate already owned duration summaries and capture primitives, but the renderer still authored the capture-completion line directly. The prompt requires tooling-visible profiling output to remain byte-identical.

## Decision

Move the existing `[RENDER PROFILE] wrote … frame samples to …` line formatting into `ribbon_profiling` and have `ribbon_render` call that formatter. Keep the original prefix, wording, spacing, sample count, path display conversion, and line output unchanged.

## Rejected alternatives

Changing the log text or introducing a new output schema would require coordinated tooling updates and break the behavior-preserving constraint.

## Consequences

A unit test locks one exact representative completion line. This protects the existing output contract as the profiling code continues to be decomposed. Build/test execution remains for the user's Windows validation matrix.
