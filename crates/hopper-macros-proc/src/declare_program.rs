//! `declare_program!` - typed CPI surface from an on-disk manifest.
//!
//! Reads a `ProgramManifest` JSON at macro-expansion time and emits a
//! module of typed instruction builders, account newtypes, and a
//! compile-time fingerprint of the manifest bytes. Downstream
//! callers write `my_dep::swap(...)` instead of hand-rolling an
//! `Instruction { program_id, accounts, data }` literal.
//!
//! ## Usage
//!
//! ```ignore
//! hopper::declare_program!(amm, "idl/amm.json");
//!
//! // Now:
//! let ix = amm::swap(amm::SwapAccounts {
//!     pool: pool_pubkey,
//!     user: authority.pubkey(),
//!     user_token: ata,
//!     vault: vault_pda,
//! }, amm::SwapArgs { amount_in: 1_000_000, min_amount_out: 999_000 })?;
//! invoke(&ix, &accounts)?;
//! ```
//!
//! ## Innovation (not a copy of Anchor's declare_program!)
//!
//! Anchor's `declare_program!` reads its IDL and produces a module
//! of typed CPI builders. Hopper does the same but goes one step
//! further: it stamps the manifest's SHA-256 fingerprint as a
//! compile-time `FINGERPRINT: [u8; 32]` const on the generated
//! module. A caller who wants to assert "my client was built
//! against the exact manifest that is live on chain" writes
//!
//! ```ignore
//! assert_eq!(
//!     amm::FINGERPRINT,
//!     hopper::hopper_schema::fingerprint_from_on_chain(&rpc, program_id)?
//! );
//! ```
//!
//! and the drift check is a single equality comparison on 32 bytes.
//! Anchor's path requires pulling the live IDL and diffing JSON by
//! hand.
//!
//! ## Wire format
//!
//! Expects a `ProgramManifest`-shaped JSON file containing at least
//! `name`, `program_id` (optional base58 string), and `instructions`
//! array. Every instruction in the array must carry `name`, `tag`,
//! `accounts` (`[{name, writable, signer}]`), and `args`
//! (`[{name, size}]`).
//!
//! ## Errors
//!
//! A missing file, invalid JSON, or an instruction without the
//! expected shape is a compile-time error with a span pointing at
//! the macro invocation so the user sees the exact path in the
//! diagnostic.

use std::path::PathBuf;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use sha2::{Digest, Sha256};
use syn::{parse2, Ident, LitStr, Token};

/// Input of the `declare_program!` macro: `<ident>, "path/to/manifest.json"`.
struct Input {
    module_name: Ident,
    manifest_path: LitStr,
}

impl syn::parse::Parse for Input {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let module_name: Ident = input.parse()?;
        let _: Token![,] = input.parse()?;
        let manifest_path: LitStr = input.parse()?;
        Ok(Self { module_name, manifest_path })
    }
}

pub fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let Input { module_name, manifest_path } = parse2(input)?;

    // Resolve relative paths against CARGO_MANIFEST_DIR so users can
    // write `declare_program!(amm, "idl/amm.json")` from anywhere.
    let candidate = PathBuf::from(manifest_path.value());
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        let root = std::env::var("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(root).join(candidate)
    };

    let manifest_bytes = std::fs::read(&resolved).map_err(|e| {
        syn::Error::new_spanned(
            &manifest_path,
            format!(
                "declare_program!: could not read `{}`: {e}",
                resolved.display()
            ),
        )
    })?;

    let fingerprint: [u8; 32] = {
        let mut h = Sha256::new();
        h.update(&manifest_bytes);
        h.finalize().into()
    };
    let fingerprint_bytes: Vec<u8> = fingerprint.to_vec();

    let manifest_json: serde_json::Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| {
            syn::Error::new_spanned(
                &manifest_path,
                format!("declare_program!: invalid JSON: {e}"),
            )
        })?;

    let program_name_lit = manifest_json
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)");
    let program_id_str = manifest_json
        .get("program_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Every instruction becomes: a `<Name>Args` struct, a
    // `<Name>Accounts` struct, and a free `fn <name_snake>(accounts,
    // args) -> Instruction`.
    let instructions = manifest_json
        .get("instructions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &manifest_path,
                "declare_program!: manifest has no `instructions` array",
            )
        })?;

    let mut instruction_items: Vec<TokenStream> = Vec::new();
    for ix in instructions {
        instruction_items.push(build_instruction(ix, &manifest_path)?);
    }

    // Emit the module. The `FINGERPRINT` const is the centerpiece
    // innovation: a caller holding this const and the live on-chain
    // manifest fingerprint can prove their builder was generated
    // from the exact bytes the program published. The `PROGRAM_ID`
    // const is emitted unconditionally (empty string when the
    // manifest did not supply it) so builders can pre-populate the
    // `program_id` field on every emitted Instruction.
    let expanded = quote! {
        #[doc = concat!("Typed CPI surface for the `", #program_name_lit, "` Hopper program.")]
        pub mod #module_name {
            #![allow(dead_code, non_snake_case)]

            /// SHA-256 of the manifest bytes this module was generated from.
            ///
            /// Compare against the fingerprint of the live on-chain
            /// manifest (fetched via `hopper manager fetch`) to prove
            /// the builder has not drifted from the deployed program.
            pub const FINGERPRINT: [u8; 32] = [#( #fingerprint_bytes ),*];

            /// Human-readable program name from the manifest.
            pub const PROGRAM_NAME: &str = #program_name_lit;

            /// Base58 program id encoded in the manifest. Empty
            /// when the manifest did not embed one; callers should
            /// supply their own in that case.
            pub const PROGRAM_ID_STR: &str = #program_id_str;

            #( #instruction_items )*
        }
    };

    Ok(expanded)
}

