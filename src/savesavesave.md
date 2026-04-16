Below is a **single master audit-and-patch doc** you can save as `docs/HOPPER_MASTER_AUDIT_AND_PATCH_PLAN.md`.

It combines:

* Hopper vs Pinocchio vs Quasar
* what Solana actually wants from zero-copy
* where Hopper is ahead
* where Hopper is still behind
* exact repo-level refactor order
* the ethos checks to keep Hopper on the right path

````markdown id="bp1o0m"
# Hopper Master Audit and Patch Plan

## Purpose

This document answers one question:

# How does Hopper become the de facto zero-copy solution for Solana?

Not just “good.”
Not just “interesting.”
Not just “more features.”

The target is:

- Pinocchio-level runtime shape
- Quasar-level DX
- stronger safety than both
- stronger introspection than both
- one unified zero-copy memory contract model
- ecosystem completeness

---

# 1. Strategic conclusion

Hopper can become the de facto zero-copy solution **if and only if** it becomes the first system that owns all of these at once:

- substrate-level zero-copy execution
- compile-time-generated ergonomics
- real runtime safety guarantees
- memory precision beyond account-level abstraction
- unified layout/runtime/schema/tooling truth
- full flow ownership through companion crates and CLI

That is the real opportunity.

The risk is not lack of invention anymore.

The risk is:

- too much richness too close to the hot path
- too much “core” that is really advanced platform behavior
- too many partially-correct public surfaces
- too much conceptual spread between runtime, macros, CLI, manager, and advanced systems

So the goal is:

# Keep the edge
# Remove the drift
# Flatten the hot path
# Split core from advanced
# Unify the public story

---

# 2. What Solana actually wants from zero-copy in 2026

Zero-copy in the ecosystem is not just “cast bytes faster.”

Solana needs zero-copy systems that improve:

## 2.1 Predictability
Teams want to know:
- what memory is touched
- what mutates
- what conflicts
- what migration/versioning risk exists

## 2.2 Safety
The ecosystem still suffers from:
- manual validation bugs
- duplicate mutable aliasing mistakes
- bad ownership assumptions
- layout/version drift

## 2.3 Toolability
Foundation-level and infra-level value increasingly comes from:
- explainable programs
- inspectable layouts
- analyzable mutation patterns
- visible access contracts

## 2.4 Parallelism-friendly behavior
Solana is built on non-conflicting access.
A zero-copy system that helps developers express more precise access patterns is genuinely useful.

## 2.5 Real DX without hidden cost
People want:
- quick authoring
- fewer footguns
- but not at the cost of hidden runtime work

Hopper’s opportunity is to solve all five.

---

# 3. Competitor audit

## 3.1 Pinocchio

### What Pinocchio gets right
- substrate ownership
- direct pointer path
- no hidden machinery
- small conceptual core
- companion crates for common program surfaces
- repo shape that communicates sharpness

### What Pinocchio gets wrong / leaves to developers
- manual safety remains developer burden
- manual validation burden
- little/no deep introspection story
- no richer memory model beyond account-level raw access
- no unified layout/runtime/schema/tooling truth

### Lesson for Hopper
Do not try to out-minimalism Pinocchio.

Do:
- keep the same execution honesty
- keep the same substrate directness
- add safety, segments, and introspection **without moving cost into the hot path**

---

## 3.2 Quasar

### What Quasar gets right
- compile-time codegen
- cleaner authoring surface
- good first-use ergonomics
- enough performance parity to be taken seriously
- repo shape that separates derive/CLI/examples/framework pieces

### What Quasar gets wrong / does not fully own
- still fundamentally account-level
- macro/codegen ergonomics are the main story, not a deeper runtime model
- limited introspection compared to what Hopper can become
- safety is mostly authoring convenience, not a richer runtime discipline
- lower innovation ceiling because it is mostly “nicer generated low-level code”

### Lesson for Hopper
Do not try to out-syntax Quasar.

Do:
- match or exceed its convenience
- exceed it in runtime guarantees
- exceed it in memory precision
- exceed it in tooling truth

---

# 4. Hopper’s intended winning position

Hopper should become:

# the first full zero-copy memory contract system for Solana

Not just:
- faster structs
- safer wrappers
- nicer macros

But:

- one access model
- one runtime path
- compile-time generated accessors
- explicit raw escape hatch
- segment-aware runtime safety
- layout/runtime/schema/tooling fed by one truth
- companion crate completeness

That is how Hopper becomes default.

---

# 5. Hopper’s correct public model

Do not describe Hopper as:
- raw mode
- segmented mode
- pipeline mode
- macro mode

Those are implementation layers, not the public mental model.

## Public model

# Access + Guarantees

