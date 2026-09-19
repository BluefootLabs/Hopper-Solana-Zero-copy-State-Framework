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

#### Authority gate: `--authority-baseline <old-manifest> [--baseline-so <old.so>] [--authority-report <out.json>] [--authority-approval <reviewed.json>]`

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
interface commitment embedded in the released ELF, and `--release` requires it,
so both sides of the diff are bound to their binaries. Compatibility tools that
score relaxations as additive changes answer a different question; run both.

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
