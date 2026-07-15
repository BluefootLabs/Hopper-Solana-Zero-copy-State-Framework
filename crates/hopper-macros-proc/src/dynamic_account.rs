//! `#[hopper::dynamic_account]`.
//!
//! Quasar-style authoring surface for bounded dynamic fields and explicit raw
//! final tails. The macro keeps Hopper's wire truth explicit: a fixed zero-copy
//! body plus one length-bounded dynamic payload encoded after the body as
//! `[u32 len][payload]`. Bounded fields encode inside the payload; a final
//! `TailStr<'a>` or `TailBytes<'a>` consumes the remaining payload bytes.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::parse::{Parse, ParseStream, Parser};
use syn::{
    parse2, Attribute, Expr, ExprLit, Field, Fields, GenericArgument, GenericParam, Ident,
    ItemStruct, Lit, LitInt, LitStr, PathArguments, Result, Token, Type, Visibility,
};

#[derive(Default)]
struct Options {
    disc: Option<LitInt>,
    version: Option<LitInt>,
}

// Build-time-only AST descriptor; the size spread between unit and
// `syn::Type`-carrying variants is irrelevant for a proc-macro that allocates
// one of these per field at compile time. Boxing would only add indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum TailKind {
    String {
        cap: LitInt,
    },
    Vec {
        ty: Type,
        cap: LitInt,
        borrowed_slice: bool,
    },
    /// Growable typed sequence: `Seq<'a, T>` (no capacity). Backed by the
    /// runtime `[count:u32][T; ..]` fixed-stride cursors. It is the SOLE,
    /// final tail of its account (one growable tail per account) and its
    /// on-wire layout — hence the layout id — carries NO capacity, so
    /// growing the account never changes the account type.
    Seq {
        ty: Type,
    },
    TailStr,
    TailBytes,
}

impl TailKind {
    fn is_raw_final(&self) -> bool {
        matches!(self, Self::TailStr | Self::TailBytes)
    }

    fn is_seq(&self) -> bool {
        matches!(self, Self::Seq { .. })
    }
}

#[derive(Clone)]
struct TailField {
    ident: Ident,
    kind: TailKind,
}

struct TailSpec {
    kind: TailKind,
}

impl Parse for TailSpec {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let keyword: Ident = input.parse()?;

        if keyword == "tail_str" || keyword == "str" {
            return Ok(Self {
                kind: TailKind::TailStr,
            });
        }

        if keyword == "tail_bytes" || keyword == "bytes" {
            return Ok(Self {
                kind: TailKind::TailBytes,
            });
        }

        input.parse::<Token![<]>()?;

        if keyword == "string" {
            let cap: LitInt = input.parse()?;
            input.parse::<Token![>]>()?;
            return Ok(Self {
                kind: TailKind::String { cap },
            });
        }

        if keyword == "vec" {
            let ty: Type = input.parse()?;
            input.parse::<Token![,]>()?;
            let cap: LitInt = input.parse()?;
            input.parse::<Token![>]>()?;

            return Ok(Self {
                kind: TailKind::Vec {
                    borrowed_slice: is_address_type(&ty),
                    ty,
                    cap,
                },
            });
        }

        if keyword == "seq" {
            // `seq<T>` — a growable typed sequence, NO capacity.
            let ty: Type = input.parse()?;
            input.parse::<Token![>]>()?;
            return Ok(Self {
                kind: TailKind::Seq { ty },
            });
        }

