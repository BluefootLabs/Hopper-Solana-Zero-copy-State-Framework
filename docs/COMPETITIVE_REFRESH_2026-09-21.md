# Competitive and network refresh, 2026-09-21

This refresh records what changed between the
[2026-09-19 refresh](COMPETITIVE_REFRESH_2026-09-19.md) and 2026-09-21. It
supersedes the time-sensitive rows it names and leaves the rest of that
document in force. Same rule as before: no "best", "fastest", or "safest"
claim without a reproducible fixture, every fact dated, and the ledger at the
end re-run before any of this reaches external material.

## 1. Network facts, verified against public RPC

| Fact | Observed 2026-09-21 | Consequence |
| --- | --- | --- |
| Live rent | `getMinimumBalanceForRentExemption(0)` returned 650,240 on both `api.devnet.solana.com` and `api.mainnet-beta.solana.com` (Agave 4.3.0-rc.0). The Rent sysvar account decodes to `lamports_per_byte_year = 5,080`, `exemption_threshold = 1.0`, `burn_percent = 50` on both. | Hopper's `hopper_init!` and `check_rent_exempt` were computing `(128 + len) * 6,960`, the launch-era constant, so every macro-path `init` overfunded the new account by 37% and the guard could reject an account holding exactly the live minimum. Fixed below. |
| Transaction v1 | `txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL` is active on mainnet-beta (slot 447,120,000, 2026-09-15), devnet (slot 492,480,000, epoch 1140, 2026-09-03), and testnet (slot 437,276,256). The public RPC endpoints decode hand-built v1 envelopes in `simulateTransaction` and `sendTransaction`; a malformed config mask is rejected at the decoder with `-32602`. | `hopper tx send --v1` builds the envelope (below). `hopper tx explain` now asks for `maxSupportedTransactionVersion: 1`; at 0 it would fail with `-32015` on every v1 transaction, and v1 transactions were already 2 of 33 in a sampled devnet block. |
| v1 semantics | If the config mask omits the compute-unit bit, the limit is 0, not the legacy 200k per instruction; if it omits the loaded-accounts-data bit, that limit is 0 and the transaction fails before executing. ComputeBudget instructions in a v1 transaction execute (150 CU each) and configure nothing. The priority fee is a total in lamports. Heap defaults to 32 KiB. | Every v1 transaction Hopper builds sets both limit bits, never prepends a ComputeBudget instruction, and refuses a zero limit locally. |

## 2. Finding: rent was a constant, and the sysvar path linked soft-float

`hopper_core::check::rent_exempt_min` hard-coded 6,960 lamports per byte, and
`hopper_init!` (the `init` and `init_if_needed` lifecycles behind
`#[account(init, ...)]`) funded every new account from it. The runtime
already had `hopper_runtime::rent::minimum_balance_live`, which reads the
Rent sysvar on-chain, and the realloc path used it; the init path, the core
`check_rent_exempt`, and five examples that create accounts by hand
(showcase, registry, treasury, migration, orderbook) did not. All of them
now do. `rent_exempt_min` stays as a deprecated launch-era snapshot, and
its remaining uses warn.

The sysvar path had its own cost. `Rent::minimum_balance` compared the
`exemption_threshold` as an `f64` and kept the launch formula's
`(integer_part as f64 * threshold) as u64` fallback. sBPF has no FPU, so that
fallback linked `__muldf3`, `__floatundidf`, and `__fixunsdfdi` into every
program that read the sysvar, for a threshold no cluster has ever stored.
The threshold is now matched by bit pattern (`1.0`, the SIMD-0194 wire
marker on every public cluster today, and the launch-era `2.0`, both exact)
and any other finite value rounds up to whole years in integer arithmetic,
which can only overfund. Measured on the framework-comparison substrate
counter below: 12,048 to 8,616 bytes from that one change, `initialize`
1,707 to 1,681 CU.

