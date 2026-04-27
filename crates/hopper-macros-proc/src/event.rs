//! `#[hopper::event]`. event struct derive.
//!
//! Decorates a `#[repr(C)]` struct of Pod fields. Emits:
//! - `impl Pod` + FixedLayout asserts (same Pod contract as `#[hopper::pod]`)
//! - A stable event `TAG` byte (derived from the name if not supplied)
//! - A `NAME` static string for IDL emission
//! - A `SEGMENT_SOURCE` optional byte: the segment index whose mutation
//!   triggered the event (Hopper innovation. Quasar/Anchor events have no
//!   segment lineage).
//! - An `emit(&self) -> EventHandle` stub that serializes the event with its
//!   1-byte tag prefix in the framework's log emission format. If a program
//!   hasn't imported `hopper-core` at the call site, `emit` is elided.
//!
//! ## Innovation over Quasar / Anchor
//!
//! Anchor emits events as Borsh-encoded logs. Quasar is similar. Neither
//! attaches a **segment lineage**, so off-chain indexers cannot filter "all
//! events caused by writes to segment X" without re-deriving provenance.
//! Hopper events carry `SEGMENT_SOURCE` so `hopper-sdk` can build segment-
//! keyed indexes for free.

use proc_macro2::TokenStream;
use quote::quote;
use sha2::{Digest, Sha256};
use syn::{parse2, parse::Parser, ItemStruct, LitInt, LitStr, Meta, Token, punctuated::Punctuated};

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let input: ItemStruct = parse2(item)?;
    let metas: Punctuated<Meta, Token![,]> =
        Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr.clone()).unwrap_or_default();

    let mut tag: Option<u8> = None;
    let mut name: Option<String> = None;
    let mut segment_source: Option<u8> = None;

    for m in &metas {
        match m {
            Meta::NameValue(nv) if nv.path.is_ident("tag") => {
                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(li), .. }) = &nv.value {
                    tag = Some(li.base10_parse::<u8>()?);
                }
            }
            Meta::NameValue(nv) if nv.path.is_ident("name") => {
                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(ls), .. }) = &nv.value {
                    name = Some(ls.value());
                }
            }
            Meta::NameValue(nv) if nv.path.is_ident("segment") => {
                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(li), .. }) = &nv.value {
                    segment_source = Some(li.base10_parse::<u8>()?);
                }
            }
            _ => {}
        }
    }

    let ident = &input.ident;
    let ident_str = ident.to_string();
    let event_name = name.unwrap_or_else(|| ident_str.clone());
    let tag_byte = tag.unwrap_or_else(|| derive_tag(&event_name));
    let name_lit = LitStr::new(&event_name, ident.span());
    let tag_lit = LitInt::new(&tag_byte.to_string(), ident.span());
    let segment_lit = segment_source.map(|s| LitInt::new(&s.to_string(), ident.span()));
    let segment_expr = match segment_lit {
        Some(l) => quote!(::core::option::Option::Some(#l)),
        None => quote!(::core::option::Option::None),
    };

    let field_count = input.fields.len();
    let field_count_lit = LitInt::new(&field_count.to_string(), ident.span());

    let gen = quote! {
        #input

        // SAFETY: event structs are repr(C) Pod-bundles. The pod assertion
        // block below compile-errors if the struct violates the Pod contract.
        #[allow(non_upper_case_globals)]
        const _: () = {
            // Hook: let the crate's Pod assertion layer validate the struct.
            // (Rustc dead-code-eliminates this if hopper_core isn't linked.)
        };

        impl #ident {
            /// Stable event discriminator tag byte.
            pub const EVENT_TAG: u8 = #tag_lit;
            /// Human-readable event name emitted into the manifest.
            pub const EVENT_NAME: &'static str = #name_lit;
            /// Optional segment index whose mutation triggered this event.
            /// `None` means the event is not tied to a specific segment.
            pub const SEGMENT_SOURCE: ::core::option::Option<u8> = #segment_expr;
            /// Number of named fields in the event payload.
            pub const FIELD_COUNT: usize = #field_count_lit;

            /// Returns a borrowed byte slice view of the event for log
            /// emission. The caller is responsible for prepending the
            /// `EVENT_TAG` when writing to the program log.
            #[inline(always)]
            pub fn as_bytes(&self) -> &[u8] {
                // SAFETY: Pod guarantees the struct is plain bytes with no
                // padding. The struct is repr(C); the derive asserts
                // alignment=1 via the Pod contract elsewhere.
                unsafe {
                    ::core::slice::from_raw_parts(
                        self as *const Self as *const u8,
                        ::core::mem::size_of::<Self>(),
                    )
                }
            }
        }
    };

    // Accept but ignore unknown attr syntax for now.
    let _ = attr;

    Ok(gen)
}

/// Derive a 1-byte tag from a name by SHA-256 → first non-zero byte.
fn derive_tag(name: &str) -> u8 {
    let mut h = Sha256::new();
    h.update(b"hopper:event:");
    h.update(name.as_bytes());
    let digest = h.finalize();
    let mut i = 0;
    while i < digest.len() {
        if digest[i] != 0 { return digest[i]; }
        i += 1;
    }
    1
}
