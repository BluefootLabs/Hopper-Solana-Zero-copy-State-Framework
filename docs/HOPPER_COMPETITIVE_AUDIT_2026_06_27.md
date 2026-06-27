# Hopper Competitive Audit - 2026-06-27

Scope: live devnet deployment hygiene, BPF loader buffer handling, and Hopper's
DX/feature position against Anchor current/v2-track, Quasar, and Pinocchio.

This is a program-framework audit. It focuses on what makes Hopper better for
shipping Solana programs, not on becoming a general client SDK.

## Live Devnet Pass

Authority: `HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT`

Toolchain:

- `solana-cli 2.3.13`
- `cargo-build-sbf 4.0.0`
- `platform-tools v1.53`

Fresh deployments completed:

| Program | Program id | Notes |
|---|---|---|
| `hopper-counter` | `5PHo4rrYpPQPeNYNDa6fXatK56QBFY8ZDXCh2CWkneZo` | Fresh deploy, `--max-len 50000`, signature `2MVgomTVpo7dXKpePD5EBzmqpAhNNn6dEYm4hjk5JbxNpwYdE3926rd6k39pQKinGstpohiNDGcoQXcM6D14nMBy` |
| `hopper-vault` | `UNnH6wbtV3RdATZ1F6YNFvLcCfTJ3JDex9MYb44pYsV` | Fresh deploy, `--max-len 50000`, signature `4hjuTtMhXCpVCD6iN2G4BEBCmHc7C7GXxCq8KT7wpnGEt8sB5HtBnofQz79kqs6wpqRWSFN1bsBC18TMhkzMQdb9` |
| `hopper-compact-vault` | `6aKUB52fa1KmGTh11GCuMhixKk9Sgo2nDrmsmMz8DZvs` | Fresh deploy after fixing missing `cdylib` artifact output, `--max-len 50000`, signature `9QpuQjtQ8B85tDnMa7HEj7fejVLQ3SWaqMjWEjfc21CS5aeTzZcGYrqwCy5qWj7ts6BGgQxgZGxyYhPXhpggB46` |
| `hopper-migration` | `7CuuiKRWqs6JPFbyfMZdAKedWULAAUBnzFRPee46bu2d` | Fresh deploy after rent top-up fix, `--max-len 50000`, signature `2EXcwtrhHm1xqyLLSjAqugAqfpmFx2XnDkSGPawRCdYMCRPCrxjfXdSzaRzLgWGSz5nj8xWuXxdCd2WmfXP1dXJG` |
| `hopper-orderbook` | `9EpzXZKmdHnkxWayAMoaxxxwgHehe2arQ3chVY9Tmvyr` | Fresh deploy after large-account devnet flow fix, `--max-len 50000`, signature `44U5LcACjsR5mWNrtRmddet7i8DtcHQe3bUAoFNVWGVbNXnPWBm6N8UYmV3JTUkdm5JiLFkmWxKPTLFDpxmd2aZ3` |

Existing deployment confirmed:

| Program | Program id | Notes |
|---|---|---|
| `hopper-devnet-audit` | `4LPSXhMpx2DrFvMSHXRB3yaGmz7iKP4nKkfD92mAtAdT` | Already deployed and upgradeable under the Hopper authority; exercises dynamic tails, contexts, segments, remaining accounts, proof checks, Token-2022 policy checks, and substrate probes. |

Live audit runner passed on devnet:

- State: `EAQdR2FjcEHuerPV4c2yhwc9Z8crtk6YRmeMtsuRntCV`
- Verified: `counter=1`, `substrate_passes=1`, `remaining_signer_checks=2`, `proof_checks=1`, `token_policy_checks=1`, `field_capability_checks=1`, `label=hopper-live`, `members=1`.
- The same program id did not exist on the custom Flux endpoint used during this pass, so the live smoke run used `https://api.devnet.solana.com`, where the program was deployed.

Additional live devnet scenarios passed:

