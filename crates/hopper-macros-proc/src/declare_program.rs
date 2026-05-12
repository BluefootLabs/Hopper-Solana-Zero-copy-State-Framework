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
        Ok(Self {
            module_name,
            manifest_path,
        })
    }
}

pub fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let Input {
        module_name,
        manifest_path,
    } = parse2(input)?;

    // Resolve relative paths against CARGO_MANIFEST_DIR so users can
    // write `declare_program!(amm, "idl/amm.json")` from anywhere.
    let candidate = PathBuf::from(manifest_path.value());
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
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

    let manifest_json: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).map_err(|e| {
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

            /// Built instruction parts returned by generated CPI builders.
            ///
            /// The account metas borrow addresses from the caller-owned
            /// `<Name>Accounts` value. Keep that value alive until the CPI
            /// invoke using the `view()` helper has completed.
            #[derive(Clone, Debug)]
            pub struct BuiltInstruction<'a, const A: usize, const D: usize> {
                pub program_id: &'a ::hopper::__runtime::Address,
                pub accounts: [::hopper::__runtime::InstructionAccount<'a>; A],
                pub data: [u8; D],
            }

            impl<'a, const A: usize, const D: usize> BuiltInstruction<'a, A, D> {
                pub fn view<'b>(&'b self) -> ::hopper::__runtime::InstructionView<'a, 'b, 'a, 'b>
                where
                    'a: 'b,
                {
                    ::hopper::__runtime::InstructionView {
                        program_id: self.program_id,
                        accounts: &self.accounts,
                        data: &self.data,
                    }
                }
            }

            /// Static resolver metadata generated from the manifest.
            #[derive(Clone, Copy, Debug)]
            pub struct AccountSpec {
                pub name: &'static str,
                pub writable: bool,
                pub signer: bool,
                pub resolver: &'static str,
                pub layout_ref: &'static str,
                pub lifecycle: &'static str,
                pub payer: &'static str,
                pub expected_address: &'static str,
                pub expected_owner: &'static str,
                pub optional: bool,
                pub seeds: &'static [&'static str],
            }

            /// Static effect metadata generated from the manifest.
            #[derive(Clone, Copy, Debug)]
            pub struct EffectSpec {
                pub kind: &'static str,
                pub target: &'static str,
                pub layout_ref: &'static str,
                pub reason: &'static str,
            }

            /// Static instruction metadata generated alongside the builder.
            #[derive(Clone, Copy, Debug)]
            pub struct InstructionSpec {
                pub name: &'static str,
                pub tag: u8,
                pub args_size: usize,
                pub account_count: usize,
                pub accounts: &'static [AccountSpec],
                pub effects: &'static [EffectSpec],
            }

            #( #instruction_items )*
        }
    };

    Ok(expanded)
}

