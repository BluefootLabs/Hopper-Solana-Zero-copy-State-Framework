# Choosing a zero-copy boundary

Hopper combines a dependency-free native layer, typed account validation,
borrowed wire layouts, checked CPI, and explicit write contracts. Choose the
layer that fits the program, then measure the complete instruction. An account
view alone does not establish its owner, authority, schema, or allowed writes.

## Source comparison, October 8, 2026

These are specific source boundaries, not a security audit or an exhaustive
feature ranking. Links pin the code examined; repository heads and published
crate versions can differ.

| Framework and revision | Boundary examined | Hopper counterpart and limit |
| --- | --- | --- |
| Pinocchio `d81610e9` | [Entrypoint parsing](https://github.com/anza-xyz/pinocchio/blob/d81610e9dafefe978c09a92aaa157686fd62d0fd/sdk/src/entrypoint/mod.rs) and [native authoring](https://github.com/anza-xyz/pinocchio/blob/d81610e9dafefe978c09a92aaa157686fd62d0fd/sdk/src/lib.rs) provide direct account views and eager or lazy input handling. | Use `hopper-native` for direct account access and checked CPI; use typed account bindings when generated admission checks help. [Migration examples](FROM_PINOCCHIO.md) describe the different types and contracts a port must preserve. |
| Pina `901ffbe5` | [Account traits](https://github.com/pina-rs/pina/blob/901ffbe5f651d23a1c922c349453f15c50585e32/crates/pina/src/traits.rs) distinguish structural validation of a zero-copy companion from application validation, including fixed-representation initialization failure handling. | Hopper's unreleased `Pod::validate_value` composes representation checks through aliases, arrays, and nested fields. It does not run application authorization or account `#[check]` rules. |
| Quasar `b0de7db4` | [AccountLoad](https://github.com/blueshift-gg/quasar/blob/b0de7db4cd271654a2dcf78807dd865e98e0b339/lang/src/account_load.rs) separates intrinsic checks from normal validation and checked duplicate-account loading. | Hopper has typed account validation, runtime borrow guards, and explicit raw escape hatches. Checked argument parsing establishes neither account uniqueness nor account provenance. |
| Anchor v2 `7fa5b114` | [Account cursor](https://github.com/otter-sec/anchor/blob/7fa5b11408ae73e3669635d13610f2207e3dfce9/lang-v2/src/cursor.rs) tracks duplicate accounts, including remaining-account walks. Its [authoring guide](https://github.com/otter-sec/anchor/blob/7fa5b11408ae73e3669635d13610f2207e3dfce9/lang-v2/README.md) describes trait-based extension points, migration changes, and an unaudited alpha. | Hopper combines generated admission with runtime borrows, explicit wire layouts, and optional write contracts. Its internal verification does not replace independent review. |

Quasar's [instruction guide](https://quasar-lang.com/docs/core-concepts/instructions)
also documents generated argument views and borrowed dynamic inputs. Hopper's
unreleased addition makes an explicitly declared argument type reusable between
manual parsing and generated handlers. It is a concrete authoring choice, not
a claim that borrowed decoding is exclusive to Hopper.

## Replace a program by preserving its contract

| Program requirement | Hopper route | What the port must preserve |
| --- | --- | --- |
| Direct loader and account access | Native eager and lazy entrypoints, account views, syscalls | Entry ABI, duplicate accounts, borrow lifetimes, owner and privilege checks |
| Signed CPI | Checked invocation and token/System Program builders | Callee identity, account order, signer seeds, supported token extensions, outcome checks |
| Existing account bytes | Explicit raw/foreign layout or a reviewed migration | Exact wire format; adding a Hopper header changes an existing account ABI |
| Typed application handlers | `#[program]`, `#[derive(Accounts)]`, declared constraints | Authority and lifecycle rules, rejected transactions, account initialization |
| Borrowed instruction payloads | Manual checked parsing or unreleased `&MyArgs` dispatch | Discriminator, fixed footprint, nested representations, explicit tail semantics |
| Tooling and mutation contracts | Generated manifests, optional write policies, evidence checks | Actual enforcement scope and compatibility with the application's clients |

Hopper owns its native runtime rather than wrapping Pinocchio. The advantage to
evaluate is keeping low-level control, typed admission, wire declarations, and
mutation metadata in one stack. Source APIs differ; framework parity and
performance must be demonstrated with the program being ported. Transaction-v1
authoring and the outstanding release-review gates remain explicit gaps.

## One declaration, several checks

The [borrowed argument APIs](BORROWED_ARGUMENTS.md) let a program select exact
payloads, a checked prefix, or a checked prefix with a borrowed tail. Nested
enum and option tags use the type's validation hook, including through aliases.
All these views refer to the original instruction buffer; the program does not
need a deserialization allocation. Handwritten `Pod` implementations must
provide their own value checks when the default accepts too much.

An ordinary handler can now accept `args: &MyArgs` from that same declaration.
The dispatcher validates it before account binding and reports its fixed size
in typed-handler metadata. The generated Rust and TypeScript clients use an
opaque fixed byte argument; they do not infer nested application encoders.

Account admission, mutation policy, and CPI are subsequent boundaries. Hopper's
[self-audit tools](SELF_AUDIT.md) compare declared effects, lock public API
signatures, map unsafe sites to tests, and verify release evidence. A declaration
or a passing test does not prove arbitrary business logic is correct.

## Compare measured programs

[Published measurements](../BENCHMARKS.md) identify fixtures, source revisions,
account sizes, PDA-check strategies, and toolchains. A stored-bump counter and a
counter that searches for its PDA on every update have different work to do.
Neither those measurements nor a source comparison establishes a universal
compute-unit advantage or complete framework parity.

The checked-arguments fixture exercises manual and generated dispatch against
the same transition and malformed-input cases. [Its guide](../bench/borrowed-args/README.md)
defines the compiled-VM workload and devnet runner. Those checks
cover its documented malformed-input and refused-write invariants, not the
entire framework or every application built with it.

## Bounded batches without an owned buffer

The next step in Hopper's wire contract is [checked borrowed batches](BORROWED_SLICES.md):
a declared capacity, a checked element layout, and reusable manual/generated
parsing. The returned view points into the instruction data. This removes the
capacity-sized owned container from that argument path while preserving
recursive checks and metadata through aliases.

The peer sources already contain substantial zero-copy authoring: Quasar
exposes borrowed dynamic inputs, Pina validates generated representations,
and Anchor v2 offers zero-copy accounts and bounded containers. Hopper's
positioning should describe the program-level benefit of combining its own
native runtime, typed admission, wire contracts, and inspectable mutation
metadata. Borrowing alone does not establish uniqueness, account authorization,
or a compute advantage over those frameworks.
