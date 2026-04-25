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
    parse::{Parse, ParseStream},
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
    /// `init_if_needed`. Anchor-parity sibling of `init`. When the
    /// account is already allocated (data_len > 0 with a Hopper
    /// header in place) the lifecycle helper returns `Ok(())` without
    /// invoking the system-program CreateAccount CPI. When the
    /// account is empty, it falls through to the same init path as
    /// `init`. Requires the same fields as `init` (`payer`, `space`,
    /// optional `seeds`/`bump`). Callers must still validate the
    /// existing layout separately — `init_if_needed` guarantees the
    /// account exists and was sized at creation time, not that its
    /// current contents match a specific layout.
    init_if_needed: bool,
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
    /// `seeds_fn = Type::seeds(&arg1, &arg2)`. Typed-seeds sugar. The
    /// provided expression must evaluate to a value that implements
    /// `AsRef<[Seed]>` or equivalently yields `&[&[u8]]`. Hopper uses
    /// it in place of the inline `seeds = [...]` array. Inspired by
    /// Quasar's `Type::seeds(...)` pattern; the point is that each
    /// type can centralize its PDA seed layout in one place and
    /// every context just calls the helper.
    seeds_fn: Option<Expr>,
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

    // ── Anchor SPL parity (audit ST2: "make Hopper the best of three") ──
    //
    // These constraints bring Hopper's declarative account layer to
    // strict parity with Anchor's `#[account(token::mint = X, ...)]`
    // family. Each attribute is parsed in the nested-meta pass below
    // and lowered into a call to the matching `require_*` helper in
    // `hopper_runtime::token`. Those helpers read exactly the bytes
    // that matter from an already-borrowed account buffer. no
    // full-struct deserialize, no new crate dependencies, no ABI
    // coupling to an external spl-token version.
    //
    /// `token::mint = expr`. require this SPL TokenAccount's bytes
    /// `[0..32]` equal the pubkey produced by `expr`.
    token_mint: Option<Expr>,
    /// `token::authority = expr`. require this SPL TokenAccount's
    /// bytes `[32..64]` equal the pubkey produced by `expr`.
    token_authority: Option<Expr>,
    /// `token::token_program = expr`. require this account's Solana
    /// owner-program equals `expr`. The usual case is pointing at
    /// Token-2022 instead of the default SPL Token program, so the
    /// program can validate a Token-2022 token account the same way
    /// it validates a legacy SPL one. Defaults to SPL Token when
    /// `token::mint` or `token::authority` are set without an
    /// explicit `token_program`.
    token_token_program: Option<Expr>,
    /// `mint::authority = expr`. require this SPL Mint's
    /// `mint_authority` COption equals `Some(expr)`.
    mint_authority: Option<Expr>,
    /// `mint::decimals = expr`. require this SPL Mint's byte 44 equal
    /// `expr as u8`.
    mint_decimals: Option<Expr>,
    /// `mint::freeze_authority = expr`. require this SPL Mint's
    /// `freeze_authority` COption equals `Some(expr)`.
    mint_freeze_authority: Option<Expr>,
    /// `mint::token_program = expr`. require this account's Solana
    /// owner-program equals `expr`. Defaults to SPL Token when any
    /// `mint::*` constraint is set without an explicit `token_program`.
    /// The Token-2022 parity lever for the mint axis.
    mint_token_program: Option<Expr>,
    /// `associated_token::mint = expr`. ATA derivation input.
    associated_token_mint: Option<Expr>,
    /// `associated_token::authority = expr`. ATA derivation input.
    associated_token_authority: Option<Expr>,
    /// `associated_token::token_program = expr`. optional token-program
    /// override. defaults to the legacy SPL Token program ID when
    /// the user omits it. Accepting this value is what lets Hopper
    /// support ATAs over Token-2022 mints without a second attribute.
    associated_token_token_program: Option<Expr>,
    /// `seeds::program = expr`. when present, PDA derivation for this
    /// field uses the given program ID instead of
    /// `ctx.program_id()`. Anchor emits this as the third positional
    /// argument to `Pubkey::find_program_address(..., program_id)`.
    seeds_program: Option<Expr>,

    // ── Token-2022 extension constraints (zero-copy TLV readers) ──
    //
    // Each lever lowers to a single call into
    // `hopper_runtime::token_2022_ext::require_*`. The readers scan
    // the mint or token-account TLV region in place, no heap, no full
    // decode. This is the surface Anchor routes through
    // `InterfaceAccount<Mint>` with a Borsh deserialize; Hopper keeps
    // it on the zero-copy path end to end.
    ext_non_transferable: bool,
    ext_immutable_owner: bool,
    ext_mint_close_authority: Option<Expr>,
    ext_permanent_delegate: Option<Expr>,
    ext_transfer_hook_authority: Option<Expr>,
    ext_transfer_hook_program: Option<Expr>,
    ext_metadata_pointer_authority: Option<Expr>,
    ext_metadata_pointer_address: Option<Expr>,
    ext_default_account_state: Option<Expr>,
    ext_interest_bearing_authority: Option<Expr>,
    ext_transfer_fee_config_authority: Option<Expr>,
    ext_transfer_fee_withdraw_authority: Option<Expr>,

    /// `dup = other_field`. Quasar-style. This slot is allowed to
    /// alias `other_field` (the caller intentionally passed the same
    /// account in two roles). Skips the "no duplicate writables" and
    /// "no duplicate signers" pipeline checks for this pair. Does
    /// NOT imply `mut`.
    dup: Option<Ident>,

    /// `sweep = target_field`. After the handler returns Ok, move
    /// any remaining lamports from this account to `target_field`'s
    /// address. Runs as a post-handler epilogue emitted by `bind`.
    /// Used for pool fee sweeps, keeper cleanup, and rent-reclaim
    /// patterns. Implies `mut` on both the source and target.
    sweep: Option<Ident>,

    /// `executable`. Anchor-parity keyword. Requires the account's
    /// `executable` flag to be true — i.e. it must be a deployed BPF
    /// program. Hopper's `Program<P>` wrapper type already implies
    /// this, but the bare keyword exists for ports of Anchor code and
    /// for cases where the field type is `AccountView` instead of a
    /// typed wrapper.
    executable: bool,

    /// `rent_exempt = enforce | skip`. Anchor-parity keyword. When set
    /// to `enforce` the context binder checks that the account's
    /// lamport balance is at or above the rent-exemption minimum for
    /// its data length. When set to `skip` the check is explicitly
    /// omitted (useful when the caller has asserted rent-exemption
    /// through a different pathway and wants the intent recorded).
    /// When unset (the default), no check is emitted and the caller
    /// is responsible for rent safety.
    rent_exempt: Option<RentExemptPolicy>,
}

/// Policy for the `rent_exempt` field keyword.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum RentExemptPolicy {
    /// `rent_exempt = enforce`. Runtime check that
    /// `account.lamports() >= Rent::minimum_balance(data_len)`.
    Enforce,
    /// `rent_exempt = skip`. Explicitly opts out; emits no check but
    /// records the intent in the generated code (and in the schema
    /// manifest) so an auditor can see the acknowledgment.
    Skip,
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

