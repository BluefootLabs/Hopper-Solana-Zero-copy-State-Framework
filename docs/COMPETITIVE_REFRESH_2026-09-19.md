# Competitive and network refresh, 2026-09-19

This refresh records what changed between the
[2026-09-06 reverification](COMPETITIVE_REFRESH_2026-09-02.md) and
2026-09-19. It supersedes the time-sensitive rows it names and leaves the rest
of that document in force. The audit-readiness rule still applies: no "best",
"fastest", "safest", or universal-uniqueness claim without an independent
report or a reproducible fixture. Every fact below is dated; re-run the ledger
at the end before carrying it into external material.

## 1. Network corrections

| Earlier statement | Observed 2026-09-19 | Consequence |
| --- | --- | --- |
| Transaction v1 is not active | The `enable_tx_v1` gate (`txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL`) activated on mainnet-beta at slot 447,120,000, epoch 1035, 2026-09-15. SIMD-0385 limits: 4,096 bytes, 64 accounts, 64 instructions. The SIMD header still reads Review. | Hopper's CLI still builds legacy envelopes capped at 1,232 bytes. Its oversize error no longer says v1 is inactive; it says this CLI does not emit v1 yet. |
| Rent is 6,333 lamports per byte | 5,080 lamports per byte since the `set_lamports_per_byte_to_5080` gate activated at slot 446,256,000 on 2026-09-11. `getMinimumBalanceForRentExemption(0)` returned 650,240 (128 x 5,080). The next reduction step had no mainnet feature account. | Every SOL quote in older docs is historical. `hopper deploy --dry-run` reads live rent and is unaffected. |
| Cicada loader-v3 deployment locks 1.053 SOL; a minimal instance 1.113 SOL | Recomputed with R(n) = (128 + n) x 5,080 for the same 165,944-byte diagnostic ELF: Program 833,120 + ProgramData 843,874,360 = **0.844707480 SOL**. Adding Config (1,219,200) and one 20-slot Shard (46,776,640) gives **0.892703320 SOL**. | Derived from the live coefficient, not a new deployment. Fees excluded, principal refundable. |

Unchanged at the same pass: SIMD-0449 direct account pointers, Account Data
Direct Mapping, and Alpenglow had no mainnet feature account. Agave v4.3.0 was
tagged 2026-09-18.

## 2. Competitor delta

| Project | Change since 2026-09-06 | Consequence for Hopper |
| --- | --- | --- |
| **Anchor** (`otter-sec/anchor`) | Tags v0.30.2, v0.31.2, v0.32.2 on 2026-09-14 (TypeScript v1-transaction parsing backports). No new crate release: `anchor-lang` 1.2.0 and 2.0.0-rc.1 remain newest. Unreleased v1 `master` (`1eb46ec`, 2026-09-18) has `anchor init` emit `security.json` and publish it through Program Metadata's `security` seed (`a69be6b`, #4177), and replaces CreateAccount plus fallback with System `CreateAccountAllowPrefund` (`0d2b8cb`, #5057). The v2 line (`abacd0e`) landed about 30 correctness commits, including close to a non-writable destination, CPI handle validation, discriminator collisions under `cfg`, and Slab post-shrink length. | No write-range, effect, or authority work. One concrete gap remains: Hopper does not yet publish security metadata through Program Metadata. The prefunded-account gap closed the same day (section 3a). |
| **Quasar** | No commit on any ref since `d981ac8` (2026-08-02). Crates remain 0.0.0. | No change to the Sept 6 assessment. |
| **Pinocchio** | No commit since `adbd48d` (2026-08-03); 0.11.2 remains the release. `main` still parses the SIMD-0449 pointer table ungated while 0449 is inactive on mainnet. | Keep mainnet fixtures pinned to 0.11.2. |
| **pina / pinapod** | 0.12.2 to **0.19.0** between 2026-09-06 and 2026-09-18 (133 commits, HEAD `b90be75`). Forked zeropod into pinapod 0.4.1. Added schema-history ABI migrations with a per-cluster publication ledger binding `executable_sha256` to `schema_sha256` (`42f4429`), `pina profile compare` with a 500 CU and 10% regression threshold (`852fcbe`), a Mollusk-verified cross-framework benchmark covering pina, Pinocchio, Quasar, and Anchor v2 (`9ef0ccc`), a HIR lint driver with 13 security lessons (`8d746d5`), and generated CLIs. | The fastest-moving competitor on tooling. Its tree has no byte-range, touch, or effect authority. Hopper is absent from its benchmark matrix; adding a Hopper row is the cheapest public comparison available. |
| **QEDGen / qedsvm** | Unchanged: `bf7f968` / v2.49.0 and `99bd5ed` / v0.12.0. | Sept 6 integration plan stands. |
| **Ratchet** (`saicharanpogul/ratchet` `4ce0c73`, 0.4.0) | 20 diff rules and 7 preflight rules over Anchor and Quasar IDLs. Polarity is client compatibility: rule R010 classifies a signer or writable flag relaxing from true to false as `Additive` and passes it. No rule covers added instructions, CPI targets, lamport permissions, owner or `has_one` changes, or byte ranges. | Credit it for compatibility. It answers "do callers break", not "did the program gain power". |
| Star Frame, Steel, Typhoon, Light | No framework release. | None. |
| Tooling | Surfpool v1.6.0 (2026-09-18) with an sBPF debugger and v1 transactions through LiteSVM. LiteSVM `d432db1` (2026-09-10) exposes state snapshots in its Node bindings. | LiteSVM snapshots make the typed field-diff adapter (Sept 6 item 3) cheaper. |