fn build_instruction(ix: &serde_json::Value, manifest_span: &LitStr) -> syn::Result<TokenStream> {
    let name = ix
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| syn::Error::new_spanned(manifest_span, "instruction missing `name`"))?;
    let tag = ix.get("tag").and_then(|v| v.as_u64()).ok_or_else(|| {
        syn::Error::new_spanned(manifest_span, format!("instruction `{name}` missing `tag`"))
    })? as u8;

    let name_ident = format_ident!("{}", camel_to_snake(name));
    let args_struct_ident = format_ident!("{}Args", name);
    let accounts_struct_ident = format_ident!("{}Accounts", name);
    let account_specs_ident = format_ident!("{}_ACCOUNT_SPECS", upper_snake(name));
    let effect_specs_ident = format_ident!("{}_EFFECT_SPECS", upper_snake(name));
    let spec_ident = format_ident!("{}_SPEC", upper_snake(name));

    let args = ix
        .get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
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
        let canonical_type = a
            .get("canonical_type")
            .or_else(|| a.get("type"))
            .and_then(|v| v.as_str());
        let field = format_ident!("{}", aname);
        let (ty, serialize_stmt) = arg_type_for_descriptor(size, canonical_type, &field);
        args_fields.push(quote! { pub #field: #ty, });
        args_serialize.push(serialize_stmt);
    }

    let mut account_fields: Vec<TokenStream> = Vec::new();
    let mut account_metas: Vec<TokenStream> = Vec::new();
    let mut account_specs: Vec<TokenStream> = Vec::new();
    for acct in &accounts {
        let aname = acct
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| syn::Error::new_spanned(manifest_span, "account missing `name`"))?;
        let writable = acct
            .get("writable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let signer = acct
            .get("signer")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let field = format_ident!("{}", aname);
        account_fields.push(quote! { pub #field: ::hopper::__runtime::Address, });
        account_metas.push(quote! {
            ::hopper::__runtime::InstructionAccount::new(&__accounts.#field, #writable, #signer)
        });

        let resolver = account_resolver(acct);
        let layout_ref = json_str(acct, "layout_ref");
        let lifecycle = account_lifecycle(acct);
        let payer = json_str(acct, "payer");
        let expected_address =
            json_str(acct, "expected_address").or_else_empty(json_str(acct, "address"));
        let expected_owner =
            json_str(acct, "expected_owner").or_else_empty(json_str(acct, "owner"));
        let optional = acct
            .get("optional")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let seeds = json_string_array(acct, "seeds");
        let seed_lits = seeds.iter().map(String::as_str);
        account_specs.push(quote! {
            AccountSpec {
                name: #aname,
                writable: #writable,
                signer: #signer,
                resolver: #resolver,
                layout_ref: #layout_ref,
                lifecycle: #lifecycle,
                payer: #payer,
                expected_address: #expected_address,
                expected_owner: #expected_owner,
                optional: #optional,
                seeds: &[ #( #seed_lits ),* ],
            }
        });
    }

    let effect_specs = instruction_effect_specs(ix, &accounts);

    let tag_byte: u8 = tag;

    let accounts_count = account_fields.len();
    let args_size: usize = args
        .iter()
        .map(|a| a.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
        .sum();
    let data_size = 1 + args_size;

    Ok(quote! {
        pub const #account_specs_ident: &[AccountSpec] = &[
            #( #account_specs ),*
        ];

        pub const #effect_specs_ident: &[EffectSpec] = &[
            #( #effect_specs ),*
        ];

        pub const #spec_ident: InstructionSpec = InstructionSpec {
            name: #name,
            tag: #tag_byte,
            args_size: #args_size,
            account_count: #accounts_count,
            accounts: #account_specs_ident,
            effects: #effect_specs_ident,
        };

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
        pub fn #name_ident<'a>(
            __program_id: &'a ::hopper::__runtime::Address,
            __accounts: &'a #accounts_struct_ident,
            __args: #args_struct_ident,
        ) -> BuiltInstruction<'a, #accounts_count, #data_size> {
            let accounts = [ #( #account_metas ),* ];
            let mut data = [0u8; 1 + #args_size];
            data[0] = #tag_byte;
            let mut __offset: usize = 1;
            #( #args_serialize )*
            let _ = __offset;
            BuiltInstruction {
                program_id: __program_id,
                accounts,
                data,
            }
        }
    })
}

/// Translate a manifest-declared type/size into a Rust arg type and
/// a matching little-endian serialization statement.
///
/// Size 1 -> `u8`, 2 -> `u16`, 4 -> `u32`, 8 -> `u64`, 16 -> `u128`,
/// 32 -> `[u8; 32]`, anything else -> `[u8; N]` with a runtime
/// copy. Semantic type hints like `Address`, `bool`, `WireU64`, and
/// `LeU64` produce richer generated Rust while preserving the raw-byte
/// fallback for older manifests.
fn arg_type_for_descriptor(
    size: usize,
    canonical_type: Option<&str>,
    field: &Ident,
) -> (TokenStream, TokenStream) {
    let canonical = canonical_type.unwrap_or("").trim();
    if matches!(canonical, "bool" | "WireBool" | "LeBool") && size == 1 {
        return (
            quote!(bool),
            quote! {
                data[__offset] = if __args.#field { 1 } else { 0 };
                __offset += 1;
            },
        );
    }
    if (matches!(
        canonical,
        "Address" | "Pubkey" | "PublicKey" | "UntypedAddress"
    ) || canonical.starts_with("TypedAddress"))
        && size == 32
    {
        return (
            quote!(::hopper::__runtime::Address),
            quote! {
                data[__offset..__offset + 32]
                    .copy_from_slice(__args.#field.as_array());
                __offset += 32;
            },
        );
    }
    if canonical.starts_with("[u8;") {
        return byte_array_arg_tokens(size, field);
    }
    match canonical {
        "u8" => arg_type_for_size(1, field),
        "i8" => (
            quote!(i8),
            quote! {
                data[__offset..__offset + 1]
                    .copy_from_slice(&__args.#field.to_le_bytes());
                __offset += 1;
            },
        ),
        "u16" | "WireU16" | "LeU16" => arg_type_for_size(2, field),
        "i16" | "WireI16" | "LeI16" => int_arg_tokens(quote!(i16), 2, field),
        "u32" | "WireU32" | "LeU32" => arg_type_for_size(4, field),
        "i32" | "WireI32" | "LeI32" => int_arg_tokens(quote!(i32), 4, field),
        "u64" | "WireU64" | "LeU64" => arg_type_for_size(8, field),
        "i64" | "WireI64" | "LeI64" => int_arg_tokens(quote!(i64), 8, field),
        "u128" | "WireU128" | "LeU128" => arg_type_for_size(16, field),
        "i128" | "WireI128" | "LeI128" => int_arg_tokens(quote!(i128), 16, field),
        _ => arg_type_for_size(size, field),
    }
}

fn int_arg_tokens(ty: TokenStream, size: usize, field: &Ident) -> (TokenStream, TokenStream) {
    (
        ty,
        quote! {
            data[__offset..__offset + #size]
                .copy_from_slice(&__args.#field.to_le_bytes());
            __offset += #size;
        },
    )
}

fn byte_array_arg_tokens(size: usize, field: &Ident) -> (TokenStream, TokenStream) {
    (
        quote!([u8; #size]),
        quote! {
            data[__offset..__offset + #size]
                .copy_from_slice(&__args.#field);
            __offset += #size;
        },
    )
}

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
        n => byte_array_arg_tokens(n, field),
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

/// Convert an instruction name into a stable SCREAMING_SNAKE const prefix.
fn upper_snake(s: &str) -> String {
    camel_to_snake(s).to_ascii_uppercase()
}

fn json_str<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

trait EmptyStrExt {
    fn or_else_empty(self, fallback: Self) -> Self;
}

impl<'a> EmptyStrExt for &'a str {
    fn or_else_empty(self, fallback: Self) -> Self {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}

fn json_string_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn account_lifecycle(account: &serde_json::Value) -> String {
    json_str(account, "lifecycle").to_ascii_lowercase()
}

fn account_resolver(account: &serde_json::Value) -> String {
    if let Some(resolver) = account.get("resolver").and_then(|v| v.as_str()) {
        return resolver.to_ascii_lowercase();
    }
    if !json_str(account, "expected_address").is_empty() || !json_str(account, "address").is_empty()
    {
        "constant".to_string()
    } else if account
        .get("seeds")
        .and_then(|v| v.as_array())
        .map(|seeds| !seeds.is_empty())
        .unwrap_or(false)
    {
        "pda".to_string()
    } else if account
        .get("optional")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        "optional".to_string()
    } else {
        "provided".to_string()
    }
}

fn instruction_effect_specs(
    ix: &serde_json::Value,
    accounts: &[serde_json::Value],
) -> Vec<TokenStream> {
    if let Some(effects) = ix.get("effects").and_then(|v| v.as_array()) {
        return effects
            .iter()
            .map(|effect| {
                let kind = effect
                    .get("kind")
                    .or_else(|| effect.get("effect"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("custom");
                let target = effect
                    .get("target")
                    .or_else(|| effect.get("account"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let layout_ref = json_str(effect, "layout_ref");
                let reason = json_str(effect, "reason");
                quote! {
                    EffectSpec {
                        kind: #kind,
                        target: #target,
                        layout_ref: #layout_ref,
                        reason: #reason,
                    }
                }
            })
            .collect();
    }

    let mut out = Vec::new();
    for account in accounts {
        let target = json_str(account, "name");
        let layout_ref = json_str(account, "layout_ref");
        let lifecycle = account_lifecycle(account);
        let writable = account
            .get("writable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let signer = account
            .get("signer")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let kind = match lifecycle.as_str() {
            "init" | "create" | "creates_account" => "creates_account",
            "realloc" | "reallocate" => "reallocates_account",
            "close" => "closes_account",
            _ if writable => "writes",
            _ if signer => "requires_signer",
            _ => "reads",
        };
        let reason = if lifecycle.is_empty() {
            json_str(account, "policy_ref")
        } else {
            lifecycle.as_str()
        };
        out.push(quote! {
            EffectSpec {
                kind: #kind,
                target: #target,
                layout_ref: #layout_ref,
                reason: #reason,
            }
        });
    }
    if ix
        .get("receipt_expected")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        out.push(quote! {
            EffectSpec {
                kind: "emits_receipt",
                target: "receipt",
                layout_ref: "",
                reason: "receipt_expected",
            }
        });
    }
    out
}
