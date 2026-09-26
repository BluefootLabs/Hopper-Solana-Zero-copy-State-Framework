# Source review: named inputs and extensible account behavior

This review covers specific source paths, not a complete security audit or a
new cross-framework performance ranking. It translates the architectural ideas
into a bounded implementation while retaining ordinary Rust authoring.

| Source pin and reviewed paths | Finding and Hopper consequence |
|---|---|
| [Quasar `0361701`](https://github.com/blueshift-gg/quasar/tree/03617018c2665340abc63dc9f7becda55a25ce48), `examples/escrow/src/instructions/make.rs`, `derive/src/account/{fixed,methods}.rs` | Named `EscrowInner` input and field-based generation make creation readable. Hopper adds named fixed-field inputs alongside positional APIs. This reviewed snapshot is distinct from default-branch `master` head `b0de7db` observed during this review. |
| [Pina `478ae3d`](https://github.com/pina-rs/pina/blob/478ae3d2811ea7f7869d2a32e6095a9727d4f9d8/examples/counter_program/src/lib.rs), counter initialization and mutation | Creation composes CPI with a mutation closure. The example explicitly uses a supplied unchecked bump and stores that byte. Hopper's composed helper retains the existing declared seed/bump path; it does not upgrade a supplied bump into proof of canonicality. |
| [Anchor v2 `038c193`](https://github.com/otter-sec/anchor/tree/038c193a9eedd5ffd4aaf831d501a74a0c90084e), `lang-v2/src/traits.rs`, `accounts/slab_hooks.rs`, README | `AnchorAccount`, `AccountInitialize`, constraints, and slab hooks provide a broader wrapper-extension surface. Hopper's new `AccountFields` is a smaller input extension point. It does not establish parity with third-party account wrappers or automatic token-extension initialization. |

## Implemented direction

One fixed-field declaration drives positional and named inputs with the same
wire conversions. Account creation remains explicit and guarded. Developers can
provide a custom initializer, use a scoped mutable closure, or write ordinary
Rust helpers around explicit CPI builders. There is no required operation DSL.
See [named initialization](NAMED_INITIALIZATION.md) for exact API boundaries.

The source audit also found that public `VerifiedAccount` constructors check
length alone while their documentation claimed all account validation had
passed. Runtime and architecture documentation now distinguish a safe typed
memory view from evidence of ownership or authorization. No owner checks were
removed from the typed account loader.

Executing the vault ELF found a missing System Program account in its deposit
context. A legacy DSL example also attempted to debit a system-owned depositor
directly and retained a read guard across a mutable borrow. These paths now use
the transfer CPI and separate borrow scopes. The getting-started account logic
is synchronized with the corrected executable example.

Testing the DSL exposed a shared wrapper bug: proc-macro states represent the
body, while declarative `hopper_layout!` structs include the header. Headered
DSL, modifier, and migration wrappers now use `HopperLayout::OVERLAY_OFFSET`
to select the right bytes. Tests cover both representations, header preservation,
ownership refusal, and the projected guard's exclusion of conflicting borrows.

Following those casts found another safety boundary: `FixedLayout::SIZE` and its
associated assertion were both overridable by a safe trait implementation.
Some consumers omitted the assertion entirely. Checked core overlays, events,
frames, registries, and collections now evaluate a framework-owned size assertion
that the implementation cannot override. Segment slices also validate element
size, count/capacity consistency, and checked region arithmetic. These fixes
protect the existing on-chain memory APIs rather than adding off-chain checks.

The same review extended to runtime headered and compact loaders. They now
independently check the actual type's projection bounds even if safe custom
traits override both sizing and validation. Compact-tail initialization checks
overlap and offset overflow before writing. This is released on a new 0.4 line
with a matching derive pin to avoid incompatible generated code entering 0.3
dependency resolutions.

## Architecture work that remains a prototype proposal

An account-placement compiler must first demonstrate equivalent accepted and
rejected behavior for one logical operation over combined and split layouts.
The experiment must measure CU, ELF size, account bytes, transaction size, and
actual writable account sets, including the fee payer. Any split requires an
explicit partitionability contract; arbitrary Rust and global invariants cannot
be treated as automatically independent.

The next extension experiment should put a custom wrapper, validation, lifecycle,
and schema implementation in a separate crate. The current initializer-input
trait proves only the input part. Cached validation also needs explicit
invalidation after writes, reallocations, and CPI before checks can be removed.

Existing byte policies, cell access, and Grillo evidence remain useful for
explaining and enforcing program-owned access. They do not change Solana's
account-level scheduler. Cicada remains a workload for bounded on-chain storage
and semantic tests, rather than evidence that the framework wins every workload.
No new consensus activation, byte-level fee discount, or universal performance
lead follows from this API release.