        Err(syn::Error::new_spanned(
            keyword,
            "unsupported #[tail(...)] spec; expected `string<N>`, `vec<T, N>`, `seq<T>`, `str`, or `bytes`",
        ))
    }
}

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let options = parse_options(attr)?;
    let mut input: ItemStruct = parse2(item)?;
    strip_authoring_lifetime_generics(&mut input)?;
    let name = input.ident.clone();
    let vis = input.vis.clone();
    let tail_name = format_ident!("{}Tail", name);
    let tail_head_name = format_ident!("{}TailHead", name);
    let tail_view_name = format_ident!("{}TailView", name);
    let tail_editor_name = format_ident!("{}TailEditor", name);
    let tail_ext_trait_name = format_ident!("{}AccountTailExt", name);

    let named = match input.fields {
        Fields::Named(named) => named.named,
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "#[hopper::dynamic_account] requires a named-field struct",
            ));
        }
    };
    let named_fields: Vec<Field> = named.into_iter().collect();
    let named_len = named_fields.len();

    let mut fixed_fields = Vec::new();
    let mut fixed_getters = Vec::new();
    let mut fixed_new_params = Vec::new();
    let mut fixed_new_inits = Vec::new();
    let mut tail_fields = Vec::new();

    for (field_index, mut field) in named_fields.into_iter().enumerate() {
        let Some(ident) = field.ident.clone() else {
            continue;
        };
        let tail_spec = parse_tail_attr(&field)?;
        let pretty_tail = if tail_spec.is_none() {
            parse_pretty_tail_type(&field.ty)?
        } else {
            None
        };
        field.attrs = strip_tail_attrs(field.attrs);

        if let Some(kind) = tail_spec.map(|spec| spec.kind).or(pretty_tail) {
            if kind.is_raw_final() && field_index + 1 != named_len {
                return Err(syn::Error::new_spanned(
                    field.ty,
                    "TailStr<'a> and TailBytes<'a> are bare final tails and must be the last account field",
                ));
            }
            tail_fields.push(TailField { ident, kind });
            continue;
        }

        if is_unbounded_dynamic_type(&field.ty) {
            return Err(syn::Error::new_spanned(
                field.ty,
                "dynamic String/Text and Vec/List fields must be bounded, for example `String<'a, 32>` or `Vec<'a, Address, 10>`; explicit systems-mode syntax can use #[tail(string<N>)] or #[tail(vec<T, N>)]",
            ));
        }

        let (wire_ty, getter_ty, getter_expr, init_expr) = fixed_wire_type(&field.ty, &ident);
        let attrs = field.attrs;
        let field_vis = field.vis;
        fixed_fields.push(quote! {
            #(#attrs)*
            #field_vis #ident: #wire_ty
        });
        fixed_new_params.push(quote! { #ident: #getter_ty });
        fixed_new_inits.push(quote! { #ident: #init_expr });
        fixed_getters.push(quote! {
            #[inline(always)]
            #vis fn #ident(&self) -> #getter_ty {
                #getter_expr
            }
        });
    }

    if tail_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            name,
            "#[hopper::dynamic_account] requires at least one #[tail(...)] field; use #[hopper::account] for fixed-only accounts",
        ));
    }

    let raw_tail_count = tail_fields
        .iter()
        .filter(|field| field.kind.is_raw_final())
        .count();
    if raw_tail_count > 1 {
        return Err(syn::Error::new_spanned(
            name,
            "only one bare final tail is allowed per dynamic account",
        ));
    }
    let has_raw_tail = raw_tail_count == 1;

    let tail_struct_fields: Vec<_> = tail_fields
        .iter()
        .filter(|field| !field.kind.is_raw_final())
        .map(tail_struct_field)
        .collect();
    let tail_value_fields: Vec<_> = tail_fields.iter().map(tail_value_field).collect();
    let tail_value_head_inits: Vec<_> = tail_fields
        .iter()
        .filter(|field| !field.kind.is_raw_final())
        .map(|field| {
            let ident = &field.ident;
            quote! { #ident: __head.#ident }
        })
        .collect();
    let tail_value_to_head_inits: Vec<_> = tail_fields
        .iter()
        .filter(|field| !field.kind.is_raw_final())
        .map(|field| {
            let ident = &field.ident;
            quote! { #ident: tail.#ident }
        })
        .collect();
    let raw_tail_field = tail_fields.iter().find(|field| field.kind.is_raw_final());
    let raw_tail_init = raw_tail_field.map(|field| {
        let ident = &field.ident;
        match &field.kind {
            TailKind::TailStr => {
                quote! { #ident: ::hopper::__runtime::TailStr::new(__raw_payload) }
            }
            TailKind::TailBytes => {
                quote! { #ident: ::hopper::__runtime::TailBytes::new(__raw_payload) }
            }
            _ => unreachable!(),
        }
    });
    let raw_tail_write_expr = raw_tail_field.map(|field| {
        let ident = &field.ident;
        quote! { tail.#ident.as_bytes() }
    });
    let raw_tail_validate = raw_tail_field.map(|field| match &field.kind {
        TailKind::TailStr => quote! {
            core::str::from_utf8(raw).map_err(|_| ::hopper::__runtime::ProgramError::InvalidAccountData)?;
        },
        TailKind::TailBytes => TokenStream::new(),
        _ => unreachable!(),
    });
    let tail_schema = tail_schema(&tail_fields);
    let tail_schema_lit = LitStr::new(&tail_schema, Span::call_site());
    let version = options
        .version
        .unwrap_or_else(|| LitInt::new("1", Span::call_site()));

    let mut state_args = Vec::new();
    if let Some(disc) = &options.disc {
        state_args.push(quote! { disc = #disc });
    }
    state_args.push(quote! { version = #version });
    if has_raw_tail {
        state_args.push(quote! { raw_tail = true });
    } else {
        state_args.push(quote! { dynamic_tail = #tail_name });
    }
    state_args.push(quote! { dynamic_tail_schema = #tail_schema_lit });

    let outer_attrs: Vec<Attribute> = input
        .attrs
        .into_iter()
        .filter(|attr| !attr.path().is_ident("derive") && !attr.path().is_ident("repr"))
        .collect();

    // ── Growable `Seq<T>` tail: the sole, capacity-free final tail ────
    //
    // A `Seq<T>` tail carries nothing bounded to materialize, so it skips
    // the owned-decode machinery entirely. It emits the fixed head as a
    // `#[hopper::state(raw_tail)]` layout PLUS streaming-cursor accessors
    // over the `[count:u32][T; ..]` tail region, and MIN_SPACE / space_for
    // instead of a fixed ALLOC_SPACE. The layout id comes from the
    // capacity-free `seq<T>` schema string, so growing the account (adding
    // capacity via `realloc`) never changes the account type.
    if tail_fields.iter().any(|field| field.kind.is_seq()) {
        return expand_seq_account(
            &name,
            &vis,
            &tail_fields,
            &fixed_fields,
            &fixed_getters,
            &fixed_new_params,
            &fixed_new_inits,
            &outer_attrs,
            options.disc.as_ref(),
            &version,
            &tail_schema_lit,
        );
    }

    let tail_view_methods: Vec<_> = tail_fields
        .iter()
        .enumerate()
        .map(|(idx, field)| tail_view_method(field, &tail_fields[..idx]))
        .collect();
    let tail_storage_ident = if has_raw_tail {
        format_ident!("head")
    } else {
        format_ident!("tail")
    };
    let account_tail_methods: Vec<_> = tail_fields
        .iter()
        .map(|field| account_tail_methods(field, &vis, has_raw_tail))
        .collect();
    let tail_editor_methods: Vec<_> = tail_fields
        .iter()
        .map(|field| tail_editor_methods(field, &tail_storage_ident, &name, has_raw_tail))
        .collect();
    let account_wrapper_trait_methods: Vec<_> = if has_raw_tail {
        Vec::new()
    } else {
        tail_fields
            .iter()
            .map(account_wrapper_trait_methods)
            .collect()
    };
    let account_wrapper_impl_methods: Vec<_> = if has_raw_tail {
        Vec::new()
    } else {
        tail_fields
            .iter()
            .map(|field| account_wrapper_impl_methods(field, &name, &tail_name))
            .collect()
    };
    let tail_element_assertions: Vec<_> = tail_fields
        .iter()
        .filter_map(tail_element_assertion)
        .collect();

    let tail_type_decl = if has_raw_tail {
        quote! {
            ::hopper::hopper_dynamic_fields! {
                #vis struct #tail_head_name {
                    #(#tail_struct_fields)*
                }
            }

            #[derive(Clone, Copy)]
            #vis struct #tail_name<'a> {
                #(#tail_value_fields)*
            }
        }
    } else {
        quote! {
            ::hopper::hopper_dynamic_fields! {
                #vis struct #tail_name {
                    #(#tail_struct_fields)*
                }
            }
        }
    };

    let dynamic_tail_impl_methods = if has_raw_tail {
        quote! {
            /// Maximum encoded bytes for bounded fields before the raw final tail.
            #vis const TAIL_HEAD_MAX_ENCODED_LEN: usize =
                <#tail_head_name as ::hopper::__runtime::TailCodec>::MAX_ENCODED_LEN;

            /// Minimum account allocation for body + tail prefix + bounded head.
            /// Use `space_for_tail(raw_len)` when allocating room for raw tail bytes.
            #vis const ALLOC_SPACE: usize = Self::INIT_SPACE + 4 + Self::TAIL_HEAD_MAX_ENCODED_LEN;

            /// Account allocation for a raw final tail of `raw_len` bytes.
            #[inline(always)]
            #vis const fn space_for_tail(raw_len: usize) -> usize {
                Self::ALLOC_SPACE + raw_len
            }

            /// Return the currently available tail payload capacity in `data`.
            #[inline]
            #vis fn tail_capacity(data: &[u8]) -> ::core::result::Result<usize, ::hopper::__runtime::ProgramError> {
                ::hopper::__runtime::tail_capacity(data, Self::TAIL_PREFIX_OFFSET)
            }

            /// Borrow the dynamic tail without decoding it into an owned value.
            #[inline]
            #vis fn tail_view<'a>(data: &'a [u8]) -> ::core::result::Result<#tail_view_name<'a>, ::hopper::__runtime::ProgramError> {
                let payload = ::hopper::__runtime::tail_payload(data, Self::TAIL_PREFIX_OFFSET)?;
                Ok(#tail_view_name { payload })
            }

            /// Borrow this account's dynamic tail. The final raw field consumes
            /// the remaining payload bytes without an inner length prefix.
            #[inline]
            #vis fn tail_read<'a>(data: &'a [u8]) -> ::core::result::Result<#tail_name<'a>, ::hopper::__runtime::ProgramError> {
                let __payload = ::hopper::__runtime::tail_payload(data, Self::TAIL_PREFIX_OFFSET)?;
                let (__head, __consumed) = <#tail_head_name as ::hopper::__runtime::TailCodec>::decode(__payload)?;
                let __raw_payload = __payload
                    .get(__consumed..)
                    .ok_or(::hopper::__runtime::ProgramError::InvalidAccountData)?;
                Ok(#tail_name {
                    #(#tail_value_head_inits,)*
                    #raw_tail_init
                })
            }

            /// Write the bounded head and raw final tail in place.
            #[inline]
            #vis fn tail_write(
                data: &mut [u8],
                tail: &#tail_name<'_>,
            ) -> ::core::result::Result<usize, ::hopper::__runtime::ProgramError> {
                let __head = #tail_head_name {
                    #(#tail_value_to_head_inits,)*
                };
                let raw = #raw_tail_write_expr;
                Self::tail_write_parts(data, &__head, raw)
            }

            /// Write the bounded head and explicit raw final tail bytes.
            #[inline]
            #vis fn tail_write_parts(
                data: &mut [u8],
                head: &#tail_head_name,
                raw: &[u8],
            ) -> ::core::result::Result<usize, ::hopper::__runtime::ProgramError> {
                #raw_tail_validate
                let __prefix_end = Self::TAIL_PREFIX_OFFSET
                    .checked_add(4)
                    .ok_or(::hopper::__runtime::ProgramError::AccountDataTooSmall)?;
                if data.len() < __prefix_end {
                    return Err(::hopper::__runtime::ProgramError::AccountDataTooSmall);
                }
                let __head_written = <#tail_head_name as ::hopper::__runtime::TailCodec>::encode(
                    head,
                    &mut data[__prefix_end..],
                )?;
                let __raw_start = __prefix_end
                    .checked_add(__head_written)
                    .ok_or(::hopper::__runtime::ProgramError::AccountDataTooSmall)?;
                let __raw_end = __raw_start
                    .checked_add(raw.len())
                    .ok_or(::hopper::__runtime::ProgramError::AccountDataTooSmall)?;
                if data.len() < __raw_end {
                    return Err(::hopper::__runtime::ProgramError::AccountDataTooSmall);
                }
                let __total = __head_written
                    .checked_add(raw.len())
                    .ok_or(::hopper::__runtime::ProgramError::InvalidAccountData)?;
                if __total > u32::MAX as usize {
                    return Err(::hopper::__runtime::ProgramError::InvalidAccountData);
                }
                data[__raw_start..__raw_end].copy_from_slice(raw);
                data[Self::TAIL_PREFIX_OFFSET..__prefix_end]
                    .copy_from_slice(&(__total as u32).to_le_bytes());
                Ok(__total)
            }

            /// Decode bounded fields into an editor. Raw final bytes are supplied
            /// when committing so no heap copy of the unbounded tail is needed.
            #[inline]
            #vis fn tail_editor<'a>(data: &'a mut [u8]) -> ::core::result::Result<#tail_editor_name<'a>, ::hopper::__runtime::ProgramError> {
                let __payload = ::hopper::__runtime::tail_payload(data, Self::TAIL_PREFIX_OFFSET)?;
                let (head, _) = <#tail_head_name as ::hopper::__runtime::TailCodec>::decode(__payload)?;
                Ok(#tail_editor_name { data, head })
            }

            #(#account_tail_methods)*
        }
    } else {
        quote! {
            /// Maximum encoded bytes for this account's compact dynamic tail.
            #vis const TAIL_MAX_ENCODED_LEN: usize =
                <#tail_name as ::hopper::__runtime::TailCodec>::MAX_ENCODED_LEN;

            /// Maximum account allocation for body + tail prefix + full tail.
            #vis const ALLOC_SPACE: usize = Self::INIT_SPACE + 4 + Self::TAIL_MAX_ENCODED_LEN;

            /// Return the currently available tail payload capacity in `data`.
            #[inline]
            #vis fn tail_capacity(data: &[u8]) -> ::core::result::Result<usize, ::hopper::__runtime::ProgramError> {
                ::hopper::__runtime::tail_capacity(data, Self::TAIL_PREFIX_OFFSET)
            }

            /// Borrow the compact dynamic tail without decoding it into an owned value.
            #[inline]
            #vis fn tail_view<'a>(data: &'a [u8]) -> ::core::result::Result<#tail_view_name<'a>, ::hopper::__runtime::ProgramError> {
                let payload = ::hopper::__runtime::tail_payload(data, Self::TAIL_PREFIX_OFFSET)?;
                Ok(#tail_view_name { payload })
            }

            /// Decode the tail into an editor. Call `commit()` to write changes back.
            #[inline]
            #vis fn tail_editor<'a>(data: &'a mut [u8]) -> ::core::result::Result<#tail_editor_name<'a>, ::hopper::__runtime::ProgramError> {
                let tail = Self::tail_read(data)?;
                Ok(#tail_editor_name { data, tail })
            }

            #(#account_tail_methods)*
        }
    };

    let account_wrapper_extension = if has_raw_tail {
        TokenStream::new()
    } else {
        quote! {
            #vis trait #tail_ext_trait_name {
                /// Decode this account's compact dynamic tail into an owned value.
                fn tail_read(&self) -> ::core::result::Result<#tail_name, ::hopper::__runtime::ProgramError>;

                /// Write this account's compact dynamic tail in place.
                fn tail_write(&self, tail: &#tail_name) -> ::core::result::Result<usize, ::hopper::__runtime::ProgramError>;

                #(#account_wrapper_trait_methods)*
            }

            impl<'info> #tail_ext_trait_name for ::hopper::prelude::Account<'info, #name> {
                /// Decode this account's compact dynamic tail into an owned value.
                /// Borrowed zero-copy views remain available through `#name::tail_view(data)`
                /// when the caller already holds the account data borrow.
                #[inline]
                fn tail_read(&self) -> ::core::result::Result<#tail_name, ::hopper::__runtime::ProgramError> {
                    let __data = self.as_account().try_borrow()?;
                    #name::tail_read(&__data)
                }

                /// Write this account's compact dynamic tail in place.
                #[inline]
                fn tail_write(&self, tail: &#tail_name) -> ::core::result::Result<usize, ::hopper::__runtime::ProgramError> {
                    let mut __data = self.as_account().try_borrow_mut()?;
                    #name::tail_write(&mut __data, tail)
                }

                #(#account_wrapper_impl_methods)*
            }

            impl<'info> #tail_ext_trait_name for ::hopper::prelude::InitAccount<'info, #name> {
                /// Decode this initialized account's compact dynamic tail into an owned value.
                #[inline]
                fn tail_read(&self) -> ::core::result::Result<#tail_name, ::hopper::__runtime::ProgramError> {
                    let __data = self.as_account().try_borrow()?;
                    #name::tail_read(&__data)
                }

                /// Write this initialized account's compact dynamic tail in place.
                #[inline]
                fn tail_write(&self, tail: &#tail_name) -> ::core::result::Result<usize, ::hopper::__runtime::ProgramError> {
                    let mut __data = self.as_account().try_borrow_mut()?;
                    #name::tail_write(&mut __data, tail)
                }

                #(#account_wrapper_impl_methods)*
            }
        }
    };

    let tail_editor_storage = if has_raw_tail {
        quote! { head: #tail_head_name, }
    } else {
        quote! { tail: #tail_name, }
    };
    let tail_editor_commit = if has_raw_tail {
        quote! {
            /// Write the edited bounded fields back with explicit raw final bytes.
            #[inline]
            #vis fn commit_with_raw(self, raw: &[u8]) -> ::hopper::prelude::ProgramResult {
                #name::tail_write_parts(self.data, &self.head, raw).map(|_| ())
            }
        }
    } else {
        quote! {
            /// Write the edited tail back into the account bytes.
            #[inline]
            #vis fn commit(self) -> ::hopper::prelude::ProgramResult {
                #name::tail_write(self.data, &self.tail).map(|_| ())
            }
        }
    };

    Ok(quote! {
        #(#tail_element_assertions)*
        #tail_type_decl

        #(#outer_attrs)*
        #[doc = "Hopper dynamic account."]
        #[doc = "Author fixed fields normally; Hopper stores dynamic data as a protocol tail."]
        #[doc = "Wire model: fixed body + [u32 len] compact dynamic tail."]
        #[doc = concat!("Tail schema: ", #tail_schema_lit)]
        #[derive(Clone, Copy)]
        #[repr(C)]
        #[hopper::state(#(#state_args),*)]
        #vis struct #name {
            #(#fixed_fields,)*
        }

        impl #name {
            /// Construct the fixed account body. Dynamic fields are written
            /// separately with `tail_write`, `tail_editor`, or generated setters.
            // One parameter per fixed field mirrors the user's struct; arity is
            // governed by their schema, not an API design choice.
            #[allow(clippy::too_many_arguments)]
            #[inline(always)]
            #vis fn new(#(#fixed_new_params),*) -> Self {
                Self {
                    #(#fixed_new_inits,)*
                }
            }

            #(#fixed_getters)*

            #dynamic_tail_impl_methods
        }

        #account_wrapper_extension

        #vis struct #tail_view_name<'a> {
            payload: &'a [u8],
        }

        impl<'a> #tail_view_name<'a> {
            #(#tail_view_methods)*
        }

        #vis struct #tail_editor_name<'a> {
            data: &'a mut [u8],
            #tail_editor_storage
        }

        impl<'a> #tail_editor_name<'a> {
            #(#tail_editor_methods)*
            #tail_editor_commit
        }
    })
}

