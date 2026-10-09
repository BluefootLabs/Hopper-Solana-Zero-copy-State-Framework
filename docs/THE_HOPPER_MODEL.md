# The Hopper Model

Hopper is a Rust framework for Solana programs. You describe account layouts
and admission checks, write a handler, and compile it to SBF with the Solana
build tools. Rust macros generate code at build time; the resulting checks and
handler run inside Solana's VM.

Start with `#[account]`, `#[derive(Accounts)]`, `#[program]`, and `Ctx<T>`.
The [counter](../examples/hopper-counter/src/lib.rs) demonstrates an update to
initialized state. The [vault](../examples/hopper-vault/src/lib.rs) also creates
accounts and moves SOL.

## The Pipeline

An ordinary typed instruction follows this path:

1. Solana supplies the program ID, accounts, privileges, and instruction bytes.
2. Hopper dispatches the instruction and decodes its declared handler arguments.
3. The generated account binding checks the requested account types and constraints.
4. Your handler borrows state, applies business rules, and makes any required CPI.
5. The handler returns a result. Solana enforces its runtime rules and commits a
   successful transaction or rolls back failed transaction state changes.

Policies, state receipts, phased execution, and migration helpers are optional.
They run only through the paths your program actually uses. CLI inspection,
client generation, and release self-audit run off chain.

| Boundary | Hopper provides | Your program supplies |
| --- | --- | --- |
| Account admission | Typed owner/layout checks, signer wrappers, declared writable, `has_one`, seed, and other constraints | Which authority, accounts, programs, and layouts to accept |
| State access | Checked borrows and supported zero-copy layouts | Arithmetic, economic invariants, lifecycle, and error propagation |
| CPI | Instruction builders and checked invocation paths | Intended callee, account relationships, supported asset policies, and outcome checks |
| Transaction execution | An SBF program using Solana's account and syscall ABI | Accounts, instruction data, signatures, and compute budget in the client transaction |
| Inspection | Manifests, generated clients, bounded receipts, and evidence tools | Tests that cover the application's success and failure behavior |

For example, `Signer` alone accepts a signing account. Adding
`has_one = authority` binds that signer to an authority stored in state. Neither
chooses your application's authority policy for you.

## State Layouts

`#[account]` defines the normal framework layout. Fixed fields use byte-backed,
alignment-safe representations such as `WireU64`. Supported views borrow bytes
from account memory instead of deserializing the whole account into an owned
Rust object. Updates write through that borrow.

Zero-copy is specific to that access path. It does not mean zero validation,
zero compute, or no copies anywhere in the program or validator. Regular
scalar handler arguments are decoded values. Dynamic fields vary: address
vectors can be borrowed, while other tail codecs return bounded owned values.
Client encoding and token-program execution have their own costs.

[Borrowed instruction arguments](BORROWED_ARGUMENTS.md) are a separate API.
The unreleased checked parsers validate nested option/enum representations;
raw parsing and overlays do not acquire that validation automatically.
The same unreleased layout can be a `&MyArgs` handler parameter: generated
dispatch borrows and validates it before binding the account context. Multiple
fixed layouts and scalars can compose in declaration order. An explicit final
`&[u8]` accepts the remaining bytes; the application validates its content.

## The 16-Byte Header

Default headered layouts store a discriminator, version, flags, layout ID, and
schema epoch in a 16-byte header. Typed admission verifies the account owner and
the selected layout contract. A matching header is not proof of authority or
business correctness.

Opt-in compact accounts use a one-byte discriminator and an explicit body-size
contract. Fixed compact loads require exact size; compact-dynamic loads require
a minimum prefix and leave tail semantics to the application. Their layout ID
lives in trusted manifest metadata, not in the account bytes.

