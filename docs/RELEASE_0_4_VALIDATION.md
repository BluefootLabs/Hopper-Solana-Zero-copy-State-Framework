# Hopper 0.4 validation

The 0.4 release adds named fixed-field initialization while retaining ordinary
Rust handlers and explicit account APIs. It corrects typed DSL header offsets,
safe projection size checks, segment geometry, and the SOL vault's System CPI
account list. See [migration](MIGRATION_0_4.md) for compatibility details.

## Native/runtime 0.4.2 — account lifecycle and borrow safety

The 0.4.2 patch keeps an account's native borrow alive for the full lifetime of
an SBF segment guard. Conflicting whole-account access, another registry,
closure, resizing, and writable checked CPI are refused while that guard lives.
Use `split_segments_mut` to edit several disjoint fields together; it validates
all ranges and holds one exclusive account borrow. Its hidden unchecked
constructor is now crate-private.

Close helpers preflight borrows, writable requirements, aliases, arithmetic,
and applicable runtime policies before changing balances. A caught refusal
leaves both accounts intact. Direct self-transfers are balance-checked net zero;
an underfunded account cannot serve as its own resize payer.

The native `batch::ResizeWithPayer` builder funds missing rent through a checked
System Program CPI, checks the current program owner and entry-time growth
limit before charging, and zeroes newly exposed bytes. Shrinking retains excess
lamports. Applications still authorize the resize and propagate CPI errors.
`Ref` and `RefMut` now provide `map`, `try_map`, and `filter_map` for field access
that retains the original account borrow.

The old published runtime fails the compiled segment regression; the patched
runtime passes. Native/runtime fixtures and the treasury, multisig, token escrow,
byte allowance, and ambient write-gate suites pass on sBPF v0 and v3. The new
lifecycle run finalized **24 devnet transactions**, including two expected
transaction refusals and successful instructions that catch and inspect local
refusals. Complete lifecycle snapshots include fees, data, balances, owners,
and the closed-account result. Both deployed v0 ELFs matched before and after.
The native lifecycle and mapped-borrow tests also pass Miri. The final projection
correction produces byte-identical lifecycle ELFs; source lineage records that
relationship instead of treating a host-only result as on-chain evidence.

[Source, test logs, signatures, snapshots, and hashes](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-lifecycle-2026-09-26).

Update existing lockfiles with `cargo update -p hopper-native -p hopper-runtime`.
The framework and CLI remain 0.4.0. These are targeted correctness and developer
experience improvements; they do not establish universal performance leadership.

## Native/runtime 0.4.1 patch — September 26, 2026

`hopper-native` and `hopper-runtime` **0.4.1 are published**. The framework and
CLI remain 0.4.0; support packages keep their independent versions. Existing
lockfiles must update the native/runtime dependencies to pick up the patch:

```sh
cargo update -p hopper-native -p hopper-runtime
```

The patch rejects missing signers in specialized checked CPI, uses immediate
SVM aborts for no-allocation failures and panics, and validates the producer
and typed prefix of CPI return data. A nested callee's unforwarded return data
is rejected. Application-level value and outcome checks are still required.

The repository also adds member-authorized SOL custody and expiring, revocable
single-use payouts, and corrects treasury funding, segment validation, and
cooldown enforcement. These examples are not published crates.

**81 devnet transactions finalized**, including 30 expected refusals across
governance/treasury/native probes (44), funded classic-token escrow (33), and
direct/nested CPI return data (4). Local compiled tests cover v0 and v3;
deployed v0 ELFs matched before and after each run. Registry downloads match
their published checksums and source files, and a registry-only consumer builds.

[Exact source pins, signatures, snapshots, hashes, and tests](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-multisig-2026-09-26).

## Original 0.4.0 scope and source

The complete local gate run and devnet run used clean source
`8ca9f5c984b81a18a87edbd1ed19c5ac3106d2cb`. Publication documentation may be a later
commit; the archived source-lineage receipt enumerates every difference and
requires executable source, manifests, and lockfiles to remain identical.

Host tests, clippy, unsafe-boundary checks, actual build-time forged-size
rejections, SBF v0/v3 named vault, byte allowance, runtime gate, mint plan, and
canonical-PDA suites passed. The orderbook compiled fixture, 23 Cicada compiled
lifecycle cases, and 698 Cicada host semantic cases passed. Host semantic
execution is distinct from compiled SBF and finalized devnet execution.

## Finalized devnet vault

Program: `3KQueyP2phWfwdnju1o98zNVU4vRrr3yerUgvkwjBzK5`.
Public devnet genesis: `EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG`.
The v0 ELF is 12,464 bytes, SHA-256
`15a3fdfc21cecc094e5e7fd88c1edd7457c7f6999b7dbd11a22e7a1556039832`. Dumps before and after testing match it exactly.

All 19 transactions finalized. Complete snapshots include the fee payer,
authority, both vaults, outsider, and System Program, with expected fees,
lamports, ownership, and data checked after every operation. This includes
fresh/prefunded initialization and refusal cases. Overflow after a successful
transfer CPI is covered in compiled SBF; that synthetic state is not created
by this devnet runner. The standalone legacy DSL handlers are host-tested and
are not dispatched by this vault ELF.