pub fn looks_dynamic_account(item: &TokenStream) -> bool {
    let Ok(input) = parse2::<ItemStruct>(item.clone()) else {
        return false;
    };
    let Fields::Named(named) = input.fields else {
        return false;
    };
    named.named.iter().any(|field| {
        field.attrs.iter().any(|attr| attr.path().is_ident("tail"))
            || is_dynamic_authoring_type(&field.ty)
    })
}

fn strip_authoring_lifetime_generics(input: &mut ItemStruct) -> Result<()> {
    if input.generics.where_clause.is_some()
        || input
            .generics
            .params
            .iter()
            .any(|param| !matches!(param, GenericParam::Lifetime(_)))
    {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "#[hopper::dynamic_account] supports concrete structs plus authoring-only lifetimes used by String<'a, N>, Vec<'a, T, N>, TailStr<'a>, and TailBytes<'a>",
        ));
    }
    input.generics.params.clear();
    Ok(())
}

/// Emit a `Seq<T>`-tail dynamic account: the fixed head as a
/// `#[hopper::state(raw_tail)]` layout plus streaming-cursor accessors
/// over the `[count:u32][T; ..]` tail region, with capacity-free sizing.
#[allow(clippy::too_many_arguments)]
fn expand_seq_account(
    name: &Ident,
    vis: &Visibility,
    tail_fields: &[TailField],
    fixed_fields: &[TokenStream],
    fixed_getters: &[TokenStream],
    fixed_new_params: &[TokenStream],
    fixed_new_inits: &[TokenStream],
    outer_attrs: &[Attribute],
    disc: Option<&LitInt>,
    version: &LitInt,
    tail_schema_lit: &LitStr,
) -> Result<TokenStream> {
    // "One growable tail per account": a `Seq` framing (`[count][elems]`)
    // is incompatible with the bounded `[byte_len][payload]` framing, so
    // it must be the sole tail.
    if tail_fields.len() != 1 {
        return Err(syn::Error::new_spanned(
            name,
            "a `Seq<'a, T>` tail must be the ONLY tail field of its account \
             (one growable tail per account); move other dynamic fields into \
             the fixed head or a separate account",
        ));
    }
    let seq_field = &tail_fields[0];
    let TailKind::Seq { ty: elem_ty } = &seq_field.kind else {
        unreachable!("caller guarantees a Seq tail");
    };
    let seq_ident = seq_field.ident.clone();
    let seq_mut_ident = format_ident!("{}_mut", seq_ident);
    let seq_init_ident = format_ident!("{}_init", seq_ident);
    let seg_upper = seq_ident.to_string().to_uppercase();
    let offset_const = format_ident!("{}_OFFSET", seg_upper);
    let abs_offset_const = format_ident!("{}_ABS_OFFSET", seg_upper);
    let size_const = format_ident!("{}_SIZE", seg_upper);

    let mut state_args: Vec<TokenStream> = Vec::new();
    if let Some(disc) = disc {
        state_args.push(quote! { disc = #disc });
    }
    state_args.push(quote! { version = #version });
    // `raw_tail` gives us `HAS_DYNAMIC_TAIL` + `TAIL_PREFIX_OFFSET = LEN`
    // (the tail region start); the `seq<T>` schema string makes the layout
    // id capacity-independent.
    state_args.push(quote! { raw_tail = true });
    state_args.push(quote! { dynamic_tail_schema = #tail_schema_lit });

    Ok(quote! {
        // Seq elements must be fixed-stride `SeqElement`s; variable-length
        // encoders stay on the bounded `Vec<T, N>` owned-decode path.
        const _: () = {
            fn __hopper_assert_seq_element<__T: ::hopper::__runtime::SeqElement>() {}
            let _ = __hopper_assert_seq_element::<#elem_ty>;
        };

        #(#outer_attrs)*
        #[doc = "Hopper dynamic account with a growable `Seq<T>` tail."]
        #[doc = "The fixed head stays zero-copy; the tail is a `[u32 count][T; ..]`"]
        #[doc = "region of fixed-stride elements, grown via `realloc`."]
        #[doc = concat!("Tail schema: ", #tail_schema_lit)]
        #[derive(Clone, Copy)]
        #[repr(C)]
        #[hopper::state(#(#state_args),*)]
        #vis struct #name {
            #(#fixed_fields,)*
        }

        impl #name {
            /// Construct the fixed head. The `Seq` tail is written
            /// separately through the streaming cursor accessors.
            #[allow(clippy::too_many_arguments)]
            #[inline(always)]
            #vis fn new(#(#fixed_new_params),*) -> Self {
                Self {
                    #(#fixed_new_inits,)*
                }
            }

            #(#fixed_getters)*

            /// Minimum allocation: fixed head + the 4-byte `Seq` count
            /// prefix (an empty sequence). Grow past this via `space_for`.
            #vis const MIN_SPACE: usize = Self::LEN + ::hopper::__runtime::SEQ_LEN_PREFIX;

            /// Account allocation for a `Seq` of `count` elements: fixed
            /// head + `4 + count * STRIDE`.
            #[inline(always)]
            #vis const fn space_for(count: usize) -> usize {
                Self::LEN + ::hopper::__runtime::seq_region_bytes_for::<#elem_ty>(count)
            }

            /// Body-relative offset of the `Seq` tail region (its `u32`
            /// count prefix). A `#[hopper::context]` `tail(...)`
            /// declaration lowers to an open-ended write range at
            /// `HEADER_LEN + this`.
            #vis const #offset_const: u32 = Self::BODY_SIZE as u32;
            /// Absolute offset of the `Seq` tail region (== `TAIL_PREFIX_OFFSET`).
            #vis const #abs_offset_const: u32 = Self::TAIL_PREFIX_OFFSET as u32;
            /// Open-ended declared write size: the tail grows without
            /// bound, so its range is `u32::MAX` (an open tail — never a
            /// whole-account grant, since it starts past the head).
            #vis const #size_const: u32 = u32::MAX;

            /// Zero the `Seq` count prefix (empty sequence). A freshly
            /// allocated, zeroed account already reads empty; use this only
            /// to reset an existing tail.
            #[inline]
            #vis fn #seq_init_ident(
                data: &mut [u8],
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                let region = data
                    .get_mut(Self::TAIL_PREFIX_OFFSET..)
                    .ok_or(::hopper::__runtime::ProgramError::AccountDataTooSmall)?;
                if region.len() < ::hopper::__runtime::SEQ_LEN_PREFIX {
                    return ::core::result::Result::Err(
                        ::hopper::__runtime::ProgramError::AccountDataTooSmall,
                    );
                }
                region[..::hopper::__runtime::SEQ_LEN_PREFIX].copy_from_slice(&0u32.to_le_bytes());
                ::core::result::Result::Ok(())
            }

            /// Streaming READ cursor over the `Seq` tail (decodes one
            /// element per access; never materializes `[T; N]`).
            #[inline]
            #vis fn #seq_ident(
                data: &[u8],
            ) -> ::core::result::Result<
                ::hopper::__runtime::TailSeq<'_, #elem_ty>,
                ::hopper::__runtime::ProgramError,
            > {
                let region = data
                    .get(Self::TAIL_PREFIX_OFFSET..)
                    .ok_or(::hopper::__runtime::ProgramError::AccountDataTooSmall)?;
                ::hopper::__runtime::TailSeq::from_region(region)
            }

            /// Streaming WRITE cursor over the `Seq` tail (O(1) `push`,
            /// `set`, `swap_remove`; capacity from the live account length).
            #[inline]
            #vis fn #seq_mut_ident(
                data: &mut [u8],
            ) -> ::core::result::Result<
                ::hopper::__runtime::TailSeqMut<'_, #elem_ty>,
                ::hopper::__runtime::ProgramError,
            > {
                let region = data
                    .get_mut(Self::TAIL_PREFIX_OFFSET..)
                    .ok_or(::hopper::__runtime::ProgramError::AccountDataTooSmall)?;
                ::hopper::__runtime::TailSeqMut::from_region(region)
            }
        }
    })
}