- `hopper-compact-vault`: exact 41-byte compact account, initialize, deposit, byte-level layout check, and manifest fingerprint guard.
- `hopper-escrow`: make round-trip plus cancel/take close paths.
- `hopper-migration`: V1 account initialized at 56 bytes, rent top-up funded through System Program transfer, migrated in place to 65-byte V2 with a changed layout id.
- `hopper-orderbook`: top-level pre-created 139,356-byte segmented account, `InitBook`, then `PostBid` touching the bids segment.
- `cross-program-read`: Program A initialized/deposited, Program B read and minimum-balance checked through Hopper interface validation, host runner verified owner/layout/balance.
- Token/payment compile lane: `hopper-token-2022-vault`, `hopper-token-2022-ata`, `hopper-token-2022-transfer-hook`, and `hopper-stablecoin-memo-pay` passed host tests; stablecoin memo pay and Token-2022 vault built to SBF.

Final authority balance after the additional deploy/test pass: `6.46773926 SOL`.

Verified program accounts:

- `hopper-counter`: BPF Loader Upgradeable, ProgramData `ESL2Nee1ZKpgh9ML53F2Hu1N9jBWJysuNzAGkt2vcru5`, data length `50000`.
- `hopper-vault`: BPF Loader Upgradeable, ProgramData `A3kHNKsixGppKWMdzXMQsY6dTkYK2hvLJYDpvtZWnqwy`, data length `50000`.

## Buffer Hygiene Findings

`solana program show --buffers --url devnet --keypair <hopper-authority>` found
three dangling buffers owned by the Hopper authority:

| Buffer | Balance |
|---|---:|
| `5nggEaBxC8RNRBjZUn811XT9wGR9MZCS94xscqKZpNBV` | `0.14235288 SOL` |
| `7vpEiV7UHrsp9MH9CqWi9vCFGwpPFvrbrGfTvwdCekEz` | `0.14235288 SOL` |
| `CfTRupUw6WmTR8H7JDx7dXrWHo5FuBAE92gAxfMVznLm` | `0.14235288 SOL` |

Attempted cleanup through Hopper:

```text
cargo run --manifest-path D:\tmp\Hopper-Solana-Zero-copy-State-Framework\Cargo.toml \
  -p hopper-cli -- close --buffers --cluster devnet \
  --keypair C:\Users\matts\KEYPAIRS_BLUEFOOT_LABS\HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT.json \
  --yes
```

Result: Hopper built and forwarded the correct Solana CLI command, but public
devnet RPC failed while sending the close transaction:

```text
Error: error sending request for url (https://api.devnet.solana.com/)
Command failed: solana program close --buffers --url https://api.devnet.solana.com ...
```

Retrying the bulk cleanup path through a custom Flux RPC endpoint also failed
with a request/response body error. Treat this as a bulk RPC resiliency issue,
not a Hopper parser failure.

Final cleanup used the single-buffer Solana CLI path:

```text
solana program close <buffer> --url devnet --keypair <hopper-authority> --bypass-warning
```

Each known buffer now returns `AccountNotFound` when queried directly:

| Buffer | Final status |
|---|---|
| `5nggEaBxC8RNRBjZUn811XT9wGR9MZCS94xscqKZpNBV` | `AccountNotFound` |
| `7vpEiV7UHrsp9MH9CqWi9vCFGwpPFvrbrGfTvwdCekEz` | `AccountNotFound` |
| `CfTRupUw6WmTR8H7JDx7dXrWHo5FuBAE92gAxfMVznLm` | `AccountNotFound` |

Post-cleanup authority balance: `8.50542646 SOL`.

The public devnet bulk listing command remained intermittently RPC-unstable
after cleanup, so the reliable evidence is the direct `AccountNotFound` result
for each previously known buffer. Recommended follow-up: add retry/backoff,
redacted custom-RPC hints, and explicit single-buffer fallback guidance around
deploy/close failures.

### CLI Bug Fixed In This Pass

