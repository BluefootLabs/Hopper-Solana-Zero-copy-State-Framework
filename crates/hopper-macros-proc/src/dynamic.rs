//! `#[hopper::dynamic(field = "name")]`. dynamic-tail marker at struct level.
//!
//! Attached to a `#[repr(C)]` struct, this attribute records which field
//! carries the account's dynamic tail region (a `DynamicRegion<T>` or
//! `DynamicSlice<T>`) so the companion `#[hopper::state]` derive and
//! `hopper_layout!` macro can locate it at compile time.
//!
//! ## Why struct-level instead of field-level
//!
//! On stable Rust, `#[proc_macro_attribute]` may attach to items (structs,
//! fns, enums, mods, impl items) but not to struct fields in isolation. To
//! stay on stable without a helper derive or a nightly feature, Hopper
//! declares the dynamic-tail binding on the enclosing struct and names the
//! field by string. The ergonomics remain two lines of additional sugar
//! over the raw `#[hopper::state(dynamic_tail = T)]` path:
//!
//! ```ignore
//! #[hopper::dynamic(field = "entries")]
//! #[hopper::state]
//! #[repr(C)]
//! pub struct Ledger {
//!     pub head: WireU64,
//!     pub tail: WireU64,
//!     pub entries: DynamicRegion<LedgerEntry>,
//! }
//! ```
//!
//! ## Innovation over Quasar
//!
//! Quasar's dynamic fields are appended at the end of a blob. Hopper's
//! dynamic region additionally supports a **tombstone ring**: a small
//! bitmap at the head of the region tracks which slot is the oldest
//! logically-removed entry so realloc-free extensions can insert into a
//! freed slot without moving later data. The generated metadata tells the
//! runtime which field carries the ring header.
//!
//! The generation here is intentionally small. the heavy lifting (ring
//! bookkeeping, realloc guard integration) lives in `hopper-core`. This
//! file emits the metadata glue that makes the field discoverable by the
//! manifest exporter and the state derive.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{parse2, parse::Parser, punctuated::Punctuated, Fields, ItemStruct, LitStr, Meta, Token};

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let input: ItemStruct = parse2(item)?;

    let metas: Punctuated<Meta, Token![,]> = Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(attr.clone())
        .unwrap_or_default();

    let mut field_name: Option<String> = None;
    for m in &metas {
        if let Meta::NameValue(nv) = m {
            if nv.path.is_ident("field") {
                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value {
                    field_name = Some(s.value());
                }
            }
        }
    }

    let tail_name = field_name.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "#[hopper::dynamic(field = \"name\")] requires `field = \"<field>\"`",
        )
    })?;

    // Validate that the named field exists and capture its type.
    let fields = match &input.fields {
        Fields::Named(n) => &n.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "#[hopper::dynamic] requires a named-field struct",
            ));
        }
    };
    let tail_field = fields
        .iter()
        .find(|f| f.ident.as_ref().map(|i| i.to_string() == tail_name).unwrap_or(false))
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &input.ident,
                format!(
                    "#[hopper::dynamic] field `{}` not found on `{}`",
                    tail_name, input.ident
                ),
            )
        })?;

    let tail_ty = tail_field.ty.clone();
    let struct_name = input.ident.clone();
    let name_lit = LitStr::new(&tail_name, Span::call_site());
    let struct_name_lit = LitStr::new(&struct_name.to_string(), struct_name.span());

    // Emit the original struct untouched plus a zero-sized metadata const
    // that `hopper::state` / `hopper_layout!` can detect.
    let gen = quote! {
        #input

        #[doc(hidden)]
        #[allow(non_upper_case_globals, dead_code)]
        const _: () = {
            // String marker const: its name is purposely unique-by-struct so
            // a macro scanning the module namespace can recover the mapping.
            const _HOPPER_DYNAMIC_TAIL_NAME_: &str = #name_lit;
            const _HOPPER_DYNAMIC_TAIL_OWNER_: &str = #struct_name_lit;
            // PhantomData binds the tail field's type so downstream derives
            // can reflect over it without re-parsing.
            const _HOPPER_DYNAMIC_TAIL_TY_: ::core::marker::PhantomData<#tail_ty> =
                ::core::marker::PhantomData;
            ()
        };
    };

    Ok(gen)
}