fn parse_options(attr: TokenStream) -> Result<Options> {
    if attr.is_empty() {
        return Ok(Options::default());
    }

    let mut options = Options::default();
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("disc") || meta.path.is_ident("discriminator") {
            options.disc = Some(meta.value()?.parse()?);
            return Ok(());
        }
        if meta.path.is_ident("version") {
            options.version = Some(meta.value()?.parse()?);
            return Ok(());
        }
        if meta.path.is_ident("tail_policy") {
            let value: LitStr = meta.value()?.parse()?;
            if value.value() == "compact" {
                return Ok(());
            }
            return Err(meta.error("only tail_policy = \"compact\" is implemented; use explicit hopper::systems APIs for indexed or segmented tails"));
        }
        Err(meta.error("unsupported dynamic_account option; expected `disc = N`, `discriminator = N`, `version = N`, or `tail_policy = \"compact\"`"))
    });
    parser.parse2(attr)?;
    Ok(options)
}

fn parse_tail_attr(field: &Field) -> Result<Option<TailSpec>> {
    let mut out = None;
    for attr in &field.attrs {
        if attr.path().is_ident("tail") {
            if out.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "only one #[tail(...)] attribute is allowed per dynamic field",
                ));
            }
            out = Some(attr.parse_args::<TailSpec>()?);
        }
    }
    Ok(out)
}

