//! `#[hopper_context]`. typed context accessor codegen.
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
    parse2, punctuated::Punctuated, token::Comma, Attribute, Expr, Fields,
    Ident, ItemStruct, Result, Token, Type, TypePath,
};

/// Parsed `#[account(...)]` attribute. the full Anchor-grade surface.
///
/// The first three groups (`is_signer`, `is_mut`, `mut_segments`,
/// `read_segments`) are the pre-Stage-2 Hopper baseline. The remainder
/// mirrors Anchor's `#[derive(Accounts)]` constraint set so programs
/// can lower declarative account validation and lifecycle (init,
/// realloc, close) through the same canonical path.
#[derive(Default)]
struct AccountAttr {
    /// Whether the account is a signer.
    is_signer: bool,
    /// Whether the entire account is mutable.
    is_mut: bool,
    /// Specific mutable segment names (from `mut(field1, field2)`).
    mut_segments: Vec<String>,
    /// Specific read-only segment names (from `read(field1, field2)`).
    read_segments: Vec<String>,

    // ── Anchor-grade declarative constraints (audit ST2) ────────────
    /// `init`. account must be created fresh this instruction.
    /// Requires `payer` and `space`; implies `mut`. PDA-init also
    /// requires `seeds` + `bump`.
    init: bool,
    /// `zero`. assert the account was previously zero-initialized.
    /// Cheaper than `init` for already-allocated accounts.
    zero: bool,
    /// `close = target`. at the end of the instruction, transfer the
    /// account's lamports to `target` and mark the data for reclaim.
    /// Implies `mut`.
    close: Option<Ident>,
    /// `realloc = new_size_expr`. resize account data before the
    /// instruction body. Requires `realloc_payer` and `realloc_zero`
    /// policy.
    realloc: Option<Expr>,
    /// Field that pays for realloc top-up lamports.
    realloc_payer: Option<Ident>,
    /// Whether realloc'd bytes must be zero-filled.
    realloc_zero: bool,
    /// `payer = field`. the field in this context struct that funds
    /// an `init` or `realloc` operation. Must itself be a signer.
    payer: Option<Ident>,
    /// `space = expr`. byte count for an `init`. Typically
    /// `Layout::LEN`.
    space: Option<Expr>,
    /// `seeds = [expr1, expr2, ...]`. PDA derivation input.
    seeds: Option<Vec<Expr>>,
    /// `bump` (inferred each call) or `bump = stored_byte`.
    bump: Option<BumpSpec>,
    /// `has_one = other_field`. require `self.field == other.key()`
    /// after layout load. Can appear multiple times.
    has_one: Vec<Ident>,
    /// `owner = expr`. require the account's owner equal `expr`.
    /// Default for layout fields is `ctx.program_id()`.
    owner: Option<Expr>,
    /// `address = expr`. require the account's key equal `expr`.
    address: Option<Expr>,
    /// `constraint = expr`. arbitrary boolean guard, evaluated as the
    /// last step of validation.
    constraint: Vec<Expr>,
}

/// How the bump for a PDA-derived account is supplied.
#[derive(Clone)]
#[allow(dead_code)]
enum BumpSpec {
    /// `bump`. re-derive via `find_program_address` each call.
    /// More expensive but removes the need to store the bump byte.
    Inferred,
    /// `bump = self.field_name.bump`. read the bump byte from a
    /// struct member, then use `create_program_address` for a cheap
    /// verification. Matches the on-chain-PDA cache pattern from
    /// `hopper_verify_pda!`.
    Stored(Expr),
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
        validate_account_attr(&field_name, &attr)?;
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

        // ── Audit page 12: deterministic validation ordering ──────────
        //
        // 1. presence (handled by `require_accounts` at top of validate())
        // 2. signer / mut / owner / executable / address
        // 3. duplicate-writable / signer rules
        // 4. PDA derivation
        // 5. init / realloc / close preconditions
        // 6. custom `constraint = expr`
        //
        // We accumulate checks into `field_checks` in that order so the
        // emitted error always points at the most specific reason first.

