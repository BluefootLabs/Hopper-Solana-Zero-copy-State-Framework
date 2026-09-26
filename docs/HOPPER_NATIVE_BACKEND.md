# Hopper Direct Runtime

Hopper's runtime path is direct account memory for Hopper programs.

Hopper turns
Solana's loader-provided account memory into its own `AccountView`, borrow
guards, validation checks, CPI helpers, and zero-copy state access.

Hopper's direct runtime is designed for:

- complete zero-copy Solana programs;
- deterministic borrow behavior;
- typed account validation;
- strict CPI safety;
- protocol-grade state mutation flows.

---

## Safety Tiers

Hopper exposes 3 tiers:

### safe

The default path.

- checked CPI (validates account count, address identity, signer/writable requirements, borrow compatibility)
- checked PDA verification
- checked borrow access
- checked realloc

### expert

Optimized advanced tools.

- bounded CPI
- zero-copy struct projection
- cross-program lenses
- typed capability views
- lazy account parsing
- batch operations
- verified CPI patterns (LamportSnapshot, DataFingerprint)
- instruction introspection

### raw

Escape hatch.

- syscalls
- unchecked CPI
- SVM memory primitives
- pointer-level ops

---

## Why Hopper Owns The Runtime Surface

Solana already has a runtime.
Hopper Native does not replace the Solana runtime.

It replaces the developer-facing execution surface with one that is:

- explicit about account ownership and privileges;
- structured around guarded account memory and checked CPI;
- usable with typed handlers or deliberate low-level control.

The structural invariant is simple: account bytes come from Solana, but the
contract that makes those bytes safe to use is Hopper's. Validation happens
before typed access. Raw access stays named and explicit.

Hopper no longer exposes alternate runtime backend feature names. Production
Hopper code runs through Hopper's direct account-memory runtime.

---

## Runtime Inventory

Hopper's direct runtime groups the following capabilities behind one account-memory boundary:

| Module | Capability |
| ------ | ---------- |
| `wire` | Alignment-safe wire types with checked arithmetic by default |
| `verify` | Post-CPI state verification (LamportSnapshot, DataFingerprint) |
| `lens` | Cross-program field reads without importing foreign types |
| `introspect` | CPI guard, precompile signature verification |
| `mem` | SVM JIT-compiled memory intrinsics |
| `lazy` | Dispatch-before-parse lazy account resolution |
| `capability` | Compile-time capability types (SignerView, WritableView, etc.) |
| `project` | Bounds-checked zero-copy struct projection |
| `budget` | Tracing helpers; remaining-CU syscall requires explicit cluster support |
| `hash` | Zero-alloc multi-part hashing via syscalls |
| `return_data` | Producer-checked typed CPI return-data prefix reads |
| `batch` | Preflighted close/transfer, direct rent top-ups, System-funded resize |
| `sysvar` | Supported sysvar readers and computed helpers |
| `safe/expert/raw` | Tiered API surface for progressive unsafe exposure |

## Grow and release application state

Use `hopper_native::batch::ResizeWithPayer` for a program-owned account funded
by a System-owned wallet or PDA. Pass the current entrypoint program ID and the
executable System Program account. The default invocation reads live rent;
`invoke_signed_with_rent` accepts a value already read in the same instruction.
Validate your application's authority first. Shrinking keeps excess SOL in the
account; a refund is an explicit application decision.

Direct `realloc_checked_with` debits its optional payer without CPI and therefore
requires a payer owned by the executing program. It is not a wallet-transfer API.
Native operations sit outside runtime write policies. Runtime programs use the
guarded account and lamport APIs for policy-controlled mutations.

Keep account borrows scoped around local computation and drop them before CPI
that needs conflicting access. `Ref::map` and `RefMut::map` select fields while
retaining their account lease; runtime `split_segments_mut` validates several
disjoint fields under one lease. A local segment registry alone is not an alias
boundary. The 0.4.2 SBF path retains both the range lease and native borrow.
