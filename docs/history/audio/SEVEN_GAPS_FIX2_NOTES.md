# Ribbon Audio Seven-Gap Fix 2

## Scope

Incremental Windows compile fix to apply after `Ribbon_Audio_Seven_Gaps_Fix1.zip`.

## Changes

- In `BusTrack::apply_effects`, iterate over the four EQ handles with `&mut self.eq` before calling Kira's mutable setters (`set_frequency`, `set_gain`, `set_q`).
- For an individual EQ band update, use `self.eq.get_mut(eq_band)` rather than `get(eq_band)`.
- Preserve the Kira 0.12.5 type corrections from Fix 1: decibel gain/drive setters receive `f32`; frequency and Q setters receive `f64`.

## Decoder integration test invocation

`plugins/audio/tests/decoder_integration.rs` is gated by `#![cfg(feature = "audio")]`. The package default enables `audio`, so the dedicated command below is the meaningful decoder integration run and should report five separately named tests:

```powershell
cargo test -p ribbon_plugin_audio --test decoder_integration -- --nocapture
```

With `--no-default-features`, zero decoder integration tests is expected because the Kira audio feature is disabled. The 60 unit tests shown in the supplied log passed in that configuration.

## Validation status

The six `E0596` errors in the supplied Windows log map to the two mutable-access corrections above. This archive was checked for path/content integrity, but the current editing environment has no `cargo`, `rustc`, or `rustfmt`; successful Windows compilation, tests and Clippy are not claimed. Re-run:

```powershell
cargo fmt --all -- --check
cargo check -p ribbon_plugin_audio --all-targets
cargo test -p ribbon_plugin_audio --all-targets --no-fail-fast
cargo clippy -p ribbon_plugin_audio --all-targets -- -D warnings
cargo test -p ribbon_plugin_audio --test decoder_integration -- --nocapture
```
