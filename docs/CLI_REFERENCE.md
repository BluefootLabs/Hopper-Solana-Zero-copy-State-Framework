# Hopper CLI reference

Release-facing `hopper` workflows, grouped by phase. The command's own `--help`
output is authoritative for the complete, current flag surface; this guide does
not claim to enumerate every diagnostic or compatibility alias.

## Lifecycle

### `hopper init [path]`

Scaffold a new Hopper project. With no path, opens the interactive wizard. With
`path`, uses saved defaults unless `--interactive` is supplied. Writes a
`Cargo.toml`, a `src/lib.rs`, and a `tests/` directory shaped by the selected
template.

Flags:

- `--name <name>` - override the package name (defaults to the directory name)
- `--template, -t <name>` - choose `minimal`, `nft-mint`,
  `token-2022-vault`, `defi-vault`, or `quasar-port`
- `--local-path <repo-root>` - point `Cargo.toml` deps at a local Hopper checkout instead of crates.io
- `--yes, -y` - skip prompts and use saved defaults from `~/.hopper/wizard.toml`
- `--interactive` - force the wizard even when `path` is supplied
- `--no-git` - skip `git init` and the initial commit
- `--force` - overwrite an existing directory

### `hopper build [--host | --sbf] [cargo args...] [--watch]`

Compile the program. `--sbf` (default) targets the Solana runtime. `--host` builds for the host triple, useful for unit tests. Every unknown flag passes straight to `cargo build`. `--watch` re-runs the build whenever `src/`, `tests/`, or `Cargo.toml` changes.

### `hopper test [cargo args...] [--watch]`

Run `cargo test` in the nearest project root. Flags and filters pass through to cargo. `--watch` re-runs tests on save.

### `hopper deploy [--dry-run] [--no-build] [-p|--package <crate>] [--cluster <name>] [solana program deploy args]`

Build and upload the current program on a cluster through the Solana CLI. This
command does not register a Hopper manifest or Program Metadata record. The automatic
SBF build is manifest-scoped and passes `--locked` to Cargo. Use `--no-build`
only with an artifact you built and hashed separately. Cluster, keypair,
program-id, and loader arguments are forwarded to the Solana CLI.

`--dry-run` sends no transaction. It queries the chosen RPC at the requested
commitment (default `confirmed`), records the slot at which those reads began,
and quotes loader-v3 rent over the exact artifact. Permanent Program +
ProgramData principal is separated from the temporary Buffer balance that the
stock Solana CLI recycles during deployment. Transaction and priority fees are
excluded.

### `hopper dump [--no-build] [-p|--package <crate>] [--tool <objdump>] [--out <path>] [-S|--source]`

Build and disassemble the current `.so` artifact into a human-readable listing,
or inspect an existing artifact with `--no-build`.

### `hopper verify --manifest <path> [--so <program.so>] [--strict | --release] [--effects <bundle-or-dir>] [--authority-baseline <old-manifest>]`

Check manifest integrity and compare it with a compiled program. `--release`
requires the ELF's versioned SHA-256 commitment to match the manifest's
canonical executable-interface commitment exactly. The commitment covers the
program name and version, account layouts, instruction discriminators,
arguments and account metas, events, policy contracts, and the context fields
represented by `ProgramManifest`. It deliberately excludes constraints absent
from that manifest, descriptions, measured CU estimates, compatibility plans,
and Manager-only hints.

The per-layout `LAYOUT_ID` search is a supplemental diagnostic. It is
informational by default and fatal only when `--strict` is explicitly present;
raw eight-byte occurrences are not accepted as release-interface proof.

#### Authority gate: `--authority-baseline <old-manifest> [--baseline-so <old.so> | --baseline-program <id>] [--candidate-buffer <addr> | --candidate-program <id>] [--cluster <name|url>] [--authority-report <out.json>] [--authority-approval <reviewed.json>]`

Diff the previously released manifest against this one and fail when any
instruction gains authority. Instructions match by exact discriminator bytes
and accounts by role name. Widening includes a dropped signer, a read-only
account that became writable, a new instruction or writable account, byte
ranges that reach another layout field (compared per field, so a moved field is
not a new permission), a removed exact-cell rule, new lamport permissions, a
raised remaining-account ceiling, and weaker context constraints (PDA seeds,
`has_one`, owner or address checks, optionality, a new `init`, `realloc`, or
`close` lifecycle). A different PDA seed list or a different expected CPI
program is reported for review.

