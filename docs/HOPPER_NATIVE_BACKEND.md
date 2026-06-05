# Hopper Direct Runtime

Hopper's runtime path is direct account memory for Hopper programs.

It is not a wrapper around Anchor, Pinocchio, or `solana-program`. Hopper turns
Solana's loader-provided account memory into its own `AccountView`, borrow
guards, validation checks, CPI helpers, and zero-copy state access.

Hopper's direct runtime is designed for:

- zero-copy state frameworks;
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

- more explicit than Anchor;
- more structured than Pinocchio;
- more state-native than generic low-level wrappers.

The structural invariant is simple: account bytes come from Solana, but the
contract that makes those bytes safe to use is Hopper's. Validation happens
before typed access. Raw access stays named and explicit.

The old `legacy-pinocchio-compat` and `solana-program-backend` feature names are
kept only as temporary Cargo compatibility aliases. They do not select alternate
runtime implementations. Production Hopper code runs through Hopper's direct
account-memory runtime.

---

## Innovation Inventory

Hopper's direct runtime includes features no other Solana framework provides:

| Module | Innovation |
| ------ | ---------- |
| `wire` | Alignment-safe wire types with checked arithmetic by default |
| `verify` | Post-CPI state verification (LamportSnapshot, DataFingerprint) |
| `lens` | Cross-program field reads without importing foreign types |
| `introspect` | CPI guard, precompile signature verification |
| `mem` | SVM JIT-compiled memory intrinsics |
| `lazy` | Dispatch-before-parse lazy account resolution |
| `capability` | Compile-time capability types (SignerView, WritableView, etc.) |
| `project` | Bounds-checked zero-copy struct projection |
| `budget` | CU budget tracking and `cu_trace!` macro |
| `hash` | Zero-alloc multi-part hashing via syscalls |
| `return_data` | Typed CPI return data deserialization |
| `batch` | Atomic close-and-transfer, realloc-checked operations |
| `sysvar` | Complete sysvar access with computed helpers |
| `safe/expert/raw` | Tiered API surface for progressive unsafe exposure |
