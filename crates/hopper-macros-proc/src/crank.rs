//! `#[hopper::crank]` - mark a handler as an autonomous crank.
//!
//! A crank is an instruction that a keeper bot can invoke with no
//! user-supplied arguments. This attribute decorates a handler inside
//! a `#[hopper::program]` module and does three things:
//!
//! 1. Records the `"Crank"` capability on the emitted
//!    `InstructionDescriptor` so `hopper manager crank list` finds it
//!    and so off-chain tools can enumerate crankable instructions
//!    without reading source.
//! 2. Statically rejects the handler if it takes any value arguments;
//!    autonomous cranks carry zero data bytes beyond the
//!    discriminator. Catching this at compile time is cheaper than
//!    discovering it at `hopper manager crank run` time.
//! 3. Optionally captures a `seeds_hint` manifest entry per PDA
//!    account the crank's context declares, so the crank loop can
//!    resolve every account with no operator configuration. Users
//!    supply the hints via `#[hopper::crank(seeds(account_name =
//!    [b"seed_1", b"seed_2"]))]` on the handler.
//!
//! ## Example
//!
//! ```ignore
//! #[hopper::crank(seeds(
//!     pool = [b"pool", mint.as_ref()],
//!     fee_vault = [b"fee_vault", pool.as_ref()],
//! ))]
//! #[instruction(42)]
//! fn settle_fees(ctx: Context<Settle>) -> ProgramResult {
//!     // ...
//! }
//! ```
//!
//! The emitted `seeds_hint` block rides along in the program
//! manifest under a top-level `seeds_hint` object keyed by account
//! name. A user running `hopper manager crank run --program-id <id>`
//! never has to pass `--account` for a crank that has hints.
//!
//! ## Innovation (not a copy of Quasar)
//!
//! Quasar's cranks are a convention; the framework does not stamp
//! anything onto the manifest. Hopper's crank attribute is an
//! opt-in but type-checked: miss the zero-arg rule and the macro
//! refuses to compile. The `seeds_hint` manifest entry is Hopper-
//! specific and lets a generic crank runner work against any
//! Hopper program without per-program config files.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse2, Attribute, Expr, Ident, ItemFn, Meta, Token};

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let func: ItemFn = parse2(item)?;

    let seed_hints = parse_seeds_hint(attr)?;

    // Count value args (everything after the context parameter). A
    // crank must have exactly one parameter: the context. Every
    // value arg the user declared is a reason this handler is NOT
    // autonomous and should go through `hopper manager invoke`
    // instead. Reject loudly at compile time.
    let value_arg_count = func
        .sig
        .inputs
        .iter()
        .enumerate()
        .filter(|(i, _)| *i > 0)
        .count();
    if value_arg_count > 0 {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "#[hopper::crank] requires a zero-arg handler. Cranks are autonomous; if this instruction needs user input, use `hopper manager invoke` instead.",
        ));
    }

    // Emit the original function plus a hidden const carrying the
    // crank marker and seed hints. The program macro's downstream
    // consumer (the manifest serializer) picks these up during
    // schema export. We name the const with a stable per-function
    // prefix so two cranks in the same module do not collide.
    let fn_name = &func.sig.ident;
    let const_name = quote::format_ident!("__HOPPER_CRANK_{}", fn_name);

    let seeds_entries: Vec<TokenStream> = seed_hints
        .iter()
        .map(|(field, seeds)| {
            let field_lit = field.to_string();
            let seed_exprs = seeds;
            quote! {
                (#field_lit, &[ #( #seed_exprs ),* ] as &[&[u8]])
            }
        })
        .collect();

    let expanded = quote! {
        #func

        /// Compile-time crank descriptor for `#fn_name`.
        ///
        /// Surfaces the `"Crank"` capability plus auto-resolvable
        /// account seeds for `hopper manager crank run` and any
        /// downstream off-chain tooling that enumerates autonomous
        /// instructions.
        #[allow(non_upper_case_globals, dead_code)]
        pub const #const_name: ::hopper::__runtime::CrankMarker = ::hopper::__runtime::CrankMarker {
            handler_name: stringify!(#fn_name),
            seed_hints: &[ #( #seeds_entries ),* ],
        };
    };

    Ok(expanded)
}

/// Parse `seeds(account_name = [seed_expr, ...], ...)` out of the
/// attribute's meta list. Empty attribute lists are accepted and
/// yield an empty hint set; the macro still records the crank
/// capability so the manifest carries the marker.
fn parse_seeds_hint(attr: TokenStream) -> syn::Result<Vec<(Ident, Vec<Expr>)>> {
    if attr.is_empty() {
        return Ok(Vec::new());
    }
    let metas: syn::punctuated::Punctuated<Meta, Token![,]> =
        syn::parse::Parser::parse2(
            syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated,
            attr,
        )?;
    let mut out: Vec<(Ident, Vec<Expr>)> = Vec::new();
    for meta in metas {
        let Meta::List(list) = meta else {
            return Err(syn::Error::new_spanned(
                meta,
                "#[hopper::crank] accepts only `seeds(...)` today",
            ));
        };
        if !list.path.is_ident("seeds") {
            return Err(syn::Error::new_spanned(
                &list.path,
                "#[hopper::crank] accepts only `seeds(...)` today",
            ));
        }
        // Parse the inner `name = [expr, ...], name = [...]` list.
        let parser = |input: syn::parse::ParseStream| -> syn::Result<Vec<(Ident, Vec<Expr>)>> {
            let mut pairs: Vec<(Ident, Vec<Expr>)> = Vec::new();
            while !input.is_empty() {
                let name: Ident = input.parse()?;
                let _eq: Token![=] = input.parse()?;
                let arr: syn::ExprArray = input.parse()?;
                let seeds: Vec<Expr> = arr.elems.into_iter().collect();
                pairs.push((name, seeds));
                if input.peek(Token![,]) {
                    let _: Token![,] = input.parse()?;
                }
            }
            Ok(pairs)
        };
        let pairs = syn::parse::Parser::parse2(parser, list.tokens.clone())?;
        out.extend(pairs);
    }
    Ok(out)
}
