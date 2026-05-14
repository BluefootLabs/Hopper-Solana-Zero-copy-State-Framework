//! `#[hopper::dynamic_account]`.
//!
//! Quasar-style authoring surface for bounded dynamic fields. The macro keeps
//! Hopper's wire truth unchanged: a fixed zero-copy body plus one compact
//! dynamic tail encoded after the body as `[u32 len][payload]`.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::parse::{Parse, ParseStream, Parser};
use syn::{
    parse2, Attribute, Field, Fields, Ident, ItemStruct, LitInt, LitStr, Result, Token, Type,
    Visibility,
};

#[derive(Default)]
struct Options {
    disc: Option<LitInt>,
    version: Option<LitInt>,
}

#[derive(Clone)]
enum TailKind {
    String { cap: LitInt },
    VecAddress { ty: Type, cap: LitInt },
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

            if !is_address_type(&ty) {
                return Err(syn::Error::new_spanned(
                    ty,
                    "#[tail(vec<T, N>)] currently supports `Address` / `Pubkey` for borrowed tail views; use hopper_dynamic_fields! for other TailCodec element types",
                ));
            }

            return Ok(Self {
                kind: TailKind::VecAddress { ty, cap },
            });
        }

        Err(syn::Error::new_spanned(
            keyword,
            "unsupported #[tail(...)] spec; expected `string<N>` or `vec<Address, N>`",
        ))
    }
}

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let options = parse_options(attr)?;
    let input: ItemStruct = parse2(item)?;
    let name = input.ident.clone();
    let vis = input.vis.clone();
    let tail_name = format_ident!("{}Tail", name);
    let tail_view_name = format_ident!("{}TailView", name);
    let tail_editor_name = format_ident!("{}TailEditor", name);

    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "#[hopper::dynamic_account] currently supports concrete account structs only",
        ));
    }

    let named = match input.fields {
        Fields::Named(named) => named.named,
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "#[hopper::dynamic_account] requires a named-field struct",
            ));
        }
    };

    let mut fixed_fields = Vec::new();
    let mut fixed_getters = Vec::new();
    let mut fixed_new_params = Vec::new();
    let mut fixed_new_inits = Vec::new();
    let mut tail_fields = Vec::new();

    for mut field in named {
        let Some(ident) = field.ident.clone() else {
            continue;
        };
        let tail_spec = parse_tail_attr(&field)?;
        field.attrs = strip_tail_attrs(field.attrs);

        if let Some(spec) = tail_spec {
            tail_fields.push(TailField {
                ident,
                kind: spec.kind,
            });
            continue;
        }

        if is_unbounded_dynamic_type(&field.ty) {
            return Err(syn::Error::new_spanned(
                field.ty,
                "String and Vec fields in #[hopper::dynamic_account] must be annotated with #[tail(string<N>)] or #[tail(vec<Address, N>)]",
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

    let tail_struct_fields: Vec<_> = tail_fields
        .iter()
        .map(|field| tail_struct_field(field))
        .collect();
    let tail_schema = tail_schema(&tail_fields);
    let tail_schema_lit = LitStr::new(&tail_schema, Span::call_site());
    let version = options
        .version
        .unwrap_or_else(|| LitInt::new("1", Span::call_site()));

    let mut state_args = Vec::new();
    if let Some(disc) = options.disc {
        state_args.push(quote! { disc = #disc });
    }
    state_args.push(quote! { version = #version });
    state_args.push(quote! { dynamic_tail = #tail_name });
    state_args.push(quote! { dynamic_tail_schema = #tail_schema_lit });

    let outer_attrs: Vec<Attribute> = input
        .attrs
        .into_iter()
        .filter(|attr| !attr.path().is_ident("derive") && !attr.path().is_ident("repr"))
        .collect();

    let tail_view_methods: Vec<_> = tail_fields
        .iter()
        .enumerate()
        .map(|(idx, field)| tail_view_method(field, &tail_fields[..idx]))
        .collect();
    let tail_editor_methods: Vec<_> = tail_fields.iter().map(tail_editor_methods).collect();
    let account_tail_methods: Vec<_> = tail_fields
        .iter()
        .map(|field| account_tail_methods(field, &vis))
        .collect();

    Ok(quote! {
        ::hopper::hopper_dynamic_fields! {
            #vis struct #tail_name {
                #(#tail_struct_fields)*
            }
        }

        #(#outer_attrs)*
        #[derive(Clone, Copy)]
        #[repr(C)]
        #[hopper::state(#(#state_args),*)]
        #vis struct #name {
            #(#fixed_fields,)*
        }

        impl #name {
            /// Maximum encoded bytes for this account's compact dynamic tail.
            #vis const TAIL_MAX_ENCODED_LEN: usize =
                <#tail_name as ::hopper::__runtime::TailCodec>::MAX_ENCODED_LEN;

            /// Maximum account allocation for body + tail prefix + full tail.
            #vis const ALLOC_SPACE: usize = Self::INIT_SPACE + 4 + Self::TAIL_MAX_ENCODED_LEN;

            /// Construct the fixed account body. Dynamic fields are written
            /// separately with `tail_write`, `tail_editor`, or generated setters.
            #[inline(always)]
            #vis fn new(#(#fixed_new_params),*) -> Self {
                Self {
                    #(#fixed_new_inits,)*
                }
            }

            #(#fixed_getters)*

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

        #vis struct #tail_view_name<'a> {
            payload: &'a [u8],
        }

        impl<'a> #tail_view_name<'a> {
            #(#tail_view_methods)*
        }

        #vis struct #tail_editor_name<'a> {
            data: &'a mut [u8],
            tail: #tail_name,
        }

        impl<'a> #tail_editor_name<'a> {
            #(#tail_editor_methods)*

            /// Write the edited tail back into the account bytes.
            #[inline]
            #vis fn commit(self) -> ::hopper::prelude::ProgramResult {
                #name::tail_write(self.data, &self.tail).map(|_| ())
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

fn is_unbounded_dynamic_type(ty: &Type) -> bool {
    path_last_ident(ty)
        .map(|ident| ident == "String" || ident == "Vec")
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
        TailKind::VecAddress { ty, cap } => quote! { #ident: vec<#ty, #cap>, },
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
            TailKind::VecAddress { ty, cap } => {
                out.push_str("vec<");
                out.push_str(&ty.to_token_stream().to_string().replace(' ', ""));
                out.push(',');
                out.push_str(&cap.base10_digits().replace('_', ""));
                out.push('>');
            }
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
            TailKind::VecAddress { cap, .. } => quote! {
                let (_, __consumed) = ::hopper::__runtime::borrow_address_slice::<#cap>(__cursor)?;
                __cursor = &__cursor[__consumed..];
            },
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
        TailKind::VecAddress { ty, cap } => quote! {
            #[inline]
            pub fn #ident(&self) -> ::core::result::Result<&'a [#ty], ::hopper::__runtime::ProgramError> {
                let mut __cursor = self.payload;
                #(#skips)*
                let (__value, _) = ::hopper::__runtime::borrow_address_slice::<#cap>(__cursor)?;
                Ok(__value)
            }
        },
    }
}

fn tail_editor_methods(field: &TailField) -> TokenStream {
    let ident = &field.ident;
    match &field.kind {
        TailKind::String { .. } => {
            let setter = format_ident!("set_{}", ident);
            quote! {
                #[inline]
                pub fn #ident(&self) -> ::core::result::Result<&str, ::hopper::__runtime::ProgramError> {
                    self.tail.#ident.as_str()
                }

                #[inline]
                pub fn #setter(&mut self, value: &str) -> ::hopper::prelude::ProgramResult {
                    self.tail.#ident.set_str(value)
                }
            }
        }
        TailKind::VecAddress { ty, .. } => {
            let singular = singular_ident(ident);
            let push = format_ident!("push_{}", singular);
            let push_unique = format_ident!("push_unique_{}", singular);
            let remove = format_ident!("remove_{}", singular);
            quote! {
                #[inline]
                pub fn #ident(&self) -> &[#ty] {
                    self.tail.#ident.as_slice()
                }

                #[inline]
                pub fn #push(&mut self, value: #ty) -> ::hopper::prelude::ProgramResult {
                    self.tail.#ident.push(value)
                }

                #[inline]
                pub fn #push_unique(&mut self, value: #ty) -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError> {
                    self.tail.#ident.push_unique(value)
                }

                #[inline]
                pub fn #remove(&mut self, value: &#ty) -> bool {
                    self.tail.#ident.remove_first(value)
                }
            }
        }
    }
}

fn account_tail_methods(field: &TailField, vis: &Visibility) -> TokenStream {
    let ident = &field.ident;
    match &field.kind {
        TailKind::String { .. } => {
            let setter = format_ident!("set_{}", ident);
            quote! {
                #[inline]
                #vis fn #ident<'a>(data: &'a [u8]) -> ::core::result::Result<&'a str, ::hopper::__runtime::ProgramError> {
                    let __view = Self::tail_view(data)?;
                    __view.#ident()
                }

                #[inline]
                #vis fn #setter(data: &mut [u8], value: &str) -> ::hopper::prelude::ProgramResult {
                    let mut __editor = Self::tail_editor(data)?;
                    __editor.#setter(value)?;
                    __editor.commit()
                }
            }
        }
        TailKind::VecAddress { ty, .. } => {
            let singular = singular_ident(ident);
            let push = format_ident!("push_{}", singular);
            let push_unique = format_ident!("push_unique_{}", singular);
            let remove = format_ident!("remove_{}", singular);
            quote! {
                #[inline]
                #vis fn #ident<'a>(data: &'a [u8]) -> ::core::result::Result<&'a [#ty], ::hopper::__runtime::ProgramError> {
                    let __view = Self::tail_view(data)?;
                    __view.#ident()
                }

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
        }
    }
}

fn singular_ident(ident: &Ident) -> Ident {
    let name = ident.to_string();
    let singular = name.strip_suffix('s').unwrap_or(&name);
    format_ident!("{}", singular)
}
