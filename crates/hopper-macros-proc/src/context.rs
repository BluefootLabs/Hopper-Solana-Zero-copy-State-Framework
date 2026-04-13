//! `#[hopper_context]` — typed context accessor codegen.
//!
//! Parses context structs with `#[account(...)]` annotations and generates:
//! - A typed binder over `hopper_runtime::Context`
//! - Per-field account accessors (`vault_account()`, `vault_load()`, etc.)
//! - Per-field segment accessors (`vault_balance_mut()`, etc.)
//! - Up-front signer, writable, owner, and layout validation
//! - Receipt scopes derived from the same mutable segment metadata
//!
//! All generated accessors are `#[inline(always)]` with const segment offsets.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::Parse,
    parse2, punctuated::Punctuated, token::Comma, Attribute, Fields,
    Ident, ItemStruct, Result, Token, Type, TypePath,
};

/// Parsed `#[account(...)]` attribute.
struct AccountAttr {
    /// Whether the account is a signer.
    is_signer: bool,
    /// Whether the entire account is mutable.
    is_mut: bool,
    /// Specific mutable segment names (from `mut(field1, field2)`).
    mut_segments: Vec<String>,
    /// Specific read-only segment names (from `read(field1, field2)`).
    read_segments: Vec<String>,
}

impl Default for AccountAttr {
    fn default() -> Self {
        Self {
            is_signer: false,
            is_mut: false,
            mut_segments: Vec::new(),
            read_segments: Vec::new(),
        }
    }
}

/// Parsed context field.
struct ContextField {
    name: Ident,
    ty: Type,
    attr: AccountAttr,
    index: usize,
}

