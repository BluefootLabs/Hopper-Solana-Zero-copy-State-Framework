# Hopper fuzzing

Hopper has two complementary fuzzing layers:

1. Five libFuzzer targets exercise shared parsers and unsafe boundaries with
   arbitrary bytes.
2. `hopper fuzz` derives seeded, program-specific adversarial cases from a
   published manifest and can execute them through an application adapter.

A manifest can derive contract mutations, but it cannot safely invent a valid
escrow, order book, oracle, or other business fixture. The adapter supplies
that valid starting state and executes the generated mutation in the program's
chosen SVM harness. Hopper owns deterministic case generation, protocol
validation, coverage accounting, and fail-closed invariant enforcement.

## Shared libFuzzer targets

| Target | Under test | Contract |
|---|---|---|
| `fuzz_instruction_frame` | `hopper_native::raw_input::parse_instruction_frame_checked` | Never panics or reads out of bounds; returned frames are consistent; forward duplicate markers are rejected |
| `fuzz_decode_header` | `hopper_schema::decode_header` | Never panics on arbitrary bytes |
| `fuzz_decode_segments` | `hopper_schema::decode_segments::<8>` | Never panics; returned count never exceeds capacity |
| `fuzz_pod_overlay` | `hopper_core::account::pod_from_bytes::<WireU64>` and `pod_read::<WireU64>` | Reference and value paths agree byte for byte when both succeed |
| `fuzz_account_view_load` | `hopper_native::AccountView::load` | Never panics or bypasses the discriminator, version, layout fingerprint, size, or alignment gate |

Run a local smoke pass with `cargo-fuzz` and a nightly toolchain:

```bash
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run fuzz_instruction_frame -- -max_total_time=60
cargo +nightly fuzz run fuzz_decode_header -- -max_total_time=60
cargo +nightly fuzz run fuzz_decode_segments -- -max_total_time=60
cargo +nightly fuzz run fuzz_pod_overlay -- -max_total_time=60
cargo +nightly fuzz run fuzz_account_view_load -- -max_total_time=60
```

## Generate and gate program cases

```bash
cargo run -p hopper-cli -- compile --emit manifest \
  --package hopper-cicada \
  --out target/hopper/fuzz/cicada.manifest.json --force

cargo run -p hopper-cli -- fuzz generate \
  --program target/hopper/fuzz/cicada.manifest.json \
  --out fuzz/plans/hopper-cicada.plan.json \
  --corpus target/hopper/fuzz/cicada-seeds

cargo run -p hopper-cli -- fuzz check \
  --program target/hopper/fuzz/cicada.manifest.json \
  --plan fuzz/plans/hopper-cicada.plan.json
```

`generate` projects layouts, field edges, instruction arguments, account
privileges, aliases, Accounts-derived typed, PDA, lifecycle, has-one, explicit
owner/address, and optional-account constraints, policy rules, static and
parametric write ranges, declared lamport permissions, remaining-account
ceilings, and declared compatibility pairs into stable cases.
Every case carries a content-derived 128-bit hexadecimal seed and generated
invariant hooks. `--corpus` writes adapter seed records as JSON. These records
are not raw libFuzzer byte inputs. The plan commitment and case seeds bind the
actual layout, layout-metadata, instruction, context, policy, and compatibility
descriptor values, including discriminators, fingerprints, PDA seed
expressions, lifecycle and relational constraints, migration policy and
backward readability, account flags, and write rules, rather than only binding
case IDs or counts.

`check` regenerates the plan and compares the complete content, including
seeds and hooks. It detects manifest drift, but it does not execute a program.

## Execute cases through an adapter

```bash
cargo build -p hopper-cicada-fuzz-adapter --locked

cargo run -p hopper-cli --locked -- fuzz run \
  --program target/hopper/fuzz/cicada.manifest.json \
  --plan fuzz/plans/hopper-cicada.plan.json \
  --adapter target/debug/hopper-cicada-fuzz-adapter \
  --require-invariant cicada-business-semantics \
  --report target/hopper/fuzz/cicada-semantic-report.json
```

`run` starts the adapter once, writes one JSON request to stdin, and reads one
JSON response from stdout. Adapter diagnostics must go to stderr. The request
uses schema `hopper.manifest-fuzz-request.v1` and contains the program identity,
plan commitment, and selected cases:

```json
{
  "schema": "hopper.manifest-fuzz-request.v1",
  "program": "example",
  "programVersion": "0.3.0",
  "contractCommitment": "<sha256>",
  "cases": [
    {
      "id": "instruction-0-account-0-missing-signer",
      "seed": "4b9c0d08fc5128b0393e5f51e32720d4",
      "kind": "account-privilege",
      "target": "instruction:create.authority",
      "expectation": "must-reject",
      "requiredInvariants": ["account-constraints", "atomic-rejection", "no-panic"],
      "parameters": { "accountIndex": 0 }
    }
  ]
}
```

