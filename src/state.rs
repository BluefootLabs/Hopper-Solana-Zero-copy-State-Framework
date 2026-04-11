//! `#[hopper_state]` — contract-aware zero-copy layout codegen.
//!
//! The canonical proc-macro path must participate in the same runtime,
//! schema, and receipt pipeline as hand-written Hopper layouts. This macro
//! therefore emits more than a `SegmentMap`: it generates field metadata,
//! layout fingerprints, typed load helpers, and schema export hooks.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use sha2::{Digest, Sha256};
use syn::{parse::Parser, parse2, Attribute, Fields, ItemStruct, LitInt, Result};

#[derive(Clone, Copy)]
struct StateOptions {
    disc: Option<u8>,
    version: u8,
}

impl Default for StateOptions {
    fn default() -> Self {
        Self {
            disc: None,
            version: 1,
        }
    }
}

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let options = parse_state_options(attr)?;
    let input: ItemStruct = parse2(item)?;
    let name = &input.ident;
    let vis = &input.vis;

    if !has_repr_c(&input.attrs) {
        return Err(syn::Error::new_spanned(
            &input,
            "hopper_state requires #[repr(C)] so segment offsets and typed loads stay stable",
        ));
    }

    let fields = match &input.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &input,
                "hopper_state requires a struct with named fields",
            ))
        }
    };

    let mut segment_entries = Vec::new();
    let mut module_items = Vec::new();
    let mut inherent_items = Vec::new();
    let mut field_name_literals = Vec::new();
    let mut field_type_literals = Vec::new();
    let mut field_types = Vec::new();
    let mut running_offset = quote! { 0u32 };

    let struct_name_upper = to_screaming_snake(&name.to_string());

    for field in fields.iter() {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let field_ty = &field.ty;
        let field_name_upper = to_screaming_snake(&field_name_str);
        let current_offset = running_offset.clone();

        field_name_literals.push(syn::LitStr::new(&field_name_str, field_name.span()));
        field_type_literals.push(syn::LitStr::new(
            &field_ty.to_token_stream().to_string().replace(' ', ""),
            field_name.span(),
        ));
        field_types.push(field_ty.clone());

        segment_entries.push(quote! {
            ::hopper::hopper_core::segment_map::StaticSegment::new(
                #field_name_str,
                #current_offset,
                core::mem::size_of::<#field_ty>() as u32,
            )
        });

        let const_name = format_ident!("{}_{}_OFFSET", struct_name_upper, field_name_upper);
        let const_size_name = format_ident!("{}_{}_SIZE", struct_name_upper, field_name_upper);
        let const_type_name = format_ident!("{}_{}_TYPE", struct_name_upper, field_name_upper);
        let assoc_offset_name = format_ident!("{}_OFFSET", field_name_upper);
        let assoc_size_name = format_ident!("{}_SIZE", field_name_upper);

        module_items.push(quote! {
            #vis const #const_name: u32 = #current_offset;
            #vis const #const_size_name: u32 = core::mem::size_of::<#field_ty>() as u32;
            #vis type #const_type_name = #field_ty;
        });

        inherent_items.push(quote! {
            #vis const #assoc_offset_name: u32 = #current_offset;
            #vis const #assoc_size_name: u32 = core::mem::size_of::<#field_ty>() as u32;
        });

        running_offset = quote! {
            #current_offset + core::mem::size_of::<#field_ty>() as u32
        };
    }

    let body_size = running_offset.clone();
    let version = options.version;
    let layout_id = layout_id_bytes(name, version, fields);
    let disc = options.disc.unwrap_or(layout_id[0]);
    let layout_id_tokens = byte_array_literal(&layout_id);
    let field_count = field_name_literals.len();

    let expanded = quote! {
        #input

        const _: () = {
            assert!(
                core::mem::align_of::<#name>() == 1,
                "hopper_state layouts must use alignment-1 field types such as WireU64 or TypedAddress",
            );
            assert!(
                core::mem::size_of::<#name>() == ((#body_size) as usize),
                "hopper_state layouts must be #[repr(C)] with no implicit padding",
            );
        };

        #(#module_items)*

        impl #name {
            #(#inherent_items)*

            pub const BODY_SIZE: usize = core::mem::size_of::<Self>();
            pub const LEN: usize = ::hopper::hopper_core::account::HEADER_LEN + Self::BODY_SIZE;
            pub const DISC: u8 = #disc;
            pub const VERSION: u8 = #version;
            pub const LAYOUT_ID: [u8; 8] = #layout_id_tokens;

            #[inline(always)]
            pub fn overlay(
                data: &[u8],
            ) -> ::core::result::Result<&Self, ::hopper::__runtime::ProgramError> {
                ::hopper::hopper_core::account::pod_from_bytes::<Self>(data)
            }

            #[inline(always)]
            pub fn overlay_mut(
                data: &mut [u8],
            ) -> ::core::result::Result<&mut Self, ::hopper::__runtime::ProgramError> {
                ::hopper::hopper_core::account::pod_from_bytes_mut::<Self>(data)
            }

            #[inline(always)]
            pub fn load<'a>(
                account: &'a ::hopper::prelude::AccountView,
                program_id: &::hopper::prelude::Address,
            ) -> ::core::result::Result<
                ::hopper::__runtime::Ref<'a, Self>,
                ::hopper::__runtime::ProgramError,
            > {
                account.check_owned_by(program_id)?;
                account.load::<Self>()
            }

            #[inline(always)]
            pub fn load_mut<'a>(
                account: &'a ::hopper::prelude::AccountView,
                program_id: &::hopper::prelude::Address,
            ) -> ::core::result::Result<
                ::hopper::__runtime::RefMut<'a, Self>,
                ::hopper::__runtime::ProgramError,
            > {
                account.check_owned_by(program_id)?.check_writable()?;
                account.load_mut::<Self>()
            }

            #[inline(always)]
            pub fn load_foreign<'a>(
                account: &'a ::hopper::prelude::AccountView,
                expected_owner: &::hopper::prelude::Address,
            ) -> ::core::result::Result<
                ::hopper::__runtime::Ref<'a, Self>,
                ::hopper::__runtime::ProgramError,
            > {
                account.check_owned_by(expected_owner)?;
                account.load::<Self>()
            }
        }

        unsafe impl ::hopper::hopper_core::account::Pod for #name {}

        impl ::hopper::hopper_core::account::FixedLayout for #name {
            const SIZE: usize = core::mem::size_of::<Self>();
        }

        impl ::hopper::hopper_core::segment_map::SegmentMap for #name {
            const SEGMENTS: &'static [::hopper::hopper_core::segment_map::StaticSegment] = &[
                #(#segment_entries),*
            ];
        }

        impl ::hopper::hopper_core::field_map::FieldMap for #name {
            const FIELDS: &'static [::hopper::hopper_core::field_map::FieldInfo] = {
                const FIELD_COUNT: usize = #field_count;
                const NAMES: [&str; FIELD_COUNT] = [#(#field_name_literals),*];
                const SIZES: [usize; FIELD_COUNT] = [#(core::mem::size_of::<#field_types>()),*];
                const FIELDS: [::hopper::hopper_core::field_map::FieldInfo; FIELD_COUNT] = {
                    let mut result = [::hopper::hopper_core::field_map::FieldInfo::new("", 0, 0); FIELD_COUNT];
                    let mut offset = ::hopper::hopper_core::account::HEADER_LEN;
                    let mut index = 0;
                    while index < FIELD_COUNT {
                        result[index] = ::hopper::hopper_core::field_map::FieldInfo::new(
                            NAMES[index],
                            offset,
                            SIZES[index],
                        );
                        offset += SIZES[index];
                        index += 1;
                    }
                    result
                };
                &FIELDS
            };
        }

        impl ::hopper::hopper_runtime::LayoutContract for #name {
            const DISC: u8 = #name::DISC;
            const VERSION: u8 = #name::VERSION;
            const LAYOUT_ID: [u8; 8] = #name::LAYOUT_ID;
            const SIZE: usize = #name::LEN;
        }

        impl ::hopper::hopper_schema::SchemaExport for #name {
            fn layout_manifest() -> ::hopper::hopper_schema::LayoutManifest {
                const FIELD_COUNT: usize = #field_count;
                const NAMES: [&str; FIELD_COUNT] = [#(#field_name_literals),*];
                const TYPES: [&str; FIELD_COUNT] = [#(#field_type_literals),*];
                const SIZES: [u16; FIELD_COUNT] = [#(core::mem::size_of::<#field_types>() as u16),*];
                const FIELDS: [::hopper::hopper_schema::FieldDescriptor; FIELD_COUNT] = {
                    let mut result = [::hopper::hopper_schema::FieldDescriptor {
                        name: "",
                        canonical_type: "",
                        size: 0,
                        offset: 0,
                        intent: ::hopper::hopper_schema::FieldIntent::Custom,
                    }; FIELD_COUNT];
                    let mut offset = ::hopper::hopper_core::account::HEADER_LEN as u16;
                    let mut index = 0;
                    while index < FIELD_COUNT {
                        result[index] = ::hopper::hopper_schema::FieldDescriptor {
                            name: NAMES[index],
                            canonical_type: TYPES[index],
                            size: SIZES[index],
                            offset,
                            intent: ::hopper::hopper_schema::FieldIntent::Custom,
                        };
                        offset += SIZES[index];
                        index += 1;
                    }
                    result
                };

                ::hopper::hopper_schema::LayoutManifest {
                    name: stringify!(#name),
                    version: #name::VERSION,
                    disc: #name::DISC,
                    layout_id: #name::LAYOUT_ID,
                    total_size: #name::LEN,
                    field_count: FIELD_COUNT,
                    fields: &FIELDS,
                }
            }
        }
    };

    Ok(expanded)
}

fn parse_state_options(attr: TokenStream) -> Result<StateOptions> {
    if attr.is_empty() {
        return Ok(StateOptions::default());
    }

    let mut options = StateOptions::default();
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("disc") {
            let value: LitInt = meta.value()?.parse()?;
            options.disc = Some(value.base10_parse()?);
            return Ok(());
        }
        if meta.path.is_ident("version") {
            let value: LitInt = meta.value()?.parse()?;
            options.version = value.base10_parse()?;
            return Ok(());
        }
        Err(meta.error("unsupported hopper_state option; expected `disc = N` or `version = N`"))
    });

    parser.parse2(attr)?;
    Ok(options)
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

fn layout_id_bytes(
    name: &syn::Ident,
    version: u8,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> [u8; 8] {
    let mut input = format!("hopper:v1:{}:{}:", name, version);
    for field in fields {
        let field_name = field.ident.as_ref().expect("named fields only");
        let field_ty = field.ty.to_token_stream().to_string().replace(' ', "");
        input.push_str(&field_name.to_string());
        input.push(':');
        input.push_str(&field_ty);
        input.push(',');
    }

    let digest = Sha256::digest(input.as_bytes());
    let mut layout_id = [0u8; 8];
    layout_id.copy_from_slice(&digest[..8]);
    layout_id
}

fn byte_array_literal(bytes: &[u8; 8]) -> TokenStream {
    let items = bytes.iter();
    quote! { [#(#items),*] }
}

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