fn strip_tail_attrs(attrs: Vec<Attribute>) -> Vec<Attribute> {
    attrs
        .into_iter()
        .filter(|attr| !attr.path().is_ident("tail"))
        .collect()
}

fn parse_pretty_tail_type(ty: &Type) -> Result<Option<TailKind>> {
    let Type::Path(type_path) = ty else {
        return Ok(None);
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Ok(None);
    };
    let ident = segment.ident.to_string();

    if ident == "TailStr" || ident == "TailBytes" {
        let args = angle_args(&segment.arguments, ty)?;
        if args.len() != 1 || !matches!(args.first(), Some(GenericArgument::Lifetime(_))) {
            return Err(syn::Error::new_spanned(
                ty,
                "bare final tails must be spelled `TailStr<'a>` or `TailBytes<'a>`",
            ));
        }
        return Ok(Some(if ident == "TailStr" {
            TailKind::TailStr
        } else {
            TailKind::TailBytes
        }));
    }

    if ident == "String" || ident == "Text" {
        let args = angle_args(&segment.arguments, ty)?;
        let (_, cap_index) = leading_lifetime_index(args);
        let cap = lit_int_arg(
            args,
            cap_index,
            ty,
            "String/Text expects `String<'a, N>` or `Text<N>`",
        )?;
        if args.len() != cap_index + 1 {
            return Err(syn::Error::new_spanned(
                ty,
                "String/Text dynamic fields expect one capacity after the optional lifetime, for example `String<'a, 32>`",
            ));
        }
        return Ok(Some(TailKind::String { cap }));
    }

    if ident == "Vec" || ident == "List" {
        let args = angle_args(&segment.arguments, ty)?;
        let (_, first_type_index) = leading_lifetime_index(args);
        let ty_arg = type_arg(
            args,
            first_type_index,
            ty,
            "Vec/List expects `Vec<'a, T, N>` or `List<T, N>`",
        )?;
        let cap = lit_int_arg(
            args,
            first_type_index + 1,
            ty,
            "Vec/List expects `Vec<'a, T, N>` or `List<T, N>`",
        )?;
        if args.len() != first_type_index + 2 {
            return Err(syn::Error::new_spanned(
                ty,
                "Vec/List dynamic fields expect an element type and capacity after the optional lifetime, for example `Vec<'a, Address, 10>`",
            ));
        }
        return Ok(Some(TailKind::Vec {
            borrowed_slice: is_address_type(&ty_arg),
            ty: ty_arg,
            cap,
        }));
    }

    if ident == "Seq" {
        // `Seq<'a, T>` or `Seq<T>` — a growable typed sequence, no capacity.
        let args = angle_args(&segment.arguments, ty)?;
        let (_, ty_index) = leading_lifetime_index(args);
        let ty_arg = type_arg(
            args,
            ty_index,
            ty,
            "Seq expects `Seq<'a, T>` (a single element type, no capacity)",
        )?;
        if args.len() != ty_index + 1 {
            return Err(syn::Error::new_spanned(
                ty,
                "Seq is capacity-free and takes exactly one element type after the optional lifetime, for example `Seq<'a, Address>`",
            ));
        }
        return Ok(Some(TailKind::Seq { ty: ty_arg }));
    }

    Ok(None)
}

