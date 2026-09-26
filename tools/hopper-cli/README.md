# hopper-cli

Create, build, test, and inspect Hopper programs from the command line.

Start a project with `init` and build its on-chain artifact with `build`.
Generate clients for application integration, inspect account data, and review
declared permissions before upgrades. Inspection over supplied files works
offline; deployment and live account or transaction queries use RPC.


## Install

Install the 0.4.0 release:

```bash
cargo install hopper-cli --version 0.4.0 --locked
```

Registry availability is tracked on the [release status page](https://hopperzero.dev/docs/release-status).
To build from a source checkout:

```bash
cargo install --path tools/hopper-cli --locked
# or: cargo run --locked -p hopper-cli -- help
```

## Commands

```
Compile
  hopper compile --emit <rust|ts|kt|py|go|c|rust-client|idl|codama|schema|manifest> [<manifest>|--package <name>|--program-id ...]
                                      Emit lowered Rust, client SDKs, Hopper public IDL, Codama, or manifest
  hopper compile --emit manifest --package <name>  Generate hopper.manifest.json (the Effect ABI carrier) from package source

Verify
  hopper verify [<manifest>] [<.so>]        Check manifest integrity and report layout-anchor diagnostics
  hopper verify --release <manifest> <.so>  Require the exact versioned interface commitment in the ELF
  hopper verify --package <name>            Infer manifest and SBF binary from a workspace package
  hopper verify --effects <bundle|dir>      Effect gate: verify evidence bundles against the manifest's
                                            published write contract (changed ⊆ acquired ⊆ authorized,
                                            via the separately runnable Grillo verifier); any violation fails
  hopper verify --authority-baseline <old-manifest> [--baseline-so <old.so> | --baseline-program <id>]
                [--candidate-buffer <addr> | --candidate-program <id>] [--cluster <name|url>]
                                            Authority gate: exit 2 when an instruction gains authority
                                            (dropped signer, new writable, wider field ranges, removed PDA,
                                            has_one, owner, or address binding, new CPI program), exit 3
                                            for review; --authority-report / --authority-approval record
                                            and approve a reviewed widening for that exact manifest pair
  hopper publish-check --package <name>     Run interface binding, docs, feature, client, and fuzz gates
  hopper publish-idl --manifest <path> --program-id <pubkey> [--cluster <name>|--url <rpc>] [--yes] [--dry-run]
                                            Publish a lossless Solana IDL v0.1.0 projection through Program Metadata
  hopper publish-security --file security.json --program-id <pubkey> [--init] [--read] [--dry-run]
                                            Publish, scaffold, or read back the security.txt record (seed "security")
  hopper publish-manifest --manifest <path> --program-id <pubkey> [--read] [--dry-run]
                                            Publish or read back the full Hopper manifest (seed "hopper-manifest")

Schema
  hopper schema export                         Static account-schema format reference
  hopper schema export --manifest <manifest>   Normalize Hopper manifest JSON
  hopper schema export --idl <manifest>        Hopper public IDL JSON
  hopper schema export --codama <manifest>     Codama-shaped JSON
  hopper schema export --anchor-idl <manifest> --program-id <pubkey>  Conditional current Solana IDL v0.1.0 projection
  hopper schema validate <manifest>  Validate a program manifest
  hopper schema diff @old-layout.json @new-layout.json  Field-level diff between versions

Inspect
  hopper inspect <hex>               Raw header decode
  hopper inspect layout <manifest> <hex>  Decode fields using a program manifest
  hopper inspect segments <hex>      Segment registry map
  hopper inspect receipt <hex>       Decode a state receipt

Explain
  hopper explain <hex>               Human-readable headered-account explanation
  hopper explain account <hex>       Explicit headered-account explanation
  hopper explain receipt <hex>       Explain a receipt in plain English
  hopper explain compat @old-layout.json @new-layout.json  Explain compatibility report
  hopper explain policy <pack>       Explain a named policy pack
  hopper explain layout <manifest>   Explain layout fields, intents, fingerprint
  hopper explain program <manifest>  Explain entire program pipeline
  hopper explain context <manifest> [--type <ContextName>]  Explain instruction contexts and generated accessors
  hopper explain instruction <manifest> <tag|name>  Explain one instruction's accounts and policy
  hopper explain <tx-signature>      Decode a confirmed transaction against local or application-provisioned legacy manifests
  hopper explain <program-id>        Fetch an application-provisioned legacy manifest PDA and explain the program

Compatibility
  hopper compat @old-layout.json @new-layout.json       Compatibility report
  hopper compat --why @old-layout.json @new-layout.json Compatibility report with explanation
  hopper plan @old-layout.json @new-layout.json         Migration plan with steps

Lifecycle
  hopper init [path]                 Create a Hopper-native project scaffold with minimal, NFT, Token-2022, or DeFi templates
  hopper add [-i|-s|-e <name>]       Scaffold instruction, state, or error files
  hopper build [--host|--sbf]        Build the current project (default: SBF)
  hopper test                        Run host-side tests for the current project
  hopper deploy [--dry-run] [--no-build]  Quote live loader-v3 rent or build and deploy with Cargo.lock pinned
  hopper upgrade --program-id <id> [--cluster <c>]  Rebuild and upgrade a deployed program in place
  hopper close --program-id <id> [--cluster <c>]    Close a program or buffer and reclaim rent
  hopper migrate --program-id <id> [--cluster <c>]  Upgrade a program carrying a layout migration
  hopper buffers <list|close>        List or close stranded loader buffers
  hopper dump [--no-build]           Disassemble the built SBF binary
  hopper clean [-a|--all]            Remove generated build artifacts while preserving keypairs

Keys
  hopper keys new <path>             Generate a program/keypair json file
  hopper keys list [<path>...]       List pubkey and path for each keypair
  hopper keys print <path>           Print the base58 pubkey of a keypair
  hopper keys sync <path> [--src <file.rs>]  Sync declare_id! from a keypair pubkey
  hopper keys pda <seed>... [--program <id>]  Derive a PDA and canonical bump

Config
  hopper config get <key>            Read a saved Hopper CLI config value
  hopper config set <key> <value>    Write a saved Hopper CLI config value
  hopper config list                 Show saved Hopper CLI config values

Transactions
  hopper tx explain <signature>      Fetch and explain an on-chain transaction
  hopper tx send --program <id> ...  Send one instruction with explicit metas and hex data, signed locally
  hopper tx send --v1 ...            Same, as a SIMD-0385 transaction v1 envelope (4,096 bytes, config mask)
  hopper tx simulate <tx-base64>     Simulate a pre-built transaction
  hopper tx submit <tx-base64>       Submit a pre-built transaction

Shell
  hopper completions <shell>         Emit bash, zsh, fish, or PowerShell completions
  hopper version                     Print CLI version and linked schema version

Profiling
  hopper profile bench               Run the primitive benchmark lab and emit JSON/CSV artifacts
  hopper profile elf <program.so>    Static SBF symbols, CU-ish estimates, sections, flamegraph export
  hopper profile elf <program.so> --baseline <folded.txt> --fail-on-growth <bytes> --fail-on-growth-pct <pct>
                                     Size gate: exit 2 when .text grew past both thresholds

Contention
  hopper contention <manifest>       Per-instruction write-lock and signature footprint the
                                     declaration fixes (no measurement), plus the accounts a
                                     proven write set shows are read-only
  hopper contention <manifest> --max-block-cost <CU>   Fail (exit 1) over a ceiling: a CI gate
                                     on declared lock footprint. Not a transaction's block
                                     cost (the requested CU limit dominates that) and not
                                     the compute a handler burns

Audit evidence
  hopper audit-check [--strict]      Verify content-addressed audit evidence, freshness, and blockers

Project Health
  hopper lint                        Run Hopper project diagnostics
  hopper lint zc                     Scan typed-context sources for zero-copy footguns
  hopper lint --deny-escapes         Policy-escape audit as CI errors (flags ledger-bypassing accessors)
  hopper solana-check [--all]        Check SBF crate shape and Hopper entrypoint invariants
  hopper expand                      Show lowered macro output for the current project
  hopper doctor                      Check toolchain and workspace health
  hopper feature-gate [--cluster <c>] [--json] [--require <name-or-pubkey>]...  Check finalized runtime prerequisites

Adversarial Testing
  hopper fuzz generate --program <manifest> [--out <plan>] [--corpus <dir>]
                                     Generate deterministic seeded cases and invariant hooks
  hopper fuzz check --program <manifest> [--plan <plan>]
                                     Fail when a committed case plan is stale
  hopper fuzz run --program <manifest> --adapter <executable> [--plan <plan>]
                                     Execute cases through an application adapter; fail closed on
                                     missing results, skips, failures, or unchecked invariants

Direct aliases
  hopper decode <hex>                Alias for inspect
  hopper segments <hex>              Alias for inspect segments
  hopper receipt <hex>               Alias for inspect receipt / receipt
  hopper diff @old-layout.json @new-layout.json  Alias for schema diff
  hopper schema-export               Static account-schema format reference

Client SDK
  hopper client gen --ts <manifest>  Generate TypeScript client SDK
  hopper client gen --kt <manifest>  Generate Kotlin client SDK (org.sol4k)
  hopper client gen --py <manifest>  Generate Python client SDK
  hopper client gen --go <manifest>  Generate Go client SDK
  hopper client gen --c <manifest>   Generate C client header
  hopper actions gen --program <manifest> --out api/actions  Generate Solana Actions route scaffolds
  hopper mobile gen --program <manifest> --target kotlin|react-native  Generate mobile binding stubs
  hopper test-gen security --program <manifest>  Generate a security test matrix scaffold

Fetch
  hopper fetch <program-id> [--rpc <url>] [--json]  Read an application-provisioned legacy MANIFEST_SEED PDA

Interactive
  hopper interactive <manifest>      Interactive terminal explorer
  hopper ui <manifest>               Alias for interactive

Manager
  hopper manager summary <manifest>  Program overview
  hopper manager identify <manifest> <hex>  Identify account type
  hopper manager decode <manifest> <hex>  Decode all fields with values
  hopper manager instruction <manifest> <tag|name>  Instruction details and policies
  hopper manager layouts <manifest>  List all layouts with fields
  hopper manager policies <manifest>  List policy packs with mappings
  hopper manager events <manifest>   List events with fields
  hopper manager fingerprints <manifest>  Show all layout fingerprints
  hopper manager compat <manifest> <hex-old> <hex-new>  Compare two account versions
  hopper manager receipt <hex>       Decode a state receipt
  hopper manager explain <manifest>  Full human-readable summary
  hopper manager diff <manifest> <hex-before> <hex-after>  Semantic field-level diff
  hopper manager simulate <manifest> <instruction>  Preview instruction requirements
  hopper manager fetch <program-id> [--rpc <url>]  Read a legacy MANIFEST_SEED PDA and show its summary
  hopper manager interactive <manifest>  Interactive terminal explorer
```

`compile --emit idl` emits Hopper's public IDL; the current Solana IDL v0.1.0
projection is the separate `schema export --anchor-idl` path. That projection
is fail-closed and succeeds only when Hopper's wire and account surface is
losslessly representable. It currently refuses Cicada because its u16-prefixed
bounded `route_data` and dynamic remaining-account contract cannot be encoded
faithfully. The supplied `--program-id` is the expected address written at the
IDL's top level; this offline export does not query RPC or prove deployment.
The projection carries Hopper's exact wire discriminators and marks account
bodies with custom `hopper-zero-copy-v1` serialization. Consumers therefore
need a Hopper-aware decoder rather than Anchor's Borsh codec.

The short compatibility, diff, and plan commands interpret unprefixed inputs as
inline layout JSON. Prefix file paths with `@`.

When run inside a Hopper package that already contains `hopper.manifest.json`,
`hopper compile --emit rust` can infer that local manifest automatically. Use
`--package <name>` to target another workspace member and `--out <path>` to
write the lowered preview instead of printing it.

Use `hopper solana-check --all --build-sbf` before publishing deployability
claims. It checks the same crate shape the Solana SBF workflow enforces:
`cdylib` output, exactly one Hopper backend feature, path-qualified SBF macros,
and a successful `cargo build-sbf` for each program crate. Actions, mobile, and
security-test commands are manifest-backed scaffolds unless a project extends
the generated files into a production workflow.

Docs: <https://docs.rs/crate/hopper-cli>

## Support

Hopper is open-source Solana infrastructure. Public-goods support and donations
can be sent to `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

Donation URI: <solana:F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT?label=solanadevdao.sol>

## License

Apache-2.0
