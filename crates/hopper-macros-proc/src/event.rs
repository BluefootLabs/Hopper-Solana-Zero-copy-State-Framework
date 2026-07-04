//! `#[hopper::event]`. event struct derive.
//!
//! Decorates a `#[repr(C)]` struct of Pod fields. Emits:
//! - `impl Pod` + FixedLayout asserts (same Pod contract as `#[hopper::pod]`)
//! - A stable event `TAG` byte (derived from the name if not supplied)
//! - A `NAME` static string for IDL emission
//! - A `SEGMENT_SOURCE` optional byte: the segment index whose mutation
//!   triggered the event (Hopper can preserve segment lineage for indexers).
//! - A borrowed `as_bytes(&self)` helper. Event log and self-CPI emitters
//!   prepend the stable 1-byte tag at the call site.
//!
//! ## Design notes
//!
//! Hopper events can carry `SEGMENT_SOURCE` so `hopper-sdk` can build segment-
//! keyed indexes for free.

use proc_macro2::TokenStream;
use quote::quote;
use sha2::{Digest, Sha256};
use syn::{
    parse::Parser, parse2, punctuated::Punctuated, Attribute, ItemStruct, LitInt, LitStr, Meta,
    Token,
};

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let input: ItemStruct = parse2(item)?;

    if !has_repr_c(&input.attrs) {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[hopper::event] requires #[repr(C)] so `as_bytes` reads a stable layout",
        ));
    }

    let metas: Punctuated<Meta, Token![,]> = Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(attr.clone())
        .unwrap_or_default();

    let mut tag: Option<u8> = None;
    let mut name: Option<String> = None;
    let mut segment_source: Option<u8> = None;

    for m in &metas {
        match m {
            Meta::NameValue(nv) if nv.path.is_ident("tag") => {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(li),
                    ..
                }) = &nv.value
                {
                    tag = Some(li.base10_parse::<u8>()?);
                }
            }
            Meta::NameValue(nv) if nv.path.is_ident("name") => {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(ls),
                    ..
                }) = &nv.value
                {
                    name = Some(ls.value());
                }
            }
            Meta::NameValue(nv) if nv.path.is_ident("segment") => {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(li),
                    ..
                }) = &nv.value
                {
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
    let field_types: Vec<syn::Type> = input.fields.iter().map(|f| f.ty.clone()).collect();

    let sum_sizes = if field_types.is_empty() {
        quote! { 0usize }
    } else {
        let pieces = field_types.iter().map(|ty| {
            quote! { ::core::mem::size_of::<#ty>() }
        });
        quote! { #(#pieces)+* }
    };
    let align_msg = format!(
        "#[hopper::event] struct `{}` must be alignment-1 (use wire types such as WireU64 \
         or byte arrays instead of raw u64/u32): `as_bytes` exposes the raw bytes and a \
         padded layout would leak uninitialized padding into the program log.",
        ident_str,
    );
    let size_msg = format!(
        "#[hopper::event] struct `{}` has implicit padding; `as_bytes` would emit \
         uninitialized padding bytes into the program log. Use alignment-1 field types \
         so #[repr(C)] packs exactly.",
        ident_str,
    );

    let gen = quote! {
        #input

        // ── Compile-time safety fence ────────────────────────────────
        // `as_bytes` reinterprets the struct as a byte run and hands it to
        // the log/CPI emitters. That is sound (and leak-free) only if the
        // layout is alignment-1 with no padding and every field is plain
        // bytes. All three obligations are discharged here at declaration
        // time — a padded or pointer-carrying event fails to compile.
        const _: () = {
            assert!(::core::mem::align_of::<#ident>() == 1, #align_msg);
            assert!(
                ::core::mem::size_of::<#ident>() == (#sum_sizes),
                #size_msg,
            );
        };

        // Field-level Pod proof: every field must satisfy Hopper's Pod
        // contract (no references, no interior pointers, no padding), so
        // the byte view below cannot expose non-byte data.
        #[doc(hidden)]
        const _: () = {
            struct __EventFieldPodProof<T: ::hopper::__runtime::Pod>(
                ::core::marker::PhantomData<T>,
            );
            #(
                #[allow(dead_code)]
                const _: __EventFieldPodProof<#field_types> =
                    __EventFieldPodProof(::core::marker::PhantomData);
            )*
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
                // SAFETY: the compile-time fence above proves Self is
                // #[repr(C)], alignment-1, padding-free, and Pod in every
                // field, so the byte view covers exactly the initialized
                // wire representation.
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
        if digest[i] != 0 {
            return digest[i];
        }
        i += 1;
    }
    1
}

fn has_repr_c(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("repr") {
            return false;
        }
        let mut has_c = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("C") {
                has_c = true;
            }
            Ok(())
        });
        has_c
    })
}