Exit status is `2` for an unapproved widening and `3` for an unapproved review
item. `--authority-report` writes the report JSON; after review, pass that file
back as `--authority-approval`. It covers only the exact manifest pair whose
digests it records. `--baseline-so` requires the baseline manifest to match the
interface commitment embedded in the released ELF; `--baseline-program <id>`
does the same against the ELF deployed under that program id, read from its
ProgramData account on `--cluster` (devnet unless named; mainnet must be
explicit). `--candidate-buffer <addr>` requires the current manifest to match
the ELF in a loader Buffer, which is how a pending upgrade is reviewed before
it is applied, and `--candidate-program <id>` reviews one after the fact.
`--release` requires a bound baseline, so both sides of the diff are tied to
their binaries. Compatibility tools that score relaxations as additive changes
answer a different question; run both.

### `hopper publish-check [--package <name> | --manifest <path> --so <program.so>] [--full]`

Run the public release gate. This wraps `hopper verify --release` with the
source checks that keep release artifacts honest: release-facing docs have no
benchmark scaffolds or stale in-tree benchmark paths, the default feature
tree excludes Pinocchio, legacy SPL Token builders stay behind
`legacy-token-instructions`, client generators still assert layout IDs, and the
fuzz target inventory is present.

Use `--source-only` before an SBF build to run every non-binary gate. Add
`--full` to also run the `hopper-systems` and `hopper-trybuild` suites.

### `hopper solana-check [--all] [--manifest-path Cargo.toml] [--build-sbf]`

Check that Hopper program crates are shaped for Solana instead of merely
Rust-valid: `cdylib` output, Solana `no_std` intent, Hopper allocator and panic
markers, a generated or explicit entrypoint, path-qualified SBF macros, and the
direct Hopper runtime dependency shape. Use `--all` to scan program-shaped packages below the
workspace root. Use `--build-sbf` in CI when you want the gate to run
`cargo build-sbf` too.

## Keys and identity

### `hopper keys new <path>`

Generate a fresh ed25519 keypair and write it as the json byte-array format that `solana-keygen` emits. Prints the pubkey to stdout.

### `hopper keys list [<path>...]`

Print pubkey plus path for every keypair. No args walks `target/deploy/*.json`.

### `hopper keys print <path>`

Emit just the base58 pubkey. Convenient in shell pipelines.

### `hopper keys sync <keypair-path> [--src <file.rs>]`

Rewrite the first `declare_id!("...")` or `hopper::declare_id!("...")` in
source so it matches a Solana keypair json file. Defaults to `src/lib.rs`; pass
`--src` for multi-program workspaces or generated source layouts.

### `hopper keys pda <seed>... --program <program_id>`

Derive a PDA from the given seeds. Seed formats:

- `b"text"` - UTF-8 bytes of `text`
- `hex:0a1b2c` - hex-encoded bytes
- `base58:...` - base58-encoded bytes (for pubkey seeds)
- anything else - treated as raw UTF-8

Prints the PDA, the canonical bump, and a normalized seed description.

## Global config

### `hopper config get <key>` / `set <key> <value>` / `list` / `reset` / `path`

Flat key-value store at `~/.hopper/config.toml`. Known keys:

- `cluster_url` - either `mainnet` / `devnet` / `localnet` or a full URL
- `payer` - path to the fee-payer keypair json
- `default_program_id` - fallback when a command needs a program id
- `default_keypair` - fallback upgrade-authority keypair
- `default_manifest` - fallback manifest json

CLI flags always override config values.

## Schema and IDL

### `hopper schema export [--manifest <manifest> | --idl <manifest> | --codama <manifest> | --anchor-idl <manifest> --program-id <pubkey>]`

With no flag, print the static Hopper account-schema format reference. The
flagged forms transform a supplied manifest into normalized Hopper manifest
JSON, Hopper public IDL, Codama-shaped JSON, or current Solana IDL v0.1.0 JSON.
Solana IDL export is conditional and fail-closed: it succeeds only when
Hopper's wire and account surface is losslessly representable. It currently
refuses Cicada because its u16-prefixed bounded `route_data` and
`execute_intent` remaining-account contract cannot be encoded faithfully.
Hopper account bodies are marked with custom `hopper-zero-copy-v1`
serialization and require a Hopper-aware decoder; they are not Anchor Borsh
accounts. `--program-id` supplies the expected address encoded at the top
level. This export is offline: it does not query RPC or prove that the address
is deployed.