/// A single `name: Type` binding inside a struct-level
/// `#[instruction(...)]` attribute.
///
/// ## Innovation over Anchor
///
/// Anchor's `#[instruction(...)]` is a **parse-only** hint. its argument
/// names never appear in generated `impl` bodies beyond the accounts
/// constraint expressions themselves, so there's no way to cross-check
/// the declared arg list against the actual instruction decoder. a
/// mismatch is only caught when the seed expression fails to typecheck.
///
/// Hopper threads the declared args through both:
/// - every per-field `validate_<field>` function, so that each constraint
///   gets the same Rust parameters (and the compiler surfaces a
///   helpful error if the type doesn't match), and
/// - the emitted `SCHEMA_METADATA` (`context_args`), so off-chain tooling
///   (hopper-sdk, Codama, IDL) can see the declared args without
///   re-parsing source.
///
/// The args also drive a dedicated `*_with_args` pair of `validate` /
/// `bind` entry points. the args-less `validate` / `bind` are **not**
/// emitted in that case, because a seed/constraint expression referring
/// to an arg cannot compile without the binding in scope. forcing the
/// user to call `bind_with_args` keeps the contract honest.
#[derive(Debug)]
struct InstructionArgDecl {
    name: Ident,
    ty: Type,
}

impl Parse for InstructionArgDecl {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;
        Ok(Self { name, ty })
    }
}

/// Scan a struct's outer attribute list for `#[instruction(...)]`,
/// returning the parsed `(name, type)` bindings and stripping the
/// attribute from the struct so it doesn't leak through the emitted
/// code path.
///
/// Accepts exactly one `#[instruction(...)]` attribute per struct.
/// Multiple attributes or duplicate arg names are rejected with a
/// span-attached compile error so the failure points at the offending
/// token rather than bubbling up as an opaque runtime symbol clash.
fn parse_instruction_attr(attrs: &mut Vec<Attribute>) -> Result<Vec<InstructionArgDecl>> {
    let mut out: Vec<InstructionArgDecl> = Vec::new();
    let mut seen = 0usize;

    for attr in attrs.iter() {
        if !attr.path().is_ident("instruction") {
            continue;
        }
        if seen > 0 {
            return Err(syn::Error::new_spanned(
                attr,
                "#[hopper::context] accepts at most one #[instruction(...)] attribute; \
                 put every arg in a single list, comma-separated",
            ));
        }
        seen += 1;

        let parsed: Punctuated<InstructionArgDecl, Comma> = attr
            .parse_args_with(Punctuated::<InstructionArgDecl, Comma>::parse_terminated)?;
        for arg in parsed {
            if out.iter().any(|a| a.name == arg.name) {
                return Err(syn::Error::new_spanned(
                    &arg.name,
                    format!(
                        "duplicate instruction argument `{}`: each binding must be uniquely named",
                        arg.name
                    ),
                ));
            }
            out.push(arg);
        }
    }

    attrs.retain(|a| !a.path().is_ident("instruction"));
    Ok(out)
}

/// Public entry point for the `#[hopper::context]` attribute.
///
/// Backward-compatible wrapper around [`expand_inner`]; emits the original
/// struct definition, since attribute macros are responsible for the
/// passthrough.
pub fn expand(_attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    expand_inner(item, /* emit_struct */ true)
}

/// Public entry point for the `#[derive(Accounts)]` proc-macro derive.
///
/// Functionally identical to [`expand`], except the original input struct
/// is **not** re-emitted (the user already declared it themselves). Helper
/// attributes — `#[account(...)]`, `#[signer]`, `#[instruction(...)]`,
/// `#[validate]` — are still parsed off the input but cannot be stripped
/// in place because the struct is not under our attribute. We rely on the
/// `attributes(...)` declaration on the derive macro to silence the
/// compiler's "unknown attribute" check; the helpers are dropped from the
/// final compilation unit by `rustc` once all derives have run.
pub fn expand_for_derive(item: TokenStream) -> Result<TokenStream> {
    expand_inner(item, /* emit_struct */ false)
}