`hopper close` previously parsed `--program-id` and `--buffer` through one
generic helper and handled `--buffers` separately. That allowed ambiguous forms
like `--program-id X --buffer Y`, duplicate target flags, or `--program-id X
--buffers` to silently choose one target and leave the rest as Solana passthrough
arguments.

Fix: `tools/hopper-cli/src/cmd/lifecycle.rs` now parses close targets as a
single enum and rejects anything except exactly one of:

- `--program-id <pubkey>`
- `--buffer <pubkey>`
- `--buffers`

Validation:

```text
cargo test --manifest-path D:\tmp\Hopper-Solana-Zero-copy-State-Framework\Cargo.toml \
  -p hopper-cli close_target_requires_exactly_one_target --locked
```

## Framework Position

| Area | Hopper | Anchor current/v2-track | Quasar | Pinocchio |
|---|---|---|---|---|
| Hot-path model | no_std/no_alloc native runtime, compact or headered zero-copy layouts | mature DX, but heavier account model and 8-byte discriminators | no_std zero-copy framework | substrate SDK, not a full framework |
| Account identity | `AccountDescriptor` / `LayoutDescriptor` feeds loader, registry, IDL, fingerprints, cost hints | IDL plus account metadata, but no Hopper-style governed registry | account macro plus zeropod-derived layout | manual |
| Compact layouts | 1-byte compact accounts and tiny instruction profile | no | yes | manual |
| Dynamic data | bounded compact tails, `String<'a, N>`, `Vec<'a, T, N>`, `TailStr`, `TailBytes` | Borsh-oriented for most variable data | strong bounded dynamic fields | manual |
| PDA bumps | stored-bump path exists; context derives bumps and supports stored bumps | strong account constraint UX | strong stored bump optimization | manual |
| Schema/client tooling | TS/Kotlin/Python/Go/C/Rust, Codama/Anchor-style exports, manifest registry | excellent TS/IDL ecosystem | TS/Python/Go/C/Rust; no Kotlin/Codama edge | none |
| SVM/testing | Hopper-owned harness exists, FFI/bindings exist, but traces can improve | mature test workflows | strong trace-oriented Quasar-SVM | none |
| Unique moat | segment borrows, receipts, policy graph, governed registry, descriptor fingerprints | ecosystem/network effects | clean dynamic field DX and SVM trace UX | minimal substrate |

## Gap Triage

Several risks that looked like possible gaps are already addressed in Hopper:

- Dynamic fields: implemented via bounded tails and documented in `docs/DYNAMIC_FIELDS_QUASAR_TO_HOPPER.md`.
- All-zero account discriminator rejection: present in `crates/hopper-macros-proc/src/state.rs` for headered and compact layouts.
- Stored bump optimization: present in context lowering and declarative macro helpers; the remaining gap is first-touch discoverability and benchmarking, not the primitive.
- `set_inner`: generated for state/account layouts and used by live examples.
- Remaining accounts: strict and passthrough helpers exist, including bounded signer parsing.
- PDA resolver metadata: schema-level resolver descriptors and manifest account metadata exist.

Real gaps to prioritize:

1. Generated clients must consume PDA resolver metadata automatically. The schema has enough data; the DX win comes when TS/Kotlin/Python/Rust builders derive accounts without handwritten PDA glue.
2. Stored-bump UX needs a golden-path example and CU assertion. Hopper has the optimization, but Quasar makes it feel obvious.
3. SVM traces should become first-class output: per-instruction CU, CPI stack depth, return data, account diffs, token balance deltas, and a JSON trace format.
4. `hopper deploy` / `hopper close` need operational resiliency: retry/backoff for public devnet, redacted custom-RPC suggestions, and possibly `hopper buffers list|close` as explicit buffer hygiene commands.
5. Multi-owner interface accounts need a polished proc-macro surface. Lower-level multi-owner helpers exist, but Token vs Token-2022 polymorphism should be first-touch.
6. The on-chain manifest/governed registry story should be wired into generated clients as a default fail-closed decode guard.
7. The devnet audit program should have a one-command live smoke runner that exercises every instruction after deployment.
8. Docs should stop framing solved Quasar parity items as open questions and instead sell Hopper's actual differentiators: descriptor coherence, segment borrows, receipts, policies, registry, Kotlin.