pub fn expand(_attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let mut input: ItemStruct = parse2(item)?;
    let name = &input.ident;
    let vis = &input.vis;
    let bound_name = format_ident!("{}Ctx", name);
    let receipt_scope_name = format_ident!("{}ReceiptScope", name);

    let fields = match &mut input.fields {
        Fields::Named(f) => &mut f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &input,
                "hopper_context requires a struct with named fields",
            ))
        }
    };

    let mut ctx_fields = Vec::new();
    for (i, field) in fields.iter_mut().enumerate() {
        let field_name = field.ident.as_ref().unwrap().clone();
        let field_ty = field.ty.clone();
        let attr = parse_account_attr(&field.attrs)?;
        if (!attr.mut_segments.is_empty() || !attr.read_segments.is_empty())
            && skips_layout_validation(&field_ty)
        {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "segment accessors require a Hopper layout type, not a raw account view",
            ));
        }
        field.attrs.retain(|attr| {
            !attr.path().is_ident("account") && !attr.path().is_ident("signer")
        });
        ctx_fields.push(ContextField {
            name: field_name,
            ty: field_ty,
            attr,
            index: i,
        });
    }

    // Generate per-field validation functions and collect check descriptions.
    let mut validation_stmts = Vec::new();
    let mut per_field_validators = Vec::new();
    let mut check_descriptions: Vec<String> = Vec::new();

    for cf in &ctx_fields {
        let idx = cf.index;
        let field_name = &cf.name;
        let validate_fn = format_ident!("validate_{}", field_name);
        let mut field_checks = Vec::new();

        if cf.attr.is_signer {
            field_checks.push(quote! {
                ctx.account(#idx)?.check_signer()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be a signer",
                idx, field_name
            ));
        }
        if cf.attr.is_mut || !cf.attr.mut_segments.is_empty() {
            field_checks.push(quote! {
                ctx.account(#idx)?.check_writable()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be writable",
                idx, field_name
            ));
        }
        if !skips_layout_validation(&cf.ty) {
            let field_ty = &cf.ty;
            field_checks.push(quote! {
                ctx.account(#idx)?.check_owned_by(ctx.program_id())?;
                let _ = ctx.account(#idx)?.load::<#field_ty>()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) owned by program, valid {} header",
                idx,
                field_name,
                type_ident(&cf.ty).map(|i| i.to_string()).unwrap_or_default()
            ));
        }

        if !field_checks.is_empty() {
            per_field_validators.push(quote! {
                /// Validate the `#field_name` account (index #idx).
                #[inline(always)]
                #vis fn #validate_fn(ctx: &::hopper::prelude::Context<'_>) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    #(#field_checks)*
                    Ok(())
                }
            });

            // Collect into monolithic validate() for backward compat
            validation_stmts.push(quote! {
                Self::#validate_fn(ctx)?;
            });
        }
    }

    let check_desc_literals: Vec<_> = check_descriptions
        .iter()
        .map(|s| quote! { #s })
        .collect();
    let check_count = check_descriptions.len();

    // Generate segment accessor methods with const segment bindings.
    let mut accessors = Vec::new();

    for cf in &ctx_fields {
        let field_name = &cf.name;
        let field_ty = &cf.ty;
        let idx = cf.index;
        let type_ident = type_ident(field_ty)?;
        let type_upper = to_screaming_snake(&type_ident.to_string());

        let account_fn = format_ident!("{}_account", field_name);
        accessors.push(quote! {
            /// Return the underlying Hopper account view for `#field_name`.
            #[inline(always)]
            #vis fn #account_fn(
                &self,
            ) -> ::core::result::Result<
                &::hopper::prelude::AccountView,
                ::hopper::__runtime::ProgramError,
            > {
                self.ctx.account(#idx)
            }
        });

        if !skips_layout_validation(field_ty) {
            let load_fn = format_ident!("{}_load", field_name);
            let raw_ref_fn = format_ident!("{}_raw_ref", field_name);

            accessors.push(quote! {
                /// Validate and load the full typed layout for `#field_name`.
                #[inline(always)]
                #vis fn #load_fn(
                    &self,
                ) -> ::core::result::Result<
                    ::hopper::__runtime::Ref<'_, #field_ty>,
                    ::hopper::__runtime::ProgramError,
                > {
                    self.ctx.account(#idx)?.load::<#field_ty>()
                }
            });

            accessors.push(quote! {
                /// Explicit raw typed read of the full buffer for `#field_name`.
                #[inline(always)]
                #vis fn #raw_ref_fn(
                    &self,
                ) -> ::core::result::Result<
                    ::hopper::__runtime::Ref<'_, #field_ty>,
                    ::hopper::__runtime::ProgramError,
                > {
                    unsafe { self.ctx.account(#idx)?.raw_ref::<#field_ty>() }
                }
            });

            if cf.attr.is_mut {
                let load_mut_fn = format_ident!("{}_load_mut", field_name);
                let raw_mut_fn = format_ident!("{}_raw_mut", field_name);

                accessors.push(quote! {
                    /// Validate and mutably load the full typed layout for `#field_name`.
                    #[inline(always)]
                    #vis fn #load_mut_fn(
                        &self,
                    ) -> ::core::result::Result<
                        ::hopper::__runtime::RefMut<'_, #field_ty>,
                        ::hopper::__runtime::ProgramError,
                    > {
                        self.ctx.account(#idx)?.load_mut::<#field_ty>()
                    }
                });

                accessors.push(quote! {
                    /// Explicit raw typed write of the full buffer for `#field_name`.
                    #[inline(always)]
                    #vis fn #raw_mut_fn(
                        &self,
                    ) -> ::core::result::Result<
                        ::hopper::__runtime::RefMut<'_, #field_ty>,
                        ::hopper::__runtime::ProgramError,
                    > {
                        unsafe { self.ctx.account(#idx)?.raw_mut::<#field_ty>() }
                    }
                });
            }
        }

        // Generate mutable segment accessors.
        for seg_name in &cf.attr.mut_segments {
            let fn_name = format_ident!("{}_{}_mut", field_name, seg_name);
            let seg_upper = to_screaming_snake(seg_name);
            let offset_const = format_ident!("{}_{}_OFFSET", type_upper, seg_upper);
            let size_const = format_ident!("{}_{}_SIZE", type_upper, seg_upper);
            let type_alias = format_ident!("{}_{}_TYPE", type_upper, seg_upper);

            accessors.push(quote! {
                /// Mutable access to the `#seg_name` segment of `#field_name`.
                #[inline(always)]
                #vis fn #fn_name(
                    &mut self,
                ) -> ::core::result::Result<
                    ::hopper::__runtime::RefMut<'_, #type_alias>,
                    ::hopper::__runtime::ProgramError,
                > {
                    const SEG: ::hopper::prelude::StaticSegment =
                        ::hopper::prelude::StaticSegment::new(#seg_name, #offset_const, #size_const);
                    let abs_offset = ::hopper::prelude::HEADER_LEN as u32 + SEG.offset;
                    self.ctx.segment_mut::<#type_alias>(#idx, abs_offset)
                }
            });
        }

        // Generate read-only segment accessors.
        for seg_name in &cf.attr.read_segments {
            let fn_name = format_ident!("{}_{}_ref", field_name, seg_name);
            let seg_upper = to_screaming_snake(seg_name);
            let offset_const = format_ident!("{}_{}_OFFSET", type_upper, seg_upper);
            let size_const = format_ident!("{}_{}_SIZE", type_upper, seg_upper);
            let type_alias = format_ident!("{}_{}_TYPE", type_upper, seg_upper);

            accessors.push(quote! {
                /// Read-only access to the `#seg_name` segment of `#field_name`.
                #[inline(always)]
                #vis fn #fn_name(
                    &mut self,
                ) -> ::core::result::Result<
                    ::hopper::__runtime::Ref<'_, #type_alias>,
                    ::hopper::__runtime::ProgramError,
                > {
                    const SEG: ::hopper::prelude::StaticSegment =
                        ::hopper::prelude::StaticSegment::new(#seg_name, #offset_const, #size_const);
                    let abs_offset = ::hopper::prelude::HEADER_LEN as u32 + SEG.offset;
                    self.ctx.segment_ref::<#type_alias>(#idx, abs_offset)
                }
            });
        }
    }

    let mut receipt_scope_fields = Vec::new();
    let mut receipt_begin_inits = Vec::new();
    let mut receipt_finish_blocks = Vec::new();

    for cf in &ctx_fields {
        if skips_layout_validation(&cf.ty) {
            continue;
        }
        if !(cf.attr.is_mut || !cf.attr.mut_segments.is_empty()) {
            continue;
        }

        let field_name = &cf.name;
        let field_ty = &cf.ty;
        let idx = cf.index;
        let receipt_field_name = format_ident!("{}_receipt", field_name);
        let layout_ident = type_ident(field_ty)?;

        receipt_scope_fields.push(quote! {
            #receipt_field_name: ::hopper::prelude::StateReceipt<SNAP>,
        });

        receipt_begin_inits.push(quote! {
            #receipt_field_name: {
                let account = ctx.account(#idx)?;
                let data = account.try_borrow()?;
                ::hopper::prelude::StateReceipt::<SNAP>::begin(
                    &<#field_ty as ::hopper::hopper_runtime::LayoutContract>::LAYOUT_ID,
                    &data,
                )
            }
        });

        let segment_pairs: Vec<_> = if cf.attr.mut_segments.is_empty() {
            vec![quote! {
                (
                    ::hopper::hopper_core::account::HEADER_LEN,
                    <#field_ty as ::hopper::hopper_runtime::LayoutContract>::SIZE
                        - ::hopper::hopper_core::account::HEADER_LEN,
                )
            }]
        } else {
            let type_upper = to_screaming_snake(&layout_ident.to_string());
            cf.attr
                .mut_segments
                .iter()
                .map(|seg_name| {
                    let seg_upper = to_screaming_snake(seg_name);
                    let offset_const = format_ident!("{}_{}_OFFSET", type_upper, seg_upper);
                    let size_const = format_ident!("{}_{}_SIZE", type_upper, seg_upper);
                    quote! {
                        (
                            ::hopper::hopper_core::account::HEADER_LEN + #offset_const as usize,
                            #size_const as usize,
                        )
                    }
                })
                .collect()
        };

        receipt_finish_blocks.push(quote! {
            {
                let account = ctx.account(#idx)?;
                let data = account.try_borrow()?;
                self.#receipt_field_name.commit_with_segments(&data, &[#(#segment_pairs),*]);
                self.#receipt_field_name.set_invariants(invariants_passed, invariants_checked);
                ::hopper::prelude::emit_receipt(&self.#receipt_field_name.to_bytes())?;
            }
        });
    }

    let account_count = ctx_fields.len();
    let receipt_expected = !receipt_scope_fields.is_empty();
    let mutable_account_count = receipt_scope_fields.len();

    let expanded = quote! {
        // Emit the original struct unchanged.
        #input

        #vis struct #bound_name<'ctx, 'a> {
            ctx: &'ctx mut ::hopper::prelude::Context<'a>,
        }

        #vis struct #receipt_scope_name<const SNAP: usize> {
            #(#receipt_scope_fields)*
        }

        impl #name {
            /// Number of accounts this context requires.
            pub const ACCOUNT_COUNT: usize = #account_count;
            pub const RECEIPT_EXPECTED: bool = #receipt_expected;
            pub const MUTABLE_ACCOUNT_COUNT: usize = #mutable_account_count;

            /// Number of individual validation checks performed.
            pub const VALIDATION_CHECK_COUNT: usize = #check_count;

            /// Human-readable descriptions of every validation check.
            ///
            /// Inspect this constant (or use `hopper compile --emit rust`) to
            /// see exactly what `validate()` enforces — nothing is hidden.
            pub const VALIDATION_CHECKS: &'static [&'static str] = &[
                #(#check_desc_literals),*
            ];

            // ── Per-field validators ─────────────────────────────────
            //
            // Each field gets its own `validate_{name}()` so the checks
            // are individually callable, testable, and visible in
            // `hopper compile --emit rust` output.
            #(#per_field_validators)*

            /// Validate the account slice against this context spec.
            ///
            /// This calls each per-field validator in order. Every check
            /// is also available as a standalone `validate_{field}()` method
            /// for fine-grained control and testing.
            #[inline]
            pub fn validate(ctx: &::hopper::prelude::Context<'_>) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                ctx.require_accounts(Self::ACCOUNT_COUNT)?;
                #(#validation_stmts)*
                Ok(())
            }

            /// Bind a raw Hopper context into the typed proc-macro wrapper.
            #[inline]
            pub fn bind<'ctx, 'a>(
                ctx: &'ctx mut ::hopper::prelude::Context<'a>,
            ) -> ::core::result::Result<#bound_name<'ctx, 'a>, ::hopper::__runtime::ProgramError> {
                Self::validate(ctx)?;
                Ok(#bound_name { ctx })
            }

            #[inline]
            pub fn begin_receipt_scope<const SNAP: usize>(
                ctx: &::hopper::prelude::Context<'_>,
            ) -> ::core::result::Result<#receipt_scope_name<SNAP>, ::hopper::__runtime::ProgramError> {
                Ok(#receipt_scope_name {
                    #(#receipt_begin_inits),*
                })
            }
        }

        impl<const SNAP: usize> #receipt_scope_name<SNAP> {
            #[inline]
            #vis fn finish(
                mut self,
                ctx: &::hopper::prelude::Context<'_>,
                invariants_passed: bool,
                invariants_checked: u16,
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                #(#receipt_finish_blocks)*
                Ok(())
            }
        }

        impl<'ctx, 'a> #bound_name<'ctx, 'a> {
            #[inline(always)]
            #vis fn raw(&mut self) -> &mut ::hopper::prelude::Context<'a> {
                self.ctx
            }

            #[inline(always)]
            #vis fn program_id(&self) -> &::hopper::prelude::Address {
                self.ctx.program_id()
            }

            #[inline(always)]
            #vis fn instruction_data(&self) -> &[u8] {
                self.ctx.instruction_data()
            }

            #[inline(always)]
            #vis fn account(
                &self,
                index: usize,
            ) -> ::core::result::Result<
                &::hopper::prelude::AccountView,
                ::hopper::__runtime::ProgramError,
            > {
                self.ctx.account(index)
            }

            #[inline(always)]
            #vis fn account_mut(
                &self,
                index: usize,
            ) -> ::core::result::Result<
                &::hopper::prelude::AccountView,
                ::hopper::__runtime::ProgramError,
            > {
                self.ctx.account_mut(index)
            }

            #[inline(always)]
            #vis fn remaining_accounts(&self) -> &[::hopper::prelude::AccountView] {
                self.ctx.remaining_accounts(#account_count)
            }

            // --- Generated segment accessors ---
            #(#accessors)*
        }
    };

    Ok(expanded)
}