### Primary safe access
```rust
let balance = ctx.vault.balance_mut()?;
````

### Explicit raw access

```rust id="8cmyi0"
let vault = unsafe { ctx.vault.raw_mut::<Vault>() };
```

### Optional advanced guarantees

```rust id="0ebmm9"
#[pipeline]
#[receipt]
#[invariant(balance >= 0)]
```

Same system.
Different guarantees.
Same runtime path underneath.

---

# 6. Repo-wide refactor principles

## 6.1 One runtime path

Everything must lower to:

* account view
* offset/size
* pointer arithmetic
* typed cast
* optional tiny borrow registration

If safe access does not visibly lower to roughly the same shape as handwritten Pinocchio-style code, Hopper loses.

## 6.2 One source of truth

Runtime/layout define truth.
Schema mirrors it.
CLI and manager consume it.

Not the other way around.

## 6.3 Macros generate structure, not behavior

Macros should generate:

* segment constants
* field metadata
* context accessors
* layout impls
* thin dispatch glue

Macros should not own:

* runtime logic
* hidden validation engines
* surprise control flow

## 6.4 Hot path stays tiny

* no heap
* no runtime strings
* no dynamic map lookups
* no graph engines
* no heavy wrapper richness

---

# 7. File-by-file patch plan

## P0 — Hot path / execution model

### 7.1 `crates/hopper-native/src/account_view.rs`

#### Goal

Make this the single substrate truth for memory access.

#### Keep

* raw account key
* raw mutable data slice
* direct pointer operations

#### Normalize around

* `segment_ref<T>(offset, size)`
* `segment_mut<T>(offset, size)`
* `unsafe raw_ref<T>()`
* `unsafe raw_mut<T>()`

#### Must do

* `#[inline(always)]` on hot methods
* safe and raw access share the same pointer path
* no names/strings/maps here
* no schema/tooling/policy logic here

#### Done when

All higher-level Hopper access lowers here.

---

### 7.2 `crates/hopper-runtime/src/account.rs`

#### Goal

Define the one public access story.

#### Public API to converge on

##### Safe full account

* `load<T>()`
* `load_mut<T>()`

##### Safe segment

* `segment_ref<T>(SEG)`
* `segment_mut<T>(SEG)`

##### Explicit raw

* `unsafe raw_ref<T>()`
* `unsafe raw_mut<T>()`

#### Remove / refactor away

* backend-first identity feel
* overlapping access APIs that feel like separate frameworks
* dynamic segment lookup
* philosophical split between safe/raw/segmented

#### Done when

A developer can learn Hopper access by reading this file alone.

---

### 7.3 `crates/hopper-runtime/src/borrow_registry.rs`

#### Goal

Runtime borrow safety that is real and cheap.

#### Required structure

* fixed-size array
* no heap
* no `Vec`
* small entries: `{ key, offset, size, access_kind }`

#### Conflict rules

* read/read = okay
* any overlapping write = reject
* overlap only for same key

#### Avoid

* graph-engine complexity
* host-side richness leaking into on-chain expectations

#### Done when

Borrow safety is:

* real enough to matter
* cheap enough not to break performance
* obvious enough to trust

---

### 7.4 `crates/hopper-runtime/src/borrow.rs`

#### Goal

Keep wrappers only where they genuinely help.

#### Do

* keep wrappers minimal
* inline hot pieces
* ensure projections lower directly into segment/native access

#### Avoid

* elegant but costly wrapper stacks
* turning borrow support into a second abstraction system

#### Done when

Borrow support feels invisible in normal access.

---

### 7.5 `crates/hopper-runtime/src/context.rs`

#### Goal

Make context boring and stable.

#### Keep

* accounts
* borrow registry
* thin indexed access

#### Avoid

* pipeline engine logic
* manager logic
* schema logic
* alternate execution models

#### Done when

Context is a thin carrier, not a framework.

---

## P1 — Segments / layout / static access

### 7.6 `crates/hopper-core/src/account/segment.rs`

#### Goal

Canonical segment primitive.

#### Keep

* `offset`
* `size`

#### Do

* keep it tiny
* keep it memory-oriented
* keep it compile-time friendly

#### Done when

It looks like a raw memory contract.

---

### 7.7 `crates/hopper-core/src/segment_map.rs`

#### Goal

Runtime segment access must be constant-driven.

#### Do

* associated constants or static tables
* deterministic ordering
* generation from same source as field metadata

#### Avoid

* runtime string lookup in hot path
* reflective runtime access

#### Done when

Hot path uses constants.
Tooling uses names.

---

### 7.8 `crates/hopper-core/src/field_map.rs`

#### Goal

