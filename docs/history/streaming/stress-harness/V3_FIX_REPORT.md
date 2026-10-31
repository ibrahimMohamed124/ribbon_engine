# Streaming Stress Harness v3 — PowerShell parser fix

## Reported issue

`run_streaming_stress.ps1` failed to parse before execution. Summary-generation strings used PowerShell double-quoted strings containing Markdown inline-code backticks. A trailing Markdown backtick adjacent to the closing quote escaped that quote in PowerShell, leaving the parser in an invalid state.

## Change

Converted the affected static Markdown summary lines to single-quoted PowerShell strings and removed unnecessary Markdown inline-code delimiters from those paths. Updated all affected summary entries, including the process-memory description, headless artifact list, GPU artifact list, and `--require-gpu` note.

## Scope

- One runtime script changed: `examples/streaming_stress/run_streaming_stress.ps1`.
- No Rust source, Cargo manifest, dependency, or test semantics changed.
- Existing v2 harness code and apply behavior are retained.

## Validation

- Reviewed all double-quoted PowerShell string lines containing Markdown backticks; none remain in the affected runner.
- ZIP integrity and per-file SHA-256 manifest are generated for this overlay.
- A PowerShell parser/runtime is not installed in the packaging environment, so native PowerShell parsing and HeadlessOnly execution remain to be confirmed on Windows.
