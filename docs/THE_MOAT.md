# The Moat — what compounds and what copies

Dated 2026-07-07. Every claim below cites the file and symbol that implements
it, and the test suite that pins it, so it can be checked against the tree
rather than taken on faith. Competitive facts come from the verified landscape
scan in `docs/audit/GAP_CLOSURE_AND_INNOVATION_2026.md` (section 2).

## The one structural fact

As of July 2026, every other low-CU Solana framework builds on Pinocchio as
its substrate: Anchor v2 (unreleased alpha) is built on pinocchio 0.11,
Typhoon and star-frame are Pinocchio-based, and Quasar shares the lineage
(Pinocchio's author contributes to both). Hopper is the only framework in the
cluster with its own substrate: `crates/hopper-native` has **zero external
dependencies** — its `[dependencies]` section in `Cargo.toml` is literally
empty.

That is not a purity flex. It is the design decision everything else in this
document compounds on. A framework that sits on someone else's account type
cannot change what an account access *is*. Hopper can, and did.

## Tier 1 — uncopyable without forking the account type

These capabilities all require that **every** access to account bytes routes
through a runtime the framework owns. A Pinocchio-based framework can bolt a
ledger on top of Pinocchio's `RefCell`-style account borrows, but any code
holding the raw account can bypass it — so honest enforcement means rewriting
every accessor, which means forking Pinocchio's account type. That is why
these ship in Hopper and nowhere else.

### 1a. The sovereign substrate itself

`crates/hopper-native`: SIMD-0321 two-register fast entrypoint with an `r2`
null-check fallback (`entrypoint.rs`), tuned memory intrinsics dispatching to
syscalls (`mem.rs`), an owned const SHA-256 (`sha256.rs`), and — the piece the
rest of the stack stands on — the in-account `borrow_state` byte on
`RuntimeAccount` (`raw_account.rs`), Hopper's own on-chain borrow primitive.
A Pinocchio-based framework cannot add any of this without forking upstream.
Pinned by the workspace suite (1,295 tests green as of 2026-07-07) and
`#![deny(unsafe_op_in_unsafe_fn)]` across the crate.

### 1b. The segment borrow ledger

`crates/hopper-runtime/src/segment_borrow.rs` and `segment_lease.rs`: a
16-slot segment registry that allows read/read sharing, rejects any
overlapping write, and uses a u64 address-fingerprint fast path with a full
32-byte verify (no false conflicts). It is enforced **on-chain** — there is no
`target_os` gating in `segment_borrow.rs` — and it is the mechanism behind
disjoint typed `&mut` views into one account
(`account.rs::segment_ref` / `segment_mut` / `segment_ref_typed`).

Pinned by: `cargo test -p hopper-runtime --lib segment_borrow` (20 tests), a
proptest disjointness invariant, and `#[kani::proof]` harnesses in
`segment_borrow.rs` (three) and `tail.rs` (three).

**Why the Pinocchio-based field cannot follow (read from their source,
2026-07-07).** Pinocchio exposes a single per-account `borrow_state` byte, so
every framework built on it tracks writes at *account* granularity. Anchor v2's
account context carries a `MUT_MASK` of `[u64; 4]` — a bitmask **over account
indices**, its finest possible write-tracking. Quasar's schema stops at a
boolean `writable` flag. None of them can name a byte range within an account,
which is the unit Hopper's ledger operates on and the unit the 2026 scheduler /
local-fee-market SIMDs price. Expressing sub-account contention requires a
centralized borrow-acquisition point over a byte-range registry — i.e. forking
the account type and routing every accessor through it, the one thing a thin
layer on someone else's account primitive cannot retrofit.

### 1c. Instruction touch maps

`crates/hopper-runtime/src/context.rs::for_each_touch` /
`touch_map_len` / `touch_map_overflowed` (behind the `touch-map` feature),
reading the ledger's append-only touch log. No other framework can enumerate
an instruction's exact `(account, offset, size, read/write)` byte footprint,
because no other framework has a ledger to log into. Measured cost: **0 CU**
— the parity vault benchmarks identically with and without it
(BENCHMARKS.md, bisect corollaries).

### 1d. Field-level write policies (`strict_writes`)

`crates/hopper-runtime/src/write_policy.rs::WritePolicy`, installed by
`#[hopper::context(strict_writes)]` and gated at borrow acquisition, with its
own `ProgramError::Custom(0xD000 | i)` error page. Every other framework
stops at Sealevel's one account-level `writable` bit; Hopper enforces declared
per-field mutable ranges. Enforcement at acquisition only works because
acquisition is centralized — the ledger again. Measured cost: **0 CU** (same
bisect). Pinned by `cargo test -p hopper-runtime --lib write_policy`
(11 tests).

### 1e. Behaviors with write-set accountability

`crates/hopper-runtime/src/behavior.rs::HopperBehavior`: parameterized
per-field lifecycle plugins whose `WRITES` contribution feeds the
`strict_writes` policy and whose checks return `BehaviorChecked<B, O>` proof
tokens. Quasar's `AccountBehavior` is side-effect-only hooks with no
accountability. Each piece alone is imitable; the closed loop —
ledger → policy → proof token — is the moat.

## Tier 2 — copyable in weeks-to-months, at ecosystem cost

- **Sealed self-proving layout stack.**
  `crates/hopper-runtime/src/zerocopy.rs`: `ZeroCopy` → `WireLayout` →
  `AccountLayout`, sealed via the doc-hidden `__sealed::HopperZeroCopySealed`,
  so a hand-rolled `unsafe impl` cannot join the safe-overlay family. "Safe
  to overlay" is a sealed capability, not a convention. Pinned by
  `cargo test -p hopper-systems --test property_tests` (85 tests) and
  trybuild `compile_fail` fixtures.
- **Layout fingerprints + schema epochs.**
  `crates/hopper-runtime/src/layout.rs`: 16-byte header carrying disc,
  version, `schema_epoch` (u32 LE at bytes 12–15), and `LAYOUT_ID`; checked on
  every load and every foreign read. Anchor retrofitting this would mean
  turning its 8-byte discriminator into a 16-byte header — an
  ecosystem-breaking change for them. Migration edges compose over it
  (`migrate.rs::MigrationEdge`, `#[hopper::migrate]`); pinned by
  `cargo test -p hopper-lang --features proc-macros --test migrate_integration`
  (8 tests).
- **Manifest-backed foreign lenses.**
  `crates/hopper-runtime/src/foreign.rs::ForeignManifest`: cross-program field
  reads with 4-way ABI-drift detection (owner, disc, wire fingerprint,
  schema-epoch range). Competitors either version-lock on the foreign crate or
  read blind offsets. Downstream of the header design existing. Pinned by
  `cargo test -p hopper-runtime --lib foreign` (5 tests).
- **Receipts and policies.**
  `crates/hopper-core/src/receipt.rs::StateReceipt`,
  `crates/hopper-core/src/policy.rs::InstructionPolicy`. Mechanically
  copyable — but only valuable when fed by the ledger's diff and touch data,
  which is Tier 1.
- **Hostile-metadata fuzz discipline.**
  `crates/hopper-core/src/collections/mod.rs::hostile_metadata_proptests`
  (8 tests, pinned regression seeds): every zero-copy collection treats its
  own stored metadata (lengths, heads, free lists) as attacker input. The
  methodology is copyable; the accumulated corpus and the discipline are not.
- **The bug-class regression suite (I20).**
  16 pinned tests across `crates/hopper-runtime/tests/competitor_bug_classes.rs`
  and `crates/hopper-core/tests/competitor_bug_classes.rs`, each turning a
  competitor bug class (CPI return-data UB, self-close lamport imbalance,
  stale migration state, overstated remaining-capacity, duplicate-account
  aliasing, and the three Anchor coarse-borrow classes tabled below) into a
  Hopper regression proof. Authoring the suite found and fixed a real Hopper
  bug — `safe_close` accepted an aliased destination and silently burned the
  drained lamports — so the process demonstrably bites both ways. A competitor
  cannot copy the suite without first admitting each bug class.

## Anchor's coarse-borrow bug classes, class by class

Anchor v2's finest write-tracking is account-index granular: an account
context carries a `MUT_MASK` of `[u64; 4]` — a 256-bit bitmask with one bit per
account *entry*, recording which accounts an instruction may mutate. It has no
representation for a byte range within an account, nor for "writable at the
transaction level but read-only in this handler." Three bug classes recur in
the anchor-next tracker as a direct consequence. The table is factual, not a
verdict on Anchor's engineering: the coarseness is inherent to tracking writes
at account granularity, which is the finest granularity a thin layer over
Pinocchio's single per-account `borrow_state` byte can reach.

| Anchor bug class | Why account-index (`MUT_MASK [u64;4]`) tracking permits it | Hopper mechanism (byte-range) that prevents it | File / symbol | Pinned by (level) |
|---|---|---|---|---|
| (i) read-only account gets mutated | The mask records only *which* accounts are writable; an account read-only for this instruction is often transaction-writable for another, so its bit cannot encode the intended write-set, and a handler/CPI write to it is invisible | A declared byte-range write-set is checked at every write acquire; a write to an account absent from the set, to an undeclared range of a partially-writable account, or any write under an empty (read-only) policy is refused with an indexed error | `write_policy.rs::WritePolicy::check_write`, gated in `context.rs::Context::check_write_policy` | `strict_writes_rejects_writes_outside_the_declared_byte_range_set` (host) |
| (ii) stale account view after CPI | The mask neither binds a borrow to a byte range nor re-checks it across a CPI, so a view taken before a CPI that mutates/reallocs the account can be used after | A live byte-range write lease rejects any conflicting acquire over those bytes (the borrow a CPI-passing helper or later reader would take) until it is released; the account borrow byte enforces the same at whole-account granularity | `segment_borrow.rs::SegmentBorrowRegistry::register`; `hopper-native::AccountView::try_borrow`/`try_borrow_mut` | `a_live_segment_borrow_blocks_the_access_a_stale_view_would_need` (host) |
| (iii) realloc payer / min-len edges | The mask marks the reallocated account as written but carries none of the rent/size arithmetic, so an underfunded grow, a non-signer/non-writable top-up payer, or a runaway grow-chain is not caught by borrow tracking | Rent, funding, and payer signer+writable are preflighted strictly before the resize; a per-instruction `ReallocGuard` caps cumulative growth | `account/lifecycle.rs::safe_realloc`; `account/realloc_guard.rs::ReallocGuard` | `realloc_preflights_payer_funding_and_bounds_growth_before_resizing` (host) |

Honesty note on levels: all three pins are **host-level** — they exercise the
guard directly over fabricated `RuntimeAccount` buffers. The `Context` wiring
that consults `WritePolicy` at each write acquire is separately exercised in
`context.rs::write_policy_tests`; classes (i) and (ii) are *structurally*
harder in Hopper because acquisition is centralized through a byte-range ledger
the runtime owns, but the byte-range enforcement of a realloc that physically
moves an account's data pointer under a held view is an on-chain effect, not
reachable in a host unit test (tracked as a `hopper-svm` follow-up).

## Tier 3 — copyable in a weekend (never lead with these)

Multi-language clientgen and IDL emission (`crates/hopper-schema`: TS, Kotlin,
Python, Go, C, off-chain Rust, Codama JSON, Anchor IDL), the CLI feature-gate
and deploy guards (`tools/hopper-cli/src/cmd/feature_gate.rs`), the CU bench
harness, 4×u64 word-compare address equality, proof-carrying typestate markers
taken alone, and tuned memory intrinsics (`crates/hopper-builtins`). These are
proof of ecosystem maturity, not the moat.

## Two honesty notes (read before quoting this document)

1. **The account-level `borrow_registry.rs` is a host-only no-op on
   `target_os = "solana"`.** Duplicate-handle alias detection runs in host
   tests; on-chain aliasing safety rests on the native `borrow_state` byte
   (`hopper-native/src/raw_account.rs`) plus the segment registry, which has
   no `target_os` gating and runs in production. "Part of the ledger is
   simulation-only" is therefore false as a safety claim: the on-chain
   enforcement surfaces are the byte and the segment ledger.
2. **There are two `Address` types** (`hopper-native/src/address.rs`,
   `hopper-runtime/src/address.rs`), bridged by `repr(transparent)` casts in
   `hopper-runtime/src/native_boundary.rs` whose soundness is held by
   compile-time size/alignment assertions. Intentional layering, statically
   checked, and a tracked consolidation item.

## Copyable vs uncopyable

| Capability | File / symbol | Copy cost for a Pinocchio-based framework |
|---|---|---|
| Sovereign substrate (own entrypoint, intrinsics, borrow byte) | `hopper-native` (zero deps) | Fork Pinocchio |
| Segment borrow ledger + leases | `segment_borrow.rs`, `segment_lease.rs` | Fork the account type; route every accessor |
| Instruction touch maps | `context.rs::for_each_touch` | Impossible without the ledger to log into |
| `strict_writes` field-level write policies | `write_policy.rs::WritePolicy` | Requires centralized borrow acquisition |
| Behaviors with write-set contribution + proof tokens | `behavior.rs::HopperBehavior` | Requires ledger + policy + proof layers together |
| Sealed layout trait stack | `zerocopy.rs::__sealed` | Weeks — macro/API redesign |
| Fingerprints + schema epochs + migration chains | `layout.rs`, `migrate.rs` | Months — header format break (ecosystem cost for Anchor) |
| Manifest-backed foreign lenses | `foreign.rs::ForeignManifest` | Months — needs the header design first |
| Receipts / policies | `receipt.rs`, `policy.rs` | Copyable, but hollow without ledger data |
| Hostile-metadata fuzzed collections | `collections/mod.rs` | Methodology copyable; corpus accumulated |
| Bug-class regression suite | `tests/competitor_bug_classes.rs` (×2) | Copyable only by admitting each bug class |
| Clientgen / IDL / CLI guards / bench harness | `hopper-schema`, `hopper-cli` | A weekend |

The positioning consequence: lead with the borrow ledger and the substrate it
needs. Everything in Tier 1 falls out of one design decision no competitor
can retrofit; everything in Tier 3 is table stakes anyone can rebuild.
