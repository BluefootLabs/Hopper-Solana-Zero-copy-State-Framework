# Hopper CLI reference

Every subcommand `hopper` ships today, grouped by workflow phase. Commands that accept passthrough cargo arguments say so; everything else documents its own flags.

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

### `hopper deploy <manifest> [--keypair <path>] [--program-keypair <path>] [--url <cluster>]`

Build, upload, and register the program on a cluster. Reads cluster URL and default keypair paths from `~/.hopper/config.toml` when flags are omitted.

### `hopper dump <manifest>`

Disassemble the compiled `.so` artifact into a human-readable listing.

### `hopper verify --manifest <path> [--so <program.so>] [--strict | --release]`

Compare a compiled program's ABI fingerprint against the manifest's `LAYOUT_ID`
values. Manifest integrity is always fatal on mismatch. Binary anchor scanning is
informational by default, fatal with `--strict`, and required + fatal with
`--release`.

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

### `hopper schema export [--manifest | --idl | --codama | --anchor-idl]`

Print the schema for the current program as a Hopper manifest, Hopper IDL,
Codama-shaped JSON, or Anchor-shaped IDL JSON.

### `hopper schema validate <manifest.json>`

Static validation of a manifest file.

### `hopper schema diff <old.json> <new.json>`

Field-level diff between two manifest versions. Emits a compatibility verdict: `compatible`, `warning`, or `incompatible`, with per-field reasons.

## Compile and emit

### `hopper compile --emit <target> [<manifest> | --package <name> | --program-id <id>]`

Emit artifacts from a local, package-inferred, or fetched manifest. Targets:

- `rust` - lowered Hopper runtime preview for auditing accessors and offsets
- `ts` - TypeScript client SDK
- `kt` - Kotlin client SDK
- `py` - Python client SDK
- `go` - Go client SDK
- `c` - C client header
- `rust-client` - off-chain Rust client SDK
- `idl` - Anchor-shaped IDL JSON
- `codama` - Codama-shaped JSON
- `schema` - Hopper program manifest JSON

Use `--out <path> --force` to write a file and `--lint` to run `hopper lint`
after emitting.

## Client generation

### `hopper client gen --ts <manifest>` / `--kt <manifest>` / `--py <manifest>` / `--go <manifest>` / `--c <manifest>`

Emit a typed TypeScript, Kotlin, Python, Go, or C client from the manifest.
Supported shapes: instruction builders, account readers with layout-id checks,
account metadata helpers where the target language has a neutral representation,
and event decoders. Use `hopper compile --emit rust-client <manifest>` for the
off-chain Rust client target and
`hopper compile --emit <ts|kt|py|go|c|rust-client|idl|codama|schema>` for
one-shot manifest-source inference via `--package` or `--program-id`.

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
signatures), `Saved` (write-lock CU demotion removed), `Proven RO` — the
non-signer accounts a mutation-complete write set makes read-only across
Hopper's supported governed APIs — and `Rem`, the ceiling on caller-supplied
remaining accounts. A client that marks such an account writable pays a flat
300 CU write lock and serializes on it for nothing. This result does not cover
arbitrary unsafe, FFI, dependency, direct Hopper Native, or unchecked-CPI
paths; hand-written clients must review those escapes before demotion. Signers
are excluded because the fee payer must stay writable at the transaction
level.

`--max-block-cost <CU>` turns it into a CI gate: exit 1 if any instruction's
`Fixed max` exceeds the ceiling. It fails closed — a zero-instruction manifest,
two positional manifests, a repeated ceiling flag, and a missing path are all
refused rather than silently passing.

`Fixed max` is an **upper bound on the declaration's fixed-role share** of a leader's price, not a
transaction's block cost: Agave also charges the requested compute limit
(200,000 CU by default and usually the largest term), the requested
loaded-data limit, instruction bytes, and the fee payer's own lock. Nor is it
the compute a handler burns — Hopper never fabricates that. See
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

### `hopper fetch <program-id>`

Pull the on-chain manifest PDA for a program. Prints the stored manifest and any schema-epoch migration history.

### `hopper manager fetch | summary | identify | decode`

Introspect the `hopper-manager` account layer. `fetch` dumps raw data, `summary` prints a one-line program health check, `identify` resolves a program ID to a declared name, `decode` extracts a specific account under the manager.

## Migrations

### `hopper compat <old.json> <new.json> [--why]`

Focused compatibility report. `--why` annotates every decision.

### `hopper plan <from-epoch> <to-epoch>`

Print the exact migration chain Hopper would execute to bridge two schema epochs.

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

Terminal UI for browsing live accounts under a program. Navigation matches the Helius Explorer key bindings.

## Shell completions

### `hopper completions <bash | zsh | fish | powershell>`

Emit a shell-completion script for the selected shell. PowerShell users can run
`hopper completions powershell >> $PROFILE`, then restart the shell.

## Compatibility aliases

Pre-existing short-command forms, kept so older scripts still work:

- `hopper decode <hex>` - alias for `inspect`
- `hopper segments <hex>` - alias for `inspect segments`
- `hopper receipt <hex>` - alias for `inspect receipt`
- `hopper compat <a> <b>` - alias for the full compat command
- `hopper diff <a> <b>` - alias for `schema diff`
- `hopper plan <a> <b>` - alias for the migration-plan command
- `hopper schema-export` - alias for `schema export --manifest`

## Global flags

- `--help` / `-h` on any command prints its own usage banner
- `hopper help` prints the top-level command list
