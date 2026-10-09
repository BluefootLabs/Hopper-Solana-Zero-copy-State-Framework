# Solana compatibility baseline

Observed October 7, 2026 through finalized RPC reads. This is a dated network
snapshot, not a guarantee that feature or rent state stays unchanged.
[Raw observations](../audit/borrowed-dispatch-2026-10-07/network.json) retain the RPC
endpoints, genesis hashes, slots, feature accounts, and validator versions.

| Feature | Devnet | Mainnet-beta | Hopper consequence |
| --- | --- | --- | --- |
| Transaction v1 (`enable_tx_v1`) | Active at slot 492,480,000 | Active at slot 447,120,000 | Hopper's transaction builder still emits legacy transactions. V1 authoring is an implementation gap, not a feature waiting for mainnet activation. |
| Direct account pointers in program input | Active at slot 474,768,000 | Feature account absent in this observation | Do not describe devnet's input path as the current mainnet path. |
| SBPF v3 deployment and execution | Active at slot 461,808,000 | Active at slot 428,976,000 | The borrowed-argument fixture is built for v3 and exercised on devnet; v0 is separately executed in the compiled VM. |
| Alpenglow | Active at slot 504,144,000 | Feature account absent in this observation | Keep consensus activation separate from program ABI and transaction support. Account locks, authorization, and compute limits still apply. |

The queried endpoints reported Agave `4.4.0-beta.0` on devnet and `4.3.0` on
mainnet-beta. These identify the RPC nodes contacted, not every validator on
the cluster. Feature keys were checked against Agave revision
[`fbcd44cc`](https://github.com/anza-xyz/agave/blob/fbcd44cc9e76144ec6285b79d7a480184d5ea84d/feature-set/src/lib.rs).

Both observed clusters returned rent-exempt minimums of 650,240 lamports for
zero data bytes, 731,520 for 16 bytes, and 1,300,480 for 128 bytes. Read the live
Rent sysvar or RPC when creating or resizing accounts. These figures are not
constants for application code.

The feature observation checks account ownership by the Feature Program and
the serialized activation slot. A validator version alone does not establish
activation. The RPC methods are documented by Solana:
[getMultipleAccounts](https://solana.com/docs/rpc/http/getmultipleaccounts),
[getGenesisHash](https://solana.com/docs/rpc/http/getgenesishash), and
[getMinimumBalanceForRentExemption](https://solana.com/docs/rpc/http/getminimumbalanceforrentexemption).

## Transaction and deployment boundaries

The latest official weekly changelog available in this review is
[October 1](https://solana.com/news/solana-changelog-october-1-2026). It reports
Agave 4.4 beta and work on ABI register arguments, CPI representation, and Rust
PDA seed validation. ABI/CPI proposals are future work, not permission to change
Hopper's deployed calling convention. Hopper already bounds PDA seed count and
length before its derivation paths. Any new ABI path needs its own feature
activation checks, compiled-VM tests, and comparison against existing behavior.

Hopper continues to enforce the legacy transaction envelope; v1 activation
does not enlarge a legacy transaction. V1 encoding, signing, and acceptance
tests are separate future work. Do not advertise support based only on RPC
decoding configuration or the version of a host SDK dependency.

Local VM results, devnet execution, compile compatibility, and hosted CI are
separate evidence lanes. The [release status](RELEASE_READINESS.md) records
which lanes passed and which still need exact clean-source attestations.
No mainnet deployment is part of this validation.

## Runtime findings from the function lab

The October 7 function probes found two differences that a successful ELF
build or deployment would not reveal:

- BLAKE3 calls through both native and runtime wrappers finalized with
  `ProgramFailedToComplete` and an unsupported-instruction log. The CLI's
  initial feature check had rejected the symbol; overriding only that local
  check allowed deployment but did not make the syscall execute. SHA-256 and
  Keccak calls have separate successful execution evidence.
- EpochSchedule's account bytes, generic getter and dedicated getter all
  reported 8,192 slots per epoch. `getEpochSchedule` returned 432,000; at slot
  508,530,057, Clock reported epoch 1,177, agreeing with RPC rather than the
  account's schedule. Hopper correctly decoded the runtime bytes. The test
  records this inconsistency instead of treating the runtime and RPC as
  interchangeable or substituting a constant in the framework.

Use Clock when the application needs the current epoch. Schedule-dependent
logic needs a consistent cluster schedule; this observation is not evidence
that a particular schedule is safe to hardcode. The function-lab runner
continues its state checks and then returns a failing overall status when
these schedule comparisons disagree.

The modular-exponentiation feature account was absent in the same observation.
Its ABI wrapper and local input validation do not establish live support.
See the [function lab](../bench/function-lab/README.md) for default and optional
test variants, and [crypto capabilities](CRYPTO_CAPABILITIES.md) for API scope.
