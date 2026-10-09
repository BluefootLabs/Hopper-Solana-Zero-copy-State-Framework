# Checked borrowed batches: October 8 verification

This archive records working-source validation for the unreleased
`BoundedSlice` argument path and bounded Rust client codecs. It is not a clean
release attestation or independent security review. The review date is local
to America/Chicago; transaction and quality-gate timestamps use UTC.

## Executed boundary

The no-allocator fixture accepts up to 32 ten-byte orders through manual and
generated dispatch. Both paths validate every representation before applying
checked count/amount arithmetic, check account ownership and privileges, and
verify that the view still points into the original instruction buffer.

- Formatting and full-workspace Clippy (all targets, warnings denied): passed.
- Full workspace tests: 2,507 reported passes, zero failures, 281 ignored.
  Ignored cases are not executions; explicit compiled VM cases are separate.
- Fresh SBF builds: v0 (3,728 bytes) and v3 (3,000 bytes), platform-tools 1.57.
- All 53 workspace deployable packages build for SBF v3 from a fresh Cargo
  target with unchanged source and no stack-frame diagnostic. This is compile
  evidence, not execution of all handlers or a clean hosted-CI attestation.
  All 52 existing default v3 binaries are byte-identical to the October 7
  artifacts; the batch fixture is the only new binary. Earlier execution
  evidence remains scoped to the exact matching bytes and workloads.
- Compiled VM execution: 1,626 instruction cases per architecture, 3,252 total.
  Successful cases compare return bytes and account state. Every refusal
  compares all supplied accounts with their original values.
- Public devnet: 44 finalized transactions across manual/generated runs,
  including two state-account creations and 36 expected refusals. The deployed
  ELF matches the local v3 build before and after each run. Successes verify
  return-data producer and bytes; refusals preserve the complete fixture state.
- Generated Rust and TypeScript clients execute independent expected-wire
  checks, capacities, element widths, UTF-8 lengths, and account privileges.
  Rust output compiles against real Solana instruction/key crates through a
  matching import facade. Nested application encoders are not inferred.

Devnet program:
[CtbmtFBWz9uF7AmhPJHtbovjqVgRJPd7Rjf1xco5Msid](https://explorer.solana.com/address/CtbmtFBWz9uF7AmhPJHtbovjqVgRJPd7Rjf1xco5Msid?cluster=devnet).
Each run starts with fresh state. The fixture accepts any signer and does not
implement custody authorization or token transfers.

| Devnet workload | Manual CU | Generated CU |
| --- | ---: | ---: |
| Empty batch | 235 | 235 |
| Two orders, one absent option | 281 | 285 |
| 32 orders | 883 | 887 |

These are observed whole-instruction costs for this fixture, including its
checks, state writes, and return data. They do not establish a performance
advantage over another framework. The benefit demonstrated is a checked view
without a capacity-sized owned argument array.

## Evidence map

- `source.json`: normalized input inventory captured before SBF compilation;
  both live runners require it to remain unchanged.
- `source-inputs.tar.gz` and `source-inputs.json`: non-audit working files and
  their raw hashes. Ignored build products and verification archives are
  excluded; this is not a committed release checkout.
- `sbf-builds.json`, `build-*.log`, `v0/`, `v3/`: fresh-target commands, build
  logs, artifact sizes and hashes, and public ELF binaries.
- `all-programs-v3/`: the source-bound build matrix, all 53 logs, and their
  public ELF artifacts. The report retains the original ignored output paths;
  archived package directories preserve the same file names and byte hashes.
- `compiled-vm.json` and `compiled-vm-*.log`: test executable hash, tested ELF
  hashes, case counts, outcomes, and scoped compute samples. The executable
  was built by the workspace test command and invoked directly while other
  workspace targets linked; no VM case is inferred from a build alone.
- `deployment.json`, `devnet-manual/`, `devnet-generated/`: deployment
  provenance, exact transactions, account snapshots, return data, and binary
  comparisons. Private keys and raw deployment recovery transcripts are
  excluded.
- `client-check/`: generated source, compiled Rust/TypeScript checks, SDK lock,
  logs, and the result. Clients check wire shape; the program enforces nested
  representation and application rules. The archived runner uses a separate
  Cargo target directory with the same assertions; provenance records that
  host scheduling adjustment.
- `binary-continuity.json`: per-package comparison against the retained
  October 7 default v3 artifacts: 52 identical, zero changed, one new.
- `peer-sources.json`: hashes and links for eight specific files across
  Pinocchio, Pina, Quasar, and Anchor v2. This is a source-boundary comparison,
  not an exhaustive parity or security ranking.
- `website-source-inputs.tar.gz` and `website-source-inputs.json`: retained
  website inputs, raw hashes, and recorded source deletions, excluding
  dependencies and build output.
- `website-source.json`, `website-gates.json`, `website-check.json`, and
  `website-qa/`: source inventory, build/type/lint results, 62 page checks with
  5,550 internal links, and production-browser checks/screenshots at desktop
  and mobile widths. Keyboard controls, malformed inputs, docs navigation,
  reduced motion, overflow, and browser errors are checked.
- `static-checks.json` and associated logs: unsafe contracts, review drift,
  documentation citations, and quality-gate regression checks. The API log
  verifies 28 library snapshots.
- Quality-gate receipts retain their complete source inventory, commands,
  exit status, and log hash. The dependency audit uses pinned RustSec revision
  `550efd3d587a29b2e2c2b21b17a440da4fede999`: 1,295 advisories, zero known
  vulnerabilities, four unmaintained-package notices.

## Client coverage limits

This pass verifies generated Rust and TypeScript clients. Kotlin, Python, Go,
and C instruction emitters still compute fixed argument offsets and are not
supported for bounded dynamic arguments. Their existing fixed-only output is
outside this codec change. Use the tested Rust/TypeScript generators or encode
the documented wire contract explicitly for these instructions.

The new borrowed-slice decoder publishes metadata through aliases. Aliases
of existing owned `BoundedString`/`BoundedVec` decoders still inherit unknown
size/fixed encoding metadata; spell those owned container types directly.
An alias-aware contract for those existing decoders remains follow-up work.

## Host build scheduling

`host-build-attempts.json` identifies two interrupted build attempts: a serial
workspace build and a subsequent run contending on the shared Cargo target.
Their logs remain as diagnostics and are not passing quality receipts. The
selected workspace and Clippy gates use `target/borrowed-slices-host-check`
with four jobs. Their receipts retain the complete executed command and source
inventory. No test assertion or source input changed to accommodate scheduling.

## Stale local token-lab artifact

The first workspace run loaded an older `target/deploy/hopper_token_lab.so`
and failed two compiled-program assertions with `InvalidInstructionData`.
The stale artifact's SHA-256 begins `e1f7da2a`; the freshly rebuilt program's
full hash is
`e9c65d672d6db5d647a078fbacb6af8d29e646440c52d898d53d9f536fe24488`.
The fresh program is byte-identical to the October 7 verified v3 artifact and
passes both unchanged tests. `token-lab-diagnostic/` retains the initial
failure, both artifacts, the fresh build log, and the passing execution.
Only the local build product was replaced. No test expectation or program
check was weakened. Workspace quality receipts describe the subsequent run.

The prior October 7 archives remain immutable evidence for their own source
and workloads. This pass adds one program; it does not re-execute all other
framework programs or close the independent-review and hosted-SBF-CI gates.
The observed BLAKE3, EpochSchedule, transaction-v1, and coverage limitations
remain tracked in release readiness.
