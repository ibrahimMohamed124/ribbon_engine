# Streaming Stress Harness Fix v5

## User-reported issue

PowerShell failed to parse `run_streaming_stress.ps1` at the GPU artifact-list line:

```powershell
$lines.Add("- `gpu/gpu-summary.md`, ...")
```

## Diagnosis

The PowerShell backtick is an escape character. The runner had static Markdown strings containing backticks enclosed in double quotes. The prior v4 patch fixed process exit-code capture but left these Markdown strings unchanged, so its claim that the runner was ready to execute was premature.

## Change

Converted all four static strings in the generated Markdown summary that contain inline-code formatting to single-quoted PowerShell strings:

- `process-memory.csv` measurement semantics
- Headless artifact list
- GPU artifact list
- GPU requirement note

This preserves the Markdown and avoids PowerShell interpolation/escape processing. The v4 process-launch implementation remains intact.

## Scope

Only `examples/streaming_stress/run_streaming_stress.ps1` is changed by this overlay. No Rust source, Cargo manifest, or engine runtime behavior is modified.

## Validation

- Checked the four target lines are now single-quoted.
- Checked no remaining double-quoted `$lines.Add(...)` string contains Markdown inline-code delimiters.
- Installer includes a pre/post SHA-256 comparison and backup.
- PowerShell runtime parsing cannot be executed in this packaging environment; Windows execution is still required.
