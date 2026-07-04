//! `#[hopper::error_code]`. stable error-code enum derive.
//!
//! Decorates a `#[repr(u32)]` enum of unit variants. Emits:
//! - `From<T> for u32` using the stable codes assigned by the user (or by
//!   SHA-256 fingerprint when no `= N` discriminant is given).
//! - `From<T> for ProgramError` lowering to `ProgramError::Custom(code)`, so a
//!   handler can `return Err(MyError::Foo.into())` like Anchor's `#[error_code]`.
//! - A `code(self) -> u32` inherent method.
//! - A `variant_name(self) -> &'static str` inherent method.
//! - A `CODE_TABLE: &[(&str, u32)]` const slice so the schema crate can
//!   export the full error registry in the manifest.
//! - An `INVARIANT_TABLE: &[(&str, &str)]` slice that links each variant to
//!   the named invariant that, when violated, produces it. the innovation
//!   that lets clients surface *which safety check actually failed*, not
//!   just a numeric code.
//!
//! ## Design notes
//!
//! Most framework errors are codes + messages. Hopper's errors can also carry
//! `invariant = "…"`
//! metadata, so when an off-chain client sees error 0x42AA it can surface
//! "Invariant `balance_nonzero` failed" instead of "Error: 0x42AA".
//!
//! ## Example
//!
//! ```ignore
//! #[hopper::error_code]
//! #[repr(u32)]
//! pub enum VaultError {
//!     #[invariant = "balance_nonzero"]
//!     InsufficientBalance = 0x1001,
//!     #[invariant = "authority_match"]
//!     Unauthorized = 0x1002,
//!     MigrationRequired,           // auto-assigned stable code
//! }
//! ```

use proc_macro2::TokenStream;
use quote::quote;
use sha2::{Digest, Sha256};
use syn::{parse2, Fields, ItemEnum, LitInt, LitStr};