fn angle_args<'a>(
    args: &'a PathArguments,
    ty: &Type,
) -> Result<&'a syn::punctuated::Punctuated<GenericArgument, Token![,]>> {
    match args {
        PathArguments::AngleBracketed(args) => Ok(&args.args),
        _ => Err(syn::Error::new_spanned(
            ty,
            "dynamic authoring fields must specify bounded generic arguments",
        )),
    }
}

fn leading_lifetime_index(
    args: &syn::punctuated::Punctuated<GenericArgument, Token![,]>,
) -> (bool, usize) {
    match args.first() {
        Some(GenericArgument::Lifetime(_)) => (true, 1),
        _ => (false, 0),
    }
}

fn lit_int_arg(
    args: &syn::punctuated::Punctuated<GenericArgument, Token![,]>,
    index: usize,
    ty: &Type,
    message: &str,
) -> Result<LitInt> {
    let Some(arg) = args.iter().nth(index) else {
        return Err(syn::Error::new_spanned(ty, message));
    };
    match arg {
        GenericArgument::Const(Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        })) => Ok(value.clone()),
        _ => Err(syn::Error::new_spanned(arg, message)),
    }
}

fn type_arg(
    args: &syn::punctuated::Punctuated<GenericArgument, Token![,]>,
    index: usize,
    ty: &Type,
    message: &str,
) -> Result<Type> {
    let Some(arg) = args.iter().nth(index) else {
        return Err(syn::Error::new_spanned(ty, message));
    };
    match arg {
        GenericArgument::Type(value) => Ok(value.clone()),
        _ => Err(syn::Error::new_spanned(arg, message)),
    }
}

fn is_dynamic_authoring_type(ty: &Type) -> bool {
    path_last_ident(ty)
        .map(|ident| {
            matches!(
                ident.as_str(),
                "String" | "Text" | "Vec" | "List" | "Seq" | "TailStr" | "TailBytes"
            )
        })
        .unwrap_or(false)
}

fn is_unbounded_dynamic_type(ty: &Type) -> bool {
    path_last_ident(ty)
        .map(|ident| matches!(ident.as_str(), "String" | "Text" | "Vec" | "List"))
        .unwrap_or(false)
}

fn is_address_type(ty: &Type) -> bool {
    path_last_ident(ty)
        .map(|ident| ident == "Address" || ident == "Pubkey")
        .unwrap_or(false)
}

fn path_last_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

