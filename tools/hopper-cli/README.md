# hopper-cli

Command-line tooling for inspecting, explaining, and managing Hopper programs.

This is the operator's interface to Hopper. Feed it hex-encoded account data and
a program manifest and it will decode headers, explain fields, diff versions,
plan migrations, decode receipts, and generate client SDKs. Works entirely
offline by default, with optional RPC connectivity for live account fetching.

## Install

Install from crates.io:

```bash
cargo install hopper-cli
```

For local development inside this workspace:

```bash
cargo run -p hopper-cli -- help
```

## Commands

```
Compile
  hopper compile --emit <rust|ts|kt|py|rust-client|idl|codama|schema> [<manifest>|--package <name>|--program-id ...]
                                      Emit lowered Rust, client SDKs, IDL JSON, Codama, or manifest

Verify
  hopper verify [<manifest>] [<.so>]        Confirm manifest layouts are present in the compiled binary
  hopper verify --package <name>            Infer manifest and SBF binary from a workspace package
  hopper publish-check --package <name>     Run release docs, feature, client, fuzz, and ABI gates

Schema
  hopper schema export [--manifest|--idl|--codama|--anchor-idl]  Schema format reference
  hopper schema validate <manifest>  Validate a program manifest
  hopper schema diff <old> <new>     Field-level diff between versions

Inspect
  hopper inspect <hex>               Raw header decode
  hopper inspect layout <manifest> <hex>  Decode fields using a program manifest
  hopper inspect segments <hex>      Segment registry map
  hopper inspect receipt <hex>       Decode a state receipt

Explain
  hopper explain <hex>               Human-readable account explanation
  hopper explain account <hex>       Explicit account explanation
  hopper explain receipt <hex>       Explain a receipt in plain English
  hopper explain compat <old> <new>  Explain compatibility report
  hopper explain policy <pack>       Explain a named policy pack
  hopper explain layout <manifest>   Explain layout fields, intents, fingerprint
  hopper explain program <manifest>  Explain entire program pipeline
  hopper explain context <manifest> [--type <ContextName>]  Explain instruction contexts and generated accessors
  hopper explain instruction <manifest> <tag|name>  Explain one instruction's accounts and policy

Compatibility
  hopper compat <old> <new>          Compatibility report
  hopper compat --why <old> <new>    Compatibility report with explanation
  hopper plan <old> <new>            Migration plan with steps

Lifecycle
  hopper init [path]                 Create a Hopper-native project scaffold with minimal, NFT, Token-2022, DeFi, or Quasar-port templates
  hopper add [-i|-s|-e <name>]       Scaffold instruction, state, or error files
  hopper build [--host|--sbf]        Build the current project (default: SBF)
  hopper test                        Run host-side tests for the current project
  hopper deploy [--no-build]         Build and deploy the current SBF program
  hopper dump [--no-build]           Disassemble the built SBF binary
  hopper clean [-a|--all]            Remove generated build artifacts while preserving keypairs

Keys
  hopper keys new <path>             Generate a program/keypair json file
  hopper keys sync <path>            Sync declare_id! from a keypair pubkey
  hopper keys pda <seed> --program <id>  Derive a PDA and canonical bump

Config
  hopper config get <key>            Read a saved Hopper CLI config value
  hopper config set <key> <value>    Write a saved Hopper CLI config value
  hopper config list                 Show saved Hopper CLI config values

Transactions
  hopper tx explain <signature>      Fetch and explain an on-chain transaction
  hopper tx simulate <tx-base64>     Simulate a pre-built transaction
  hopper tx submit <tx-base64>       Submit a pre-built transaction

Shell
  hopper completions <shell>         Emit bash, zsh, fish, or PowerShell completions
  hopper version                     Print CLI version and linked schema version

Profiling
  hopper profile bench               Run the primitive benchmark lab and emit JSON/CSV artifacts
  hopper profile elf <program.so>    Static SBF symbols, CU-ish estimates, sections, flamegraph export

Project Health
  hopper lint                        Run Hopper project diagnostics
  hopper lint svm                    Scan typed-context sources for duplicate manual SVM checks
  hopper solana-check [--all]        Check SBF crate shape and Hopper entrypoint invariants
  hopper expand                      Show lowered macro output for the current project
  hopper doctor                      Check toolchain and workspace health

Direct aliases
  hopper decode <hex>                Alias for inspect
  hopper segments <hex>              Alias for inspect segments
  hopper receipt <hex>               Alias for inspect receipt / receipt
  hopper diff <old> <new>            Alias for schema diff
  hopper schema-export               Alias for schema export

Client SDK
  hopper client gen --ts <manifest>  Generate TypeScript client SDK
  hopper client gen --kt <manifest>  Generate Kotlin client SDK (org.sol4k)
  hopper client gen --py <manifest>  Generate Python client SDK
  hopper actions gen --program <manifest> --out api/actions  Generate Solana Actions route scaffolds
  hopper mobile gen --program <manifest> --target kotlin|react-native  Generate mobile bindings
  hopper test-gen security --program <manifest>  Generate a security test matrix

Fetch
  hopper fetch <program-id> [--rpc <url>] [--json]  Fetch manifest from on-chain

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
  hopper manager fetch <program-id> [--rpc <url>]  Fetch manifest and show summary
  hopper manager interactive <manifest>  Interactive terminal explorer
```

When run inside a Hopper package that already contains `hopper.manifest.json`,
`hopper compile --emit rust` can infer that local manifest automatically. Use
`--package <name>` to target another workspace member and `--out <path>` to
write the lowered preview instead of printing it.

Docs: <https://docs.rs/crate/hopper-cli/0.2.1>

## Support

Hopper is open-source Solana infrastructure. Public-goods support and donations
can be sent to `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

Donation URI: <solana:F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT?label=solanadevdao.sol>

## License

Apache-2.0