The adapter must return schema `hopper.manifest-fuzz-response.v1`, echo the
contract commitment, and return exactly one result for every requested case:

```json
{
  "schema": "hopper.manifest-fuzz-response.v1",
  "contractCommitment": "<sha256>",
  "results": [
    {
      "id": "instruction-0-account-0-missing-signer",
      "outcome": "passed",
      "checkedInvariants": ["account-constraints", "atomic-rejection", "no-panic"],
      "detail": ""
    }
  ]
}
```

The runner fails on a stale plan, adapter crash, malformed response, wrong
commitment, missing, duplicate, or unknown result, explicit failure, skipped
case, or omitted invariant hook. `--allow-skips` is an explicit local escape
hatch; release CI should not use it. Repeat `--case <exact-id>` for focused
replay and `--adapter-arg <value>` for adapter-specific arguments.

An adapter should use each seed to build reproducible keys and hostile bytes,
snapshot relevant account data and lamports, execute against the real program
artifact or host-equivalent dispatcher, and report `passed` only after all
required hooks have been checked. Custom `--require-invariant` hooks are how an
application adds properties the manifest cannot express, such as token
conservation, monotonic sequence numbers, or custody ownership.

Generated case kinds have the following adapter contract:

| Kind | Mutation or probe the adapter performs |
|---|---|
| `layout-truncation` | Resize an otherwise valid typed account image to `length` and bind or load it |
| `layout-identity` | Replace the named header identity byte or fingerprint before binding |
| `field-boundary` | Exercise the declared field offset and size against the typed layout gate |
| `migration-direction`, `migration-backward-read` | Exercise the manifest's declared forward policy and backward-reader contract from a valid source fixture, then verify rollback or postconditions |
| `instruction-truncation`, `argument-boundary`, `bounded-argument` | Materialize instruction bytes from the discriminator and argument metadata, then apply the selected malformed length |
| `account-privilege`, `typed-account-substitution`, `pda-substitution` | Replace the declared meta privilege, owner/layout, or PDA before dispatch |
| `account-lifecycle`, `has-one-substitution`, `owner-substitution`, `address-substitution`, `optional-account-boundary` | Exercise the matching Accounts context's creation, resize, close, relational, pinned-key, and optional-role contract |
| `policy-requirement`, `policy-invariant`, `policy-attachment` | Violate or attach the named manifest policy and verify enforcement before commit |
| `receipt-contract` | Execute a successful fixture and validate the declared receipt contract |
| `duplicate-account-alias` | Bind both named account positions to the same seeded pubkey while preserving the requested privileges |
| `write-range-boundary`, `write-range-escape`, `parametric-write-selector` | Exercise the enforced write-policy oracle at the selected byte or cell; use a test handler when the production handler has no input that can request an illegal write |
| `lamport-permission` | Attempt the selected lamport mutation through the enforced mutation API |
| `remaining-account-boundary` | Supply the selected number of deterministic suffix accounts before dispatch |

Some structural cases intentionally require a host-equivalent policy or loader
probe rather than a reachable production instruction. An adapter must state
which real entrypoint or equivalent enforced primitive it exercised; simply
echoing generated hooks is not fuzz evidence. The fixture under
`tools/hopper-cli/tests/fixtures` tests CLI transport and fail-closed
enforcement only.

## Current C3 status

The deterministic generator, drift gate, strict execution protocol, invariant
hook enforcement, focused replay, and report writer are implemented. Cicada's
checked-in plan contains 698 cases after the semantic adapter exposed and the
generator removed 114 falsely hostile adjacent-range probes plus 2 duplicate
union-boundary probes. It covers 16 matched PDA constraints, 19 typed account
constraints (38 owner/layout substitution cases), 4 lifecycle constraints,
and 12 has-one constraints. Its zero migration and lamport case counts are
intentional: this Cicada manifest declares no compatibility pair and no
mutation-complete lamport surface.

`hopper-cicada-fuzz-adapter` now runs every case with skips disabled in CI. It
recomputes each case against Cicada's live `PROGRAM_MANIFEST`, rejects modified
or unsupported cases and invariants, and executes Cicada's actual private
claim, execution, duplicate-meta, mint-delegation, route-bound, and lamport
floor guards under seeded host reference states. This is no-skip **host
semantic evidence**, not 698 SBF transactions. Reachable transaction/runtime
behavior remains covered by the compiled-SBF Cicada lifecycle suite; structural
loader, layout, byte-policy, and context cases truthfully remain host-equivalent
probes where no production instruction can request the hostile primitive.

## Interpreting a libFuzzer crash

A crash file is the serialized input that triggered an assertion. Preserve it
as a regression and feed it directly to the underlying function, for example:

```rust
let bytes = std::fs::read("fuzz/artifacts/fuzz_instruction_frame/crash-<hash>").unwrap();
let _ = hopper_native::raw_input::parse_instruction_frame_checked(&bytes);
```