### `hopper publish-idl --manifest <path> --program-id <pubkey> [--cluster <name> | --url <rpc>] [--keypair <path>] [--overwrite] [--yes] [--dry-run]`

Project a losslessly representable manifest to Solana IDL v0.1.0 and publish it
through the official Program Metadata program. Small payloads use inline
Initialize; fresh larger payloads use Allocate/Write/Initialize; `--overwrite`
uses SetData when it fits one transaction. A large overwrite that would require
chunking is refused. This publishes only the IDL projection, not Hopper's full
manifest, write authority, touch evidence, or Effect ABI contract.
Named devnet is the default. Named Mainnet and every raw, custom, or
`SOLANA_RPC_URL` endpoint require an explicit `yes` confirmation unless
`--yes` is supplied. `--dry-run` exits before target resolution, prompting,
keypair loading, or network access.

### `hopper publish-security --file <security.json> --program-id <pubkey> [--cluster <name> | --url <rpc>] [--keypair <path>] [--seed <str>] [--overwrite] [--allow-incomplete] [--allow-unknown-keys] [--yes] [--dry-run]`

Publish a program's `security.txt` record through Program Metadata at the
canonical `[program, "security"]` PDA, the record Solana Explorer reads on the
program page and the successor to the ELF-embedded `security_txt!` section.
Same header, same Utf8 + Zlib + Json tags, same signed-send path and cluster
guards as `publish-idl`; the signer must be the program's upgrade authority.
The document is validated before anything is compressed: unknown keys are
refused (the consumer parser ignores them, so a typo would publish silently),
every value must be a string or an array of strings, and `name`,
`project_url`, `contacts`, and `policy` must be present and non-empty. The
published bytes are the minified document with keys sorted.

`hopper publish-security --init [path]` writes a `security.json` template with
every known field empty (it fails validation until filled in).
`hopper publish-security --read --program-id <pubkey>` fetches the published
record, decodes the header, inflates the payload, and prints it; `--seed` reads
any other record at the canonical PDA.

### `hopper publish-manifest --manifest <path> --program-id <pubkey> [--cluster <name> | --url <rpc>] [--keypair <path>] [--seed <str>] [--overwrite] [--yes] [--dry-run]`

Publish the normalized Hopper manifest, the document `hopper verify
--authority-baseline` diffs, at the canonical `[program, "hopper-manifest"]`
Program Metadata PDA. Program Metadata permits custom seeds but does not
reserve them, so this is a Hopper convention: the record puts the exact
declaration next to the program it describes so a reviewer can fetch the
baseline from the ledger instead of trusting a file handed over out of band.
Large manifests take the Allocate, chunked Write, Initialize path; a large
`--overwrite` is refused like `publish-idl`. `--read` fetches and prints the
published manifest. Publishing the declaration does not bind it to the
deployed ELF; `hopper verify --release` and `--baseline-program` do that.

### `hopper schema validate <manifest.json>`

Static validation of a manifest file.

### `hopper schema diff @old-layout.json @new-layout.json`

Field-level diff between two layout versions. Emits a compatibility verdict:
`compatible`, `warning`, or `incompatible`, with per-field reasons.

## Compile and emit

### `hopper compile --emit <target> [<manifest> | --package <name> | --program-id <id>]`

Most targets normalize a supplied, package-inferred, or fetched manifest.
`manifest` is the exception: it generates `hopper.manifest.json` from a
package's exported `PROGRAM_MANIFEST` and therefore requires `--package`.
Targets:

- `rust` - lowered Hopper runtime preview for auditing accessors and offsets
- `ts` - TypeScript client SDK
- `kt` - Kotlin client SDK
- `py` - Python client SDK
- `go` - Go client SDK
- `c` - C client header
- `rust-client` - off-chain Rust client SDK
- `idl` - Hopper public IDL JSON (not the Solana IDL v0.1 projection)
- `codama` - Codama-shaped JSON
- `schema` - normalize an existing or fetched Hopper program manifest as JSON
- `manifest` - generate `hopper.manifest.json` from package source (`--package` required)

The six SDK targets, Hopper public IDL, Codama JSON, and conditional Solana IDL
form nine interop formats. The full manifest/schema and lowered Rust audit
preview are separate artifacts.

Use `--out <path> --force` to write a file and `--lint` to run `hopper lint`
after emitting.

## Client generation

### `hopper client gen --ts <manifest>` / `--kt <manifest>` / `--py <manifest>` / `--go <manifest>` / `--c <manifest>`

