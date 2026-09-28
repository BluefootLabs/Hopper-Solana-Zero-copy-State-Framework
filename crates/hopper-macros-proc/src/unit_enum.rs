//! `#[hopper::unit_enum]`.
//!
//! Implements `hopper_runtime::UnitEnum` for a fieldless enum so that it
//! can be stored in a layout or an argument struct as `EnumByte<E>`. The
//! macro forces `#[repr(u8)]`, adds `Clone, Copy, PartialEq, Eq, Debug`
//! when the enum declares no derive of its own, and generates the
//! byte-to-variant mapping from the variants themselves, so the mapping
//! cannot drift from the declaration.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse2, Fields, ItemEnum, Result};

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new_spanned(
            attr,
            "#[hopper::unit_enum] takes no arguments",
        ));
    }
    let mut input: ItemEnum = parse2(item)?;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "#[hopper::unit_enum] supports plain enums; generics cannot be stored in one byte",
        ));
    }
    if input.variants.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[hopper::unit_enum] needs at least one variant",
        ));
    }
    if input.variants.len() > 256 {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[hopper::unit_enum] stores the enum in one byte: at most 256 variants",
        ));
    }
    for variant in &input.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "#[hopper::unit_enum] variants carry no data; a variant with fields has no \
                 one-byte encoding",
            ));
        }
    }

    // `repr`: accept an existing `#[repr(u8)]`, refuse any other repr, add
    // `#[repr(u8)]` when there is none.
    let mut has_repr_u8 = false;
    for attr in &input.attrs {
        if attr.path().is_ident("repr") {
            let mut is_u8 = false;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("u8") {
                    is_u8 = true;
                }
                Ok(())
            })?;
            if !is_u8 {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[hopper::unit_enum] requires #[repr(u8)] (or no repr, which the macro \
                     sets to u8)",
                ));
            }
            has_repr_u8 = true;
        }
    }
    let repr = if has_repr_u8 {
        TokenStream::new()
    } else {
        quote! { #[repr(u8)] }
    };
    let has_derive = input.attrs.iter().any(|a| a.path().is_ident("derive"));
    let derives = if has_derive {
        TokenStream::new()
    } else {
        quote! { #[derive(Clone, Copy, PartialEq, Eq, Debug)] }
    };

    let name = &input.ident;
    let variants: Vec<_> = input.variants.iter().map(|v| &v.ident).collect();
    let count = variants.len();
    // Strip nothing from the user's enum; discriminants stay where they
    // were written. The mapping compares against `Self::V as u8`, so an
    // explicit discriminant expression of any form is honoured.
    input.attrs.retain(|_| true);

    Ok(quote! {
        #derives
        #repr
        #input

        impl ::hopper::__runtime::UnitEnum for #name {
            #[inline(always)]
            fn to_byte(self) -> u8 {
                self as u8
            }

            #[inline]
            fn from_byte(byte: u8) -> ::core::option::Option<Self> {
                #(
                    if byte == Self::#variants as u8 {
                        return ::core::option::Option::Some(Self::#variants);
                    }
                )*
                ::core::option::Option::None
            }
        }

        impl #name {
            /// Number of variants.
            pub const VARIANT_COUNT: usize = #count;

            /// Every variant, in declaration order.
            pub const VARIANTS: [Self; #count] = [#(Self::#variants),*];
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact(tokens: TokenStream) -> String {
        tokens
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    #[test]
    fn expands_a_plain_enum_with_repr_derives_and_the_mapping() {
        let out = expand(
            TokenStream::new(),
            quote! {
                pub enum Status { Open = 1, Settled, Cancelled = 7 }
            },
        )
        .unwrap();
        let s = compact(out);
        assert!(s.contains("#[repr(u8)]"), "{s}");
        assert!(
            s.contains("#[derive(Clone,Copy,PartialEq,Eq,Debug)]"),
            "{s}"
        );
        assert!(
            s.contains("impl::hopper::__runtime::UnitEnumforStatus"),
            "{s}"
        );
        assert!(
            s.contains(
                "ifbyte==Self::Settledasu8{return::core::option::Option::Some(Self::Settled);}"
            ),
            "{s}"
        );
        assert!(s.contains("pubconstVARIANT_COUNT:usize=3usize;"), "{s}");
    }

    #[test]
    fn keeps_the_users_own_repr_and_derives() {
        let out = expand(
            TokenStream::new(),
            quote! {
                #[derive(Clone, Copy)]
                #[repr(u8)]
                pub enum Side { Bid, Ask }
            },
        )
        .unwrap();
        let s = compact(out);
        assert_eq!(s.matches("#[repr(u8)]").count(), 1, "{s}");
        assert!(!s.contains("PartialEq,Eq,Debug"), "{s}");
    }

    #[test]
    fn refuses_data_variants_other_reprs_generics_and_arguments() {
        assert!(expand(TokenStream::new(), quote! { enum E { A(u8) } }).is_err());
        assert!(expand(TokenStream::new(), quote! { enum E { A { x: u8 } } }).is_err());
        assert!(expand(TokenStream::new(), quote! { #[repr(u16)] enum E { A } }).is_err());
        assert!(expand(TokenStream::new(), quote! { enum E<T> { A, B(T) } }).is_err());
        assert!(expand(TokenStream::new(), quote! { enum E {} }).is_err());
        assert!(expand(quote! { extra }, quote! { enum E { A } }).is_err());
    }
}
