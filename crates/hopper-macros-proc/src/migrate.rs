//! `#[hopper::migrate(from = N, to = M)]`. declare a schema-epoch edge.
//!
//! Closes audit innovation I4 macro side. The attribute decorates a
//! user-authored function that mutates an account body in-place from
//! `from` to `to`. The macro emits the fn unchanged plus a paired
//! `EDGE` constant of type `hopper_runtime::MigrationEdge` inside the
//! same module so downstream composition (`hopper::layout_migrations!`)
//! can collect edges into a layout's `LayoutMigration::MIGRATIONS`
//! slice.
//!
//! # Example
//!
//! ```ignore
//! #[hopper::migrate(from = 1, to = 2)]
//! pub fn vault_v1_to_v2(body: &mut [u8]) -> ProgramResult {
//!     // rearrange bytes, zero new fields, etc.
//!     Ok(())
//! }
//!
//! // Registration is explicit. no global inventory, no_std safe:
//! hopper::layout_migrations! {
//!     Vault = [vault_v1_to_v2, vault_v2_to_v3],
//! }
//! ```
//!
//! At compile time the macro enforces:
//!
//! - `to > from` (forward-only migrations)
//! - function signature `fn(&mut [u8]) -> Result<(), ProgramError>`
//! - epochs fit in `u32`
//!
//! The runtime side validates chain continuity (`apply_pending_migrations`
//! in `crates/hopper-runtime/src/migrate.rs`).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{parse2, ItemFn, LitInt, Result};

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let (from, to) = parse_attr(attr)?;
    if to <= from {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "#[hopper::migrate] requires to > from (got from={}, to={}). Migrations only move forward.",
                from, to
            ),
        ));
    }

    let input: ItemFn = parse2(item)?;
    let fn_name = &input.sig.ident;
    let vis = &input.vis;
    // Compose a predictable edge-const name: {fn}_EDGE. Capitalising
    // the identifier keeps it distinct from the fn and avoids colliding
    // with user code.
    let edge_ident = format_ident!("{}_EDGE", fn_name.to_string().to_uppercase());

    // Emit the user's function unchanged + the paired EDGE const.
    // The const's `migrator` field takes the function as a fn pointer,
    // which enforces the expected signature at monomorphisation time.
    let expanded = quote! {
        #input

        #[doc = concat!(
            "Migration edge declared by `#[hopper::migrate(from = ",
            stringify!(#from), ", to = ", stringify!(#to), ")]`.\n\n",
            "Consumed by `hopper::layout_migrations!` when composing a ",
            "layout's `MIGRATIONS` chain."
        )]
        #vis const #edge_ident: ::hopper::__runtime::MigrationEdge =
            ::hopper::__runtime::MigrationEdge {
                from_epoch: #from,
                to_epoch: #to,
                migrator: #fn_name,
            };
    };

    Ok(expanded)
}

/// Parse `from = N, to = M` out of the attribute token stream.
fn parse_attr(attr: TokenStream) -> Result<(u32, u32)> {
    let mut from: Option<u32> = None;
    let mut to: Option<u32> = None;

    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("from") {
            let lit: LitInt = meta.value()?.parse()?;
            from = Some(lit.base10_parse()?);
            return Ok(());
        }
        if meta.path.is_ident("to") {
            let lit: LitInt = meta.value()?.parse()?;
            to = Some(lit.base10_parse()?);
            return Ok(());
        }
        Err(meta.error("unrecognized #[hopper::migrate] attribute. only `from` and `to` are accepted"))
    });
    syn::parse::Parser::parse2(parser, attr)?;

    let from = from.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[hopper::migrate] requires `from = N`",
        )
    })?;
    let to = to.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[hopper::migrate] requires `to = N`",
        )
    })?;

    Ok((from, to))
}