Emit a typed TypeScript, Kotlin, Python, Go, or C client from the manifest.
Supported shapes: instruction builders, account readers with layout-id checks,
account metadata helpers where the target language has a neutral representation,
and event decoders. Hopper account bodies use Hopper's wire format and require
these Hopper-aware readers; a generic Anchor Borsh decoder is insufficient. Use
`hopper compile --emit rust-client <manifest>` for the
off-chain Rust client target and
`hopper compile --emit <ts|kt|py|go|c|rust-client|idl|codama|schema>` for
one-shot manifest-source inference via `--package` or `--program-id`.
Use `hopper compile --emit manifest --package <name>` when no manifest has yet
been generated from that package's `PROGRAM_MANIFEST` export.

### `hopper actions gen --program <manifest> --out <dir> [--framework next]`

Generate a Solana Actions scaffold from the manifest. The first target is Next:
`actions.json` plus a `route.ts` that exposes instruction tags and CORS-safe
GET/POST handlers.

### `hopper mobile gen --program <manifest> --target <kotlin | react-native> [--out <dir>]`

Generate mobile binding stubs from the manifest. Kotlin emits an instruction tag
object. React Native emits a TypeScript helper with typed instruction names.

### `hopper test-gen security --program <manifest> [--out <path>]`

Generate a security test matrix with per-instruction cases for missing signer,
wrong owner, wrong PDA, wrong layout, non-writable mutable accounts, and token
extension mistakes when token capabilities are present.

### `hopper fuzz generate --program <manifest> [--out <plan.json>] [--corpus <dir>]`

Derive a deterministic adversarial plan from the supported contracts published
in the manifest. Coverage includes layout identity and truncation, every field
edge, declared compatibility policy and backward readability, bounded
arguments, signer and writable roles, Accounts-derived typed, PDA, lifecycle,
has-one, explicit owner/address, and optional-account constraints, policy
requirements and invariants, every duplicate-account pair, static and
parametric write boundaries, declared lamport permissions, and
remaining-account limits. Every case includes a stable 128-bit seed and
required invariant hooks.
`--corpus` writes one JSON adapter seed record per case; these records are not
raw libFuzzer byte inputs.

### `hopper fuzz check --program <manifest> [--plan <plan.json>]`

Regenerate the plan in memory and compare its complete structured content with
the committed contract plan. CI fails when source changes add or alter a layout,
instruction, context constraint, compatibility pair, policy, layout metadata,
account role, or effect declaration without updating the fuzz surface.
Hopper's Cicada gate first emits the manifest from source, so stale
checked-in manifests cannot make this check pass.

### `hopper fuzz run --program <manifest> --adapter <executable> [options]`

Execute generated cases through an application-owned SVM or program-test
adapter. The runner writes one `hopper.manifest-fuzz-request.v1` JSON object to
adapter stdin and requires one strict `hopper.manifest-fuzz-response.v1` JSON
object on stdout. Diagnostics belong on stderr.

Useful options are `--plan <plan.json>` to require a current committed plan,
repeatable `--case <exact-id>` for replay, repeatable `--adapter-arg <value>`,
repeatable `--require-invariant <hook>` for application properties, and
`--report <report.json>` for an archived result. Missing, duplicate, unknown,
failed, or skipped cases and missing invariant confirmations fail by default.
`--allow-skips` is intended only for local adapter bring-up. The complete
protocol and an example are in `fuzz/README.md`.

The manifest derives hostile mutations, seeds, and structural hooks. The
adapter supplies valid business-state fixtures and executes either the real
program path or an explicitly identified host-equivalent enforced primitive
when a production instruction cannot request the hostile operation. Therefore
`generate` plus `check` is coverage planning, while `run` plus a no-skip adapter
is execution evidence only at the level the adapter reports. Cicada's 698-case
adapter is host semantic evidence paired with a separate compiled-SBF lifecycle
suite; it is not evidence of 698 SBF transactions.

## Project linting

### `hopper lint zc [--project <path>] [--fail-on-warn]`

Scan typed-context sources for zero-copy footguns: duplicate manual signer,
writable, and owner checks that should usually live in account constraints,
account-data copies into `Vec`, and deserialization calls where a Hopper view is
usually the safer path. The lint stays conservative and treats raw
remaining-account checks as review items, not hard errors unless
`--fail-on-warn` is set. `hopper lint svm` remains as a compatibility alias.

## Contention