| Operation | CU | Result | Devnet signature |
|---|---:|---|---|
| fund-authority | 150 | success | `23vo2by5qWuHpgrtQpXnHm1SwNsi2huEHY7oBK6vAa1XQTZDj5kxKiigUsuDEoHoSEqPaj9Mg5uVT5qz7cgZGqfp` |
| fund-outsider | 150 | success | `2Ax7HeEeiMqGFUysh3yC4isNY4gWmDUFBpRxHukwKL7zgotDC33DVwrj1hYDALMWY9MdzRoBjmHHGoUdAcbBbABE` |
| prefund-vault | 150 | success | `35jNwzy4cxSCrt1NpNGteTSiHDKBMLG7eGq6Hod3jqomR72HLDutMRJmsJynn1wCGAcygvAiGQEMzXrGyDDB6HgE` |
| initialize | 1709 | success | `2UXxZGYaCTP2zY95guM9v9rkjBhid6dxLusDcmazceXgmfWi2uTGCQrAkEEQFYuzk2u51CDV3KzFCx7Vq2Utz2S6` |
| initialize-prefunded | 1707 | success | `3XVNfCynB9a5U85z98YXiDubDrjWRY3kVP9sDhBKAew742GjxzkfgdQ7yFi24eAQFw7DcFxJKTCnr1M3yqMnUVGi` |
| reinitialize | 279 | {'InstructionError': [0, 'AccountAlreadyInitialized']} | `8DqjpQ8WvnsUZ2ZzU5tyeDAbZJw6UC8yVB514gW7nafwhCw9GbvhhpzEnraErzcQV2FHYnu6DMrxBKGmrrEm3KZ` |
| deposit-zero | 202 | {'InstructionError': [0, {'Custom': 6002}]} | `5P3E4apQkwtVt7QsrGnA1iMXpE6hTFKfZYVzH1UP31E6FtzZDaEmBXhSyepXUucrVhE1tZND6QCBNqQ2Fm9gzD9r` |
| deposit-unsigned | 114 | {'InstructionError': [0, 'MissingRequiredSignature']} | `uoiPv3N6k6zbp7TCVieEmHxfqMQFfvBKKqDx8WZprMi34tLKfprVe1QZweDBCLF14R1dJD6n2W8EcKU9ZMFtTJg` |
| deposit-readonly | 115 | {'InstructionError': [0, 'Immutable']} | `K54eqrUKN9QtkkaCi8Zb5SnxYiQ3VR75Ea9xybX5bAF7nJDweH5CTFYmM3q8hrmJSPytgRsWkjo6h6vFuSiW8L2` |
| deposit-wrong-authority | 181 | {'InstructionError': [0, 'InvalidAccountData']} | `3jzrXiDXnAfRYh6e38PY65XAcVyCnyPAy28fPVgCksA9Q2DmBM8ptSDBjJwcM7d57p8imA5ZANensdVs1SJP6YGw` |
| deposit | 1602 | success | `2XadY8H7wkmVwRokn7Bq9RyzwRSBKRzJUCPpNRV4BPqfZ6iis2VGK66hHiUqvnkxrYadv1PzG1ggvS8UxDWQZGkd` |
| withdraw-zero | 175 | {'InstructionError': [0, {'Custom': 6002}]} | `3K3Hg4tvNvHh6eHWQggoFEBbBPQyy3xYVGcFTUccogzVRhpo9E2kdpriJVohh2Y167yFRV2s1Z9eH6XTNzMHvYcs` |
| withdraw-unsigned | 99 | {'InstructionError': [0, 'MissingRequiredSignature']} | `ienMh3JhBHhzYPJn73k4dHxBjhQQopDyFuafQyuYuz5p7QgHCji8jpEVYi3UDJxnPFNJ6zUb8ppS8R7stA8u2z8` |
| withdraw-readonly | 100 | {'InstructionError': [0, 'Immutable']} | `2ZciTzaugeB2nadCvpW3D5z1w6i1TBnDJ2wJCbf3nvV6Uubz5LpznZoBKJGeptYGqVFffVziARzU22JfLF1cGE8H` |
| withdraw-wrong-authority | 166 | {'InstructionError': [0, 'InvalidAccountData']} | `3tsAUZrtmZDkSVbmg2SJqHzaMDeht7dSRwoSyPnVjkxRNJVQfVDqcuDU4wZfH9u4zuUfyrKHkBnpAC46e7YGJFh6` |
| withdraw-over-balance | 199 | {'InstructionError': [0, {'Custom': 6001}]} | `41jnf89fNtcfBSUvTqtUS6BRgGSmrYombCY5XZ1FFcfB5Y8wjDBLEq17XvjZSUALwZpSR54FZwC6XWP5CYqEow1w` |
| withdraw | 240 | success | `2BaHpg2JJoDjLuVXQrovuSy8VFBRatys3erDHfEivAQE4N91f1rnF3kYL4EQzCsNUVny2wqAE9MfxWD8hVGCX69` |
| deposit-insufficient-funds | 1550 | {'InstructionError': [0, {'Custom': 1}]} | `8AD4NSzGs58vyFvjsZmGJ2t94NKQtD1CHcvUVz8ZDGTgJtyf7uYg4r6uKDiXwpNqAtexogbpiCsKeCoEhBbsTxw` |
| deposit-missing-system | 89 | {'InstructionError': [0, 'NotEnoughAccountKeys']} | `64RWsMjsWXjAwqK3WP9w3sXirsBgKiPfV2QWub1dS5WDz2n7LemhYZiesVSjzCr61vbbKSwKkxDWroc5y1NhrEkg` |

These figures describe this artifact and account set. They do not replace the
dated peer benchmark table or establish universal performance leadership.

## Architecture and evidence limits

The placement compiler and broad account-wrapper extension model remain
proposals. Named inputs implement an open, safe value-writing trait; they do
not grant ownership, authorization, rollback, or arbitrary wrapper support.
Cicada remains an integration workload and Grillo an optional evidence
consumer. Hopper's runtime checks execute in the program.

Hosted CI and registry results are captured separately in the final
[evidence archive](../audit/dx-memory-safety-2026-09-25/README.md) and
[publication receipts](../audit/registry-publication-0.4.0-2026-09-25/README.md).
This local validation is not an independent security audit.