fn expand_inner(item: TokenStream, emit_struct: bool) -> Result<TokenStream> {
    let mut input: ItemStruct = parse2(item)?;

    // ── Instruction-arg typing (audit Stage 2.6) ──────────────────────
    //
    // Parse the struct-level `#[instruction(name: Type, ...)]` attribute
    // before anything else touches `input.attrs`. we strip it in place
    // so the emitted struct doesn't re-export an attribute with no
    // attached proc-macro (Rust would emit `unknown attribute` otherwise).
    //
    // When non-empty, the declared args are threaded as ordinary Rust
    // parameters into every per-field validator and into the top-level
    // entry points. seed / constraint / owner / address expressions
    // that reference these names compile the same way any other local
    // binding compiles. no magic, no hidden thread-local, no runtime
    // lookup. this is the piece that lets declarative seeds say
    // `seeds = [b"vault", nonce.to_le_bytes().as_ref()]` and have
    // `nonce` resolve to the typed instruction argument.
    let instruction_args = parse_instruction_attr(&mut input.attrs)?;
    let has_instruction_args = !instruction_args.is_empty();

    // Anchor-parity `#[validate]` opt-in. When the author adds
    // `#[validate]` at the struct level, `bind()` calls a
    // user-provided inherent method
    // `fn validate(&self) -> Result<(), ProgramError>` on the bound
    // context struct after every built-in constraint has passed.
    //
    // Why a marker instead of auto-detect: Rust trait dispatch cannot
    // tell "user implemented validate" apart from "user didn't touch
    // it" without specialization. An explicit opt-in keeps the call
    // path honest, and an unset `#[validate]` on a struct that
    // happens to have its own `validate(&self)` is a dead method the
    // compiler warns about, which is the correct failure mode.
    let user_validate = input.attrs.iter().any(|a| a.path().is_ident("validate"));
    input.attrs.retain(|a| !a.path().is_ident("validate"));

    // Prebuilt fragments for the declared instruction args. each one
    // is used in several places in the emitted output (per-field
    // validator signatures, top-level validate/bind signatures, call
    // sites that forward args down), so we compute them once.
    let arg_params: Vec<TokenStream> = instruction_args
        .iter()
        .map(|a| {
            let n = &a.name;
            let t = &a.ty;
            quote! { #n: #t }
        })
        .collect();
    let arg_names: Vec<Ident> = instruction_args.iter().map(|a| a.name.clone()).collect();
    // `_with_args` suffix on the top-level entry points when the user
    // has declared any typed args. this gives callers a distinct symbol
    // and lets us *omit* the args-less `validate`/`bind` entirely when
    // they'd be incomplete (a seed expression that references an arg
    // can't compile without the binding in scope, so silently emitting
    // a half-validated `validate` would be a footgun).
    let top_validate_ident = if has_instruction_args {
        format_ident!("validate_with_args")
    } else {
        format_ident!("validate")
    };
    let top_bind_ident = if has_instruction_args {
        format_ident!("bind_with_args")
    } else {
        format_ident!("bind")
    };

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

    // Bumps captured during the PDA-derivation pass. Each entry is
    // `(field_ident, derive_expr)` where `derive_expr` evaluates to a
    // `::core::result::Result<u8, ProgramError>` inside `bind(...)`.
    // Inferred bumps re-run `find_program_address` in a dedicated
    // helper on the bound-context path (accept the extra derivation
    // cost for the ergonomic win; stored bumps are free). Stored bumps
    // read the user-supplied byte directly. Fields without `seeds = ...`
    // never appear here and never show up on the `Bumps` struct,
    // matching Anchor's shape exactly. That asymmetry is deliberate:
    // a `Bumps` struct with a `u8` slot for every account would invite
    // readers to assume every slot had a meaning, and writing `0` for
    // non-PDAs is worse than omitting them.
    let mut bump_entries: Vec<(Ident, TokenStream)> = Vec::new();

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
        if cf.attr.executable {
            // Anchor-parity `executable` keyword. Routes through
            // AccountView::check_executable which returns an error
            // when the `executable` flag on the loader-provided
            // account header is unset.
            field_checks.push(quote! {
                ctx.account(#idx)?.check_executable()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be executable (deployed BPF program)",
                idx, field_name
            ));
        }
        if let Some(policy) = cf.attr.rent_exempt {
            match policy {
                RentExemptPolicy::Enforce => {
                    // Anchor-parity `rent_exempt = enforce`. Requires
                    // `lamports() >= Rent::minimum_balance(data_len)`.
                    // Uses the runtime helper that reads the Rent
                    // sysvar lazily — the check is explicit, not a
                    // heuristic.
                    field_checks.push(quote! {
                        ::hopper::hopper_runtime::rent::check_rent_exempt(
                            ctx.account(#idx)?,
                        )?;
                    });
                    check_descriptions.push(format!(
                        "accounts[{}] ({}) must be rent-exempt (lamports >= Rent::minimum_balance(data_len))",
                        idx, field_name
                    ));
                }
                RentExemptPolicy::Skip => {
                    // `rent_exempt = skip` is an explicit acknowledgment
                    // that the caller is handling rent-exemption through
                    // a different pathway. Emits no check; only records
                    // the intent in the schema so auditors can see it.
                    check_descriptions.push(format!(
                        "accounts[{}] ({}) rent-exemption intentionally skipped (rent_exempt = skip)",
                        idx, field_name
                    ));
                }
            }
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

        // -- Stage 4a: typed-seeds sugar (`seeds_fn = Type::seeds(...)`) --
        //
        // Quasar-style sugar: the user centralizes their PDA seed
        // layout on the account type via a `seeds(...) -> ...` helper,
        // and every context references it by name. We lower to
        // `find_program_address(expr(), program_id)` and verify the
        // resulting pubkey matches the account at `#idx`. Bumps come
        // back on the returned value from `find_program_address` so
        // no separate `bump` attribute is needed with this form.
        if let Some(seeds_fn_expr) = &cf.attr.seeds_fn {
            // Reject a combination that would be ambiguous: the user
            // supplied both a seeds array AND a seeds_fn. Which
            // derivation wins is a coin flip the author should not
            // depend on.
            if cf.attr.seeds.is_some() {
                return Err(syn::Error::new_spanned(
                    seeds_fn_expr,
                    "`seeds_fn = ...` cannot be combined with `seeds = [...]`. Pick one.",
                ));
            }
            let pda_program_expr = if let Some(prog) = &cf.attr.seeds_program {
                quote! { &(#prog) }
            } else {
                quote! { ctx.program_id() }
            };
            field_checks.push(quote! {
                {
                    let __seed_slices: &[&[u8]] = (#seeds_fn_expr).as_ref();
                    let (expected, _bump) = ::hopper::prelude::find_program_address(
                        __seed_slices,
                        #pda_program_expr,
                    );
                    if ctx.account(#idx)?.address() != &expected {
                        return ::core::result::Result::Err(
                            ::hopper::__runtime::ProgramError::InvalidSeeds
                        );
                    }
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) matches PDA derived from typed seeds helper",
                idx, field_name
            ));
            bump_entries.push((
                field_name.clone(),
                quote! {
                    {
                        let __seed_slices: &[&[u8]] = (#seeds_fn_expr).as_ref();
                        let (_, __b) = ::hopper::prelude::find_program_address(
                            __seed_slices,
                            #pda_program_expr,
                        );
                        __b
                    }
                },
            ));
        }

        // -- Stage 4: PDA derivation (seeds + bump) ----------------------
        if let (Some(seeds), Some(bump)) = (&cf.attr.seeds, &cf.attr.bump) {
            let seed_exprs: Vec<_> = seeds.iter().collect();
            // `seeds::program = X` (Anchor-compat) redirects PDA
            // derivation to a program ID other than the currently
            // executing one. This is how a program verifies that an
            // account is a PDA of *another* program. a common pattern
            // when interoperating with governance or registry programs.
            // When omitted, we keep the existing behavior of using
            // `ctx.program_id()`.
            let pda_program_expr = if let Some(prog) = &cf.attr.seeds_program {
                quote! { &(#prog) }
            } else {
                quote! { ctx.program_id() }
            };
            let verify_call = match bump {
                BumpSpec::Inferred => quote! {
                    {
                        let (expected, _bump) = ::hopper::prelude::find_program_address(
                            &[ #( AsRef::<[u8]>::as_ref(&(#seed_exprs)) ),* ],
                            #pda_program_expr,
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
                            #pda_program_expr,
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
                "accounts[{}] ({}) matches PDA derived from declared seeds{}",
                idx,
                field_name,
                if cf.attr.seeds_program.is_some() {
                    " (under custom program ID)"
                } else {
                    ""
                }
            ));

            // Build the derive expression used by the generated Bumps
            // struct gatherer. Stored bumps read the user-supplied byte
            // straight from scope; Inferred bumps re-run
            // `find_program_address` on the bound-context path. The
            // extra derivation for Inferred is the cost of the
            // ergonomic win: the whole point of surfacing
            // `ctx.bumps().field` is to save the caller from redoing
            // the work in a CPI signer-seeds block one line later.
            // Stored bumps cost zero CU.
            let bump_gather_expr: TokenStream = match bump {
                BumpSpec::Stored(bump_expr) => quote! {
                    { let __b: u8 = #bump_expr; __b }
                },
                BumpSpec::Inferred => quote! {
                    {
                        let (_, __b) = ::hopper::prelude::find_program_address(
                            &[ #( AsRef::<[u8]>::as_ref(&(#seed_exprs)) ),* ],
                            #pda_program_expr,
                        );
                        __b
                    }
                },
            };
            bump_entries.push((field_name.clone(), bump_gather_expr));
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

        // -- Stage 7: Anchor SPL parity -----------------------------------
        //
        // token::mint / token::authority / mint::authority / mint::decimals /
        // mint::freeze_authority / associated_token::{mint,authority,token_program}.
        //
        // Each of these lowers to a single call to a `hopper_runtime::token`
        // precondition helper, each of which reads only the exact bytes
        // it needs from the already-borrowed account buffer. no
        // full-struct deserialize, no new crate dependencies.
        //
        // The helpers live in `hopper_runtime::token` (for Token + Mint
        // shape checks) and in `hopper_solana::ata` (for ATA
        // derivation. only on-chain via `#[cfg(target_os = "solana")]`).
        // Owner-program override for the `token::*` family. Emitted
        // exactly once when any `token::mint` / `token::authority` /
        // `token::token_program` is present, so the owner check runs
        // before the byte-level shape checks and rejects a wrong-program
        // account without reading its payload.
        //
        // Default: SPL Token. Explicit `token::token_program = X`
        // routes to X instead (the Token-2022 pattern). A standalone
        // `token::token_program` with no shape check is valid and
        // still enforces owner alone, matching Anchor's behavior.
        let has_token_shape =
            cf.attr.token_mint.is_some() || cf.attr.token_authority.is_some();
        if has_token_shape || cf.attr.token_token_program.is_some() {
            let prog_expr = if let Some(tp) = &cf.attr.token_token_program {
                quote! { &(#tp) }
            } else {
                quote! { &::hopper::__runtime::token::TOKEN_PROGRAM_ID }
            };
            field_checks.push(quote! {
                ctx.account(#idx)?.check_owned_by(#prog_expr)?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) is owned by the declared token program{}",
                idx,
                field_name,
                if cf.attr.token_token_program.is_some() {
                    " (explicit token_program override)"
                } else {
                    " (SPL Token default)"
                }
            ));
        }

        if let Some(expected_mint) = &cf.attr.token_mint {
            field_checks.push(quote! {
                ::hopper::__runtime::token::require_token_mint(
                    ctx.account(#idx)?,
                    &(#expected_mint),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) is a token account for the declared mint",
                idx, field_name
            ));
        }
        if let Some(expected_authority) = &cf.attr.token_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token::require_token_owner_eq(
                    ctx.account(#idx)?,
                    &(#expected_authority),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) token account authority matches declared authority",
                idx, field_name
            ));
        }
        // Owner-program override for the `mint::*` family. Same
        // pattern as the token-axis check: emit once whenever any
        // `mint::authority` / `mint::decimals` / `mint::freeze_authority` /
        // `mint::token_program` appears, so the owner is pinned before
        // any layout-byte check runs.
        let has_mint_shape = cf.attr.mint_authority.is_some()
            || cf.attr.mint_decimals.is_some()
            || cf.attr.mint_freeze_authority.is_some();
        if has_mint_shape || cf.attr.mint_token_program.is_some() {
            let prog_expr = if let Some(tp) = &cf.attr.mint_token_program {
                quote! { &(#tp) }
            } else {
                quote! { &::hopper::__runtime::token::TOKEN_PROGRAM_ID }
            };
            field_checks.push(quote! {
                ctx.account(#idx)?.check_owned_by(#prog_expr)?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) is a mint owned by the declared token program{}",
                idx,
                field_name,
                if cf.attr.mint_token_program.is_some() {
                    " (explicit token_program override)"
                } else {
                    " (SPL Token default)"
                }
            ));
        }

        if let Some(expected_mint_authority) = &cf.attr.mint_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token::require_mint_authority(
                    ctx.account(#idx)?,
                    &(#expected_mint_authority),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) mint_authority matches declared authority",
                idx, field_name
            ));
        }
        if let Some(decimals_expr) = &cf.attr.mint_decimals {
            field_checks.push(quote! {
                ::hopper::__runtime::token::require_mint_decimals(
                    ctx.account(#idx)?,
                    (#decimals_expr) as u8,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) mint decimals equals declared value",
                idx, field_name
            ));
        }
        if let Some(expected_freeze) = &cf.attr.mint_freeze_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token::require_mint_freeze_authority(
                    ctx.account(#idx)?,
                    &(#expected_freeze),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) mint freeze_authority matches declared authority",
                idx, field_name
            ));
        }

        // Token-2022 extension constraints. Each lowers to a single
        // TLV-scan call on the Token-2022 account bytes. Extensions
        // are only valid on Token-2022 accounts, so the usual
        // `token::token_program = TOKEN_2022_ID` or
        // `mint::token_program = TOKEN_2022_ID` constraint should
        // precede them in source; the emitted owner check has
        // already run before any of this lowers.
        if cf.attr.ext_non_transferable {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_non_transferable(
                    ctx.account(#idx)?,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) carries NonTransferable extension",
                idx, field_name
            ));
        }
        if cf.attr.ext_immutable_owner {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_immutable_owner(
                    ctx.account(#idx)?,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) carries ImmutableOwner extension",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_mint_close_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_mint_close_authority(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) MintCloseAuthority matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_permanent_delegate {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_permanent_delegate(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) PermanentDelegate matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_transfer_hook_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_transfer_hook_authority(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) TransferHook authority matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_transfer_hook_program {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_transfer_hook_program(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) TransferHook program_id matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_metadata_pointer_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_metadata_pointer_authority(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) MetadataPointer authority matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_metadata_pointer_address {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_metadata_pointer_address(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) MetadataPointer metadata_address matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_default_account_state {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_default_account_state(
                    ctx.account(#idx)?,
                    (#expected) as u8,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) DefaultAccountState matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_interest_bearing_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_interest_bearing_authority(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) InterestBearing rate_authority matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_transfer_fee_config_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_transfer_fee_config_authority(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) TransferFeeConfig authority matches",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_transfer_fee_withdraw_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_transfer_fee_withdraw_authority(
                    ctx.account(#idx)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) TransferFeeConfig withdraw_authority matches",
                idx, field_name
            ));
        }

        // `dup = other_field`. Require this slot to alias the named
        // other slot. The caller explicitly opted into aliasing by
        // declaring it, which is the safe pattern. If the caller
        // actually passes different accounts, we reject rather than
        // silently accept, matching Quasar's dup semantic.
        if let Some(other) = &cf.attr.dup {
            let other_idx = ctx_fields
                .iter()
                .position(|f| &f.name == other)
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        other,
                        format!("`dup = {}` must name a sibling field on the same context", other),
                    )
                })?;
            field_checks.push(quote! {
                if ctx.account(#idx)?.address() != ctx.account(#other_idx)?.address() {
                    return ::core::result::Result::Err(
                        ::hopper::__runtime::ProgramError::InvalidAccountData
                    );
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) aliases accounts[{}] ({})",
                idx, field_name, other_idx, other
            ));
        }
        // ATA derivation: both mint and authority must be declared to
        // be verifiable. Validation enforces that in
        // `validate_account_attr`. Here we can assume the pair is
        // coherent.
        if let (Some(ata_mint), Some(ata_auth)) = (
            &cf.attr.associated_token_mint,
            &cf.attr.associated_token_authority,
        ) {
            // Optional token-program override. when omitted we fall
            // back to the canonical SPL Token program ID re-exported
            // from `hopper_runtime::token`.
            let token_program_expr = if let Some(tp) = &cf.attr.associated_token_token_program {
                quote! { &(#tp) }
            } else {
                quote! { &::hopper::__runtime::token::TOKEN_PROGRAM_ID }
            };
            field_checks.push(quote! {
                {
                    // On-chain PDA derivation is only available when
                    // targeting the Solana runtime. Off-chain tooling
                    // (IDL dumps, hopper-sdk) does not build these
                    // checks into the same binary, so we gate the
                    // call under the Solana target triple.
                    //
                    // `derive_ata_for_program` returns `(Address, u8)`.
                    // We only need the address; the bump byte is
                    // meaningful only if the caller wants to cache it
                    // in account data.
                    #[cfg(target_os = "solana")]
                    {
                        let (expected, _bump) =
                            ::hopper::hopper_associated_token::derive_ata_for_program(
                                &(#ata_auth),
                                &(#ata_mint),
                                #token_program_expr,
                            );
                        if ctx.account(#idx)?.address() != &expected {
                            return ::core::result::Result::Err(
                                ::hopper::__runtime::ProgramError::InvalidSeeds
                            );
                        }
                    }
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) is the ATA for (authority, mint, token_program)",
                idx, field_name
            ));
        }

        if !field_checks.is_empty() {
            // When the user declared `#[instruction(...)]` at the struct
            // level, every per-field validator threads the declared
            // args through its signature. The fragment
            // `#(#arg_params),*` expands to an empty token span when
            // `has_instruction_args` is false, so the args-less case
            // is still `fn validate_<field>(ctx: &Context<'_>)` exactly
            // as before. The leading comma is guarded the same way,
            // giving us a single unified emission path.
            // Quote's repetition `#(#v)*` consumes `v` via `IntoIterator`,
            // so we clone the arg token streams per call site. this matches
            // the pattern used in `error.rs` (`idents_for_from`,
            // `idents_for_code`, etc.) and keeps the outer loop safe.
            let arg_param_fragment = if has_instruction_args {
                let aps = arg_params.clone();
                quote! { , #(#aps),* }
            } else {
                TokenStream::new()
            };
            let arg_name_fragment = if has_instruction_args {
                let ans = arg_names.clone();
                quote! { , #(#ans),* }
            } else {
                TokenStream::new()
            };
            per_field_validators.push(quote! {
                /// Validate the `#field_name` account (index #idx).
                #[inline(always)]
                #vis fn #validate_fn(
                    ctx: &::hopper::prelude::Context<'_>
                    #arg_param_fragment
                ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    #(#field_checks)*
                    Ok(())
                }
            });

            // Monolithic `validate` / `validate_with_args` composition.
            // Forwards whatever args were declared at the struct level
            // so each per-field validator sees the same typed bindings.
            validation_stmts.push(quote! {
                Self::#validate_fn(ctx #arg_name_fragment)?;
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

        // `sweep = target` emits an inherent method `sweep_<field>()`
        // on the bound context. The method moves every remaining
        // lamport from this slot into the target slot. Calling it is
        // up to the user: bind() does not auto-run sweeps because
        // handler semantics (short-circuit on error, skip cleanup on
        // failure) vary per program. Typically called in the happy
        // path right before the handler returns Ok.
        if let Some(target) = &cf.attr.sweep {
            let target_idx = ctx_fields
                .iter()
                .position(|f| &f.name == target)
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        target,
                        format!("`sweep = {}` must name a sibling field on the same context", target),
                    )
                })?;
            let sweep_fn = format_ident!("sweep_{}", field_name);
            accessors.push(quote! {
                /// Drain every lamport from this slot into the declared
                /// sweep target. Call in the happy path just before
                /// returning. Returns the drained amount.
                #[inline]
                #vis fn #sweep_fn(&mut self)
                    -> ::core::result::Result<u64, ::hopper::__runtime::ProgramError>
                {
                    let src = self.ctx.account(#idx)?;
                    let dst = self.ctx.account(#target_idx)?;
                    let amount = src.lamports();
                    if amount == 0 {
                        return Ok(0);
                    }
                    src.try_borrow_mut_lamports()?
                        .checked_sub(amount)
                        .map(|v| *src.try_borrow_mut_lamports().unwrap() = v)
                        .ok_or(::hopper::__runtime::ProgramError::ArithmeticOverflow)?;
                    let dst_lam = dst.try_borrow_mut_lamports()?;
                    *dst_lam = dst_lam
                        .checked_add(amount)
                        .ok_or(::hopper::__runtime::ProgramError::ArithmeticOverflow)?;
                    Ok(amount)
                }
            });
        }

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

        if cf.attr.init || cf.attr.init_if_needed {
            let is_if_needed = cf.attr.init_if_needed;
            let init_fn = format_ident!("init_{}", field_name);
            let payer_ident = cf
                .attr
                .payer
                .as_ref()
                .expect("validate_account_attr guarantees init/init_if_needed has payer");
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
                        "#[account(init | init_if_needed)] requires a `system_program` field in the context",
                    )
                })?;

            // Two emission shapes:
            //
            //   init            — unconditionally call hopper_init!
            //                     (which errors if the account is
            //                     already allocated).
            //
            //   init_if_needed  — skip the CreateAccount CPI entirely
            //                     when the account already has data.
            //                     The account is then assumed to be
            //                     set up by a prior invocation; the
            //                     caller is responsible for verifying
            //                     the existing layout separately.
            let body = if is_if_needed {
                quote! {
                    let account = self.ctx.account(#idx)?;
                    if account.data_len() > 0 {
                        // Already allocated; nothing to do. Caller
                        // should still validate the layout via
                        // `<ctx>_load()` or equivalent.
                        return ::core::result::Result::Ok(());
                    }
                    let payer = self.ctx.account(#payer_idx)?;
                    let system_program = self.ctx.account(#system_program_idx)?;
                    ::hopper::hopper_init!(
                        payer,
                        account,
                        system_program,
                        self.ctx.program_id(),
                        #field_ty
                    )
                }
            } else {
                quote! {
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
            };

            let doc = if is_if_needed {
                "Create the account via System Program CPI if it doesn't exist yet (init_if_needed). \
                 If the account is already allocated (data_len > 0) the helper returns Ok(()) without \
                 touching lamports or data — caller is responsible for validating the existing layout."
            } else {
                "Create the account via System Program CPI, zero-init its data, and write the Hopper header. \
                 Errors if the account is already allocated."
            };

            accessors.push(quote! {
                #[doc = #doc]
                #[inline]
                #vis fn #init_fn(&self) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    #body
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
                // If a failure was recorded for this instruction, stamp the
                // receipt *before* emission so off-chain consumers can map the
                // code → invariant name via the program's ErrorRegistry.
                if let ::core::option::Option::Some((__hp_code, __hp_idx, __hp_stage)) = failure {
                    self.#receipt_field_name.set_failure(__hp_code, __hp_idx, __hp_stage);
                }
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

    // Precomputed signature / call-site fragments for the top-level
    // `validate` / `bind` entry points. Kept as one-shot `TokenStream`s
    // so the `quote! { ... }` block below stays readable. The leading
    // comma is only emitted when there are actually args to declare. // which is how we keep the args-less case byte-for-byte identical
    // to pre-instruction-args output.
    let top_arg_param_fragment = if has_instruction_args {
        let aps = arg_params.clone();
        quote! { , #(#aps),* }
    } else {
        TokenStream::new()
    };
    let top_arg_name_fragment = if has_instruction_args {
        let names = arg_names.clone();
        quote! { , #(#names),* }
    } else {
        TokenStream::new()
    };

    // ── Tooling surface for declared instruction args ─────────────────
    //
    // Expose the declared arg list as `(name, canonical_type)` pairs
    // so tooling (hopper-sdk, Codama, IDL generators) can see the
    // context's instruction-arg contract without re-parsing source.
    // The canonical-type rendering is best-effort: we stringify the
    // Rust type via `quote`, matching the same vocabulary the
    // `#[hopper::args]` derive already uses ("u64", "[u8; 32]", etc.).
    //
    // Emitted as a `pub const CONTEXT_ARGS: &[(&str, &str)]` on the
    // impl block (see the `quote!` block below). We keep this off
    // `ContextDescriptor` for now so this change remains purely
    // additive. the schema crate can grow a dedicated field in a
    // future pass without breaking the runtime ABI here.
    let context_arg_entries: Vec<TokenStream> = instruction_args
        .iter()
        .map(|a| {
            let n = a.name.to_string();
            let ty = &a.ty;
            let t = quote!(#ty).to_string();
            quote! { (#n, #t) }
        })
        .collect();

    // ── Bumps struct (Anchor-parity ergonomic) ─────────────────────
    //
    // For every field with a `seeds = ...` constraint, emit a `u8`
    // slot on `<Name>Bumps` and populate it during `bind()`. The
    // resulting struct is reachable as `ctx.bumps()` on the bound
    // context, which is exactly what a CPI signer-seeds block wants:
    //
    //   let bumps = vault_ctx.bumps();
    //   let seeds: &[&[u8]] = &[b"vault", authority.as_ref(), &[bumps.vault]];
    //
    // Contexts with zero PDAs still get a unit-ish `struct <Name>Bumps {}`
    // so downstream code can spell the type unconditionally. `#[derive]`
    // is split: `Default` is always on (so construction is trivial),
    // `Copy / Clone / Debug` only when at least one field exists (an
    // empty-fields struct still derives them cleanly, so emit both
    // paths identically for simplicity).
    let bumps_name = format_ident!("{}Bumps", name);
    let bumps_field_defs: Vec<TokenStream> = bump_entries
        .iter()
        .map(|(ident, _)| quote! { pub #ident: u8, })
        .collect();
    let bumps_gather_stmts: Vec<TokenStream> = bump_entries
        .iter()
        .map(|(ident, expr)| quote! { __hopper_bumps.#ident = #expr; })
        .collect();
    let bumps_registry_entries: Vec<TokenStream> = bump_entries
        .iter()
        .map(|(ident, _)| {
            let s = ident.to_string();
            quote! { #s }
        })
        .collect();

    // Emit the user-validate call only when the author opted in.
    // The call is spelled `<Bound>::validate(&bound)` so a user who
    // forgets to define the method sees a clean "no method named
    // `validate`" error pointing at their own impl block, not at
    // macro-generated code.
    let user_validate_call: TokenStream = if user_validate {
        quote! {
            #bound_name::validate(&__hopper_bound)?;
        }
    } else {
        TokenStream::new()
    };

    // When called from `#[derive(Accounts)]` the struct already exists in
    // the user's source. Skip re-emitting it — emitting twice would be a
    // duplicate-definition error. When called from `#[hopper::context]`
    // we keep the original passthrough since attribute macros own the
    // item they decorate.
    let original_struct: TokenStream = if emit_struct {
        quote! { #input }
    } else {
        TokenStream::new()
    };

    let expanded = quote! {
        // Emit the original struct unchanged (attribute macro path only).
        #original_struct

        /// Captured PDA bumps for every `seeds = ...` field in this
        /// context. One `u8` slot per PDA, named after the field. Read
        /// from the bound context as `ctx.bumps().<field>` and hand
        /// straight to a CPI signer-seeds block.
        ///
        /// Anchor's `ctx.bumps.<field>` pattern, spelled out: the field
        /// set is derived at macro-expansion time from the fields that
        /// carry `seeds`, so there is zero runtime lookup and zero
        /// allocation. A context with no PDA fields still gets a valid
        /// type (empty body) so downstream code can name it uniformly.
        #[derive(::core::default::Default, ::core::clone::Clone, ::core::marker::Copy, ::core::fmt::Debug)]
        #vis struct #bumps_name {
            #( #bumps_field_defs )*
        }

        impl #bumps_name {
            /// Field names that carry a PDA bump, in declaration order.
            /// Lets off-chain tooling iterate the PDA slot set without
            /// needing reflection or a JSON descriptor.
            pub const FIELDS: &'static [&'static str] = &[
                #( #bumps_registry_entries ),*
            ];
        }

        #vis struct #bound_name<'ctx, 'a> {
            ctx: &'ctx mut ::hopper::prelude::Context<'a>,
            bumps: #bumps_name,
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

            /// Declared instruction-arg bindings for this context, as
            /// `(name, canonical_type)` pairs in the order given to
            /// `#[instruction(...)]`. Empty when no args were declared.
            ///
            /// Tooling (hopper-sdk, IDL / Codama projectors, client
            /// generators) consumes this slice directly rather than
            /// re-parsing the source. the same contract Anchor's
            /// `#[derive(Accounts)] #[instruction(...)]` exposes, but
            /// backed by real typed Rust bindings so a mismatch is a
            /// compile error, not a runtime surprise.
            pub const CONTEXT_ARGS: &'static [(&'static str, &'static str)] = &[
                #( #context_arg_entries ),*
            ];

            // ── Per-field validators ─────────────────────────────────
            //
            // Each field gets its own `validate_{name}()` so the checks
            // are individually callable, testable, and visible in
            // `hopper compile --emit rust` output.
            //
            // When the struct declares `#[instruction(...)]`, each
            // per-field validator takes the declared args as ordinary
            // parameters. the same mechanism Anchor users expect,
            // but threaded through *typed* Rust bindings rather than
            // free identifiers that happen to resolve to the right
            // thing.
            #(#per_field_validators)*

            /// Validate the account slice against this context spec.
            ///
            /// This calls each per-field validator in order. Every check
            /// is also available as a standalone `validate_{field}()` method
            /// for fine-grained control and testing.
            ///
            /// When the struct declares `#[instruction(...)]`, this
            /// entry point is renamed to `validate_with_args(...)` and
            /// carries the declared typed args as additional
            /// parameters. the args-less `validate(...)` is **not**
            /// emitted in that case, because any seed / constraint
            /// expression referencing an instruction arg would not
            /// compile without the binding in scope.
            #[inline]
            pub fn #top_validate_ident(
                ctx: &::hopper::prelude::Context<'_>
                #top_arg_param_fragment
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                ctx.require_accounts(Self::ACCOUNT_COUNT)?;
                #(#validation_stmts)*
                Ok(())
            }

            /// Bind a raw Hopper context into the typed proc-macro wrapper.
            ///
            /// Mirrors `validate`: when `#[instruction(...)]` is
            /// declared at the struct level, this becomes
            /// `bind_with_args(ctx, arg0, arg1, ...)` and the args-less
            /// variant is omitted.
            #[inline]
            pub fn #top_bind_ident<'ctx, 'a>(
                ctx: &'ctx mut ::hopper::prelude::Context<'a>
                #top_arg_param_fragment
            ) -> ::core::result::Result<#bound_name<'ctx, 'a>, ::hopper::__runtime::ProgramError> {
                Self::#top_validate_ident(ctx #top_arg_name_fragment)?;
                // `validate` already proved every PDA matches its seeds,
                // so each gather expression can assume the derivation
                // will produce the same pubkey. For stored bumps this
                // is a byte read; for inferred bumps it is a second
                // `find_program_address` call, which is the cost of
                // handing the caller a ready-to-use bump without
                // them re-deriving it at the CPI site. Stored bumps
                // are the recommended path in hot handlers.
                let mut __hopper_bumps = <#bumps_name as ::core::default::Default>::default();
                #( #bumps_gather_stmts )*
                let __hopper_bound = #bound_name { ctx, bumps: __hopper_bumps };
                #user_validate_call
                Ok(__hopper_bound)
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
            /// Seal and emit every receipt tracked by this scope.
            ///
            /// `failure` carries `(error_code, invariant_idx, stage)` when
            /// a guard or invariant failed during the handler; it is
            /// stamped into every mutable account's receipt so the
            /// off-chain SDK can resolve the failure to a named
            /// invariant via the program's `ErrorRegistry`. Pass `None`
            /// on the success path.
            #[inline]
            #vis fn finish(
                mut self,
                ctx: &::hopper::prelude::Context<'_>,
                invariants_passed: bool,
                invariants_checked: u16,
                failure: ::core::option::Option<(
                    u32,
                    u8,
                    ::hopper::prelude::FailureStage,
                )>,
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

            /// Captured PDA bumps for every `seeds = ...` field.
            ///
            /// Returns a reference, not a copy, so the type can grow
            /// fields later without forcing existing call sites to
            /// update. Hand straight to a CPI signer-seeds block:
            ///
            /// ```ignore
            /// let bumps = ctx.bumps();
            /// let signer_seeds: &[&[u8]] = &[
            ///     b"vault",
            ///     authority_key.as_ref(),
            ///     ::core::slice::from_ref(&bumps.vault),
            /// ];
            /// ```
            #[inline(always)]
            #vis fn bumps(&self) -> &#bumps_name {
                &self.bumps
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
            // Handle double-segment paths first (`token::mint`,
            // `mint::authority`, `associated_token::mint`,
            // `seeds::program`). These are Anchor's established
            // vocabulary for SPL-specific constraints; accepting
            // them by the same spelling makes Anchor programs a
            // mechanical port to Hopper rather than a rewrite.
            // Three-segment Token-2022 extension paths:
            // `extensions::transfer_hook::authority = X`, etc.
            if meta.path.segments.len() == 3
                && meta.path.segments[0].ident == "extensions"
            {
                let group = meta.path.segments[1].ident.to_string();
                let field = meta.path.segments[2].ident.to_string();
                return match (group.as_str(), field.as_str()) {
                    ("mint_close_authority", "authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_mint_close_authority = Some(expr);
                        Ok(())
                    }
                    ("permanent_delegate", "delegate") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_permanent_delegate = Some(expr);
                        Ok(())
                    }
                    ("transfer_hook", "authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_transfer_hook_authority = Some(expr);
                        Ok(())
                    }
                    ("transfer_hook", "program_id") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_transfer_hook_program = Some(expr);
                        Ok(())
                    }
                    ("metadata_pointer", "authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_metadata_pointer_authority = Some(expr);
                        Ok(())
                    }
                    ("metadata_pointer", "metadata_address") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_metadata_pointer_address = Some(expr);
                        Ok(())
                    }
                    ("default_account_state", "state") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_default_account_state = Some(expr);
                        Ok(())
                    }
                    ("interest_bearing", "rate_authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_interest_bearing_authority = Some(expr);
                        Ok(())
                    }
                    ("transfer_fee_config", "authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_transfer_fee_config_authority = Some(expr);
                        Ok(())
                    }
                    ("transfer_fee_config", "withdraw_withheld_authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.ext_transfer_fee_withdraw_authority = Some(expr);
                        Ok(())
                    }
                    _ => Err(meta.error(format!(
                        "unrecognized extension constraint `extensions::{group}::{field}`. \
                         accepted: extensions::{{mint_close_authority,permanent_delegate,transfer_hook,metadata_pointer,default_account_state,interest_bearing,transfer_fee_config}}::*",
                    ))),
                };
            }

            if meta.path.segments.len() == 2 {
                let ns = meta.path.segments[0].ident.to_string();
                let key = meta.path.segments[1].ident.to_string();
                return match (ns.as_str(), key.as_str()) {
                    ("token", "mint") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.token_mint = Some(expr);
                        Ok(())
                    }
                    ("token", "authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.token_authority = Some(expr);
                        Ok(())
                    }
                    ("token", "token_program") => {
                        // Anchor-parity lever for Token-2022 routing.
                        // Without this, a `token::mint` / `token::authority`
                        // check validates the *content* of the token
                        // account but not which token program owns it.
                        // Setting `token::token_program = TOKEN_2022_ID`
                        // binds the account to Token-2022 so a legacy
                        // Token account pasted into the same slot is
                        // rejected before any byte-level check runs.
                        let expr: Expr = meta.value()?.parse()?;
                        result.token_token_program = Some(expr);
                        Ok(())
                    }
                    ("mint", "authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.mint_authority = Some(expr);
                        Ok(())
                    }
                    ("mint", "decimals") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.mint_decimals = Some(expr);
                        Ok(())
                    }
                    ("mint", "freeze_authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.mint_freeze_authority = Some(expr);
                        Ok(())
                    }
                    ("mint", "token_program") => {
                        // Mint-axis twin of `token::token_program`. Lets
                        // a program assert that a Mint account is owned
                        // by Token-2022 (or any specific program) before
                        // trusting its layout bytes.
                        let expr: Expr = meta.value()?.parse()?;
                        result.mint_token_program = Some(expr);
                        Ok(())
                    }
                    ("associated_token", "mint") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.associated_token_mint = Some(expr);
                        Ok(())
                    }
                    ("associated_token", "authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.associated_token_authority = Some(expr);
                        Ok(())
                    }
                    ("associated_token", "token_program") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.associated_token_token_program = Some(expr);
                        Ok(())
                    }
                    ("seeds", "program") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.seeds_program = Some(expr);
                        Ok(())
                    }
                    // Token-2022 extension constraints. Three-segment
                    // paths (extensions::foo::bar) are routed via the
                    // fall-through below; two-segment `extensions::foo`
                    // flags (non_transferable, immutable_owner) hit here.
                    ("extensions", "non_transferable") => {
                        result.ext_non_transferable = true;
                        Ok(())
                    }
                    ("extensions", "immutable_owner") => {
                        result.ext_immutable_owner = true;
                        Ok(())
                    }
                    _ => Err(meta.error(format!(
                        "unrecognized nested account attribute `{ns}::{key}`. \
                         accepted namespaces: token::{{mint,authority,token_program}}, \
                         mint::{{authority,decimals,freeze_authority,token_program}}, \
                         associated_token::{{mint,authority,token_program}}, \
                         seeds::{{program}}",
                    ))),
                };
            }

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
                "init_if_needed" => {
                    // Anchor-parity. Like `init` but the lifecycle
                    // helper skips the CreateAccount CPI when the
                    // account already has non-zero data. Same
                    // implication: `mut` is required. Doesn't imply
                    // `init` because the two flags emit different
                    // lifecycle-helper bodies.
                    result.init_if_needed = true;
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
                "seeds_fn" => {
                    // `seeds_fn = Type::seeds(&arg1, &arg2)`
                    // One expression evaluating to a slice-of-byte-slices
                    // (or anything that coerces to `&[&[u8]]`). The
                    // type author owns the seed layout; every context
                    // reuses it.
                    let expr: Expr = meta.value()?.parse()?;
                    result.seeds_fn = Some(expr);
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
                "dup" => {
                    let ident: Ident = meta.value()?.parse()?;
                    if result.dup.is_some() {
                        return Err(meta.error("`dup` may only be set once per field"));
                    }
                    result.dup = Some(ident);
                    Ok(())
                }
                "sweep" => {
                    let ident: Ident = meta.value()?.parse()?;
                    if result.sweep.is_some() {
                        return Err(meta.error("`sweep` may only be set once per field"));
                    }
                    result.sweep = Some(ident);
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
                "executable" => {
                    result.executable = true;
                    Ok(())
                }
                "rent_exempt" => {
                    // `rent_exempt = enforce` or `rent_exempt = skip`.
                    // Accept both as plain idents (the canonical Anchor
                    // spelling). Anything else is rejected so typos
                    // don't silently degrade to a no-op.
                    let policy: Ident = meta.value()?.parse()?;
                    match policy.to_string().as_str() {
                        "enforce" => result.rent_exempt = Some(RentExemptPolicy::Enforce),
                        "skip" => result.rent_exempt = Some(RentExemptPolicy::Skip),
                        other => {
                            return Err(meta.error(format!(
                                "rent_exempt must be `enforce` or `skip`, got `{}`",
                                other
                            )));
                        }
                    }
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
    if attr.init && attr.init_if_needed {
        return Err(syn::Error::new_spanned(
            field_name,
            "use either `init` or `init_if_needed`, not both",
        ));
    }
    if attr.init || attr.init_if_needed {
        let kw = if attr.init_if_needed { "init_if_needed" } else { "init" };
        if attr.payer.is_none() {
            return Err(syn::Error::new_spanned(
                field_name,
                format!("#[account({})] requires `payer = <field>`", kw),
            ));
        }
        if attr.space.is_none() {
            return Err(syn::Error::new_spanned(
                field_name,
                format!("#[account({})] requires `space = <expr>`", kw),
            ));
        }
        if attr.seeds.is_some() && attr.bump.is_none() {
            return Err(syn::Error::new_spanned(
                field_name,
                format!(
                    "#[account({}, seeds = ...)] requires `bump` (inferred) or `bump = <stored_byte>`",
                    kw
                ),
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
    // `seeds::program = X` only makes sense when `seeds = [...]` is
    // declared. otherwise there's no PDA derivation to redirect.
    if attr.seeds_program.is_some() && attr.seeds.is_none() {
        return Err(syn::Error::new_spanned(
            field_name,
            "#[account(seeds::program = ...)] requires `seeds = [...]`",
        ));
    }
    // Associated-token pair coherence. the mint/authority inputs are
    // joint input to the ATA PDA derivation and declaring just one
    // would produce an ATA derivation with a missing dimension.
    // Rather than silently skip the check, we raise a compile error
    // pointing at the field with an actionable message.
    match (
        attr.associated_token_mint.is_some(),
        attr.associated_token_authority.is_some(),
    ) {
        (true, false) => {
            return Err(syn::Error::new_spanned(
                field_name,
                "#[account(associated_token::mint = ...)] also requires `associated_token::authority = ...`",
            ));
        }
        (false, true) => {
            return Err(syn::Error::new_spanned(
                field_name,
                "#[account(associated_token::authority = ...)] also requires `associated_token::mint = ...`",
            ));
        }
        _ => {}
    }
    // `associated_token::token_program` only has meaning alongside
    // the derivation pair. on its own it configures nothing.
    if attr.associated_token_token_program.is_some()
        && attr.associated_token_mint.is_none()
        && attr.associated_token_authority.is_none()
    {
        return Err(syn::Error::new_spanned(
            field_name,
            "#[account(associated_token::token_program = ...)] requires `associated_token::mint = ...` and `associated_token::authority = ...`",
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

// -----------------------------------------------------------------------------
// Regression tests
// -----------------------------------------------------------------------------
//
// The proc-macro expansion path itself is best exercised through a
// downstream trybuild suite (see `tests/context/ui/*.rs`). These unit
// tests target the pure-function helpers that don't require spawning
// a fresh `rustc` invocation. `parse_instruction_attr` is one of the
// more fragile pieces because it combines attribute-walking with a
// hand-rolled `Parse` impl for `name: Type` pairs, so it gets the
// lion's share of coverage here.
#[cfg(test)]
mod instruction_arg_tests {
    use super::*;
    use quote::ToTokens;
    use syn::{parse_quote, ItemStruct};

    fn args_of(mut s: ItemStruct) -> Vec<(String, String)> {
        let decls = parse_instruction_attr(&mut s.attrs).expect("parse ok");
        decls
            .into_iter()
            .map(|a| (a.name.to_string(), a.ty.to_token_stream().to_string()))
            .collect()
    }

    #[test]
    fn parses_single_primitive_arg() {
        let input: ItemStruct = parse_quote! {
            #[instruction(amount: u64)]
            pub struct Swap {}
        };
        let out = args_of(input);
        assert_eq!(out, vec![("amount".into(), "u64".into())]);
    }

    #[test]
    fn parses_multiple_args_including_array() {
        let input: ItemStruct = parse_quote! {
            #[instruction(nonce: u64, memo: [u8; 32], kind: u8)]
            pub struct Swap {}
        };
        let out = args_of(input);
        // We verify count + names + scalar types exactly. the array
        // type's stringified form is quote-spacing-dependent, so for
        // that one we just check that both `u8` and `32` appear in
        // the rendered token stream.
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].0, "nonce");
        assert_eq!(out[1].0, "memo");
        assert_eq!(out[2].0, "kind");
        assert_eq!(out[0].1, "u64");
        assert!(out[1].1.contains("u8"));
        assert!(out[1].1.contains("32"));
        assert_eq!(out[2].1, "u8");
    }

    #[test]
    fn rejects_duplicate_arg_names() {
        let mut input: ItemStruct = parse_quote! {
            #[instruction(amount: u64, amount: u128)]
            pub struct Swap {}
        };
        let err = parse_instruction_attr(&mut input.attrs).expect_err("expected error");
        let msg = err.to_string();
        assert!(msg.contains("duplicate"), "got: {msg}");
        assert!(msg.contains("amount"), "got: {msg}");
    }

    #[test]
    fn rejects_multiple_instruction_attributes() {
        let mut input: ItemStruct = parse_quote! {
            #[instruction(amount: u64)]
            #[instruction(extra: u8)]
            pub struct Swap {}
        };
        let err = parse_instruction_attr(&mut input.attrs).expect_err("expected error");
        let msg = err.to_string();
        assert!(msg.contains("at most one"), "got: {msg}");
    }

    #[test]
    fn empty_on_struct_without_instruction_attr() {
        let input: ItemStruct = parse_quote! {
            pub struct NoArgs {}
        };
        let out = args_of(input);
        assert!(out.is_empty());
    }

    /// `#[derive(Accounts)]` mirrors `#[hopper::context]` exactly except
    /// it does NOT re-emit the user's input struct (the user already
    /// declared it). This pins the flag plumbing: we lower the same
    /// constraint surface to a binder type but skip the struct
    /// passthrough. If this test starts asserting `pub struct Deposit`
    /// in the derive output, the duplicate-definition guard regressed.
    #[test]
    fn derive_does_not_reemit_struct_definition() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Deposit {
                #[signer]
                pub authority: AccountView,
            }
        };
        let derived = expand_for_derive(item).expect("derive expand ok");
        let s = derived.to_string();
        // Generated items still include the binder.
        assert!(
            s.contains("DepositCtx"),
            "derive output missing the bound context type: {s}"
        );
        // But the input struct itself is NOT in the emitted token stream
        // — that would be a duplicate definition once the user's
        // declaration compiles.
        assert!(
            !s.contains("pub struct Deposit "),
            "derive must not re-emit the user's struct: {s}"
        );
    }

    /// And the attribute form keeps emitting the struct, since it owns
    /// the item it decorates. This is the existing contract the rest
    /// of the codebase depends on.
    #[test]
    fn attr_does_reemit_struct_definition() {
        let item: TokenStream = quote! {
            pub struct Deposit {
                #[signer]
                pub authority: AccountView,
            }
        };
        let attr = expand(TokenStream::new(), item).expect("attr expand ok");
        let s = attr.to_string();
        assert!(
            s.contains("pub struct Deposit"),
            "attribute form must re-emit the input struct: {s}"
        );
        assert!(
            s.contains("DepositCtx"),
            "attribute form missing the bound context type: {s}"
        );
    }

    #[test]
    fn strips_attribute_in_place() {
        let mut input: ItemStruct = parse_quote! {
            #[instruction(nonce: u64)]
            #[derive(Clone)]
            pub struct Keep {}
        };
        let _ = parse_instruction_attr(&mut input.attrs).expect("parse ok");
        // After parsing, the #[instruction(...)] attr is removed but
        // other outer attributes (#[derive(Clone)], etc.) are kept. // the emitted struct therefore retains whatever derives the
        // user declared.
        assert!(
            input
                .attrs
                .iter()
                .all(|a| !a.path().is_ident("instruction")),
            "instruction attr was not stripped"
        );
        assert!(
            input.attrs.iter().any(|a| a.path().is_ident("derive")),
            "non-instruction attrs must be preserved"
        );
    }

    #[test]
    fn rejects_positional_form() {
        // `#[instruction(u64)]` (positional, no name) is rejected because
        // seed / constraint expressions need a named binding to refer
        // to. Anchor accepts the positional form but the generated
        // code is harder to read and impossible to regenerate
        // consistently for client tooling.
        let mut input: ItemStruct = parse_quote! {
            #[instruction(u64)]
            pub struct Bad {}
        };
        let err = parse_instruction_attr(&mut input.attrs).expect_err("expected error");
        // The underlying syn error comes from the `:` parser failing
        // once it consumes the type without finding a colon.
        let _ = err;
    }
}