### `hopper contention <manifest> [--max-block-cost <CU>]`

Report the write-lock and signature footprint each instruction *declares*,
computed from the manifest's account list plus the same `writeRanges` /
`lamportAccounts` the runtime enforces. Role counts are exact, offline, and
reproducible; their CU product is a fixed-role upper bound because optional
roles may be absent and duplicate roles may share one Pubkey.

Columns are `W` (accounts declared writable), `W-eff` (still writable after
sound demotion), `Sigs`, `Fixed max` (fixed-role write locks after demotion,
plus
signatures), `Saved` (write-lock CU demotion removed), `Proven RO`, the
non-signer accounts a mutation-complete write set makes read-only across
Hopper's supported governed APIs, and `Rem`, the ceiling on caller-supplied
remaining accounts. A client that marks such an account writable pays a flat
300 CU write lock and serializes on it for nothing. This result does not cover
arbitrary unsafe, FFI, dependency, direct Hopper Native, or unchecked-CPI
paths; hand-written clients must review those escapes before demotion. Signers
are excluded because the fee payer must stay writable at the transaction
level.

`--max-block-cost <CU>` turns it into a CI gate: exit 1 if any instruction's
`Fixed max` exceeds the ceiling. It fails closed, a zero-instruction manifest,
two positional manifests, a repeated ceiling flag, and a missing path are all
refused rather than silently passing.

`Fixed max` is an **upper bound on the declaration's fixed-role share** of a leader's price, not a
transaction's block cost: Agave also charges the requested compute limit
(200,000 CU by default and usually the largest term), the requested
loaded-data limit, instruction bytes, and the fee payer's own lock. Nor is it
the compute a handler burns, Hopper never fabricates that. See
[CONTENTION.md](CONTENTION.md) for the full cost model, the constants, and
their sources.

## Inspection

### `hopper inspect <hex-data>`

Parse raw account bytes and print the decoded header, discriminator, version, and layout id.

### `hopper inspect [layout | segments | receipt] ...`

Drill-downs for each piece: named segment offsets, receipt wire-format decode.

### `hopper explain [account | receipt | compat | policy | layout | program | context | instruction]`

Human-readable narratives. `explain receipt <hex>` turns a raw receipt into "Invariant `balance_nonzero` failed at stage Invariant, code 0x1001".
`explain instruction <manifest> <tag|name>` prints the instruction's account order, signer/writable requirements, argument bytes, capabilities, policy pack, and receipt expectation.

## Transactions

### `hopper tx send --program <pubkey> [--data <hex>] --account <pubkey|payer>[:s][:w]... --keypair <path> [--signer <path>]... [--rpc <url>] [--compute-limit <units>] [--allow-failure] [--dry-run] [--v1 [--loaded-data-limit <bytes>] [--priority-fee <lamports>]]`

Send one instruction with explicit, ordered account metas and raw hex data,
signed locally. Every signer-flagged slot must be covered by the fee payer or
a `--signer` keypair, checked before any RPC round trip. `--allow-failure`
skips preflight so an on-chain refusal lands and is reported as data.
`--dry-run` prints the plan and the wire size without touching the network.
After confirmation the transaction is fetched back and its measured compute
units and fee are printed.

`--v1` builds a SIMD-0385 transaction v1 envelope instead of legacy: a
4,096-byte ceiling, up to 64 addresses, 64 instructions, and 12 signatures,
serialized with the wire codec the RPC client uses for v1. The compute-unit
limit (default 200,000), the loaded-accounts-data-size limit (default 4 MiB),
and the optional priority fee travel in the message's config mask; the
runtime treats an absent limit as zero, so both limits are always set, and a
ComputeBudget instruction is never added to a v1 send because v1 executes it
without honoring it. `--priority-fee` is a total in lamports, not a per-CU
price. The v1 gate is active on mainnet-beta (slot 447,120,000), devnet, and
testnet; `--loaded-data-limit` and `--priority-fee` are refused without
`--v1` rather than silently dropped.

### `hopper tx explain <signature> [--rpc <url>]`

Fetch a confirmed transaction (legacy, v0, or v1) and explain every
instruction against the touched Hopper programs' manifests.

## On-chain fetch

### `hopper fetch <program-id> [--rpc <url>] [--json]`

Fetch the legacy Hopper `MANIFEST_SEED` PDA for a program. By default the CLI
parses the stored JSON and prints a `ProgramManifest` summary; `--json` prints
the stored JSON. This command does not discover a Program Metadata IDL and does
not report independently observed schema-epoch migration history.

