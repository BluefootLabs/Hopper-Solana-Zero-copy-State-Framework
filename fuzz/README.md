# Hopper Fuzz Suite

Libfuzzer-driven fuzz harness for the parsers the Hopper Safety Audit
flagged as highest risk. Closes audit item **D3. Fuzzing low-level
loaders/parsers**.

## Targets

| Target | Under test | Contract |
|---|---|---|
| `fuzz_instruction_frame` | `hopper_native::raw_input::parse_instruction_frame_checked` | Never panics, never OOB-reads; any returned `FrameInfo` is internally consistent; forward duplicate markers always rejected |
| `fuzz_decode_header` | `hopper_schema::decode_header` | Never panics on arbitrary bytes |
| `fuzz_decode_segments` | `hopper_schema::decode_segments::<8>` | Never panics; returned count ≤ capacity |
| `fuzz_pod_overlay` | `hopper_core::account::pod_from_bytes::<WireU64>` + `pod_read::<WireU64>` | Reference and value paths agree byte-for-byte when both succeed |

## Running locally

Requires `cargo-fuzz` and a host with libFuzzer (Linux or macOS; Windows
works with clang-cl but the canonical CI surface is Linux):

```bash
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run fuzz_instruction_frame -- -max_total_time=60
cargo +nightly fuzz run fuzz_decode_header       -- -max_total_time=60
cargo +nightly fuzz run fuzz_decode_segments     -- -max_total_time=60
cargo +nightly fuzz run fuzz_pod_overlay         -- -max_total_time=60
```

Smoke-run (~4 minutes across all four) is suitable for every PR.
Schedule a longer run (8+ hours) on nightly CI; fuzz corpora land in
`fuzz/corpus/<target>/` and should be committed so regressions replay
on subsequent runs.

## Interpreting a crash

A crash output is a serialized input that triggered the assertion.
Drop it into a unit test via `libfuzzer_sys::fuzz_target!` locally or
load it directly into the underlying Rust function:

```rust
let bytes = std::fs::read("fuzz/artifacts/fuzz_instruction_frame/crash-<hash>").unwrap();
let _ = hopper_native::raw_input::parse_instruction_frame_checked(&bytes);
```

## Why these four

The Hopper Safety Audit's Must-Fix #1 was malformed duplicate-account
rejection (`crates/hopper-native/src/raw_input.rs`). The `fuzz_instruction_frame`
target continuously exercises that rule on adversarial inputs. The
three schema targets guard the off-chain decode paths that RPC tooling,
the `hopper dump` CLI, and Hopper Manager use to read accounts. the
classes of input they see (network-sourced, potentially malicious)
exactly match what fuzzing is designed to catch.