fn fixed_wire_type(
    ty: &Type,
    field: &Ident,
) -> (TokenStream, TokenStream, TokenStream, TokenStream) {
    match path_last_ident(ty).as_deref() {
        Some("i16") => (
            quote! { ::hopper::prelude::WireI16 },
            quote! { i16 },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireI16::new(#field) },
        ),
        Some("i32") => (
            quote! { ::hopper::prelude::WireI32 },
            quote! { i32 },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireI32::new(#field) },
        ),
        Some("i64") => (
            quote! { ::hopper::prelude::WireI64 },
            quote! { i64 },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireI64::new(#field) },
        ),
        Some("i128") => (
            quote! { ::hopper::prelude::WireI128 },
            quote! { i128 },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireI128::new(#field) },
        ),
        Some("u16") => (
            quote! { ::hopper::prelude::WireU16 },
            quote! { u16 },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireU16::new(#field) },
        ),
        Some("u32") => (
            quote! { ::hopper::prelude::WireU32 },
            quote! { u32 },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireU32::new(#field) },
        ),
        Some("u64") => (
            quote! { ::hopper::prelude::WireU64 },
            quote! { u64 },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireU64::new(#field) },
        ),
        Some("u128") => (
            quote! { ::hopper::prelude::WireU128 },
            quote! { u128 },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireU128::new(#field) },
        ),
        Some("bool") => (
            quote! { ::hopper::prelude::WireBool },
            quote! { bool },
            quote! { self.#field.get() },
            quote! { ::hopper::prelude::WireBool::new(#field) },
        ),
        _ => (
            quote! { #ty },
            quote! { #ty },
            quote! { self.#field },
            quote! { #field },
        ),
    }
}

fn tail_struct_field(field: &TailField) -> TokenStream {
    let ident = &field.ident;
    match &field.kind {
        TailKind::String { cap } => quote! { #ident: string<#cap>, },
        TailKind::Vec { ty, cap, .. } => quote! { #ident: vec<#ty, #cap>, },
        // `Seq` tails never reach the owned-decode tail struct — they are
        // handled by `expand_seq_account`.
        TailKind::Seq { .. } | TailKind::TailStr | TailKind::TailBytes => TokenStream::new(),
    }
}

fn tail_value_field(field: &TailField) -> TokenStream {
    let ident = &field.ident;
    match &field.kind {
        TailKind::String { cap } => quote! { pub #ident: ::hopper::__runtime::HopperString<#cap>, },
        TailKind::Vec { ty, cap, .. } => {
            quote! { pub #ident: ::hopper::__runtime::HopperVec<#ty, #cap>, }
        }
        TailKind::TailStr => quote! { pub #ident: ::hopper::__runtime::TailStr<'a>, },
        TailKind::TailBytes => quote! { pub #ident: ::hopper::__runtime::TailBytes<'a>, },
        // `Seq` tails do not participate in the owned tail value struct.
        TailKind::Seq { .. } => TokenStream::new(),
    }
}

fn tail_element_assertion(field: &TailField) -> Option<TokenStream> {
    match &field.kind {
        TailKind::String { .. } => None,
        TailKind::Vec { ty, .. } => Some(quote! {
            const _: () = {
                fn __hopper_assert_tail_element<T: ::hopper::__runtime::TailElement>() {}
                let _ = __hopper_assert_tail_element::<#ty>;
            };
        }),
        // `Seq` elements are asserted `SeqElement` inside `expand_seq_account`.
        TailKind::Seq { .. } | TailKind::TailStr | TailKind::TailBytes => None,
    }
}

fn tail_schema(fields: &[TailField]) -> String {
    let mut out = String::new();
    for (idx, field) in fields.iter().enumerate() {
        if idx > 0 {
            out.push(';');
        }
        out.push_str(&field.ident.to_string());
        out.push(':');
        match &field.kind {
            TailKind::String { cap } => {
                out.push_str("string<");
                out.push_str(&cap.base10_digits().replace('_', ""));
                out.push('>');
            }
            TailKind::Vec { ty, cap, .. } => {
                out.push_str("vec<");
                out.push_str(&ty.to_token_stream().to_string().replace(' ', ""));
                out.push(',');
                out.push_str(&cap.base10_digits().replace('_', ""));
                out.push('>');
            }
            TailKind::Seq { ty } => {
                // NO capacity in the schema — the layout id stays
                // capacity-independent, so growing never changes the type.
                out.push_str("seq<");
                out.push_str(&ty.to_token_stream().to_string().replace(' ', ""));
                out.push('>');
            }
            TailKind::TailStr => out.push_str("tail_str"),
            TailKind::TailBytes => out.push_str("tail_bytes"),
        }
    }
    out
}

fn tail_skip_tokens(previous: &[TailField]) -> Vec<TokenStream> {
    previous
        .iter()
        .map(|field| match &field.kind {
            TailKind::String { cap } => quote! {
                let (_, __consumed) = ::hopper::__runtime::borrow_bounded_str::<#cap>(__cursor)?;
                __cursor = &__cursor[__consumed..];
            },
            TailKind::Vec {
                cap,
                borrowed_slice: true,
                ..
            } => quote! {
                let (_, __consumed) = ::hopper::__runtime::borrow_address_slice::<#cap>(__cursor)?;
                __cursor = &__cursor[__consumed..];
            },
            TailKind::Vec {
                ty,
                cap,
                borrowed_slice: false,
            } => quote! {
                let (_, __consumed) = <::hopper::__runtime::HopperVec<#ty, #cap> as ::hopper::__runtime::TailCodec>::decode(__cursor)?;
                __cursor = &__cursor[__consumed..];
            },
            // `Seq` is always the sole tail, so it never appears as a
            // `previous` field to skip over.
            TailKind::Seq { .. } | TailKind::TailStr | TailKind::TailBytes => TokenStream::new(),
        })
        .collect()
}

fn tail_view_method(field: &TailField, previous: &[TailField]) -> TokenStream {
    let ident = &field.ident;
    let skips = tail_skip_tokens(previous);
    match &field.kind {
        TailKind::String { cap } => quote! {
            #[inline]
            pub fn #ident(&self) -> ::core::result::Result<&'a str, ::hopper::__runtime::ProgramError> {
                let mut __cursor = self.payload;
                #(#skips)*
                let (__value, _) = ::hopper::__runtime::borrow_bounded_str::<#cap>(__cursor)?;
                Ok(__value)
            }
        },
        TailKind::Vec {
            ty,
            cap,
            borrowed_slice: true,
        } => quote! {
            #[inline]
            pub fn #ident(&self) -> ::core::result::Result<&'a [#ty], ::hopper::__runtime::ProgramError> {
                let mut __cursor = self.payload;
                #(#skips)*
                let (__value, _) = ::hopper::__runtime::borrow_address_slice::<#cap>(__cursor)?;
                Ok(__value)
            }
        },
        TailKind::Vec {
            ty,
            cap,
            borrowed_slice: false,
        } => quote! {
            #[inline]
            pub fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::HopperVec<#ty, #cap>, ::hopper::__runtime::ProgramError> {
                let mut __cursor = self.payload;
                #(#skips)*
                let (__value, _) = <::hopper::__runtime::HopperVec<#ty, #cap> as ::hopper::__runtime::TailCodec>::decode(__cursor)?;
                Ok(__value)
            }
        },
        TailKind::TailStr => quote! {
            #[inline]
            pub fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::TailStr<'a>, ::hopper::__runtime::ProgramError> {
                let mut __cursor = self.payload;
                #(#skips)*
                Ok(::hopper::__runtime::TailStr::new(__cursor))
            }
        },
        TailKind::TailBytes => quote! {
            #[inline]
            pub fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::TailBytes<'a>, ::hopper::__runtime::ProgramError> {
                let mut __cursor = self.payload;
                #(#skips)*
                Ok(::hopper::__runtime::TailBytes::new(__cursor))
            }
        },
        // `Seq` tails are handled by `expand_seq_account` and never reach
        // the owned tail-view surface.
        TailKind::Seq { .. } => TokenStream::new(),
    }
}

fn tail_editor_methods(
    field: &TailField,
    storage: &Ident,
    account_name: &Ident,
    has_raw_tail: bool,
) -> TokenStream {
    let ident = &field.ident;
    match &field.kind {
        TailKind::String { .. } => {
            let setter = format_ident!("set_{}", ident);
            quote! {
                #[inline]
                pub fn #ident(&self) -> ::core::result::Result<&str, ::hopper::__runtime::ProgramError> {
                    self.#storage.#ident.as_str()
                }

                #[inline]
                pub fn #setter(&mut self, value: &str) -> ::hopper::prelude::ProgramResult {
                    self.#storage.#ident.set_str(value)
                }
            }
        }
        TailKind::Vec { ty, .. } => {
            let singular = singular_ident(ident);
            let push = format_ident!("push_{}", singular);
            let push_unique = format_ident!("push_unique_{}", singular);
            let remove = format_ident!("remove_{}", singular);
            quote! {
                #[inline]
                pub fn #ident(&self) -> &[#ty] {
                    self.#storage.#ident.as_slice()
                }

                #[inline]
                pub fn #push(&mut self, value: #ty) -> ::hopper::prelude::ProgramResult {
                    self.#storage.#ident.push(value)
                }

                #[inline]
                pub fn #push_unique(&mut self, value: #ty) -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError> {
                    self.#storage.#ident.push_unique(value)
                }

                #[inline]
                pub fn #remove(&mut self, value: &#ty) -> bool {
                    self.#storage.#ident.remove_first(value)
                }
            }
        }
        TailKind::TailStr => {
            let setter = format_ident!("set_{}", ident);
            let _ = has_raw_tail;
            quote! {
                #[inline]
                pub fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::TailStr<'_>, ::hopper::__runtime::ProgramError> {
                    let __view = #account_name::tail_view(self.data)?;
                    __view.#ident()
                }

                #[inline]
                pub fn #setter(&mut self, value: &str) -> ::hopper::prelude::ProgramResult {
                    #account_name::tail_write_parts(self.data, &self.#storage, value.as_bytes()).map(|_| ())
                }
            }
        }
        TailKind::TailBytes => {
            let setter = format_ident!("set_{}", ident);
            let _ = has_raw_tail;
            quote! {
                #[inline]
                pub fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::TailBytes<'_>, ::hopper::__runtime::ProgramError> {
                    let __view = #account_name::tail_view(self.data)?;
                    __view.#ident()
                }

                #[inline]
                pub fn #setter(&mut self, value: &[u8]) -> ::hopper::prelude::ProgramResult {
                    #account_name::tail_write_parts(self.data, &self.#storage, value).map(|_| ())
                }
            }
        }
        // `Seq` tails are handled by `expand_seq_account`; no owned editor.
        TailKind::Seq { .. } => TokenStream::new(),
    }
}

fn account_tail_methods(field: &TailField, vis: &Visibility, has_raw_tail: bool) -> TokenStream {
    let ident = &field.ident;
    match &field.kind {
        TailKind::String { .. } => {
            let setter = format_ident!("set_{}", ident);
            let setter_method = if has_raw_tail {
                TokenStream::new()
            } else {
                quote! {
                    #[inline]
                    #vis fn #setter(data: &mut [u8], value: &str) -> ::hopper::prelude::ProgramResult {
                        let mut __editor = Self::tail_editor(data)?;
                        __editor.#setter(value)?;
                        __editor.commit()
                    }
                }
            };
            quote! {
                #[inline]
                #vis fn #ident<'a>(data: &'a [u8]) -> ::core::result::Result<&'a str, ::hopper::__runtime::ProgramError> {
                    let __view = Self::tail_view(data)?;
                    __view.#ident()
                }

                #setter_method
            }
        }
        TailKind::Vec {
            ty,
            cap,
            borrowed_slice,
        } => {
            let singular = singular_ident(ident);
            let push = format_ident!("push_{}", singular);
            let push_unique = format_ident!("push_unique_{}", singular);
            let remove = format_ident!("remove_{}", singular);
            let getter_return = if *borrowed_slice {
                quote! { &'a [#ty] }
            } else {
                quote! { ::hopper::__runtime::HopperVec<#ty, #cap> }
            };
            let mutation_methods = if has_raw_tail {
                TokenStream::new()
            } else {
                quote! {
                    #[inline]
                    #vis fn #push(data: &mut [u8], value: #ty) -> ::hopper::prelude::ProgramResult {
                        let mut __editor = Self::tail_editor(data)?;
                        __editor.#push(value)?;
                        __editor.commit()
                    }

                    #[inline]
                    #vis fn #push_unique(data: &mut [u8], value: #ty) -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError> {
                        let mut __editor = Self::tail_editor(data)?;
                        let __inserted = __editor.#push_unique(value)?;
                        __editor.commit()?;
                        Ok(__inserted)
                    }

                    #[inline]
                    #vis fn #remove(data: &mut [u8], value: &#ty) -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError> {
                        let mut __editor = Self::tail_editor(data)?;
                        let __removed = __editor.#remove(value);
                        __editor.commit()?;
                        Ok(__removed)
                    }
                }
            };
            quote! {
                #[inline]
                #vis fn #ident<'a>(data: &'a [u8]) -> ::core::result::Result<#getter_return, ::hopper::__runtime::ProgramError> {
                    let __view = Self::tail_view(data)?;
                    __view.#ident()
                }

                #mutation_methods
            }
        }
        TailKind::TailStr => {
            let setter = format_ident!("set_{}", ident);
            quote! {
                #[inline]
                #vis fn #ident<'a>(data: &'a [u8]) -> ::core::result::Result<::hopper::__runtime::TailStr<'a>, ::hopper::__runtime::ProgramError> {
                    let __view = Self::tail_view(data)?;
                    __view.#ident()
                }

                #[inline]
                #vis fn #setter(data: &mut [u8], value: &str) -> ::hopper::prelude::ProgramResult {
                    let mut __editor = Self::tail_editor(data)?;
                    __editor.#setter(value)
                }
            }
        }
        TailKind::TailBytes => {
            let setter = format_ident!("set_{}", ident);
            quote! {
                #[inline]
                #vis fn #ident<'a>(data: &'a [u8]) -> ::core::result::Result<::hopper::__runtime::TailBytes<'a>, ::hopper::__runtime::ProgramError> {
                    let __view = Self::tail_view(data)?;
                    __view.#ident()
                }

                #[inline]
                #vis fn #setter(data: &mut [u8], value: &[u8]) -> ::hopper::prelude::ProgramResult {
                    let mut __editor = Self::tail_editor(data)?;
                    __editor.#setter(value)
                }
            }
        }
        // `Seq` tails emit their own accessors in `expand_seq_account`.
        TailKind::Seq { .. } => TokenStream::new(),
    }
}