### Selected `hopper manager` commands

- `manager fetch <program-id>` fetches that same legacy manifest PDA and prints
  the parsed program summary, not raw bytes.
- `manager summary <manifest>` prints a manifest summary.
- `manager identify <manifest> <hex-data>` matches supplied headered account
  bytes to a declared layout; it does not map a program ID to a name.
- `manager decode <manifest> <hex-data>` decodes supplied headered account bytes
  against that layout; it does not fetch an account "under" a manager.

## Migrations

### `hopper compat @old-layout.json @new-layout.json`

Focused compatibility report. Use
`hopper compat --why @old-layout.json @new-layout.json` for the explanatory
path. The `@` prefix is required for file input; without it these short commands
interpret the argument as inline layout JSON.

### `hopper plan @old-layout.json @new-layout.json`

Generate a field-level migration plan between two layout JSON objects. It
reports policy, sizes, copy/zero-fill spans, and backward readability; it does
not inspect a package or prove that an on-chain migration executed.

## Profiling

### `hopper profile bench [options]`

Run the primitive benchmark lab against a live cluster. Emits JSON and CSV regression artifacts. See `hopper profile bench --help` for the full flag list.

### `hopper profile elf <path/to/program.so>`

Static SBF ELF analysis. Prints the top N symbols by size, estimated SBF instruction counts, and a static CU-ish prioritization score. It can also write Brendan-Gregg folded-stack input or a self-contained HTML flamegraph for public launch demos and regression review.

See [PROFILING.md](PROFILING.md) for the release profile workflow and same-provenance benchmark checklist.

Flags:

- `--top N` - how many symbols to print (default 20)
- `--folded out.txt` - write flamegraph input
- `--html out.html` - write an interactive single-file flamegraph
- `--baseline folded.txt` - compare symbol sizes against a previous run
- `--sections` - include the largest ELF sections by size
- `--open` - open the HTML flamegraph after writing it
- `--no-demangle` - skip rustc-demangle on symbol names

Global profile option:

- `-w`, `--watch` - re-run `profile bench` or `profile elf` on source changes

## Runtime features

### `hopper feature-gate [--cluster <cluster-or-url>] [--json] [--require <name-or-pubkey>]...`

Observe runtime feature accounts in one finalized RPC snapshot. The report
records the genesis hash, observation slot, feature keys and activation slots.
Known names include `SIMD-0321`, `SIMD-0385`, `SIMD-0449`, `SBPF-v3`, and
`SIMD-0512`. An optional positional name or pubkey queries one feature;
use repeated `--require` instead to enforce deployment prerequisites.

```bash
hopper feature-gate --cluster devnet --require SIMD-0321 --require SIMD-0449 --json
```

Exit 0 means a valid observation and all requested requirements active.
Exit 2 means a required feature is absent or pending. Exit 1 means invalid
arguments, an RPC failure, a named-cluster genesis mismatch, or malformed
feature-account evidence, including a wrong owner. Without `--require`,
absent features do not make a valid report fail. The default is devnet.
`--url` and `-u` are aliases for `--cluster`.

These are RPC observations, not authenticated ledger proofs. A proposal's
status or a feature key in Agave source does not prove network activation.
An active feature also does not change the transaction format a client emits.

## Interactive

### `hopper interactive <manifest>` or `hopper ui <manifest>`

Terminal UI for exploring a supplied manifest, or a legacy manifest fetched by
program ID. It can decode account bytes pasted as hex; it does not enumerate or
subscribe to live program accounts.

## Shell completions

### `hopper completions <bash | zsh | fish | powershell>`

Emit a shell-completion script for the selected shell. PowerShell users can run
`hopper completions powershell >> $PROFILE`, then restart the shell.

## Compatibility aliases

Pre-existing short-command forms, kept so older scripts still work:

- `hopper decode <hex>` - alias for `inspect`
- `hopper segments <hex>` - alias for `inspect segments`
- `hopper receipt <hex>` - alias for `inspect receipt`
- `hopper compat @a.json @b.json` - compatibility report for two layout files
- `hopper diff @a.json @b.json` - alias for `schema diff`
- `hopper plan @a.json @b.json` - migration plan for two layout files
- `hopper schema-export` - static Hopper account-schema format reference

## Global flags

- `--help` / `-h` prints help at the top level and on commands that expose it
- `hopper help` prints the top-level command list
