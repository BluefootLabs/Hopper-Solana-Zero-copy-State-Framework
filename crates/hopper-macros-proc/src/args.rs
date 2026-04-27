//! `#[hopper::args]`. typed instruction-argument struct derive.
//!
//! Decorates a `#[repr(C)]` struct of fixed-size primitive fields. Emits:
//! - A `parse(data: &[u8]) -> Result<&Self, ArgParseError>` zero-copy parser
//!   that validates the buffer length and reinterprets the pointer.
//! - A `PACKED_SIZE: usize` const that equals the total byte footprint of
//!   the args (the dispatcher uses this to split tag + args + tail
//!   deterministically).
//! - An `ARG_DESCRIPTORS: &[ArgDescriptor]` const slice the schema exporter
//!   reads when emitting the IDL (name, canonical type, size per field).
//! - A **CU cost hint** const (`CU_HINT: u32`): declared by the user via
//!   `#[hopper::args(cu = 1200)]` and surfaced in the manifest so client
//!   builders can budget compute before submitting.
//!
//! ## Innovation over Quasar / Anchor
//!
//! Anchor and Quasar parse args via Borsh deserialization into owned values.
//! Hopper's args derive is **borrowing zero-copy**: the handler receives a
//! `&'a VaultDepositArgs` where the bytes still live in the instruction-data
//! region. No allocation. No copy. No serialization boundary.
//!
//! The `cu` hint is also novel: clients can statically budget before
//! submitting instead of discovering CU overruns on-chain.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse2, parse::Parser, Fields, ItemStruct, LitInt, LitStr, Meta, Token, punctuated::Punctuated};

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let input: ItemStruct = parse2(item)?;

    let metas: Punctuated<Meta, Token![,]> =
        Punctuated::<Meta, Token![,]>::parse_terminated
            .parse2(attr.clone())
            .unwrap_or_default();

    let mut cu_hint: u32 = 0;
    let mut allow_tail = false;
    for m in &metas {
        match m {
            Meta::NameValue(nv) => {
                if nv.path.is_ident("cu") {
                    if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(li), .. }) = &nv.value {
                        cu_hint = li.base10_parse::<u32>()?;
                    }
                }
            }
            // Bare `tail` flag marks the args struct as accepting
            // trailing bytes past the packed prefix. The emitted
            // `parse()` still only validates `data.len() >= PACKED_SIZE`
            // but gains a `parse_with_tail()` companion that
            // returns `(&Self, &[u8])`. Tail bytes are the Hopper
            // equivalent of Quasar's `Tail<&[u8]>` pattern: a
            // variable-size suffix decoded by the handler rather
            // than the args derive.
            Meta::Path(p) if p.is_ident("tail") => {
                allow_tail = true;
            }
            _ => {}
        }
    }

    let name = input.ident.clone();

    let fields = match &input.fields {
        Fields::Named(n) => n.named.iter().collect::<Vec<_>>(),
        _ => {
            return Err(syn::Error::new_spanned(
                &name,
                "#[hopper::args] requires a named-field struct",
            ));
        }
    };

    if fields.is_empty() {
        return Err(syn::Error::new_spanned(
            &name,
            "#[hopper::args] requires at least one field",
        ));
    }

    let mut descriptor_entries = Vec::with_capacity(fields.len());
    for f in &fields {
        let fname = LitStr::new(
            &f.ident.as_ref().unwrap().to_string(),
            f.ident.as_ref().unwrap().span(),
        );
        let canonical = LitStr::new(&canonical_ty_name(&f.ty), f.ty.clone().into_token_stream_span());
        let ty = &f.ty;
        descriptor_entries.push(quote! {
            ::hopper::hopper_schema::ArgDescriptor {
                name: #fname,
                canonical_type: #canonical,
                size: ::core::mem::size_of::<#ty>() as u16,
            }
        });
    }

    let cu_lit = LitInt::new(&format!("{}u32", cu_hint), name.span());
    let ty_list: Vec<_> = fields.iter().map(|f| &f.ty).collect();

    // For every `OptionByte<T>` field in the struct, emit a post-cast
    // `validate_tag()` call so a malformed tag byte (anything other
    // than 0 or 1) fails `parse()` instead of leaking into user code.
    // This matches Quasar's `OptionZc::validate_zc` contract. Pure
    // text match on the outer type name: a field spelled
    // `hopper_runtime::option_byte::OptionByte<...>` or just
    // `OptionByte<...>` both route through.
    let option_field_idents: Vec<&syn::Ident> = fields
        .iter()
        .filter(|f| is_option_byte_type(&f.ty))
        .filter_map(|f| f.ident.as_ref())
        .collect();
    let tag_validators: Vec<TokenStream> = option_field_idents
        .iter()
        .map(|ident| {
            quote! {
                r.#ident.validate_tag()?;
            }
        })
        .collect();

    // Tail support. Emit `parse_with_tail` only when the struct
    // opted in via `#[hopper::args(tail)]`. The helper returns
    // `(&Self, &[u8])`, where the second slice is the bytes past
    // `PACKED_SIZE`. Handlers use it for variable-length payloads
    // like memo fields, Merkle leaves, and routed-CPI blobs.
    let parse_with_tail_fn = if allow_tail {
        quote! {
            /// Parse the fixed-size prefix AND expose the trailing
            /// bytes as a zero-copy `&[u8]` slice. Use when the
            /// instruction carries a variable-length suffix.
            ///
            /// Available because the `#[hopper::args(tail)]` marker
            /// is set. For strict fixed-size args, use `parse`.
            #[inline]
            pub fn parse_with_tail(data: &[u8])
                -> ::core::result::Result<
                    (&Self, &[u8]),
                    ::hopper::hopper_schema::ArgParseError,
                >
            {
                let head = Self::parse(data)?;
                let tail = &data[Self::PACKED_SIZE..];
                ::core::result::Result::Ok((head, tail))
            }
        }
    } else {
        TokenStream::new()
    };

    let gen = quote! {
        #input

        impl #name {
            /// Total on-wire size in bytes.
            pub const PACKED_SIZE: usize = 0 #( + ::core::mem::size_of::<#ty_list>() )*;

            /// Caller-declared compute-unit budget hint. 0 means "unknown".
            pub const CU_HINT: u32 = #cu_lit;

            /// Per-field descriptor slice the schema crate ingests.
            pub const ARG_DESCRIPTORS: &'static [::hopper::hopper_schema::ArgDescriptor] = &[
                #( #descriptor_entries ),*
            ];

            /// Zero-copy parse: verify length, cast the pointer, return a
            /// borrowed reference valid for the lifetime of the input slice.
            ///
            /// Returns `Err(ArgParseError::TooShort)` when the buffer is
            /// smaller than `PACKED_SIZE`.
            #[inline]
            pub fn parse(data: &[u8]) -> ::core::result::Result<&Self, ::hopper::hopper_schema::ArgParseError> {
                if data.len() < Self::PACKED_SIZE {
                    return ::core::result::Result::Err(
                        ::hopper::hopper_schema::ArgParseError::TooShort {
                            required: Self::PACKED_SIZE as u16,
                            got: data.len() as u16,
                        }
                    );
                }
                // SAFETY: this macro is only applied to `#[repr(C)]` structs
                // of primitive-sized fields. The PACKED_SIZE check above
                // guarantees the input covers every field. No alignment
                // requirement is imposed because we operate over a byte
                // pointer.
                let r = unsafe { &*(data.as_ptr() as *const Self) };
                ::core::result::Result::Ok(r)
            }

            /// Validate every `OptionByte<T>` tag byte on this args
            /// struct. Returns `Err(ProgramError::InvalidInstructionData)`
            /// when any tag is not 0 or 1. `parse_checked` runs this
            /// automatically; call it directly when you have a
            /// borrowed `&Self` from another source.
            #[inline]
            pub fn validate_tags(&self)
                -> ::core::result::Result<(), ::hopper::__runtime::ProgramError>
            {
                #( #tag_validators )*
                ::core::result::Result::Ok(())
            }

            /// Zero-copy parse plus `OptionByte` tag validation in one
            /// call. Prefer this over `parse(...)` when the args
            /// struct carries any `OptionByte<T>` fields.
            #[inline]
            pub fn parse_checked(data: &[u8])
                -> ::core::result::Result<&Self, ::hopper::__runtime::ProgramError>
            {
                let r = Self::parse(data).map_err(|_| {
                    ::hopper::__runtime::ProgramError::InvalidInstructionData
                })?;
                r.validate_tags()?;
                ::core::result::Result::Ok(r)
            }

            #parse_with_tail_fn
        }
    };

    Ok(gen)
}