fn account_wrapper_trait_methods(field: &TailField) -> TokenStream {
    let ident = &field.ident;
    match &field.kind {
        TailKind::String { cap } => {
            let setter = format_ident!("set_{}", ident);
            quote! {
                fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::HopperString<#cap>, ::hopper::__runtime::ProgramError>;
                fn #setter(&self, value: &str) -> ::hopper::prelude::ProgramResult;
            }
        }
        TailKind::Vec { ty, cap, .. } => {
            let singular = singular_ident(ident);
            let push = format_ident!("push_{}", singular);
            let push_unique = format_ident!("push_unique_{}", singular);
            let remove = format_ident!("remove_{}", singular);
            quote! {
                fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::HopperVec<#ty, #cap>, ::hopper::__runtime::ProgramError>;
                fn #push(&self, value: #ty) -> ::hopper::prelude::ProgramResult;
                fn #push_unique(&self, value: #ty) -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError>;
                fn #remove(&self, value: &#ty) -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError>;
            }
        }
        TailKind::Seq { .. } | TailKind::TailStr | TailKind::TailBytes => TokenStream::new(),
    }
}

fn account_wrapper_impl_methods(
    field: &TailField,
    account_name: &Ident,
    tail_name: &Ident,
) -> TokenStream {
    let ident = &field.ident;
    match &field.kind {
        TailKind::String { cap } => {
            let setter = format_ident!("set_{}", ident);
            quote! {
                #[inline]
                fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::HopperString<#cap>, ::hopper::__runtime::ProgramError> {
                    let __tail: #tail_name = self.tail_read()?;
                    Ok(__tail.#ident)
                }

                #[inline]
                fn #setter(&self, value: &str) -> ::hopper::prelude::ProgramResult {
                    let mut __data = self.as_account().try_borrow_mut()?;
                    let mut __editor = #account_name::tail_editor(&mut __data)?;
                    __editor.#setter(value)?;
                    __editor.commit()
                }
            }
        }
        TailKind::Vec { ty, cap, .. } => {
            let singular = singular_ident(ident);
            let push = format_ident!("push_{}", singular);
            let push_unique = format_ident!("push_unique_{}", singular);
            let remove = format_ident!("remove_{}", singular);
            quote! {
                #[inline]
                fn #ident(&self) -> ::core::result::Result<::hopper::__runtime::HopperVec<#ty, #cap>, ::hopper::__runtime::ProgramError> {
                    let __tail: #tail_name = self.tail_read()?;
                    Ok(__tail.#ident)
                }

                #[inline]
                fn #push(&self, value: #ty) -> ::hopper::prelude::ProgramResult {
                    let mut __data = self.as_account().try_borrow_mut()?;
                    let mut __editor = #account_name::tail_editor(&mut __data)?;
                    __editor.#push(value)?;
                    __editor.commit()
                }

                #[inline]
                fn #push_unique(&self, value: #ty) -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError> {
                    let mut __data = self.as_account().try_borrow_mut()?;
                    let mut __editor = #account_name::tail_editor(&mut __data)?;
                    let __inserted = __editor.#push_unique(value)?;
                    __editor.commit()?;
                    Ok(__inserted)
                }

                #[inline]
                fn #remove(&self, value: &#ty) -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError> {
                    let mut __data = self.as_account().try_borrow_mut()?;
                    let mut __editor = #account_name::tail_editor(&mut __data)?;
                    let __removed = __editor.#remove(value);
                    __editor.commit()?;
                    Ok(__removed)
                }
            }
        }
        TailKind::Seq { .. } | TailKind::TailStr | TailKind::TailBytes => TokenStream::new(),
    }
}

fn singular_ident(ident: &Ident) -> Ident {
    let name = ident.to_string();
    let singular = name.strip_suffix('s').unwrap_or(&name);
    format_ident!("{}", singular)
}