        // ── Audit Stage 2.3: wrapper-type auto-promotion ───────────────
        //
        // If the field type is a Hopper-owned wrapper
        // (`Signer<'info>`, `Account<'info, T>`,
        // `InitAccount<'info, T>`, `Program<'info, P>`), emit the
        // wrapper-specific checks first. Attribute-based constraints
        // layer on top of the wrapper-derived defaults. both paths
        // compose, neither overrides.
        let wrapper = classify_wrapper(&cf.ty);
        let wrapper_is_signer = matches!(wrapper, Some(WrapperKind::Signer));
        let wrapper_is_init = matches!(wrapper, Some(WrapperKind::InitAccount { .. }));
        // `wrapper.inner_layout()` is consumed by the has_layout /
        // layout_ty computation below. it resolves `Account<'info, T>`
        // → `T` so `load::<T>()` targets the right layout.
        if let Some(WrapperKind::Program) = &wrapper {
            // Program<'info, P>. require address == P::ID + executable.
            // P is the last type arg of the path.
            if let Type::Path(TypePath { path, .. }) = &cf.ty {
                if let Some(segment) = path.segments.last() {
                    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(program_ty) =
                            args.args.iter().find_map(|arg| {
                                if let syn::GenericArgument::Type(t) = arg {
                                    Some(t.clone())
                                } else {
                                    None
                                }
                            })
                        {
                            field_checks.push(quote! {
                                if ctx.account(#idx)?.address()
                                    != &<#program_ty as ::hopper::__runtime::ProgramId>::ID
                                {
                                    return ::core::result::Result::Err(
                                        ::hopper::__runtime::ProgramError::IncorrectProgramId
                                    );
                                }
                                if !ctx.account(#idx)?.executable() {
                                    return ::core::result::Result::Err(
                                        ::hopper::__runtime::ProgramError::InvalidAccountData
                                    );
                                }
                            });
                            check_descriptions.push(format!(
                                "accounts[{}] ({}) must be the declared program (address + executable pin)",
                                idx, field_name
                            ));
                        }
                    }
                }
            }
        }

        // -- Stage 2: signer / mut / address / owner / layout -------------