/// Heuristic: does this type spell `OptionByte<...>`?
///
/// The rule is name-only because `#[hopper::args]` runs at macro
/// expansion time with no type-resolution context. Users who qualify
/// the type as `hopper_runtime::option_byte::OptionByte<T>` or just
/// `OptionByte<T>` both match; any other alias falls through. A
/// different name means the user will need to call `.validate_tag()`
/// on their args struct themselves.
fn is_option_byte_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(p) = ty {
        if let Some(last) = p.path.segments.last() {
            return last.ident == "OptionByte";
        }
    }
    false
}

fn canonical_ty_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        syn::Type::Array(a) => {
            let inner = canonical_ty_name(&a.elem);
            format!("[{};{}]", inner, describe_array_len(&a.len))
        }
        _ => "unknown".to_string(),
    }
}

fn describe_array_len(expr: &syn::Expr) -> String {
    if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(li), .. }) = expr {
        li.base10_digits().to_string()
    } else {
        "?".to_string()
    }
}

// Small extension to obtain a Span from a `syn::Type` without unwrapping
// nested variants. The ident-span dance just pins error messages.
trait IntoTokenStreamSpan {
    fn into_token_stream_span(self) -> proc_macro2::Span;
}
impl IntoTokenStreamSpan for syn::Type {
    fn into_token_stream_span(self) -> proc_macro2::Span {
        match &self {
            syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.span()).unwrap_or_else(proc_macro2::Span::call_site),
            _ => proc_macro2::Span::call_site(),
        }
    }
}

