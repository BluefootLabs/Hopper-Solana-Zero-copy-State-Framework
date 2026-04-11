//! `#[hopper_state]` — SegmentMap codegen for zero-copy layout structs.
//!
//! Computes field offsets from the struct definition and generates a
//! `SegmentMap` impl with a const segment array. All offsets are derived
//! from `core::mem::size_of::<FieldType>()`, which Rust evaluates at
//! compile time.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{parse2, Fields, ItemStruct, Result};

pub fn expand(_attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let input: ItemStruct = parse2(item)?;
    let name = &input.ident;
    let vis = &input.vis;

    // Extract named fields only.
    let fields = match &input.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &input,
                "hopper_state requires a struct with named fields",
            ))
        }
    };

    // Build segment entries. We use `core::mem::size_of::<T>()` for each
    // field type, computing offsets as a running sum.
    let mut segment_entries = Vec::new();
    let mut module_items: Vec<TokenStream> = Vec::new();
    let mut inherent_items: Vec<TokenStream> = Vec::new();
    let mut prev_offset = quote! { 0u32 };

    let struct_name_upper = to_screaming_snake(&name.to_string());

    for field in fields.iter() {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let field_ty = &field.ty;
        let field_name_upper = to_screaming_snake(&field_name_str);

        // Current offset = previous offset (accumulated)
        let current_offset = prev_offset.clone();

        segment_entries.push(quote! {
            ::hopper::prelude::StaticSegment::new(
                #field_name_str,
                #current_offset,
                core::mem::size_of::<#field_ty>() as u32,
            )
        });

        // Const name for direct access: VAULT_BALANCE_OFFSET, etc.
        let const_name = format_ident!(
            "{}_{}_OFFSET",
            struct_name_upper,
            field_name_upper,
        );
        let const_size_name = format_ident!(
            "{}_{}_SIZE",
            struct_name_upper,
            field_name_upper,
        );
        let const_type_name = format_ident!(
            "{}_{}_TYPE",
            struct_name_upper,
            field_name_upper,
        );
        let assoc_offset_name = format_ident!("{}_OFFSET", field_name_upper);
        let assoc_size_name = format_ident!("{}_SIZE", field_name_upper);

        module_items.push(quote! {
            /// Byte offset of field `#field_name` within the layout body.
            #vis const #const_name: u32 = #current_offset;
            /// Byte size of field `#field_name`.
            #vis const #const_size_name: u32 = core::mem::size_of::<#field_ty>() as u32;
            /// Concrete Rust type of field `#field_name`.
            #vis type #const_type_name = #field_ty;
        });

        inherent_items.push(quote! {
            /// Byte offset of field `#field_name` within the layout body.
            #vis const #assoc_offset_name: u32 = #current_offset;
            /// Byte size of field `#field_name`.
            #vis const #assoc_size_name: u32 = core::mem::size_of::<#field_ty>() as u32;
        });

        // Next offset = current + size_of
        prev_offset = quote! { #current_offset + core::mem::size_of::<#field_ty>() as u32 };
    }

    let expanded = quote! {
        // Emit the original struct unchanged.
        #input

        // Module-level constants/type aliases used by generated context accessors.
        #(#module_items)*

        impl #name {
            #(#inherent_items)*
        }

        // SegmentMap implementation with compile-time segment table.
        impl ::hopper::prelude::SegmentMap for #name {
            const SEGMENTS: &'static [::hopper::prelude::StaticSegment] = &[
                #(#segment_entries),*
            ];
        }
    };

    Ok(expanded)
}

/// Convert an identifier to SCREAMING_SNAKE_CASE.
fn to_screaming_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_ascii_uppercase());
    }
    result
}