        if cf.attr.is_signer || wrapper_is_signer {
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
        if let Some(addr_expr) = &cf.attr.address {
            field_checks.push(quote! {
                if ctx.account(#idx)?.address() != &(#addr_expr) {
                    return ::core::result::Result::Err(
                        ::hopper::__runtime::ProgramError::InvalidAccountData
                    );
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) address must match `address = ...`",
                idx, field_name
            ));
        }
        let owner_expr = cf.attr.owner.clone();
        // Wrapper-aware layout handling:
        //   Account<'info, T>     → layout = T, has_layout = true
        //   InitAccount<'info, T> → has_layout = false at validate time
        //   Signer/Program/raw    → has_layout = false
        //   Plain T               → has_layout from skips_layout_validation
        let (has_layout, layout_ty): (bool, Option<Type>) = match &wrapper {
            Some(WrapperKind::Account { inner }) => (true, Some(inner.clone())),
            Some(WrapperKind::InitAccount { .. })
            | Some(WrapperKind::Signer)
            | Some(WrapperKind::Program) => (false, None),
            None => {
                let h = !skips_layout_validation(&cf.ty);
                (h, if h { Some(cf.ty.clone()) } else { None })
            }
        };

        // For `init` accounts the account hasn't been created yet, so we
        // skip the owner+load step. the `init_{field}()` lifecycle
        // helper will allocate and write the header later. Other cases
        // (including `zero`) assume the account already exists. The same
        // reasoning applies when the field is typed as `InitAccount<T>`.
        let is_init_field = cf.attr.init || wrapper_is_init;
        if has_layout && !is_init_field {
            let field_ty = layout_ty.as_ref().unwrap_or(&cf.ty);
            let owner_check = if let Some(expr) = &owner_expr {
                quote! {
                    ctx.account(#idx)?.check_owned_by(&(#expr))?;
                }
            } else {
                quote! {
                    ctx.account(#idx)?.check_owned_by(ctx.program_id())?;
                }
            };
            field_checks.push(quote! {
                #owner_check
                let _ = ctx.account(#idx)?.load::<#field_ty>()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) owner matches, valid {} header",
                idx,
                field_name,
                type_ident(field_ty).map(|i| i.to_string()).unwrap_or_default()
            ));
        } else if !has_layout {
            // For raw AccountView fields, still honor an explicit
            // `owner = expr` even without a layout header to validate.
            if let Some(expr) = &owner_expr {
                field_checks.push(quote! {
                    ctx.account(#idx)?.check_owned_by(&(#expr))?;
                });
                check_descriptions.push(format!(
                    "accounts[{}] ({}) owner must match `owner = ...`",
                    idx, field_name
                ));
            }
        }

        // -- Stage 4: PDA derivation (seeds + bump) ----------------------
        if let (Some(seeds), Some(bump)) = (&cf.attr.seeds, &cf.attr.bump) {
            let seed_exprs: Vec<_> = seeds.iter().collect();
            let verify_call = match bump {
                BumpSpec::Inferred => quote! {
                    {
                        let (expected, _bump) = ::hopper::prelude::find_program_address(
                            &[ #( AsRef::<[u8]>::as_ref(&(#seed_exprs)) ),* ],
                            ctx.program_id(),
                        );
                        if ctx.account(#idx)?.address() != &expected {
                            return ::core::result::Result::Err(
                                ::hopper::__runtime::ProgramError::InvalidSeeds
                            );
                        }
                    }
                },
                BumpSpec::Stored(bump_expr) => quote! {
                    {
                        let bump: u8 = #bump_expr;
                        let seeds_with_bump: &[&[u8]] = &[
                            #( AsRef::<[u8]>::as_ref(&(#seed_exprs)) ),*,
                            ::core::slice::from_ref(&bump),
                        ];
                        let expected = ::hopper::prelude::create_program_address(
                            seeds_with_bump,
                            ctx.program_id(),
                        )?;
                        if ctx.account(#idx)?.address() != &expected {
                            return ::core::result::Result::Err(
                                ::hopper::__runtime::ProgramError::InvalidSeeds
                            );
                        }
                    }
                },
            };
            field_checks.push(verify_call);
            check_descriptions.push(format!(
                "accounts[{}] ({}) matches PDA derived from declared seeds",
                idx, field_name
            ));
        }

        // -- Stage 5: init / realloc / close preconditions ----------------
        //
        // The preconditions live with validate(); the *execution* of
        // init / realloc / close happens via the per-field lifecycle
        // methods on the bound context. The payer/space/target
        // existence checks are cheap and catch malformed Context
        // wiring up-front.
        if cf.attr.init {
            // Precondition: the account must be writable and, once the
            // lifecycle helper runs, owned by this program. The
            // helper itself handles CPI + header write.
            field_checks.push(quote! {
                ctx.account(#idx)?.check_writable()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be writable (init precondition)",
                idx, field_name
            ));
        }
        if cf.attr.realloc.is_some() {
            field_checks.push(quote! {
                ctx.account(#idx)?.check_writable()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be writable (realloc precondition)",
                idx, field_name
            ));
        }
        if cf.attr.close.is_some() {
            field_checks.push(quote! {
                ctx.account(#idx)?.check_writable()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be writable (close precondition)",
                idx, field_name
            ));
        }

        // -- Stage 5.5: has_one. field value must equal other account's key.
        // Runs after layout load so we can read the struct field.
        for target_ident in &cf.attr.has_one {
            let target_name = target_ident.to_string();
            let target_idx = ctx_fields
                .iter()
                .position(|c| c.name == *target_ident)
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        target_ident,
                        format!(
                            "has_one = `{}`: no field named `{}` in this context",
                            target_name, target_name
                        ),
                    )
                })?;
            let field_ty = &cf.ty;
            let target_field_ident = target_ident.clone();
            field_checks.push(quote! {
                {
                    let view = ctx.account(#idx)?;
                    let layout = view.load::<#field_ty>()?;
                    let expected_key = ctx.account(#target_idx)?.address();
                    // Convention: the cross-referenced field on the
                    // layout must be named identically to the target
                    // account's field, and must coerce to an `Address`.
                    if ::core::convert::AsRef::<[u8; 32]>::as_ref(&layout.#target_field_ident)
                        != ::core::convert::AsRef::<[u8; 32]>::as_ref(expected_key)
                    {
                        return ::core::result::Result::Err(
                            ::hopper::__runtime::ProgramError::InvalidAccountData
                        );
                    }
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) .{} must equal accounts[{}] ({}) key",
                idx, field_name, target_name, target_idx, target_name
            ));
        }

        // -- Stage 6: arbitrary `constraint = expr` -----------------------
        for (i, expr) in cf.attr.constraint.iter().enumerate() {
            field_checks.push(quote! {
                if !({ #expr }) {
                    return ::core::result::Result::Err(
                        ::hopper::__runtime::ProgramError::Custom(0xc0_00 | (#idx as u32))
                    );
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) custom constraint #{} must hold",
                idx, field_name, i
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
                let segment_mut_fn = format_ident!("{}_segment_mut", field_name);
                let segment_ref_fn = format_ident!("{}_segment_ref", field_name);

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

                // General-purpose typed segment escape for full-mut fields.
                // Lets callers project any segment of `#field_name` without
                // pre-declaring it via `mut(field1, field2)`. The `abs_offset`
                // argument is intended to be a const segment offset (e.g.
                // `HEADER_LEN as u32 + VAULT_BALANCE_OFFSET`) so the call
                // collapses to the same const arithmetic as the named accessors.
                accessors.push(quote! {
                    /// Mutable segment escape: project an arbitrary
                    /// typed sub-slice of `#field_name`. Borrow tracking
                    /// is registered against the instruction-scoped
                    /// segment registry as a RAII **lease**. the
                    /// returned [`SegRefMut`] releases both the account
                    /// byte guard and the registry entry on drop.
                    #[inline(always)]
                    #vis fn #segment_mut_fn<__SegT: ::hopper::__runtime::Pod>(
                        &mut self,
                        abs_offset: u32,
                    ) -> ::core::result::Result<
                        ::hopper::__runtime::SegRefMut<'_, __SegT>,
                        ::hopper::__runtime::ProgramError,
                    > {
                        self.ctx.segment_mut::<__SegT>(#idx, abs_offset)
                    }
                });

                accessors.push(quote! {
                    /// Read-only segment escape for `#field_name`.
                    #[inline(always)]
                    #vis fn #segment_ref_fn<__SegT: ::hopper::__runtime::Pod>(
                        &mut self,
                        abs_offset: u32,
                    ) -> ::core::result::Result<
                        ::hopper::__runtime::SegRef<'_, __SegT>,
                        ::hopper::__runtime::ProgramError,
                    > {
                        self.ctx.segment_ref::<__SegT>(#idx, abs_offset)
                    }
                });
            }
        }

        // Generate mutable segment accessors.
        //
        // We reference both the module-level constants (`VAULT_BALANCE_OFFSET`,
        // `VAULT_BALANCE_TYPE`) emitted by `#[hopper::state]` and the inherent
        // associated constants (`Vault::BALANCE_OFFSET`) it also emits. Using
        // the inherent constant for the offset means contexts compile cleanly
        // even when the layout type is imported from another module.
        for seg_name in &cf.attr.mut_segments {
            let fn_name = format_ident!("{}_{}_mut", field_name, seg_name);
            let seg_upper = to_screaming_snake(seg_name);
            let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
            let type_alias = format_ident!("{}_{}_TYPE", type_upper, seg_upper);

            accessors.push(quote! {
                /// Mutable access to the `#seg_name` segment of `#field_name`.
                ///
                /// Returns a [`SegRefMut`](::hopper::__runtime::SegRefMut)
                ///. a RAII-leased guard that releases both the account
                /// byte borrow and the segment registry entry on drop.
                #[inline(always)]
                #vis fn #fn_name(
                    &mut self,
                ) -> ::core::result::Result<
                    ::hopper::__runtime::SegRefMut<'_, #type_alias>,
                    ::hopper::__runtime::ProgramError,
                > {
                    // const offset folded at the call site; this lowers to a
                    // single immediate add over `data_ptr` on Solana SBF.
                    const ABS_OFFSET: u32 =
                        ::hopper::prelude::HEADER_LEN as u32 + <#field_ty>::#assoc_offset;
                    self.ctx.segment_mut::<#type_alias>(#idx, ABS_OFFSET)
                }
            });
        }

        // Generate read-only segment accessors.
        for seg_name in &cf.attr.read_segments {
            let fn_name = format_ident!("{}_{}_ref", field_name, seg_name);
            let seg_upper = to_screaming_snake(seg_name);
            let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
            let type_alias = format_ident!("{}_{}_TYPE", type_upper, seg_upper);

            accessors.push(quote! {
                /// Read-only access to the `#seg_name` segment of `#field_name`.
                #[inline(always)]
                #vis fn #fn_name(
                    &mut self,
                ) -> ::core::result::Result<
                    ::hopper::__runtime::SegRef<'_, #type_alias>,
                    ::hopper::__runtime::ProgramError,
                > {
                    const ABS_OFFSET: u32 =
                        ::hopper::prelude::HEADER_LEN as u32 + <#field_ty>::#assoc_offset;
                    self.ctx.segment_ref::<#type_alias>(#idx, ABS_OFFSET)
                }
            });
        }
    }

    // ── Stage 2.4 lifecycle helpers (init / realloc / close) ───────────
    //
    // Emit `init_{field}()`, `realloc_{field}()`, and `close_{field}()`
    // methods on the bound context struct so programs can execute the
    // account-lifecycle step declared in `#[account(init/realloc/close)]`
    // with one call instead of hand-plumbing the System Program CPI
    // sequence + header write + receipt.
    //
    // The helpers call into the existing declarative macros
    // (`hopper_init!`, `hopper_close!`) so there's exactly one code
    // path for CPI + zero-init + header write. That also means
    // lifecycle flows honor whatever policy those declarative macros
    // enforce (rent-exempt minimum, sentinel-protected close, etc.).
    for cf in &ctx_fields {
        let field_name = &cf.name;
        let field_ty = &cf.ty;
        let idx = cf.index;

        if cf.attr.init {
            let init_fn = format_ident!("init_{}", field_name);
            let payer_ident = cf
                .attr
                .payer
                .as_ref()
                .expect("validate_account_attr guarantees init has payer");
            let payer_idx = ctx_fields
                .iter()
                .position(|c| c.name == *payer_ident)
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        payer_ident,
                        format!(
                            "init payer `{}`: no field named `{}` in this context",
                            payer_ident, payer_ident
                        ),
                    )
                })?;
            // Find the system_program field. by convention named
            // `system_program` and typed as AccountView or Program<'info, System>.
            let system_program_idx = ctx_fields
                .iter()
                .position(|c| c.name == format_ident!("system_program"))
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        field_name,
                        "#[account(init)] requires a `system_program` field in the context",
                    )
                })?;

            accessors.push(quote! {
                /// Create the `#field_name` account via System Program CPI,
                /// zero-init its data, and write the Hopper header.
                ///
                /// Audit Stage 2.4 lifecycle lowering. Callers should
                /// invoke this once per `init`-declared account, at the
                /// top of the instruction body.
                #[inline]
                #vis fn #init_fn(&self) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    let payer = self.ctx.account(#payer_idx)?;
                    let account = self.ctx.account(#idx)?;
                    let system_program = self.ctx.account(#system_program_idx)?;
                    ::hopper::hopper_init!(
                        payer,
                        account,
                        system_program,
                        self.ctx.program_id(),
                        #field_ty
                    )
                }
            });
        }

        if let Some(close_target) = &cf.attr.close {
            let close_fn = format_ident!("close_{}", field_name);
            let close_target_idx = ctx_fields
                .iter()
                .position(|c| c.name == *close_target)
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        close_target,
                        format!(
                            "close target `{}`: no field named `{}` in this context",
                            close_target, close_target
                        ),
                    )
                })?;
            accessors.push(quote! {
                /// Drain lamports from `#field_name` into the declared
                /// close target and mark the data for reclaim. Uses the
                /// sentinel-protected close path so a double-close (via
                /// a re-entered instruction) is detected rather than
                /// silently zeroing a reused account.
                #[inline]
                #vis fn #close_fn(&self) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    let account = self.ctx.account(#idx)?;
                    let destination = self.ctx.account(#close_target_idx)?;
                    ::hopper::hopper_close!(account, destination)
                }
            });
        }

        if let Some(realloc_expr) = &cf.attr.realloc {
            let realloc_fn = format_ident!("realloc_{}", field_name);
            let zero = cf.attr.realloc_zero;
            let payer_path = cf
                .attr
                .realloc_payer
                .as_ref()
                .map(|p_ident| {
                    let p_idx = ctx_fields
                        .iter()
                        .position(|c| c.name == *p_ident)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(
                                p_ident,
                                format!("realloc_payer `{}`: no field named `{}` in this context", p_ident, p_ident),
                            )
                        })?;
                    Ok::<_, syn::Error>(quote! { Some(self.ctx.account(#p_idx)?) })
                })
                .transpose()?
                .unwrap_or_else(|| quote! { None });

            accessors.push(quote! {
                /// Resize `#field_name`'s data to the declared length,
                /// topping up the rent-exempt lamport minimum from the
                /// declared `realloc_payer` if needed, and zero-filling
                /// any newly-appended bytes per `realloc_zero` policy.
                #[inline]
                #vis fn #realloc_fn(&self) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    let account = self.ctx.account(#idx)?;
                    let new_len: usize = (#realloc_expr) as usize;
                    let old_len = account.data_len() as usize;
                    ::hopper::__runtime::__hopper_native::batch::realloc_checked(
                        account,
                        new_len,
                        #payer_path,
                    )?;
                    if #zero && new_len > old_len {
                        let mut data = account.try_borrow_mut()?;
                        for byte in data[old_len..new_len].iter_mut() {
                            *byte = 0;
                        }
                    }
                    ::core::result::Result::Ok(())
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

    // ── Stage 2.5 schema-metadata emission (audit ST2/D4 closure) ──
    //
    // For every `#[hopper::context]` struct, emit a `const
    // SCHEMA_METADATA: ContextDescriptor` that captures every audit-
    // grade constraint field so downstream tooling (IDL generators,
    // Codama, client builders, `hopper compile --emit schema`) can
    // consume the full picture without re-parsing the source. The
    // same data is available at runtime via
    // `Deposit::SCHEMA_METADATA` and at compile time as a `const`.
    let account_schema_entries: Vec<TokenStream> = ctx_fields
        .iter()
        .map(|cf| {
            let name_lit = cf.name.to_string();
            let kind_lit = type_ident(&cf.ty)
                .map(|i| i.to_string())
                .unwrap_or_else(|_| "AccountView".to_string());
            let has_layout = !skips_layout_validation(&cf.ty);
            let layout_lit = if has_layout { kind_lit.clone() } else { String::new() };
            let writable = cf.attr.is_mut || !cf.attr.mut_segments.is_empty();
            let signer = cf.attr.is_signer;
            let optional = false;
            let seeds_lits: Vec<String> = cf
                .attr
                .seeds
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|e| quote!(#e).to_string())
                .collect();
            let has_one_lits: Vec<String> = cf
                .attr
                .has_one
                .iter()
                .map(|i| i.to_string())
                .collect();
            let lifecycle_path = if cf.attr.init {
                quote! { ::hopper::hopper_schema::accounts::AccountLifecycle::Init }
            } else if cf.attr.realloc.is_some() {
                quote! { ::hopper::hopper_schema::accounts::AccountLifecycle::Realloc }
            } else if cf.attr.close.is_some() {
                quote! { ::hopper::hopper_schema::accounts::AccountLifecycle::Close }
            } else {
                quote! { ::hopper::hopper_schema::accounts::AccountLifecycle::Existing }
            };
            let payer_lit = cf
                .attr
                .payer
                .as_ref()
                .map(|i| i.to_string())
                .unwrap_or_default();
            let init_space_expr = if let Some(expr) = &cf.attr.space {
                quote! { (#expr) as u32 }
            } else {
                quote! { 0u32 }
            };
            let expected_address_lit = cf
                .attr
                .address
                .as_ref()
                .map(|e| quote!(#e).to_string())
                .unwrap_or_default();
            let expected_owner_lit = cf
                .attr
                .owner
                .as_ref()
                .map(|e| quote!(#e).to_string())
                .unwrap_or_default();

            quote! {
                ::hopper::hopper_schema::accounts::ContextAccountDescriptor {
                    name: #name_lit,
                    kind: #kind_lit,
                    writable: #writable,
                    signer: #signer,
                    layout_ref: #layout_lit,
                    policy_ref: "",
                    seeds: &[ #( #seeds_lits ),* ],
                    optional: #optional,
                    lifecycle: #lifecycle_path,
                    payer: #payer_lit,
                    init_space: #init_space_expr,
                    has_one: &[ #( #has_one_lits ),* ],
                    expected_address: #expected_address_lit,
                    expected_owner: #expected_owner_lit,
                }
            }
        })
        .collect();

    let ctx_name_lit = name.to_string();

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
            /// see exactly what `validate()` enforces. nothing is hidden.
            pub const VALIDATION_CHECKS: &'static [&'static str] = &[
                #(#check_desc_literals),*
            ];

            /// Full Anchor-grade schema metadata: lifecycle role, PDA
            /// seeds, `has_one` edges, `payer`/`space` for init,
            /// `address`/`owner` pins. everything the audit's
            /// Stage 2.5 closure asks client generators and IDL tools
            /// to consume without re-parsing source. The `const`
            /// guarantees it's available at compile time too.
            pub const SCHEMA_METADATA: ::hopper::hopper_schema::accounts::ContextDescriptor =
                ::hopper::hopper_schema::accounts::ContextDescriptor {
                    name: #ctx_name_lit,
                    accounts: &[ #( #account_schema_entries ),* ],
                    policies: &[],
                    receipts_expected: #receipt_expected,
                    mutation_classes: &[],
                };

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
///
/// Recognizes the full Anchor-grade surface: `signer`, `mut`, `mut(seg,...)`,
/// `read(seg,...)`, `init`, `zero`, `close = target`, `realloc = expr`,
/// `realloc_payer = field`, `realloc_zero = bool`, `payer = field`,
/// `space = expr`, `seeds = [...]`, `bump` or `bump = stored_byte`,
/// `has_one = field` (repeatable), `owner = expr`, `address = expr`,
/// `constraint = expr` (repeatable).
///
/// After parsing, `validate_account_attr` runs cross-attribute
/// consistency rules (e.g. `init` requires `payer` + `space`).
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
            let ident = meta.path.get_ident().cloned();
            let name = ident.as_ref().map(|i| i.to_string()).unwrap_or_default();

            match name.as_str() {
                "signer" => {
                    result.is_signer = true;
                    Ok(())
                }
                "mut" => {
                    // `mut(field1, field2)` or bare `mut`
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
                    Ok(())
                }
                "read" => {
                    if meta.input.peek(syn::token::Paren) {
                        let content;
                        syn::parenthesized!(content in meta.input);
                        let segments: Punctuated<Ident, Comma> =
                            content.parse_terminated(Ident::parse, Token![,])?;
                        for seg in segments {
                            result.read_segments.push(seg.to_string());
                        }
                    }
                    Ok(())
                }
                "init" => {
                    result.init = true;
                    // `init` implies `mut`. the lifecycle helper must
                    // mutate the account. Callers don't need to write
                    // `init, mut` twice.
                    result.is_mut = true;
                    Ok(())
                }
                "zero" => {
                    result.zero = true;
                    Ok(())
                }
                "close" => {
                    let target: Ident = meta.value()?.parse()?;
                    result.close = Some(target);
                    // `close` implies `mut`. lamports are drained.
                    result.is_mut = true;
                    Ok(())
                }
                "realloc" => {
                    let expr: Expr = meta.value()?.parse()?;
                    result.realloc = Some(expr);
                    result.is_mut = true;
                    Ok(())
                }
                "realloc_payer" => {
                    let ident: Ident = meta.value()?.parse()?;
                    result.realloc_payer = Some(ident);
                    Ok(())
                }
                "realloc_zero" => {
                    // Accept both `realloc_zero = true/false` and bare
                    // `realloc_zero` (meaning true, matching Anchor).
                    if meta.input.peek(Token![=]) {
                        let lit: syn::LitBool = meta.value()?.parse()?;
                        result.realloc_zero = lit.value;
                    } else {
                        result.realloc_zero = true;
                    }
                    Ok(())
                }
                "payer" => {
                    let ident: Ident = meta.value()?.parse()?;
                    result.payer = Some(ident);
                    Ok(())
                }
                "space" => {
                    let expr: Expr = meta.value()?.parse()?;
                    result.space = Some(expr);
                    Ok(())
                }
                "seeds" => {
                    // `seeds = [a, b, c]`
                    let content;
                    // meta.value()? consumes the `=`; then an array literal.
                    let _eq = meta.value()?;
                    syn::bracketed!(content in _eq);
                    let items: Punctuated<Expr, Comma> =
                        content.parse_terminated(Expr::parse, Token![,])?;
                    result.seeds = Some(items.into_iter().collect());
                    Ok(())
                }
                "bump" => {
                    // `bump` (inferred) or `bump = stored_expr`.
                    if meta.input.peek(Token![=]) {
                        let expr: Expr = meta.value()?.parse()?;
                        result.bump = Some(BumpSpec::Stored(expr));
                    } else {
                        result.bump = Some(BumpSpec::Inferred);
                    }
                    Ok(())
                }
                "has_one" => {
                    let ident: Ident = meta.value()?.parse()?;
                    result.has_one.push(ident);
                    Ok(())
                }
                "owner" => {
                    let expr: Expr = meta.value()?.parse()?;
                    result.owner = Some(expr);
                    Ok(())
                }
                "address" => {
                    let expr: Expr = meta.value()?.parse()?;
                    result.address = Some(expr);
                    Ok(())
                }
                "constraint" => {
                    let expr: Expr = meta.value()?.parse()?;
                    result.constraint.push(expr);
                    Ok(())
                }
                _ => Err(meta.error(format!("unrecognized account attribute `{}`", name))),
            }
        })?;
    }

    Ok(result)
}

/// Post-parse consistency checks. Emits spanned errors for declarations
/// that are syntactically valid but semantically incoherent (e.g. `init`
/// without `payer`). The Hopper Safety Audit's compile-fail matrix
/// (D2. page 4) enumerates these; each violation here corresponds to
/// one entry in the trybuild suite.
fn validate_account_attr(field_name: &Ident, attr: &AccountAttr) -> Result<()> {
    if attr.init {
        if attr.payer.is_none() {
            return Err(syn::Error::new_spanned(
                field_name,
                "#[account(init)] requires `payer = <field>`",
            ));
        }
        if attr.space.is_none() {
            return Err(syn::Error::new_spanned(
                field_name,
                "#[account(init)] requires `space = <expr>`",
            ));
        }
        if attr.seeds.is_some() && attr.bump.is_none() {
            return Err(syn::Error::new_spanned(
                field_name,
                "#[account(init, seeds = ...)] requires `bump` (inferred) or `bump = <stored_byte>`",
            ));
        }
    }
    if attr.realloc.is_some() {
        if attr.realloc_payer.is_none() {
            return Err(syn::Error::new_spanned(
                field_name,
                "#[account(realloc = ...)] requires `realloc_payer = <field>`",
            ));
        }
        if !attr.realloc_zero {
            return Err(syn::Error::new_spanned(
                field_name,
                "#[account(realloc = ...)] requires an explicit `realloc_zero` policy (use `realloc_zero = true` to zero the newly-allocated bytes)",
            ));
        }
    }
    if attr.close.is_some() && !attr.is_mut {
        return Err(syn::Error::new_spanned(
            field_name,
            "#[account(close = ...)] requires `mut`. lamports must be drainable",
        ));
    }
    if attr.seeds.is_some() && attr.bump.is_none() && !attr.init {
        return Err(syn::Error::new_spanned(
            field_name,
            "#[account(seeds = ...)] requires `bump` (or `bump = <stored_byte>`)",
        ));
    }
    Ok(())
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
            .map(|segment| {
                matches!(
                    segment.ident.to_string().as_str(),
                    "AccountView"
                        | "Signer"
                        | "HopperSigner"
                        | "UncheckedAccount"
                        | "ProgramRef"
                        | "Program"
                )
            })
            .unwrap_or(false),
        _ => false,
    }
}

/// Audit Stage 2.3: classify wrapper types so the context macro can
/// auto-derive the appropriate checks from the type name alone.
#[derive(Clone)]
enum WrapperKind {
    /// `Signer<'info>`. emit `check_signer`.
    Signer,
    /// `Program<'info, P>`. emit `check_address == P::ID` and
    /// `check_executable`. Layout validation skipped.
    Program,
    /// `Account<'info, T>`. emit `check_owned_by(program_id)` +
    /// `load::<T>()` using `T` as the layout. Inner type accessible
    /// via `.inner_layout()`.
    Account { inner: Type },
    /// `InitAccount<'info, T>`. skip pre-instruction layout check
    /// (account doesn't exist yet); the `init_{field}` lifecycle
    /// helper will create + initialise it.
    InitAccount { inner: Type },
}

impl WrapperKind {
    fn inner_layout(&self) -> Option<&Type> {
        match self {
            WrapperKind::Account { inner } | WrapperKind::InitAccount { inner } => Some(inner),
            _ => None,
        }
    }
}

/// Recognize typed wrapper types (`Signer<'info>`, `Account<'info, T>`,
/// `InitAccount<'info, T>`, `Program<'info, P>`) and extract the inner
/// layout type where applicable. Returns `None` for raw `AccountView`
/// or plain layout types.
fn classify_wrapper(ty: &Type) -> Option<WrapperKind> {
    let Type::Path(TypePath { path, .. }) = ty else {
        return None;
    };
    let segment = path.segments.last()?;
    let name = segment.ident.to_string();

    match name.as_str() {
        "Signer" | "HopperSigner" => Some(WrapperKind::Signer),
        "Program" => Some(WrapperKind::Program),
        "Account" | "InitAccount" => {
            // Pull out the generic `T` arg. `Account<'info, T>` has
            // a lifetime arg first, then a type arg. we want the
            // last type arg.
            let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
                return None;
            };
            let inner = args.args.iter().find_map(|arg| {
                if let syn::GenericArgument::Type(ty) = arg {
                    Some(ty.clone())
                } else {
                    None
                }
            })?;
            if name == "Account" {
                Some(WrapperKind::Account { inner })
            } else {
                Some(WrapperKind::InitAccount { inner })
            }
        }
        _ => None,
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
