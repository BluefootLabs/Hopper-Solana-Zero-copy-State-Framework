# Argus proof

Argus is the release proof sheet for Hopper's core claim:

> Anchor/Quasar-class DX, Hopper-grade safety/state contracts, Pinocchio-class raw control.

It is intentionally evidence-based. Each row names the claim, the enforcing surface, and the command that keeps it from drifting.

| Claim | Enforcing surface | Regression command |
| --- | --- | --- |
| `#[program]` owns dispatch ergonomics | `crates/hopper-macros-proc/src/program.rs` emits the dispatcher and entrypoint bridge | `cargo check -q -p hopper-lang --features proc-macros --test wrapper_context_integration --locked` |
| Option-A dynamic authoring is canonical | `src/prelude.rs` exposes `String<'a, N>` and `Vec<'a, T, N>` authoring aliases; `dynamic_account.rs` lowers them to compact tails | `cargo check -q -p hopper-lang --features proc-macros --test wrapper_context_integration --locked` |
| Token-2022 extension checks stay zero-copy | `crates/hopper-runtime/src/token_2022_ext.rs` scans TLV regions and validates `ExtensionPolicy` without heap allocation | `cargo test -q -p hopper-runtime token_2022_ext --locked` |
| Account validation can carry proof types | `crates/hopper-runtime/src/proof.rs` and `AccountView::proof()` return typed proof chains | `cargo check -q -p hopper-runtime --locked` |
| Field mutation intent can be carried by types | `FieldCapability<T, OFFSET, ROLE, POLICY>` in `segment.rs` binds offset, type, role, and mutation policy as a ZST | `cargo check -q -p hopper-devnet-audit --features devnet-client --locked` |
| Hopper programs are SBF-valid, not just Rust-valid | `hopper solana-check` checks `cdylib`, Solana `no_std` intent, allocator and panic markers, entrypoint shape, and backend selection | `cargo run -q -p hopper-cli -- solana-check --all` |
| Generated user surfaces come from the manifest | `hopper actions gen`, `hopper mobile gen`, and `hopper test-gen security` read `ProgramManifest` and write concrete artifacts | `cargo check -q -p hopper-cli --locked` |
| Devnet proof covers new primitives | `examples/hopper-devnet-audit` invokes proof, Token-2022 policy, field capability, remaining signer, segment, and substrate probes | `cargo check -q -p hopper-devnet-audit --features devnet-client --locked` |
| Dependency freshness is intentional | `docs/DEPENDENCY_AUDIT.md` records current crate lines and compatibility exceptions | `cargo tree -p hopper-cli --depth 1` |

## Release checklist

```powershell
cargo fmt -- --check
cargo check -q -p hopper-runtime --locked
cargo test -q -p hopper-runtime token_2022_ext --locked
cargo check -q -p hopper-lang --features proc-macros --test wrapper_context_integration --locked
cargo check -q -p hopper-cli --locked
cargo check -q -p hopper-devnet-audit --features devnet-client --locked
cargo run -q -p hopper-cli -- solana-check --all
```

For live proof, rebuild and deploy `examples/hopper-devnet-audit`, then run its devnet client with the explicit deployer keypair recorded in the example README.