Pinocchio 0.11's `try_minimum_balance` ignores the threshold entirely and
multiplies by `lamports_per_byte`, which is correct on the SIMD-0194 wire and
wrong on a legacy `2.0` sysvar (Mollusk's default). Hopper honors both.

## 3. A Hopper row in pina's cross-framework matrix

pina publishes a like-for-like table (hello world, PDA counter) for Pina,
hand-written Pinocchio, Quasar, and Anchor v2, each built with one release
recipe and measured once in Mollusk with post-state verification. Hopper
was absent. `bench/framework-comparison/` now holds Hopper fixtures written
to pina's exact contracts, a verifier that is a port of pina's, pina's own
pinocchio fixtures vendored as the cross-check, and a driver that
reproduces pina's build recipe. The cross-check reproduced pina's published
pinocchio numbers to the byte and to the compute unit (3,160 B / 111 CU;
6,512 B / 1,490 / 1,721 CU) on cargo-build-sbf 4.1.0, platform-tools v1.54,
Mollusk 0.15.1, so the toolchain delta against pina's Agave 4.2.2 / Mollusk
0.14 run is zero for these fixtures.

| Fixture | Row | Size (bytes) | CU | Notes |
| --- | --- | ---: | --- | --- |
| hello | Hopper (substrate) | 1,656 | 116 | smallest binary in the table; pinocchio 3,160 / 111, Anchor v2 1,880 / 127, Quasar 2,520 / 115, Pina 4,680 / 145 |
| hello | Hopper (macro) | 2,376 | 186 | `#[program]` + `Signer` context; the highest CU in the table, 70 over the substrate |
| counter | Hopper (substrate) | 8,448 | 1,681 / 1,762 | like-for-like: 10-byte compact account, plain `CreateAccount`, PDA re-derived on `increment`; pinocchio 6,512 / 1,490 / 1,721 |
| counter | Hopper (macro) | 11,184 | 1,843 / 386 | `init`/`seeds`/`bump` context, 25-byte headered account; `initialize` beats Pina 3,301, Anchor v2 3,458, Quasar 3,488 by 1,458 CU or more while reading the live rent sysvar; `increment` 386 against Pina 1,753, Anchor 2,117, Quasar 330 |

The table also paid for itself a second time. Instrumenting the substrate
`increment` with `sol_log_compute_units` put the PDA re-derivation at 1,573
CU against a 1,500 CU syscall: the `create_program_address` wrapper was
zero-filling a 256-byte staging buffer and repacking the seeds before every
call, although a `&[&[u8]]` already is the `(ptr, len)` array the syscall
reads. The wrapper now passes the slice through (the hash wrappers had the
same pattern and got the same fix): 24 CU per derivation on every PDA check
in every Hopper program.

The second lesson came from reading how Quasar reaches 330 CU on
`increment`: for an account that is already owner- and discriminator-
validated, it reads the stored bump and verifies the PDA with one
`sol_sha256`, no `create_program_address` syscall and no curve check. The
argument is sound: a hash output can only be an address someone holds a key
for if they can invert the hash or solve a discrete log, so a program-owned,
layout-validated account at that address is a PDA, and an `init` account is
protected by the CreateAccount CPI's own signer check, which refuses an
on-curve address. Hopper's substrate already had the one-hash verifier
(`verify_program_address`) and the no-curve bump search
(`find_bump_for_address`); the derive was wired to the syscall. It now
picks the one-hash path for `init` fields and typed program-owned wrappers
and keeps the curve-checked derivation for unchecked and system accounts.
That is what took the macro `increment` from 1,748 to 386 CU and
`initialize` from 3,207 to 1,843, on every `seeds` context in every Hopper
program, not only the fixture.

Where Hopper does not win, in the table's own terms, with the measured
split behind each gap:

- The macro hello path spends 70 CU more than the substrate. Measured with
  compute-unit checkpoints: a hand-written `Context::new` plus `Hello::bind`
  costs nothing over the substrate (115 vs 116 CU); the `#[program]` table
  dispatch costs about 18 CU; the typed handler wrapper, which is where the
  `Context` is actually materialized and the bound struct built and moved,
  costs about 43 CU before the handler body runs. `max_accounts = 1` saves
  another 8 CU and 128 bytes, and the `Signer` check itself is 5 CU. None
  of that is a single hot spot; closing it means slimmer scaffolding on the
  typed dispatch path, which is the next `profile = "tiny"` target. Anchor
  v2 does the same job in 127 CU.
- The macro counter binary (11,184 bytes) is larger than Anchor v2 (8,696)
  and Quasar (7,808). The substrate row shows the framework's floor is not
  the cause; the macro-generated lifecycle helpers, header writes, and
  layout checks are. Every Hopper program also links about 1.7 KB for the
  full body of the ambient write gate (`strict_writes`), whose no-gate fast
  path is three loads and branches per mutable borrow; the code is only
  executed under an installed policy but LTO cannot drop it. Quasar's own
  binaries carry a comparable self-inflicted cost (about 950 bytes of
  `u64 <-> ProgramError` round-trip tables, 42% of its hello ELF), so the
  size race is about which scaffolding each framework carries, not the
  entrypoint.
- The macro `increment` is 56 CU over Quasar's 330 for strictly more work
  (Hopper validates the 16-byte header and layout id; Quasar checks a
  1-byte discriminator), and the substrate `increment` is 41 CU over
  pinocchio: entry and dispatch 58, signer and owner checks 20, the gated
  typed load 27, add and release 6, the rest the log and the PDA syscall
  the substrate row keeps on purpose (like-for-like with pinocchio and
  Pina). The typed load is the gate fast path plus exact-length and
  discriminator validation, which pinocchio's raw slice access does not do.

The full tables, the driver, and the upstream recipe are in
`bench/framework-comparison/`. The Hopper fixtures are written so they drop
into pina's `frameworksFor()` unchanged apart from the dependency line.

## 4. Program Metadata: security.txt and the full manifest

Anchor's PR #4177 made `anchor init` write a `security.json` and upload it
through `npx @solana-program/program-metadata@0.5.1 write security`. Solana
Explorer reads the canonical `[program, "security"]` record
(`PMP_SECURITY_SEED`, canonical only). The Rust client for the metadata
program is unpublished (`spl-program-metadata-client` 0.0.0), and Anchor's
path cannot write a non-canonical record. `hopper publish-security` does the
whole job in Rust over the same signed-send path as `publish-idl`, validates
the document first (unknown keys refused, values typed, the four neodyme
required fields present), scaffolds a template with `--init`, and reads the
record back with `--read`. `hopper publish-manifest` publishes the full
Hopper manifest under the custom seed `hopper-manifest`, so the document
`hopper verify --authority-baseline` diffs can be fetched from the ledger.
Both are declarations; the ELF-embedded release commitment remains the
binding.

Both ran on devnet against the sentinel deployment the same day: the
security record landed in one inline Initialize (signature `5hrvXz83…`, slot
502,011,159) and read back as the minified document; the 13,369-byte
manifest took the Allocate, two Write, Initialize path (`3wu9wYbm…` to
`46hnQG3C…`, slots 502,011,247 to 502,011,260) and read back equal to the
CLI's normalized rendering. Logs and checksums are under
`audit/devnet-evidence-2026-09-21/metadata-and-txv1/`.

## 5. Transaction v1 from the CLI

`hopper tx send --v1` compiles a SIMD-0385 message with
`solana-message 4.4.1` (`v1::Message::try_compile_with_config`), validates it
client-side (64 addresses, 64 instructions, 12 signatures, no duplicate
keys), signs it as a `VersionedTransaction`, checks the 4,096-byte ceiling
from the serialized message plus bare signatures, and sends it through the
RPC client's wincode path. The compute-unit limit, the loaded-accounts-data
limit (default 4 MiB, never zero), and the optional priority fee travel in
the config mask. The legacy path is unchanged and remains the default.

Proven on devnet the same day: a one-signer SPL Memo sent with `--v1` landed
as a 220-byte v1 envelope (signature `2oqSeV99…`, slot 502,011,279,
`getTransaction` reports `"version": 1`), and `hopper tx explain` decoded it.

## 6. Ledger

- Open: a pull request adding the Hopper rows to pina's matrix; size
  profiling of the macro counter path; the 70 CU macro hello overhead;
  non-canonical metadata records and the `Close`/`Extend` instructions in
  the publishers.
- Closed here: the hard-coded rent constant, the soft-float rent path, the
  missing Hopper row, security.txt publication parity with Anchor, manifest
  publication, transaction v1 sending, and the `maxSupportedTransactionVersion`
  cap in `tx explain`.
