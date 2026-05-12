# Hopper CLI reference

Every subcommand `hopper` ships today, grouped by workflow phase. Commands that accept passthrough cargo arguments say so; everything else documents its own flags.

## Lifecycle

### `hopper init <path>`

Scaffold a new Hopper project. Writes a `Cargo.toml`, a `src/lib.rs` with a minimal `#[program]` module, and a `tests/` directory.

Flags:

- `--name <name>` - override the package name (defaults to the directory name)
- `--local-path <repo-root>` - point `Cargo.toml` deps at a local Hopper checkout instead of crates.io
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
benchmark placeholders or stale in-tree benchmark paths, the default feature
tree excludes Pinocchio, legacy SPL Token builders stay behind
`legacy-token-instructions`, client generators still assert layout IDs, and the
fuzz target inventory is present.

Use `--source-only` before an SBF build to run every non-binary gate. Add
`--full` to also run the `hopper-systems` and `hopper-trybuild` suites.

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
- `rust-client` - off-chain Rust client SDK
- `idl` - Anchor-shaped IDL JSON
- `codama` - Codama-shaped JSON
- `schema` - Hopper program manifest JSON

Use `--out <path> --force` to write a file and `--lint` to run `hopper lint`
after emitting.

## Client generation

### `hopper client gen --ts <manifest>` / `--kt <manifest>` / `--py <manifest>`

Emit a typed TypeScript, Kotlin, or Python client from the manifest. Supported
shapes: instruction builders, account readers, PDA helpers, event decoders. Use
`hopper compile --emit rust-client <manifest>` for the off-chain Rust client
target and `hopper compile --emit <ts|kt|py|rust-client|idl|codama|schema>` for
one-shot manifest-source inference via `--package` or `--program-id`.

## Inspection

### `hopper inspect <hex-data>`

Parse raw account bytes and print the decoded header, discriminator, version, and layout id.

### `hopper inspect [layout | segments | receipt] ...`

Drill-downs for each piece: named segment offsets, receipt wire-format decode.

### `hopper explain [account | receipt | compat | policy | layout | program | context]`

Human-readable narratives. `explain receipt <hex>` turns a raw receipt into "Invariant `balance_nonzero` failed at stage Invariant, code 0x1001".

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