Keep field metadata aligned with segment metadata.

#### Do

* generate field info and segment info from same codegen source
* use field maps primarily for schema/CLI/manager/inspection

#### Done when

Field and segment metadata cannot drift.

---

### 7.9 `crates/hopper-runtime/src/layout.rs`

#### Goal

Layout/runtime remain authoritative.

#### Keep

* `LayoutContract`
* version/discriminator/layout id rules
* compatibility helpers

#### Do

* drive layout/segment facts from compile-time constants
* keep runtime semantics here

#### Avoid

* schema or manager owning runtime truth
* turning layout into a tooling dump

#### Done when

Runtime truth clearly lives here.

---

## P1 — Narrow core

### 7.10 `crates/hopper-core/Cargo.toml`

### 7.11 `crates/hopper-core/src/lib.rs`

#### Goal

Make `hopper-core` mean only the hot-path-adjacent shared foundation.

#### Keep in true core

* ABI primitives
* minimal account/header/pod/segment/overlay if truly central
* field/segment maps
* maybe tiny state helpers
* maybe tiny fast-check / invariant primitives

#### Reclassify out of core identity

* `frame/*`
* `receipt.rs`
* `policy.rs`
* `check/graph.rs`
* `migrate/*`
* `accounts/explain.rs`
* `accounts/migrating.rs`
* richer collections not needed in launch story
* virtual/lifecycle/platform-ish systems

#### Done when

“Core” means hot-path shared foundation, not the whole Hopper platform.

---

### 7.12 `crates/hopper-core/src/frame/*`

### 7.13 `crates/hopper-core/src/receipt.rs`

### 7.14 `crates/hopper-core/src/policy.rs`

### 7.15 `crates/hopper-core/src/check/graph.rs`

### 7.16 `crates/hopper-core/src/migrate/*`

### 7.17 `crates/hopper-core/src/accounts/explain.rs`

### 7.18 `crates/hopper-core/src/accounts/migrating.rs`

#### Goal

Reclassify as advanced/optional systems.

#### Do

* feature-gate if needed
* document as optional guarantees / lifecycle / observability / explainability
* stop letting them define launch identity

#### Done when

A normal Hopper user can ignore them and still build great programs.

---

## P1 — Macro / codegen story

### 7.19 `crates/hopper-macros/src/lib.rs`

#### Goal

Low-level declarative support only.

#### Keep

* low-level helpers
* compile-time assertions
* useful low-level layout support

#### Do

* stop making this the full authoring story
* separate old declarative Hopper from new top-level DX Hopper

#### Done when

This crate feels like support infrastructure, not the main public entry.

---

### 7.20 Add `crates/hopper-macros-proc`

#### Add

* `#[hopper::state]`
* `#[hopper::context]`
* maybe `#[hopper::program]`

#### `#[hopper::state]` generates

* segment constants
* static segment tables
* field maps
* layout impls
* schema hooks

#### `#[hopper::context]` generates

* account index bindings
* accessors like `vault_balance_mut()`
* direct static segment constant usage

#### `#[hopper::program]` generates

* thin dispatch glue only

#### Avoid

* runtime behavior generation
* hidden validation engines
* surprise control flow
* owning execution semantics

#### Done when

Generated Rust is reviewable and direct.

---

### 7.21 Generated accessor shape

#### Required emitted shape

```rust id="7vtd8g"
#[inline(always)]
pub fn vault_balance_mut(&mut self) -> Result<&mut u64, ProgramError> {
    const SEG: Segment = Segment { offset: 0, size: 8 };
    self.ctx.accounts[0].segment_mut(&mut self.ctx.borrows, SEG.offset, SEG.size)
}
```

#### Avoid

* names
* strings
* dynamic lookup
* extra dispatch layers

#### Done when

Pinocchio-style developers trust the emitted code instantly.

---

## P2 — CLI / Manager alignment

### 7.22 `tools/hopper-cli/*`

#### Goal

CLI becomes a trust engine.

#### Keep and polish first

* `hopper build`
* `hopper compile --emit rust`
* `hopper inspect`
* `hopper explain`

#### `hopper compile --emit rust`

Must prove:

* no hidden cost
* no hidden runtime
* codegen honesty

#### `hopper inspect`

Show:

* fields
* segments
* offsets
* sizes
* version/layout info if useful

#### `hopper explain`

Show:

* reads
* writes
* segment touches
* borrow conflicts / guarantees

#### Hide/postpone

* orchestration-heavy commands
* unstable manager-style flows
* anything whose semantics outpace runtime truth

#### Done when

CLI makes Hopper:

* more transparent than Pinocchio
* more explainable than Quasar

---

### 7.23 `hopper-manager/*`

