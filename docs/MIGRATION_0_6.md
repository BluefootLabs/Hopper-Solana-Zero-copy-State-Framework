# Preparing for Hopper 0.6

The workspace targets 0.6.0. The published release remains 0.5.0 until the
package train completes; use the release status before choosing a dependency.
Upgrade the framework, runtime, schema and proc macros together.

## Instruction contracts

`#[hopper::args]` declares checked borrowed fixed layouts. `BoundedSlice<'_, T, N>`
borrows a bounded sequence of alignment-1 `Pod` elements directly from instruction
data. Both check nested representations before admitting a handler. Account
authorization and application rules remain the program's responsibility.

Type aliases now preserve scalar widths, fixed-array widths, bounded capacities
and string encodings in generated manifests. Opaque aliases are represented as
exact-width bytes in clients; the generator does not recover their Rust meaning.

Owned instruction vectors require fixed-width element codecs. Custom `TailCodec`
implementations must set `FIXED_ENCODED_LEN = Some(width)` only when every value
encodes to that exact width, equal to `MAX_ENCODED_LEN`. Variable-length account
tails retain their existing codecs. Nested variable-stride instruction vectors
are rejected because the current manifest cannot describe them faithfully.

## Regenerate clients

| Client | Bounded instruction arguments in 0.6 |
| --- | --- |
| Rust | Checked encoding and decoding; builders return `Result`; elements are `Vec<[u8; WIDTH]>` |
| TypeScript | Checked capacities and element byte widths; named byte elements use `Uint8Array` |
| Python | Checked encoding; vector elements are `list[bytes]`; string capacities count UTF-8 bytes |
| C, Go, Kotlin | Explicit unsupported-encoding failure before constructing instruction data |

Rust builders for fixed-only instructions keep their signatures. For dynamic
instructions, `*_DATA_LEN` is the maximum footprint, not the length of every
encoded payload. Handle encoding errors before signing or submitting a transaction.
Python builders also reject fixed byte arguments of the wrong length instead
of silently truncating or padding them.

## Runtime behavior

Runtime hashes share the native boundary and its 20,000-slice ABI limit. This is
an interface ceiling, not a guarantee that a transaction has enough memory or
compute for that many slices. Epoch arithmetic saturates overflow and guards
zero epoch lengths. Modular exponentiation checks operand shapes before calling
the syscall. Generated raw entrypoints and receipt logging also gain SBF fixes.

Consult the [network baseline](SOLANA_NETWORK_BASELINE.md) before enabling optional
syscalls. Recorded BLAKE3 failures, unavailable modular exponentiation and the
EpochSchedule discrepancy remain distinct from successful fixture execution.

## Verification scope

The [release dossier](RELEASE_READINESS.md) records outstanding checks and the
exact evidence selected for the working source. Building every program does not
execute every handler or feature combination. Internal checks and devnet receipts
do not constitute an independent review.
