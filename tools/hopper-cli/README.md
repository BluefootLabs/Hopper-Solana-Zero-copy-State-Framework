# hopper-cli

Command-line tooling for inspecting, explaining, and managing Hopper programs.

This is the operator's interface to Hopper. Feed it hex-encoded account data and
a program manifest and it will decode headers, explain fields, diff versions,
plan migrations, decode receipts, and generate client SDKs. Works entirely
offline by default, with optional RPC connectivity for live account fetching.

## Install

`hopper-cli` is not published to crates.io yet. Build it from source:

```bash
cargo build --release -p hopper-cli
```

Or run it directly from the workspace:

```bash
cargo run -p hopper-cli -- help
```

## Commands

```
Compile
  hopper compile --emit rust [<manifest>]  Emit lowered runtime Rust: accessors, offsets, pointer path
  hopper compile --emit rust-client <manifest>  Emit off-chain Rust client SDK
  hopper compile --emit py <manifest>      Emit Python client SDK

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

Compatibility
  hopper compat <old> <new>          Compatibility report
  hopper compat --why <old> <new>    Compatibility report with explanation
  hopper plan <old> <new>            Migration plan with steps

Lifecycle
  hopper init <path>                 Create a Hopper-native project scaffold
  hopper build [--host|--sbf]        Build the current project (default: SBF)
  hopper test                        Run host-side tests for the current project
  hopper deploy [--no-build]         Build and deploy the current SBF program
  hopper dump [--no-build]           Disassemble the built SBF binary

Profiling
  hopper profile bench               Run the primitive benchmark lab and emit JSON/CSV artifacts

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

## License

Apache-2.0