#### Goal

Manager becomes inspector, not engine.

#### Keep

* account visualization
* segment visualization
* borrow/access graphs
* schema-driven inspection

#### Avoid

* defining runtime semantics
* becoming a second framework model

#### Done when

Manager clearly consumes truth instead of defining it.

---

## P3 — Companion crates / parity completion

### 7.24 Add `crates/hopper-system`

#### Start thin

* transfer
* create account
* assign
* maybe close/realloc later

#### Rule

Explicit builder/helper surface only.

---

### 7.25 Add `crates/hopper-token`

#### Start thin

* transfer
* mint_to
* burn
* approve
* close account

#### Rule

Use Hopper-owned types and runtime surfaces.

---

### 7.26 Later crates

* `hopper-token-2022`
* `hopper-associated-token`
* maybe `hopper-memo`
* maybe `hopper-sysvar`

Only after the core is aligned.

---

# 8. Ethos checks

Every major Hopper change should pass these checks.

## 8.1 Pinocchio honesty check

Can a low-level dev inspect the emitted Rust and still trust the runtime path?

If not, Hopper is drifting.

## 8.2 Quasar DX check

Can a normal developer write the obvious instruction without learning five Hopper concepts first?

If not, Hopper is drifting.

## 8.3 Solana fit check

Does the feature preserve:

* `no_std`
* SBF compatibility
* zero-copy pointer path
* predictable compute
* low hot-path complexity

If not, Hopper is drifting.

## 8.4 One access model check

Does this feature strengthen:

* safe access
* explicit unsafe raw access
* optional guarantees

Or does it create another “mode”?

If it creates another mode, it is drifting.

## 8.5 Tooling truth check

Does CLI/manager/schema consume runtime/layout truth?
Or invent their own semantics?

If they invent their own semantics, Hopper is drifting.

---

# 9. Benchmark plan

Hopper must prove itself against real alternatives.

## Benchmark targets

1. Handwritten Pinocchio-equivalent
2. Quasar equivalent
3. Hopper generated equivalent

## Compare

* CU
* code size
* safety surface
* developer complexity
* visibility/explainability

## Example benchmark program

Use one killer example only:

* vault / staking-style state
* safe segment access
* explicit raw path
* borrow safety
* layout visibility

## Success condition

Hopper should show:

* close enough runtime shape to Pinocchio to be trusted
* clearly easier authoring than manual Pinocchio
* clearly deeper safety/introspection than Quasar

---

# 10. Publish criteria

Do not publish as “the standard” until these are true:

## Runtime

* one access model is obvious
* hot path is flattened
* borrow registry is cheap and explicit
* runtime string lookups are gone from hot path

## Codegen

* proc-macro DX exists
* generated code is thin and trustworthy
* `emit rust` proves it

## Core width

* advanced systems are no longer defining the launch identity
* “core” actually means core

## Tooling

* CLI trust commands are clean
* manager is clearly an inspector

## Parity

* plan or initial implementation exists for `hopper-system` and `hopper-token`

---

# 11. Start-now order

Do these in this order:

1. `crates/hopper-native/src/account_view.rs`
2. `crates/hopper-runtime/src/account.rs`
3. `crates/hopper-runtime/src/borrow_registry.rs`
4. `crates/hopper-runtime/src/borrow.rs`
5. `crates/hopper-runtime/src/context.rs`
6. `crates/hopper-core/src/account/segment.rs`
7. `crates/hopper-core/src/segment_map.rs`
8. `crates/hopper-core/src/field_map.rs`
9. `crates/hopper-runtime/src/layout.rs`
10. `crates/hopper-core/Cargo.toml`
11. `crates/hopper-core/src/lib.rs`
12. `crates/hopper-macros/src/lib.rs`
13. add `crates/hopper-macros-proc`
14. generate accessor/constants-only DX path
15. `crates/hopper-core/src/frame/*`
16. `crates/hopper-core/src/receipt.rs`
17. `crates/hopper-core/src/policy.rs`
18. `crates/hopper-core/src/check/graph.rs`
19. `crates/hopper-core/src/migrate/*`
20. `crates/hopper-core/src/accounts/explain.rs`
21. `crates/hopper-core/src/accounts/migrating.rs`
22. `tools/hopper-cli/*`
23. `hopper-manager/*`
24. `crates/hopper-system`
25. `crates/hopper-token`

---

# 12. Final reminder

Do not gut Hopper.

Do not flatten it into Quasar-with-safety.

Do:

* preserve the edge
* reduce the drift
* make one access model obvious
* push advanced systems into optional guarantees and tooling
* prove the runtime path is still honest

That is the path to making Hopper the de facto zero-copy solution.

`