See [the wire format](ARCHITECTURE.md#wire-format) for offsets and fingerprint
construction.

## One Access System

Choose the checks needed at the point where bytes become a typed view:

| Access | What it establishes |
| --- | --- |
| Typed `Account<T>` binding | Owner and selected layout contract, plus declared context constraints |
| `ctx.accounts.state.get()` / `get_mut()` | A checked shared or mutable borrow of an admitted account |
| Runtime account loaders | The checks documented by that loader; compact loading alone does not authorize a write |
| Slice overlays / `pod_from_bytes` | A supported in-memory representation; caller establishes provenance and application meaning |
| Unsafe raw access | Caller must uphold the function's full safety contract |

The runtime account view wraps the native backend representation. Framework
handlers and lower-level APIs are exposed by the same facade, but the native
and runtime view types are distinct. Moving down a layer requires reviewing the
checks and guards that layer supplies.

## Specialized Validation Helpers

Headered, compact, foreign, compatible-version, and observational loaders make
different promises. A foreign view must establish the intended owner and ABI;
a compatibility loader needs an explicit accepted-version policy. Observational
tooling reads are not an authorization mechanism.

Use [memory access](MEMORY_ACCESS.md) and the individual API's contract when
selecting a loader. No universal compute cost applies to all these paths.

## Validation and Checks

Typed account constraints run when the context binds. Handler checks enforce
application rules after admission. Checked integer operations reject overflow;
propagate the error to the instruction boundary when the transaction must fail.
A helper returning an error does not undo earlier local writes if your handler
catches and ignores it.

Checked data borrows track live references. Shared reads can coexist; mutable
access needs exclusivity. Checked CPI tests the requested account privileges
against the live borrow state. A read-only CPI may accept a shared read, while a
writable CPI requires releasing all conflicting borrows. Drop a mutable guard
before the call, then obtain a fresh view of the resulting state.

The default runtime CPI tier also checks meta/view identity, required privilege
coverage, and repeated writable roles. Borrow-only and builder tiers have their
own contracts. Supplied PDA seeds are ultimately derived and checked by the SVM
against the calling program; a host preflight cannot substitute for that check.

## Policy and Capabilities

Optional write contracts can constrain tracked data ranges, lamport operations,
and writable CPI delegation. Explicit capability declarations and policy packs
help express the required checks. They do not infer business rules or sandbox
every instruction a downstream program might run. Raw access and bypass APIs
require review against their documented obligations.

## Phased Execution

The optional `Frame` API uses typestate to order resolve, validate, and execute
phases. The compiler enforces that API's ordering; the application supplies the
validation and mutation closures. Passing a validation closure does not prove
that its checks are sufficient. Ordinary `Ctx<T>` handlers do not automatically
construct a frame.

## State Receipts

`StateReceipt` can summarize a configured mutation scope, including before/after
fingerprints, changes, and recorded invariant outcomes. Programs explicitly
capture and emit these receipts. Their FNV fingerprints are not cryptographic
proofs, and recorded flags do not prove that an arbitrary rule was correctly
implemented. A receipt covers the scope and observations supplied by its caller.

`hopper receipt <hex>` decodes a receipt off chain. Release self-audit execution
receipts are a separate host-tool artifact, described in [self-audit](SELF_AUDIT.md).

## Segments and Roles

Segments divide account data into bounded regions. Segment leases can allow
disjoint local borrows and reject incompatible overlapping ranges. Roles help
express preservation, migration, and access rules through the APIs that enforce
them.

Solana locks whole writable accounts. Two byte ranges in one account do not
allow two transactions to write that account concurrently. Splitting state
across accounts may reduce contention, with rent, account-list, and lifecycle
tradeoffs. Segment operations do not create a new scheduler or fee model.

## Fingerprints and Compatibility

A layout fingerprint identifies a declared wire contract. The current proc
macros hash an ordered descriptor and retain eight SHA-256 bytes. They do not
recursively inspect every nested user type or alias. Changes to those types
still need versioning and compatibility review.

Manifest comparison and migration planning are off-chain tools. Applying a
migration requires an authorized on-chain path that checks source and target
layouts, funds any required rent, and preserves application invariants. A CLI
plan does not change a deployed account by itself.

## Invariants

Applications define and run their own invariants. Hopper offers checked
arithmetic, validation helpers, invariant collections, and optional recorded
results. Use tests for hostile accounts, malformed inputs, arithmetic edges,
and failures after CPI. Ensure errors reach the transaction boundary when
rollback is required.

## Collections

Account-backed collections include `FixedVec`, `RingBuffer`, `PackedMap`,
`SortedVec`, `BitSet`, `Slab`, `Journal`, and `SlotMap`. Their constructors validate
stored metadata within caller-provided slices. Capacity, placement, ownership,
and authorization remain application choices. These collections do not provide
a matching engine, token custody, or automatic account growth.

## CLI Tooling

The CLI scaffolds projects, builds programs, exports manifests, generates
clients, inspects state, and collects evidence. These are development and
operations tools. Ordinary Hopper instructions need no separate execution
service. A client must still submit a valid Solana transaction.

Host Rust tests are useful for parsing, validation, and arithmetic. Some host
System Program calls are emulated; other CPI paths are validation-only no-ops.
A successful host call therefore does not prove tokens moved. Run the compiled
program and actual callee in an SVM or on devnet to check transfer behavior.

## Cross-Program Interfaces

Foreign account adapters validate an explicit owner and wire contract before
reading another program's state. They do not confer write authority over it.
Use that program's supported CPI instructions to change its state. Token
accounts belong to the selected Token Program; writing an amount field in your
own account does not transfer tokens.

## Error Handling

Return errors for rejected actions and propagate failed CPI and invariant
checks. Solana rolls back state changes from a failed transaction; fees can
still be charged. A receipt, log message, or host-side success response is not
a substitute for checking the transaction result and relevant account state.

## Design Principles

Use ordinary Rust, make account admission explicit, keep supported state views
borrowed, and choose optional machinery when it solves a concrete problem.
Measure complete instructions, including their validation and CPI. Keep
published features, unreleased APIs, and fixture-specific evidence distinct.

## Where to Go Next

- [First program](FIRST_FIVE_MINUTES.md)
- [Funded SOL vault](../examples/hopper-vault/src/lib.rs)
- [Program capabilities](PROGRAM_CAPABILITIES.md)
- [Framework boundaries](FRAMEWORK_BOUNDARIES.md)
- [Release evidence](RELEASE_EVIDENCE.md)
- [Solana account rules](https://solana.com/docs/core/accounts)
- [Solana CPI rules](https://solana.com/docs/core/cpi)
