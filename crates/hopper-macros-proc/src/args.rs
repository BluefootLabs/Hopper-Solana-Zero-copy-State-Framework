//! `#[hopper::args]`: typed instruction-argument struct derive.
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
//! ## Design notes
//!
//! Hopper's args derive is **borrowing zero-copy**: the handler receives a
//! `&'a VaultDepositArgs` where the bytes still live in the instruction-data
//! region. No allocation. No copy. No serialization boundary.
//!
//! The `cu` hint lets clients request a declared compute budget before
//! submitting instead of discovering the estimate only after simulation.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    parse::Parser, parse2, punctuated::Punctuated, Attribute, Fields, ItemStruct, LitInt, LitStr,
    Meta, Token,
};

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let input: ItemStruct = parse2(item)?;

    if !has_repr_c(&input.attrs) {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[hopper::args] requires #[repr(C)] so the zero-copy overlay has a stable layout",
        ));
    }

    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;

    let mut cu_hint: u32 = 0;
    let mut has_cu = false;
    let mut allow_tail = false;
    for m in &metas {
        match m {
            Meta::NameValue(nv) if nv.path.is_ident("cu") => {
                if has_cu {
                    return Err(syn::Error::new_spanned(m, "duplicate `cu` hint"));
                }
                let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(li),
                    ..
                }) = &nv.value
                else {
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        "`cu` requires a u32 integer literal",
                    ));
                };
                cu_hint = li.base10_parse::<u32>()?;
                has_cu = true;
            }
            // Bare `tail` flag marks the args struct as accepting
            // trailing bytes past the packed prefix. The emitted
            // `parse()` still only validates `data.len() >= PACKED_SIZE`
            // but gains a `parse_with_tail()` companion that
            // returns `(&Self, &[u8])`. The variable-size suffix is
            // decoded by the handler rather than the args derive.
            Meta::Path(p) if p.is_ident("tail") => {
                if allow_tail {
                    return Err(syn::Error::new_spanned(m, "duplicate `tail` option"));
                }
                allow_tail = true;
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    m,
                    "expected `tail` or `cu = <u32>`",
                ))
            }
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
        let canonical = LitStr::new(
            &canonical_ty_name(&f.ty),
            f.ty.clone().into_token_stream_span(),
        );
        let ty = &f.ty;
        descriptor_entries.push(quote! {
            ::hopper::hopper_schema::ArgDescriptor {
                name: #fname,
                canonical_type: #canonical,
                size: ::core::mem::size_of::<#ty>() as u16,
                encoding: ::hopper::hopper_schema::ArgEncoding::Fixed,
            }
        });
    }

    let cu_lit = LitInt::new(&format!("{}u32", cu_hint), name.span());
    let ty_list: Vec<_> = fields.iter().map(|f| &f.ty).collect();

    // Trait dispatch follows aliases and nested Pod implementations. The
    // default scalar validator is empty and can be optimized out.
    let field_idents: Vec<_> = fields.iter().filter_map(|f| f.ident.as_ref()).collect();

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
            /// is set. This performs a raw overlay without value validation.
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

            /// Validate the fixed prefix and return it with the borrowed tail.
            /// The caller validates the tail's application-specific encoding.
            #[inline]
            pub fn parse_with_tail_checked(data: &[u8])
                -> ::core::result::Result<(&Self, &[u8]), ::hopper::__runtime::ProgramError>
            {
                let head = Self::parse_checked(data)?;
                ::core::result::Result::Ok((head, &data[Self::PACKED_SIZE..]))
            }
        }
    } else {
        TokenStream::new()
    };

    let align_msg = format!(
        "#[hopper::args] struct `{}` must be alignment-1: instruction data is parsed at \
         arbitrary byte offsets, so an aligned field type (u64, u32, ...) would make the \
         generated `parse()` reference misaligned. Use wire types (WireU64, WireU32, ...) \
         or byte arrays.",
        name,
    );
    let size_msg = format!(
        "#[hopper::args] struct `{}` has implicit padding: size_of::<Self>() exceeds the \
         sum of field sizes, so `parse()`'s length check would under-validate the buffer. \
         Use alignment-1 field types so #[repr(C)] packs exactly.",
        name,
    );
    let nonzero_msg = format!(
        "#[hopper::args] struct `{}` has zero size; zero-sized argument overlays are rejected.",
        name,
    );

    let gen = quote! {
        #input

        // ── Compile-time safety fence ──────────────────────────────────
        // `parse()` reinterprets attacker-controlled instruction bytes as
        // `&Self`. That cast is sound only if the struct is alignment-1
        // (no misaligned reference at any input offset), padding-free
        // (the PACKED_SIZE length check covers every byte the reference
        // spans), and every field tolerates every bit pattern (Pod). All
        // three obligations are discharged here, at declaration time.
        const _: () = {
            assert!(::core::mem::align_of::<#name>() == 1, #align_msg);
            assert!(
                ::core::mem::size_of::<#name>() == (0 #( + ::core::mem::size_of::<#ty_list>() )*),
                #size_msg,
            );
            assert!(::core::mem::size_of::<#name>() > 0, #nonzero_msg);
        };

        // Field-level Pod proof: every field must satisfy Hopper's
        // all-bits-valid contract, because `parse()` materializes the
        // struct from arbitrary caller-supplied bytes. A `bool`, `char`,
        // reference, or non-`Pod` nested field fails this bound at the
        // declaration, not as UB at dispatch time.
        #[doc(hidden)]
        const _: () = {
            struct __ArgsFieldPodProof<T: ::hopper::__runtime::Pod>(
                ::core::marker::PhantomData<T>,
            );
            #(
                #[allow(dead_code)]
                const _: __ArgsFieldPodProof<#ty_list> =
                    __ArgsFieldPodProof(::core::marker::PhantomData);
            )*
        };

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
                // SAFETY: the compile-time fence above proves Self is
                // #[repr(C)], alignment-1 (a byte pointer at any offset is
                // correctly aligned), padding-free (PACKED_SIZE ==
                // size_of::<Self>(), so the length check covers every byte
                // the reference spans), and all-bits-valid (per-field Pod
                // proof). Reinterpreting any sufficiently long byte slice
                // is therefore sound.
                let r = unsafe { &*(data.as_ptr() as *const Self) };
                ::core::result::Result::Ok(r)
            }

            /// Validate each field through `Pod::validate_value`, including
            /// aliases, arrays, nested layouts, enums, and present options.
            /// All representation errors become `InvalidInstructionData`.
            #[inline]
            pub fn validate_values(&self) -> ::hopper::__runtime::ProgramResult {
                #(::hopper::__runtime::Pod::validate_value(&self.#field_idents)
                    .map_err(|_| ::hopper::__runtime::ProgramError::InvalidInstructionData)?;)*
                ::core::result::Result::Ok(())
            }

            /// Compatibility name for `validate_values`.
            #[inline]
            pub fn validate_tags(&self)
                -> ::core::result::Result<(), ::hopper::__runtime::ProgramError>
            {
                self.validate_values()
            }

            /// Borrow and validate a fixed prefix, allowing trailing bytes.
            /// Use `parse_exact_checked` when the complete payload must match.
            #[inline]
            pub fn parse_checked(data: &[u8])
                -> ::core::result::Result<&Self, ::hopper::__runtime::ProgramError>
            {
                let r = Self::parse(data).map_err(|_| {
                    ::hopper::__runtime::ProgramError::InvalidInstructionData
                })?;
                r.validate_values()?;
                ::core::result::Result::Ok(r)
            }

            /// Borrow and validate a complete fixed-size payload. Both short
            /// buffers and trailing bytes return `InvalidInstructionData`.
            #[inline]
            pub fn parse_exact_checked(data: &[u8])
                -> ::core::result::Result<&Self, ::hopper::__runtime::ProgramError>
            {
                if data.len() != Self::PACKED_SIZE {
                    return ::core::result::Result::Err(
                        ::hopper::__runtime::ProgramError::InvalidInstructionData,
                    );
                }
                Self::parse_checked(data)
            }

            #parse_with_tail_fn
        }

        impl<'__hopper_args> ::hopper::__macro_support::DecodeInstructionArg<'__hopper_args>
            for &'__hopper_args #name
        {
            const WIRE_SIZE: usize = #name::PACKED_SIZE;

            #[inline]
            fn decode(decoder: &mut ::hopper::__macro_support::Decoder<'__hopper_args>)
                -> ::core::result::Result<Self, ::hopper::__runtime::ProgramError>
            {
                let bytes = decoder.read_array_ref::<{ #name::PACKED_SIZE }>()?;
                #name::parse_exact_checked(bytes)
            }
        }
    };

    Ok(gen)
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
    if let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(li),
        ..
    }) = expr
    {
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
            syn::Type::Path(p) => p
                .path
                .segments
                .last()
                .map(|s| s.ident.span())
                .unwrap_or_else(proc_macro2::Span::call_site),
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
    fn fields_use_trait_validation() {
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
        assert!(expanded.contains("Pod :: validate_value (& self . flag)"));
    }

    #[test]
    fn invalid_options_report_an_error() {
        let item = quote! { #[repr(C)] pub struct Args { pub value: u8 } };
        for attr in [
            quote!(tails),
            quote!(tail, tail),
            quote!(tail = true),
            quote!(cu = "100"),
            quote!(cu = 1, cu = 2),
            quote!(cu = 4294967296),
            quote!(cu =),
        ] {
            assert!(expand(attr.clone(), item.clone()).is_err(), "{attr}");
        }
    }
}
