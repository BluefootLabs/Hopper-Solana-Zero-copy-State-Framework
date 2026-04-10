# Hopper Native Backend

Hopper Native is Hopper's sovereign low-level runtime substrate for Solana.

It exists so Hopper is not dependent on:
- Pinocchio
- solana-program
- any external runtime surface for its public API

Hopper Native is designed specifically for:
- zero-copy state frameworks
- deterministic borrow behavior
- typed account validation
- strict CPI safety
- protocol-grade state mutation flows

---

# Safety Tiers

Hopper Native exposes 3 tiers:

## safe
The default path.
- checked CPI (validates account count, address identity, signer/writable requirements, borrow compatibility)
- checked PDA verification
- checked borrow access
- checked realloc

## expert
Optimized advanced tools.
- bounded CPI
- zero-copy struct projection
- cross-program lenses
- typed capability views
- lazy account parsing
- batch operations
- verified CPI patterns (LamportSnapshot, DataFingerprint)
- instruction introspection

## raw
Escape hatch.
- syscalls
- unchecked CPI
- SVM memory primitives
- pointer-level ops

---

# Why Hopper Native Exists

Solana already has a runtime.
Hopper Native does not replace the Solana runtime.

It replaces the **developer-facing execution surface** with one that is:
- more explicit than Anchor
- more structured than Pinocchio
- more state-native than generic low-level wrappers

---

# Innovation Inventory

Hopper Native includes features no other Solana framework provides:

| Module | Innovation |
|--------|-----------|
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
