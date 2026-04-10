# Hopper Proc Macro Policy

## Core Rule

**No proc macros are required for correctness or core functionality.**

Hopper must remain fully usable for layouts, overlays, pod access,
validation, phases, receipts, compatibility checks, and migration
tooling without relying on proc macros for soundness.

## What proc macros may do

Proc macros are allowed for optional ergonomic and schema-generation
tasks:

- `#[derive(HopperSchema)]` -- emit LayoutManifest const from struct
- `#[derive(HopperInstruction)]` -- emit instruction metadata
- `#[derive(HopperEvent)]` -- emit event metadata
- `#[derive(HopperManifest)]` -- assemble full program manifest

These generate **metadata**, not runtime semantics. The program works
identically with or without them.

## What proc macros must not do

Proc macros must not hide:

- Memory model guarantees (alignment, bounds, aliasing)
- Unsafe access patterns
- Validation correctness
- Compatibility rules
- Phase transition semantics
- Trust-critical runtime behavior

If a proc macro would make a user unable to understand the real runtime
behavior by reading the code, it should not be part of Hopper's critical
path.

## Why this policy exists

### Trust preservation

Users can always drop to explicit code and inspect what Hopper does.
Every `hopper_layout!` expansion is a `#[repr(C)]` struct with known
offsets and compile-time assertions. No hidden transforms.

### Escape hatch preservation

Serious builders can use Hopper without buying into framework magic.
The macro-free path (manual `#[repr(C)]` struct + `Pod` impl +
`FixedLayout` impl) is always available and produces identical runtime
behavior.

### Product quality

Hopper can use proc macros to improve:

- DX (less boilerplate for common patterns)
- Schema generation (manifest/IDL export)
- Documentation generation
- Client tooling

without compromising its trust model.

## Current macro inventory

All current Hopper macros are `macro_rules!` (declarative, not proc):

| Macro | Purpose |
|-------|---------|
| `hopper_layout!` | Define `#[repr(C)]` layout with fingerprint |
| `hopper_dispatch!` | Instruction dispatch table |
| `hopper_error!` | Sequential error code definitions |
| `hopper_init!` | Account initialization |
| `hopper_close!` | Account closure with sentinel |
| `hopper_check!` | Inline validation assertion |
| `hopper_require!` | Conditional error return |
| `hopper_register_discs!` | Compile-time discriminator uniqueness |
| `hopper_verify_pda!` | PDA seed verification |
| `hopper_invariant!` | Invariant assertion |
| `hopper_manifest!` | Manifest constant generation |
| `hopper_segment!` | Segment descriptor |
| `hopper_validate!` | Validation bundle |
| `hopper_virtual!` | Virtual state slots |
| `hopper_assert_compatible!` | Compile-time compat check |
| `hopper_assert_fingerprint!` | Compile-time fingerprint check |
| `hopper_interface!` | Cross-program read-only view |

No proc macros required. This is a design choice, not a limitation.

## Doctrine

> Hopper does not require proc macros for correctness or core
> functionality. Proc macros are optional ergonomic and schema-generation
> tools only. The runtime model, memory model, validation model, and
> compatibility guarantees remain explicit and usable without them.