## 3. Shipped in this pass: the upgrade authority gate

The Sept 6 refresh ranked a release-bound authority diff as open work. It now
exists:

- `grillo_manifest::authority` diffs two Hopper manifests. It matches
  instructions by exact discriminator bytes and accounts by role name, then
  classifies every change as widened, review, narrowed, or informational.
- Widening covers a dropped signer, a read-only account that became writable,
  new instructions and writable accounts, byte ranges that reach another
  layout field, removed exact-cell rules, new lamport permissions, a lost
  `strict_writes` or lamport contract, a raised remaining-account ceiling, and
  weaker context constraints (PDA seeds, `has_one`, owner or address checks,
  optionality, and new `init`, `realloc`, or `close` lifecycles). A seed swap
  or a different expected CPI program goes to review.
- Byte ranges are compared per layout field, so inserting a field that shifts
  every offset is not reported as a new permission, while header bytes are
  named as header bytes.
- `grillo authority-diff old new` and
  `hopper verify --authority-baseline old --baseline-so old.so` exit 2 on an
  unapproved widening and 3 on an unapproved review item. A reviewed report
  approves its findings only for the manifest pair whose canonical-JSON
  SHA-256 digests it records. Under `--release`, the baseline must match the
  interface commitment embedded in its released ELF.

Searches of the repositories named above found no public tool that fails a
release when declared authority widens. Ratchet's polarity is the opposite,
and pina's migration ledger binds executables to schemas, not to authority.
That is a scoped finding about named public repositories as of 2026-09-19,
not a proof that no private or differently named implementation exists.

The gate compares declarations. It does not inspect bytecode, prove handler
behavior, or cover constraints absent from the manifest. Its value rests on
the other two legs: `hopper verify --release` binds each manifest to its ELF,
and Grillo checks observed effects against the declared ranges.

The loop from the Sept 6 refresh is therefore now **Declare, Enforce,
Observe, Verify, Compare**.

Later the same day the Compare step became ledger-bound:
`--baseline-program` reads the deployed ProgramData ELF and
`--candidate-buffer` reads the loader Buffer holding a pending upgrade, and
each must carry its manifest's interface commitment before the diff runs.

## 3a. Learned from the competition this pass

- **`CreateAccountAllowPrefund`** (Anchor v1 #5057, v2 #4945): shipped in
  Hopper's `init` lifecycle as one CPI, verified against agave v4.3.0
  `system_processor.rs` (tag 13, account order `[to, from]`, lamports as a
  delta, payer omitted at zero) and the feature account on all three
  clusters.
- **Crate type defeats LTO** (pina #407): confirmed and adopted; see the
  changelog for the measured sizes. The earlier "LTO is a near no-op for
  SBF" note in this repository was an artifact of the dual crate type.
- **Anchor v2 correctness fixes since 2026-09-05** were checked one by one
  against Hopper: close to a non-writable destination (#4886, immune),
  CPI-handle validation (#5043, immune), cfg-gated discriminator collisions
  (#5015, immune by construction), Slab post-shrink length (#4906, hardened:
  a stored count above capacity is now refused at load), and tail-slab
  minimum length (#4888, **Hopper shared the class**: `safe_realloc` could
  shrink below the layout minimum; fixed with a `required_len()` floor in
  every generated realloc accessor). All five are pinned in the regression
  suite.
- **pina 0.19** items assessed: its Mollusk matrix state-verification rule
  and the dual absolute-plus-relative regression gate are worth adopting;
  its local hash-chained publication ledger is weaker than Hopper's on-chain
  publication and was not copied.

## 4. Ranked follow-up work

1. **Multisig-proposal review.** The gate now reads ProgramData and Buffer
   accounts directly. The remaining step is decoding a Squads V4 upgrade
   proposal to its buffer address so signers can run the gate from the
   proposal alone, and rendering the widenings for a signer who has no
   manifest on hand.
2. **Publish the authority verdict and security metadata through Program
   Metadata.** Hopper already has the Program Metadata writer used by
   `hopper publish-idl`. Adopt the `security` seed convention Anchor's `master`
   now uses, and define a versioned custom seed for the authority report.
3. **Transaction v1 envelopes in the CLI.** Build and size-check v1 sends where
   the payload needs more than 1,232 bytes, with golden fixtures.
4. **A Hopper row in pina's Mollusk matrix**, with the same state-verification
   rule that matrix already applies to the other frameworks, and the same
   post-state rule applied to Hopper's own four-framework vault matrix.
5. **Declarative value validation** (`min`, `max`, `max_len`) on layouts and
   instruction arguments, generated on the typed load path with no
   formatting or heap use.

## 5. Reproduction ledger

- Finalized mainnet feature-account queries and
  `getMinimumBalanceForRentExemption` on 2026-09-19 near slot 448.31M.
- GitHub commit and tag history plus crates.io metadata for Anchor
  (`1eb46ec`, `abacd0e`), Quasar (`d981ac8`), Pinocchio (`adbd48d`), pina
  (`b90be75`), zeropod 0.3.6 (`74c4757`), QEDGen (`bf7f968`), qedsvm
  (`99bd5ed`), Ratchet (`4ce0c73`), Surfpool v1.6.0, and LiteSVM (`d432db1`),
  read on 2026-09-19.
- Authority-gate behavior: `crates/grillo-manifest/src/authority.rs` unit
  tests and `crates/grillo-manifest/tests/authority_diff_real_manifests.rs`
  over the checked-in Sentinel and Cicada manifests.