## Priority Roadmap

1. Client PDA auto-resolution from manifest resolvers.
   - Rationale: eliminates the most common client-side drift bug.
   - Test: generated TS/Kotlin/Python clients build an initialize instruction by taking only user-supplied accounts; tests assert derived PDAs match on-chain seeds.

2. Buffer hygiene command group.
   - Rationale: devnet deploy failures strand rent, and program authors need a safe recovery path.
   - Shape: `hopper buffers list`, `hopper buffers close --all`, `hopper buffers close <buffer>`, JSON output, redacted RPC failure hints, retry/backoff.
   - Test: parser unit tests plus a mocked Solana CLI invocation verifying exact command construction.

3. Stored-bump golden path.
   - Rationale: Quasar makes this optimization visible; Hopper should make it automatic and measured.
   - Test: example with `bump: u8`, `#[account(seeds = ..., bump = state.bump)]`, and CU/trace assertion that the stored-bump path avoids `find_program_address` in validation.

4. Trace-grade Hopper SVM output.
   - Rationale: complex CPI debugging is where frameworks win developer loyalty.
   - Shape: JSON trace with outer instruction, inner CPI frames, CU per frame, logs, return data, pre/post account summaries, token deltas.
   - Test: fixture that creates an ATA and transfers tokens; snapshot trace includes the expected CPI tree.

5. Interface account macro polish.
   - Rationale: accepting Token and Token-2022 through one account type should be boring.
   - Shape: `#[account(owner_any = [token::ID, token_2022::ID])]` or a typed `InterfaceAccount<'info, T>` wrapper.
   - Test: one handler accepts both mint owners and rejects unrelated owners with a clear error.

6. Manifest-backed fail-closed clients.
   - Rationale: `AccountDescriptor` and fingerprints are a Hopper moat only if generated clients use them by default.
   - Test: generated client refuses to decode when advertised fingerprint differs from embedded fingerprint.

7. Devnet audit runner.
   - Rationale: `hopper-devnet-audit` is deployed but should be a reproducible live proof.
   - Shape: `cargo run -p hopper-devnet-audit --features devnet-client --bin devnet_audit -- --program-id ...` documented and CI-gated behind `HOPPER_DEVNET=1`.
   - Test: initialize, rename, add member, segment increment, substrate probe, remaining signers, proof probe, token policy probe, field capability probe.

8. Docs consolidation against competitors.
   - Rationale: Hopper now has several Quasar parity items; docs should lead with current truth.
   - Shape: update `COMPARISON.md`, `WHY_HOPPER.md`, and website docs to say where Hopper wins outright and where Quasar is still ahead on polish.

9. Deploy-size and upgrade-headroom policy.
   - Rationale: fresh deploys used `--max-len 50000`; this should become a named default or recommendation per package class.
   - Test: `hopper deploy --dry-run` prints artifact bytes, requested max-len, rent estimate, and upgrade headroom.

10. Release-blocking operational smoke.
    - Rationale: a zero-copy program framework is only credible when the CLI, deploy, explain, close, and generated clients work together.
    - Shape: one scripted lane: build SBF, deploy to devnet, run live smoke, list buffers, close abandoned buffers, explain a transaction, emit manifest/client artifacts.

## Bottom Line

Hopper's primitives are stronger than the initial comparison suggested. The
framework already has many of the Quasar parity features and has several unique
advantages no competitor has: segment-safe account borrows, receipts, policy
graphs, governed descriptors/registry, and Kotlin/Codama-fluent client metadata.

The next leap is DX integration: make the strongest primitives automatic in the
first five minutes, make live deploy/buffer recovery boring, and make generated
clients fail closed by default.