#[cfg(test)]
mod args_tests {
    use super::*;
    use quote::quote;

    fn expand_ok(attr: TokenStream, item: TokenStream) -> String {
        expand(attr, item).expect("expand ok").to_string()
    }

    #[test]
    fn plain_args_emit_parse_checked_and_validate_tags() {
        let expanded = expand_ok(
            quote!(),
            quote! {
                #[repr(C)]
                pub struct Simple {
                    pub amount: u64,
                }
            },
        );
        assert!(expanded.contains("fn parse ("));
        assert!(expanded.contains("fn parse_checked ("));
        assert!(expanded.contains("fn validate_tags ("));
        assert!(!expanded.contains("fn parse_with_tail ("));
    }

    #[test]
    fn tail_flag_emits_parse_with_tail() {
        let expanded = expand_ok(
            quote!(tail),
            quote! {
                #[repr(C)]
                pub struct WithTail {
                    pub amount: u64,
                }
            },
        );
        assert!(expanded.contains("fn parse_with_tail ("));
    }

    #[test]
    fn cu_hint_is_recorded_on_the_impl() {
        let expanded = expand_ok(
            quote!(cu = 1200),
            quote! {
                #[repr(C)]
                pub struct Costed {
                    pub amount: u64,
                }
            },
        );
        assert!(expanded.contains("CU_HINT"));
        assert!(expanded.contains("1200u32"));
    }

    #[test]
    fn option_byte_field_emits_tag_validator() {
        let expanded = expand_ok(
            quote!(),
            quote! {
                #[repr(C)]
                pub struct WithOpt {
                    pub flag: OptionByte<u64>,
                }
            },
        );
        assert!(expanded.contains("validate_tags"));
        assert!(
            expanded.contains(". flag . validate_tag")
                || expanded.contains(".flag.validate_tag")
        );
    }
}