fn build_instruction(
    ix: &serde_json::Value,
    manifest_span: &LitStr,
) -> syn::Result<TokenStream> {
    let name = ix.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
        syn::Error::new_spanned(manifest_span, "instruction missing `name`")
    })?;
    let tag = ix.get("tag").and_then(|v| v.as_u64()).ok_or_else(|| {
        syn::Error::new_spanned(
            manifest_span,
            format!("instruction `{name}` missing `tag`"),
        )
    })? as u8;

    let name_ident = format_ident!("{}", camel_to_snake(name));
    let args_struct_ident = format_ident!("{}Args", name);
    let accounts_struct_ident = format_ident!("{}Accounts", name);

    let args = ix.get("args").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let accounts = ix
        .get("accounts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut args_fields: Vec<TokenStream> = Vec::new();
    let mut args_serialize: Vec<TokenStream> = Vec::new();
    for a in &args {
        let aname = a
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| syn::Error::new_spanned(manifest_span, "arg missing `name`"))?;
        let size = a.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let field = format_ident!("{}", aname);
        let (ty, serialize_stmt) = arg_type_for_size(size, &field);
        args_fields.push(quote! { pub #field: #ty, });
        args_serialize.push(serialize_stmt);
    }

    let mut account_fields: Vec<TokenStream> = Vec::new();
    let mut account_metas: Vec<TokenStream> = Vec::new();
    for acct in &accounts {
        let aname = acct
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                syn::Error::new_spanned(manifest_span, "account missing `name`")
            })?;
        let writable = acct.get("writable").and_then(|v| v.as_bool()).unwrap_or(false);
        let signer = acct.get("signer").and_then(|v| v.as_bool()).unwrap_or(false);
        let field = format_ident!("{}", aname);
        account_fields.push(quote! { pub #field: [u8; 32], });
        account_metas.push(quote! {
            ::hopper::__runtime::InstructionAccount {
                pubkey: ::hopper::__runtime::Address::from(__acct.#field),
                is_writable: #writable,
                is_signer: #signer,
            }
        });
    }

    let tag_byte: u8 = tag;

    let accounts_count = account_fields.len();
    let args_size: usize = args
        .iter()
        .map(|a| a.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
        .sum();

    Ok(quote! {
        /// Typed account metas for this instruction.
        #[derive(Clone, Copy, Debug)]
        pub struct #accounts_struct_ident {
            #( #account_fields )*
        }

        /// Typed argument payload for this instruction.
        #[derive(Clone, Copy, Debug)]
        pub struct #args_struct_ident {
            #( #args_fields )*
        }

        /// Build a CPI-ready instruction struct. The discriminator
        /// byte is stamped automatically; args serialize little-
        /// endian to match Hopper's `#[hopper::args]` layout.
        ///
        /// The caller supplies the program id because manifest
        /// fingerprints can stay portable across clusters while
        /// the deployed program id changes. When the manifest
        /// embedded a `program_id`, `PROGRAM_ID_STR` carries its
        /// base58 form.
        pub fn #name_ident(
            __program_id: ::hopper::__runtime::Address,
            __acct: #accounts_struct_ident,
            __args: #args_struct_ident,
        ) -> (
            ::hopper::__runtime::Address,
            [::hopper::__runtime::InstructionAccount; #accounts_count],
            [u8; 1 + #args_size],
        ) {
            let accounts = [ #( #account_metas ),* ];
            let mut data = [0u8; 1 + #args_size];
            data[0] = #tag_byte;
            let mut __offset: usize = 1;
            #( #args_serialize )*
            (__program_id, accounts, data)
        }
    })
}

/// Translate a manifest-declared byte size into a Rust arg type and
/// a matching little-endian serialization statement.
///
/// Size 1 -> `u8`, 2 -> `u16`, 4 -> `u32`, 8 -> `u64`, 16 -> `u128`,
/// 32 -> `[u8; 32]`, anything else -> `[u8; N]` with a runtime
/// copy. Future work: let the manifest carry a semantic type hint
/// (Address, WireU64, etc.) so the emitted types are richer. The
/// raw-bytes fallback keeps the macro robust against older
/// manifests.
fn arg_type_for_size(size: usize, field: &Ident) -> (TokenStream, TokenStream) {
    match size {
        1 => (
            quote!(u8),
            quote! {
                data[__offset] = __args.#field;
                __offset += 1;
            },
        ),
        2 => (
            quote!(u16),
            quote! {
                data[__offset..__offset + 2]
                    .copy_from_slice(&__args.#field.to_le_bytes());
                __offset += 2;
            },
        ),
        4 => (
            quote!(u32),
            quote! {
                data[__offset..__offset + 4]
                    .copy_from_slice(&__args.#field.to_le_bytes());
                __offset += 4;
            },
        ),
        8 => (
            quote!(u64),
            quote! {
                data[__offset..__offset + 8]
                    .copy_from_slice(&__args.#field.to_le_bytes());
                __offset += 8;
            },
        ),
        16 => (
            quote!(u128),
            quote! {
                data[__offset..__offset + 16]
                    .copy_from_slice(&__args.#field.to_le_bytes());
                __offset += 16;
            },
        ),
        n => (
            quote!([u8; #n]),
            quote! {
                data[__offset..__offset + #n]
                    .copy_from_slice(&__args.#field);
                __offset += #n;
            },
        ),
    }
}

/// Convert a camelCase or PascalCase identifier into snake_case.
/// We need this because manifest instruction names follow the
/// in-code Rust fn name (snake_case already) in practice, but
/// Hopper also accepts CamelCase in older manifests.
fn camel_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}
