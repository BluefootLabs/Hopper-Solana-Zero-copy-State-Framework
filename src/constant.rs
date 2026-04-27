//! `#[hopper::constant]` — public-facing program constant marker.
//!
//! Anchor-compatible surface for `#[constant]`. Decorates a `pub const`
//! declaration to surface its `(name, type, value)` triple in the
//! Anchor IDL emitter (see [`hopper_schema::AnchorIdlWithConstants`]).
//!
//! The original constant is preserved verbatim so call sites continue
//! to compile unchanged. Alongside it the macro emits a sibling
//! `pub const __HOPPER_CONST_<NAME>: ConstantDescriptor` containing
//! the stringified type and initializer expression. Program authors
//! collect those descriptors into a `&'static [ConstantDescriptor]`
//! slice and hand it to the IDL emitter.
//!
//! No behavior, no runtime cost: the descriptor is a `&'static`
//! string-tuple constant with the same evaluation profile as any
//! other `pub const`.
//!
//! # Example
//!
//! ```ignore
//! use hopper::prelude::*;
//!
//! #[hopper::constant]
//! /// Maximum lamports per deposit.
//! pub const MAX_DEPOSIT: u64 = 1_000_000;
//!
//! pub const PROGRAM_CONSTANTS: &[ConstantDescriptor] = &[__HOPPER_CONST_MAX_DEPOSIT];
//! ```

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{parse2, Attribute, ItemConst, Result};

pub fn expand(_attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let original: ItemConst = parse2(item)?;

    let name_ident = &original.ident;
    let name_str = name_ident.to_string();
    let ty = &original.ty;
    let expr = &original.expr;

    let ty_str = quote!(#ty).to_string();
    let value_str = quote!(#expr).to_string();
    let docs_str = collect_doc_comments(&original.attrs);

    let descriptor_ident = format_ident!("__HOPPER_CONST_{}", name_ident);

    Ok(quote! {
        #original

        #[doc(hidden)]
        #[allow(non_upper_case_globals, non_snake_case)]
        pub const #descriptor_ident: ::hopper::hopper_schema::ConstantDescriptor =
            ::hopper::hopper_schema::ConstantDescriptor {
                name: #name_str,
                ty: #ty_str,
                value: #value_str,
                docs: #docs_str,
            };
    })
}

fn collect_doc_comments(attrs: &[Attribute]) -> String {
    let mut out = String::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let syn::Meta::NameValue(nv) = &attr.meta {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(s.value().trim());
            }
        }
    }
    out
}
