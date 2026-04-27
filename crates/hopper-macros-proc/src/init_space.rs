//! `#[derive(HopperInitSpace)]` — standalone derive that emits
//! `const INIT_SPACE: usize` equal to `size_of::<Self>()`.
//!
//! Mirror of Anchor's `#[derive(InitSpace)]` with the Hopper naming
//! prefix (derive names share the same global namespace as item names
//! in some attribute positions, so a prefix avoids clashing with
//! Anchor's derive when both are in scope during a migration).
//!
//! For structs declared through `#[hopper::state]`, this value is
//! already emitted as part of the state's layout constants. The
//! standalone derive is for hand-authored `#[repr(C)]` structs that
//! want to participate in Hopper's `space =` idiom without adopting
//! the full layout attribute:
//!
//! ```ignore
//! use hopper::prelude::*;
//!
//! #[derive(HopperInitSpace)]
//! #[repr(C)]
//! pub struct Registration {
//!     pub bump: u8,
//!     pub delegate: [u8; 32],
//! }
//!
//! // Generated:
//! // impl Registration {
//! //     pub const INIT_SPACE: usize = core::mem::size_of::<Self>();
//! // }
//! //
//! // Then use at the call site:
//! // #[account(init, payer = authority, space = 16 + Registration::INIT_SPACE)]
//! // pub registration: Account<'info, Registration>,
//! ```
//!
//! The `16 +` offset accounts for Hopper's versioned header; the
//! derive only reports the body size. Programs that skip Hopper's
//! header (the raw-Pod path) use `Registration::INIT_SPACE` directly.
//!
//! ## Why `size_of` and not field-by-field summation
//!
//! Anchor's derive walks fields because its wire format is Borsh,
//! which packs variable-length types (`Vec<T>`, `Option<T>`, `String`)
//! with dynamic framing. Hopper's zero-copy layouts are
//! `#[repr(C)]`-stable alignment-1 byte runs with no variable framing;
//! `size_of::<Self>()` is exactly the wire size. A field-walking
//! implementation would be strictly less correct for Hopper's case
//! because it would miss trailing padding that Rust inserts when the
//! user declares an unusual layout (which `#[repr(C)]` + alignment-1
//! fields already prevents, but the derive should not assume).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Result};

pub fn expand(input: TokenStream) -> Result<TokenStream> {
    let input: DeriveInput = syn::parse2(input)?;
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #name #ty_generics #where_clause {
            /// Byte size of this type's body when laid out in an
            /// on-chain account. Equal to `size_of::<Self>()`.
            pub const INIT_SPACE: usize = ::core::mem::size_of::<Self>();
        }
    })
}