/// Parse `#[account(...)]` attributes from a field.
fn parse_account_attr(attrs: &[Attribute]) -> Result<AccountAttr> {
    let mut result = AccountAttr::default();

    for attr in attrs {
        if attr.path().is_ident("signer") {
            result.is_signer = true;
            continue;
        }

        if !attr.path().is_ident("account") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("signer") {
                result.is_signer = true;
                return Ok(());
            }

            if meta.path.is_ident("mut") {
                // Check for `mut(field1, field2)` or just `mut`
                if meta.input.peek(syn::token::Paren) {
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let segments: Punctuated<Ident, Comma> =
                        content.parse_terminated(Ident::parse, Token![,])?;
                    for seg in segments {
                        result.mut_segments.push(seg.to_string());
                    }
                } else {
                    result.is_mut = true;
                }
                return Ok(());
            }

            if meta.path.is_ident("signer") {
                result.is_signer = true;
                return Ok(());
            }

            if meta.path.is_ident("read") {
                if meta.input.peek(syn::token::Paren) {
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let segments: Punctuated<Ident, Comma> =
                        content.parse_terminated(Ident::parse, Token![,])?;
                    for seg in segments {
                        result.read_segments.push(seg.to_string());
                    }
                }
                return Ok(());
            }

            Err(meta.error("unrecognized account attribute"))
        })?;
    }

    Ok(result)
}

fn type_ident(ty: &Type) -> Result<Ident> {
    match ty {
        Type::Path(TypePath { path, .. }) => path
            .segments
            .last()
            .map(|segment| segment.ident.clone())
            .ok_or_else(|| syn::Error::new_spanned(ty, "expected a path type for account field")),
        _ => Err(syn::Error::new_spanned(
            ty,
            "hopper_context segment accessors require path types such as `Vault`",
        )),
    }
}

fn skips_layout_validation(ty: &Type) -> bool {
    match ty {
        Type::Path(TypePath { path, .. }) => path
            .segments
            .last()
            .map(|segment| matches!(segment.ident.to_string().as_str(), "AccountView" | "Signer" | "UncheckedAccount" | "ProgramRef"))
            .unwrap_or(false),
        _ => false,
    }
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
