# Hopper Full Line-by-Line Audit — 2026

Exhaustive whole-repo audit: every line of every Rust file. Multi-session;
this document is the durable state. Each batch marks files audited, findings
are logged with `file:line`, severity, and disposition. Cross-referenced
against the competitor sources at `E:\Frameworks\{anchor,quasar,pinocchio}`
where their approach suggests a latent issue or a better pattern.

## Severity scale

- **P0** — soundness: UB, memory safety, wrong on-chain validation.
- **P1** — correctness: wrong results, broken invariants, API misbehavior.
- **P2** — footgun: public API that invites misuse; missing check a caller
  would reasonably assume exists.
- **P3** — cleanup: dead code, duplication, inefficiency, unidiomatic.
- **DOC** — docs/comments contradict behavior.

Disposition: `fixed@<commit>` | `accepted-risk (<why>)` | `open`.

## Progress checklist

Order is highest-risk-first. A crate is checked only when every file in it has
been read line-by-line and its findings logged.

### Batch 1 — hopper-native (33 files, ~9.2k lines) — IN PROGRESS

- [x] raw_input.rs (re-audited post-hardening; see findings)
- [x] entrypoint.rs
- [x] raw_account.rs
- [x] address.rs (moved up: needed for entrypoint soundness proof)
- [ ] instruction.rs (re-audit post-hardening)
- [ ] account_view.rs
- [ ] raw.rs
- [ ] borrow.rs
- [ ] mem.rs
- [ ] pod.rs
- [ ] project.rs
- [ ] wire.rs
- [ ] pda.rs
- [ ] cpi.rs
- [ ] syscalls.rs
- [ ] sha256.rs / hash.rs
- [ ] batch.rs / budget.rs / capability.rs / error.rs / expert.rs /
      introspect.rs / lazy.rs / lens.rs / lib.rs / log.rs / return_data.rs /
      safe.rs / system.rs / sysvar.rs / token.rs / verify.rs

### Batch 2 — hopper-runtime (46 files, ~20.3k lines)

- [ ] account.rs, compact.rs, tail.rs, segment_borrow.rs, segment_lease.rs,
      layout.rs, zerocopy.rs, account_wrappers.rs, borrow*.rs, address.rs,
      cpi.rs, token*.rs, remaining files

### Batch 3 — hopper-core (69 files, ~18.6k lines)

- [ ] manifest.rs, collections/*, account/*, accounts/*, check/*, frame/*,
      virtual_state/*, segment_map.rs, dispatch/*, cpi/*, remaining files

### Batch 4 — macros (hopper-macros-proc 15 files ~12.2k + hopper-macros ~1.9k)

- [ ] state.rs, context.rs, program.rs, dynamic.rs, declare_program.rs,
      pod.rs, remaining; audit the *generated* code paths

### Batch 5 — hopper-schema (9 files, ~13.8k lines)

- [ ] lib.rs, clientgen.rs, rust_client.rs, python_client.rs, go_client.rs,
      c_client.rs, codama.rs, anchor_idl.rs, accounts.rs

### Batch 6 — solana surface

hopper-solana (19 files ~4.2k), hopper-spl/* (~1.9k), hopper-system,
hopper-memo, hopper-svm, hopper-anchor.

- [ ] all files

### Batch 7 — domain and tooling crates

finance, lending, staking, vesting, distribute, multisig (~0.9k total),
hopper-sdk (~2k), hopper-manager (~0.9k), hopper-test.

- [ ] all files

### Batch 8 — examples, tools/hopper-cli, tests

- [ ] all files

## Findings log

### Fixed earlier in this pass (pre-audit, 2026-06-27..30)

- **P0 fixed@0061fe0** `hopper-native/raw_input.rs:183,254,285` — aligned
  `*(p as *const u64)` reads of the BPF input buffer replaced with
  `read_unaligned`; same for `instruction.rs:130` (u32 header read).
- **P2 fixed@0061fe0** `hopper-runtime/account.rs::load_cross_program` —
  missing `required_len` guard before pointer cast (defense-in-depth against
  overridden `validate_header`); regression-tested with a lax foreign contract.

### Batch 1 findings

- **P3 fixed (this commit)** `hopper-native/raw_input.rs:142-160` — the
  trailing `while slot < frame.account_count` loop in `deserialize_accounts`
  advanced `offset`/`slot` past the MAX-clamped accounts but neither value was
  used afterward (instruction data and program id already come from
  `scan_instruction_frame`). Dead work; burned CU whenever a transaction
  passed more accounts than the entrypoint's MAX. Loop deleted.
- **accepted-risk** `hopper-native/raw_input.rs` (struct derefs) —
  `(*raw).data_len` and other `RuntimeAccount` field reads assume the loader
  input buffer is 8-aligned. This is a documented Solana loader guarantee
  (`BPF_ALIGN_OF_U128` padding is part of the wire format), and the record
  offsets are maintained 8-aligned by construction (`align_offset(8)` each
  iteration). Scalar reads were hardened to `read_unaligned` at fixed@0061fe0;
  whole-struct derefs retain the loader invariant. No action.
- **verified sound** `hopper-native/entrypoint.rs` — `from_raw_parts(ptr,
  count)` in both entrypoint macros is bounded: `deserialize_accounts` clamps
  `count = frame.account_count.min(MAX)` (raw_input.rs:93) and initializes
  every slot below `count` (duplicate markers must reference earlier,
  already-initialized slots or the parser traps). The SIMD-0321 fast path's
  `ptr::read(... as *const Address)` is sound because `Address` is
  `repr(transparent)` over `[u8; 32]` (align 1). `BumpAllocator` keeps the
  cursor word intact and null-returns on exhaustion; `no_allocator!` aborts
  rather than returning null (upholds `GlobalAlloc`).
- **verified sound** `hopper-native/raw_account.rs` — `RuntimeAccount` is
  `repr(C)`, size compile-asserted to 88; field offsets keep `lamports`
  and `data_len` 8-aligned within the record. The duplicate-marker byte is
  repurposed as `borrow_state` (0xFF == NOT_BORROWED matches the loader's
  canonical-account marker), documented at the type.
- **verified sound** `hopper-native/raw_input.rs::deserialize_accounts` —
  forward/self-referencing duplicate markers trap via
  `malformed_duplicate_marker` (the pre-audit silent-fallback aliasing bug
  stays fixed).