pub fn expand(_attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let input: ItemEnum = parse2(item)?;
    let enum_name = input.ident.clone();
    let enum_name_str = enum_name.to_string();

    if input
        .variants
        .iter()
        .any(|v| !matches!(v.fields, Fields::Unit))
    {
        return Err(syn::Error::new_spanned(
            &enum_name,
            "#[hopper::error_code] only supports unit variants",
        ));
    }

    let mut variant_idents = Vec::with_capacity(input.variants.len());
    let mut variant_names = Vec::with_capacity(input.variants.len());
    let mut variant_codes = Vec::with_capacity(input.variants.len());
    let mut variant_code_values = Vec::with_capacity(input.variants.len());
    let mut variant_invariants = Vec::with_capacity(input.variants.len());

    for v in &input.variants {
        let vname = v.ident.clone();
        let vname_str = vname.to_string();

        let code = match &v.discriminant {
            Some((
                _,
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(li),
                    ..
                }),
            )) => li.base10_parse::<u32>()?,
            Some((_, other)) => {
                return Err(syn::Error::new_spanned(
                    other,
                    "only integer-literal discriminants are supported",
                ));
            }
            None => derive_code(&enum_name_str, &vname_str),
        };

        let mut invariant_name = String::new();
        for a in &v.attrs {
            if a.path().is_ident("invariant") {
                if let Ok(nv) = a.meta.require_name_value() {
                    if let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(s),
                        ..
                    }) = &nv.value
                    {
                        invariant_name = s.value();
                    }
                }
            }
        }

        variant_idents.push(vname);
        variant_names.push(LitStr::new(&vname_str, input.ident.span()));
        variant_codes.push(LitInt::new(&code.to_string(), input.ident.span()));
        variant_code_values.push(code);
        variant_invariants.push(LitStr::new(&invariant_name, input.ident.span()));
    }

    // Build a cleaned enum that strips #[invariant] attrs (those are
    // hopper-internal) but keeps any explicit `= N` discriminants the user
    // wrote. We regenerate a new ItemEnum to emit the cleaned version.
    //
    // Variants WITHOUT an explicit discriminant get their SHA-derived code
    // written back as one. Without this, Rust assigns sequential
    // discriminants (previous + 1), so `MyError::Variant as u32` would
    // silently disagree with `code()` / `CODE_TABLE` / the wire value in
    // `ProgramError::Custom` — an audit-trail integrity hole for any
    // consumer using the natural `as` cast. Writing the code into the enum
    // also makes rustc reject a collision between a derived code and a
    // user-explicit one at compile time.
    let mut cleaned = strip_invariant_attrs(input.clone());
    for (variant, code) in cleaned.variants.iter_mut().zip(variant_code_values.iter()) {
        if variant.discriminant.is_none() {
            let lit = syn::Expr::Lit(syn::ExprLit {
                attrs: Vec::new(),
                lit: syn::Lit::Int(LitInt::new(&code.to_string(), variant.ident.span())),
            });
            variant.discriminant = Some((<syn::Token![=]>::default(), lit));
        }
    }

    let idents_for_from = variant_idents.clone();
    let codes_for_from = variant_codes.clone();
    let idents_for_name = variant_idents.clone();
    let names_for_name = variant_names.clone();
    let idents_for_code = variant_idents.clone();
    let codes_for_code = variant_codes.clone();
    let idents_for_invariant = variant_idents.clone();
    let invariants_for_variant = variant_invariants.clone();
    let idents_for_idx = variant_idents.clone();
    // Position indices for `invariant_idx(self)`. The index matches the
    // variant's position in both `CODE_TABLE` and `INVARIANT_TABLE`, so
    // a receipt carrying `failed_invariant_idx` can be resolved to a
    // name without even parsing the error code.
    //
    // The receipt slot is a single byte, so cap indices at 255. Anything
    // past that is squashed to 0xFF (= "no invariant" sentinel) rather
    // than overflowing. and we surface an early compile error so the
    // user can see the design limit up front.
    if variant_idents.len() > 255 {
        return Err(syn::Error::new_spanned(
            &enum_name,
            "#[hopper::error_code] supports at most 255 variants (receipt's failed_invariant_idx is a u8)",
        ));
    }
    let idx_values: Vec<LitInt> = (0..variant_idents.len())
        .map(|i| LitInt::new(&format!("{}u8", i), enum_name.span()))
        .collect();

    let gen = quote! {
        #cleaned

        impl #enum_name {
            /// Stable numeric error code for this variant.
            #[inline]
            pub const fn code(self) -> u32 {
                match self {
                    #( Self::#idents_for_code => #codes_for_code ),*
                }
            }

            /// Human name of this error variant.
            #[inline]
            pub const fn variant_name(self) -> &'static str {
                match self {
                    #( Self::#idents_for_name => #names_for_name ),*
                }
            }

            /// Invariant name this variant stands for, or `""` if none
            /// was declared with `#[invariant = "..."]`.
            #[inline]
            pub const fn invariant(self) -> &'static str {
                match self {
                    #( Self::#idents_for_invariant => #invariants_for_variant ),*
                }
            }

            /// Zero-based index of this variant in both `CODE_TABLE` and
            /// `INVARIANT_TABLE`. Stamped into receipts as
            /// `failed_invariant_idx` so off-chain consumers can jump
            /// straight to the registry row without a linear scan.
            ///
            /// Returns `0xFF` only if the enum ever grows beyond 255
            /// variants (which would also break the receipt wire slot
            /// by design. such an enum is already an anti-pattern).
            #[inline]
            pub const fn invariant_idx(self) -> u8 {
                match self {
                    #( Self::#idents_for_idx => #idx_values ),*
                }
            }

            /// Full error-code registry as `(name, code)` tuples. The schema
            /// crate consumes this via the `SchemaExport` path to emit the
            /// manifest's `errors[]` array.
            pub const CODE_TABLE: &'static [(&'static str, u32)] = &[
                #( (#variant_names, #variant_codes) ),*
            ];

            /// Map each variant name to the invariant it represents
            /// (empty string if no `#[invariant = "…"]` was declared).
            pub const INVARIANT_TABLE: &'static [(&'static str, &'static str)] = &[
                #( (#variant_names, #variant_invariants) ),*
            ];
        }

        impl ::core::convert::From<#enum_name> for u32 {
            #[inline]
            fn from(e: #enum_name) -> u32 {
                match e {
                    #( #enum_name::#idents_for_from => #codes_for_from ),*
                }
            }
        }

        // Lower straight into the program-return channel so handlers can write
        // `return Err(MyError::Foo.into())` exactly like Anchor's `#[error_code]`.
        // The stable code rides in `ProgramError::Custom(u32)`; the runtime then
        // packs it into the syscall return slot.
        impl ::core::convert::From<#enum_name> for ::hopper::__runtime::ProgramError {
            #[inline]
            fn from(e: #enum_name) -> ::hopper::__runtime::ProgramError {
                ::hopper::__runtime::ProgramError::Custom(e.code())
            }
        }
    };

    Ok(gen)
}

fn strip_invariant_attrs(mut e: ItemEnum) -> ItemEnum {
    for v in &mut e.variants {
        v.attrs.retain(|a| !a.path().is_ident("invariant"));
    }
    e
}

fn derive_code(enum_name: &str, variant_name: &str) -> u32 {
    let mut h = Sha256::new();
    h.update(b"hopper:error:");
    h.update(enum_name.as_bytes());
    h.update(b":");
    h.update(variant_name.as_bytes());
    let d = h.finalize();
    // Low 31 bits only, to keep space for user-explicit high-bit codes.
    let code = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) & 0x7FFF_FFFF;
    if code == 0 {
        1
    } else {
        code
    }
}
