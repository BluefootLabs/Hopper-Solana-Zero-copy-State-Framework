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
    parse::{Parse, ParseStream, Parser},
    parse2,
    punctuated::Punctuated,
    token::Comma,
    Attribute, Expr, Fields, GenericParam, Ident, ItemStruct, Result, Token, Type, TypePath,
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
    /// Growable `Seq<T>` tail segment names (from `tail(field)`). Each
    /// lowers to an OPEN-ENDED `WriteRange::tail_from` grant: the whole
    /// tail region is writable and growable, while the fixed head stays
    /// byte-protected and CPI writable-meta delegation stays refused
    /// (an open tail range starting past the head is not a whole-account
    /// grant). A tail-declared field is writable and may carry `realloc`.
    tail_segments: Vec<String>,

    // ── Anchor-grade declarative constraints (audit ST2) ────────────
    /// `init`. account must be created fresh this instruction.
    /// Requires `payer` and `space`; implies `mut`. PDA-init also
    /// requires `seeds` + `bump`.
    init: bool,
    /// `init_if_needed`. Anchor-parity sibling of `init`. Empty account slots
    /// are allowed through binding so the handler can create them with the
    /// generated lifecycle helper. Nonempty slots are owner-checked and
    /// layout-checked during binding before the handler receives a typed
    /// account wrapper.
    init_if_needed: bool,
    /// Per-field opt-in for bind-time lifecycle execution.
    auto_lifecycle: bool,
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
    /// Parallel to `has_one`: the optional Anchor-style `@ CustomError`
    /// override for each `has_one` entry. `has_one_errs[i]` is the error
    /// named by `has_one = field @ Err`; `None` when that constraint
    /// used no `@`, in which case the generic error is emitted unchanged.
    has_one_errs: Vec<Option<Expr>>,
    /// `owner = expr`. require the account's owner equal `expr`.
    /// Default for layout fields is `ctx.program_id()`.
    owner: Option<Expr>,
    /// Optional `@ CustomError` for `owner = expr @ Err`. `None` keeps
    /// the generic owner-mismatch error unchanged.
    owner_err: Option<Expr>,
    /// `owner_any = [expr, ...]`. require the account's owner to equal *one of*
    /// the listed program ids. The motivating case is accepting an account from
    /// either SPL Token or Token-2022:
    /// `owner_any = [token::ID, token_2022::ID]`. Mutually exclusive with
    /// `owner`.
    owner_any: Vec<Expr>,
    /// `address = expr`. require the account's key equal `expr`.
    address: Option<Expr>,
    /// Optional `@ CustomError` for `address = expr @ Err`. `None` keeps
    /// the generic `InvalidAccountData` error unchanged.
    address_err: Option<Expr>,
    /// `constraint = expr`. arbitrary boolean guard, evaluated as the
    /// last step of validation.
    constraint: Vec<Expr>,
    /// Parallel to `constraint`: the optional Anchor-style `@ CustomError`
    /// override for each guard. `constraint_errs[i]` is the error named
    /// by `constraint = expr @ Err`; `None` keeps the generic
    /// `Custom(0xC000 | idx)` error for that guard unchanged.
    constraint_errs: Vec<Option<Expr>>,

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
    /// Optional `@ CustomError` for `token::mint = expr @ Err`. `None`
    /// keeps the generic `require_token_mint` error unchanged.
    token_mint_err: Option<Expr>,
    /// `token::authority = expr`. require this SPL TokenAccount's
    /// bytes `[32..64]` equal the pubkey produced by `expr`.
    token_authority: Option<Expr>,
    /// Optional `@ CustomError` for `token::authority = expr @ Err`.
    /// `None` keeps the generic `require_token_owner_eq` error unchanged.
    token_authority_err: Option<Expr>,
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
    ext_cpi_guard: bool,
    ext_confidential_transfer_mint: bool,
    ext_confidential_transfer_account: bool,
    ext_scaled_ui_amount_config: bool,
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

    // ── Metaplex Token Metadata constraints / CPI helpers ─────────────
    /// `metadata::name = expr`. Name for `CreateMetadataAccountV3`.
    metadata_name: Option<Expr>,
    /// `metadata::symbol = expr`. Symbol for `CreateMetadataAccountV3`.
    metadata_symbol: Option<Expr>,
    /// `metadata::uri = expr`. URI for `CreateMetadataAccountV3`.
    metadata_uri: Option<Expr>,
    /// `metadata::seller_fee_basis_points = expr`. Royalty basis points.
    metadata_seller_fee_basis_points: Option<Expr>,
    /// `metadata::is_mutable = expr`. Defaults to true when omitted.
    metadata_is_mutable: Option<Expr>,
    /// `metadata::mint = field`. Mint account used by the CPI helper.
    metadata_mint: Option<Ident>,
    /// `metadata::mint_authority = field`. Mint-authority signer.
    metadata_mint_authority: Option<Ident>,
    /// `metadata::payer = field`. Payer signer+writable account.
    metadata_payer: Option<Ident>,
    /// `metadata::update_authority = field`. Update-authority signer.
    metadata_update_authority: Option<Ident>,
    /// `metadata::system_program = field`. System Program account.
    metadata_system_program: Option<Ident>,
    /// `metadata::rent = field`. Optional rent sysvar account.
    metadata_rent: Option<Ident>,

    /// `master_edition::max_supply = expr`. Accepts u64 or Option<u64>.
    master_edition_max_supply: Option<Expr>,
    /// `master_edition::mint = field`. Mint account used by the CPI helper.
    master_edition_mint: Option<Ident>,
    /// `master_edition::metadata = field`. Metadata account sibling.
    master_edition_metadata: Option<Ident>,
    /// `master_edition::update_authority = field`. Update-authority signer.
    master_edition_update_authority: Option<Ident>,
    /// `master_edition::mint_authority = field`. Mint-authority signer.
    master_edition_mint_authority: Option<Ident>,
    /// `master_edition::payer = field`. Payer signer+writable account.
    master_edition_payer: Option<Ident>,
    /// `master_edition::token_program = field`. SPL Token program account.
    master_edition_token_program: Option<Ident>,
    /// `master_edition::system_program = field`. System Program account.
    master_edition_system_program: Option<Ident>,
    /// `master_edition::rent = field`. Optional rent sysvar account.
    master_edition_rent: Option<Ident>,

    /// `dup = other_field`. This slot is allowed to
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

    /// `migrate(from = OldLayout, with = path::to::transform)`. Lazy
    /// migration at bind: every instruction that touches this account
    /// becomes a migration crank. `(old_type, transform_path)`.
    ///
    /// `bind()` first checks — on a scoped read borrow — whether the
    /// slot still holds a **fully-valid** `OldLayout` header
    /// (`LayoutContract::validate_header`, the complete disc / version /
    /// layout_id / epoch identity, never a partial sniff). If it does,
    /// `::hopper::migration::migrate_layout::<Old, New, _>` rewrites the
    /// account in place through the typed transform BEFORE any context
    /// validator runs, so the normal New-layout validation sees the
    /// upgraded account. An already-current account skips the call; any
    /// other header falls through to the normal New-layout validation
    /// error, unchanged. `validate()` (the read-only standalone surface)
    /// accepts EITHER version without writing: the field's layout-header
    /// check becomes "valid New, or fully-valid Old that already fits
    /// the New shape" — exactly the sets `bind()` accepts.
    ///
    /// v1 restrictions (all compile errors): requires `mut` on the same
    /// field (a migration writes); not combinable with `init` /
    /// `init_if_needed` / `zero` / `close` / `realloc` / `sweep`, with
    /// `Option<..>` fields, or with `#[composite]` fields; `from` must
    /// name a different layout than the field's own. Because the
    /// pre-step lives in this context's own `bind()` — an outer
    /// `#[composite]` bind runs inner *validators*, never inner bind
    /// pre-steps — a context carrying a migrate field is NOT embeddable:
    /// it advertises `__HOPPER_EMBEDDABLE = false`, and embedding it is
    /// refused at compile time rather than silently skipping the
    /// migration. Resizing is out of scope: the New shape must already
    /// fit the existing allocation (`realloc` first when it does not).
    migrate: Option<(Type, syn::Path)>,

    /// `executable`. Anchor-parity keyword. Requires the account's
    /// `executable` flag to be true - i.e. it must be a deployed BPF
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

    /// `#[composite]` (bare marker). This field is a NESTED context: its
    /// type is another `#[derive(Accounts)]` / `#[hopper::context]` struct
    /// whose account slots flatten into this one in declaration order
    /// (Anchor's composite-accounts feature). The field is NOT a single
    /// account slot; it consumes `<Inner>::ACCOUNT_COUNT` slots. A
    /// composite field carries no per-account constraints of its own — its
    /// validation is the inner context's, run at the flattened offset.
    ///
    /// Detection is an explicit marker (not Anchor's auto-detect-by-
    /// non-wrapper) because Hopper already assigns meaning to a bare layout
    /// type in a context field ("an account of this layout"), so
    /// "not a known wrapper" is ambiguous here. The explicit marker keeps
    /// every existing layout-typed field valid and makes nesting opt-in.
    composite: bool,
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

/// Role of a macro-synthesized (auto-appended) context field.
///
/// `#[hopper::context(event_cpi)]` appends the two Anchor-parity
/// trailing accounts without the author declaring them. Synthetic
/// fields are real account slots — they count toward `ACCOUNT_COUNT`,
/// get per-field validators, accessors, schema entries, and (for the
/// authority) a `Bumps` slot — but they are NOT fields of the user's
/// struct, so the typed `accounts` facade must skip them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SyntheticFieldRole {
    /// The event-authority PDA (seeds `[b"__hopper_event_authority"]`).
    /// Verified at bind via the runtime's sha256 verify loop (on-chain);
    /// its bump lands on the generated `Bumps` struct so
    /// `emit_event_cpi` signs the self-CPI with a stored bump.
    EventAuthority,
    /// The program's own account (`address == ctx.program_id()`),
    /// required in the instruction so the runtime can resolve the
    /// self-CPI target. Validated with the plain `address = ...`
    /// lowering.
    EventProgram,
}

/// Parsed context field.
struct ContextField {
    name: Ident,
    ty: Type,
    attr: AccountAttr,
    index: usize,
    /// `Some(role)` when this field was auto-appended by a context
    /// option rather than declared by the author. See
    /// [`SyntheticFieldRole`].
    synthetic: Option<SyntheticFieldRole>,
}

struct AccountsBindingFragments {
    field_decl: TokenStream,
    init_stmt: TokenStream,
    bound_field: TokenStream,
}

/// A single `name: Type` binding inside a struct-level
/// `#[instruction(...)]` attribute.
///
/// ## Design notes
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
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let name: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;
        Ok(Self { name, ty })
    }
}

#[derive(Default)]
struct ContextOptions {
    /// Opt-in Anchor-like lifecycle execution. When true, bind runs generated
    /// init/realloc/close helpers in field declaration order after built-in
    /// validation and before returning the bound context.
    auto_lifecycle: bool,
    /// Innovation I12: compile the context's `mut` / `mut(seg, ...)` /
    /// lifecycle declarations into a `static` [`WritePolicy`] installed on
    /// the raw context during `bind()`. Every Context-mediated write
    /// acquire outside the declared set then fails with
    /// `Custom(0xD000 | account_index)` at acquisition time — Sealevel's
    /// account-level `writable` flag, enforced at byte-range granularity.
    strict_writes: bool,
    /// BLD-MUT: the declared **lamport dimension** of the write set.
    ///
    /// `Some(fields)` when the context carries `lamports(field, ...)`
    /// (requires `strict_writes`): the named fields — plus the implied
    /// lifecycle set (whole-`mut` accounts, init account + payer, close
    /// account + destination, realloc account + payer, sweep account +
    /// target) — are the ONLY accounts whose lamports the instruction
    /// may mutate; the runtime refuses everything else at the lamport
    /// choke points, making the write set mutation-complete.
    /// `lamports()` (empty list) is valid: only the implied lifecycle
    /// set may move lamports.
    ///
    /// `None` (the default) leaves the dimension undeclared: lamport
    /// mutation stays ungoverned exactly as before BLD-MUT, and the
    /// context is NOT mutation-complete. The dimension is opt-in
    /// because retroactively refusing lamport writes on existing
    /// `strict_writes` programs would silently change deployed
    /// behavior on upgrade.
    lamports: Option<Vec<Ident>>,
    /// Innovation I7: opt-in self-describing transactions. When `true`
    /// (the context carries `emit_touch_map`) the context advertises
    /// `EMIT_TOUCH_MAP = true` as a public associated const. The
    /// generated dispatcher reads that const on the handler's **Ok** path
    /// and, only then, calls `Context::finish_with_touch_map()`, which
    /// emits the instruction's cumulative touch map as a single
    /// `sol_log_data` record — **only** when the downstream build also
    /// enables hopper-runtime's `touch-map` feature; with the feature off
    /// the helper is a no-op, so the generated call emits nothing.
    ///
    /// The emit is routed through the dispatcher (not a `Drop`) precisely
    /// because only the dispatcher can see the handler's `Result`: a
    /// `Drop` would run on every scope exit — including `?`/`Err` returns —
    /// and would emit a misleading record advertising Write ranges for a
    /// failed, rolled-back instruction (CONFIRMED P2). Routing on the Ok
    /// path makes the record fire exclusively on success.
    ///
    /// `false` (the default) sets `EMIT_TOUCH_MAP = false`, so the
    /// dispatcher's `const`-guarded call is dead-code-eliminated to zero
    /// instructions and nothing is ever emitted. Opt-in because each emit
    /// costs a `sol_log_data` record (compute + log budget) and must not
    /// tax handlers that did not ask for it.
    emit_touch_map: bool,
    /// Self-CPI events with Anchor's `#[event_cpi]` ergonomics. When
    /// `true` the macro auto-appends two trailing account slots — the
    /// event-authority PDA (seeds `[b"__hopper_event_authority"]`,
    /// verified at bind via the runtime sha256 verify loop, bump
    /// captured on the `Bumps` struct) and the program's own account
    /// (`address == ctx.program_id()`) — WITHOUT the author declaring
    /// them: `ACCOUNT_COUNT` grows by 2, both slots get validators,
    /// accessors, and schema entries, and the bound context gains
    /// `emit_event_cpi(&event)`, which encodes
    /// `[0xE0, 0x1E, tag, payload]` and self-invokes with the
    /// event-authority signer seeds.
    ///
    /// The spec type also advertises `EVENT_CPI = true` as a public
    /// associated const; the `#[hopper::program]` dispatcher ORs that
    /// const across its typed contexts to decide (at compile time)
    /// whether the reserved `[0xE0, 0x1E]` sink arm is live.
    ///
    /// `false` (the default) appends nothing, emits no method, and sets
    /// `EVENT_CPI = false`, so the dispatcher's sink guard
    /// dead-code-eliminates and non-participating programs pay zero.
    event_cpi: bool,
}

/// Parse the field list of a `lamports(field, ...)` option into the
/// options struct, merging with any previously declared list so the
/// option composes across the attribute forms. An empty list is valid
/// (only the implied lifecycle set may move lamports).
fn merge_lamports_fields(options: &mut ContextOptions, fields: Vec<Ident>) {
    match &mut options.lamports {
        Some(existing) => {
            for f in fields {
                if !existing.iter().any(|e| *e == f) {
                    existing.push(f);
                }
            }
        }
        None => options.lamports = Some(fields),
    }
}

fn parse_context_options(attr: TokenStream, attrs: &mut Vec<Attribute>) -> Result<ContextOptions> {
    let mut options = ContextOptions::default();

    if !attr.is_empty() {
        // `lamports(field, ...)` is a list, not a bare ident, so the
        // option list parses as `Meta` items.
        let parsed: Punctuated<syn::Meta, Comma> =
            Punctuated::<syn::Meta, Comma>::parse_terminated.parse2(attr)?;
        for meta in parsed {
            if meta.path().is_ident("auto_lifecycle") {
                options.auto_lifecycle = true;
            } else if meta.path().is_ident("strict_writes") {
                options.strict_writes = true;
            } else if meta.path().is_ident("emit_touch_map") {
                options.emit_touch_map = true;
            } else if meta.path().is_ident("event_cpi") {
                options.event_cpi = true;
            } else if meta.path().is_ident("lamports") {
                let list = meta.require_list()?;
                let fields: Punctuated<Ident, Comma> =
                    list.parse_args_with(Punctuated::<Ident, Comma>::parse_terminated)?;
                merge_lamports_fields(&mut options, fields.into_iter().collect());
            } else {
                return Err(syn::Error::new_spanned(
                    meta,
                    "unknown Hopper context option; supported: `auto_lifecycle`, \
                     `strict_writes`, `emit_touch_map`, `event_cpi`, `lamports(field, ...)`",
                ));
            }
        }
    }

    let mut retained = Vec::with_capacity(attrs.len());
    for attr in attrs.drain(..) {
        if attr.path().is_ident("accounts") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("auto_lifecycle") {
                    options.auto_lifecycle = true;
                    Ok(())
                } else if meta.path.is_ident("strict_writes") {
                    options.strict_writes = true;
                    Ok(())
                } else if meta.path.is_ident("emit_touch_map") {
                    options.emit_touch_map = true;
                    Ok(())
                } else if meta.path.is_ident("event_cpi") {
                    options.event_cpi = true;
                    Ok(())
                } else if meta.path.is_ident("lamports") {
                    let mut fields = Vec::new();
                    meta.parse_nested_meta(|inner| match inner.path.get_ident() {
                        Some(ident) => {
                            fields.push(ident.clone());
                            Ok(())
                        }
                        None => Err(inner.error("`lamports(...)` takes field names")),
                    })?;
                    merge_lamports_fields(&mut options, fields);
                    Ok(())
                } else {
                    Err(meta.error(
                        "unknown #[accounts(...)] option; supported: auto_lifecycle, \
                         strict_writes, emit_touch_map, event_cpi, lamports(field, ...)",
                    ))
                }
            })?;
            continue;
        }

        if attr.path().is_ident("account") {
            let mut only_context_options = true;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("auto_lifecycle") {
                    options.auto_lifecycle = true;
                    Ok(())
                } else if meta.path.is_ident("strict_writes") {
                    options.strict_writes = true;
                    Ok(())
                } else if meta.path.is_ident("emit_touch_map") {
                    options.emit_touch_map = true;
                    Ok(())
                } else if meta.path.is_ident("event_cpi") {
                    options.event_cpi = true;
                    Ok(())
                } else if meta.path.is_ident("lamports") {
                    let mut fields = Vec::new();
                    meta.parse_nested_meta(|inner| match inner.path.get_ident() {
                        Some(ident) => {
                            fields.push(ident.clone());
                            Ok(())
                        }
                        None => Err(inner.error("`lamports(...)` takes field names")),
                    })?;
                    merge_lamports_fields(&mut options, fields);
                    Ok(())
                } else {
                    only_context_options = false;
                    Ok(())
                }
            })?;
            if only_context_options {
                continue;
            }
        }

        retained.push(attr);
    }
    *attrs = retained;

    if options.lamports.is_some() && !options.strict_writes {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`lamports(...)` declares the lamport dimension of a strict write set; \
             it requires `strict_writes` on the same context",
        ));
    }

    Ok(options)
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

        let parsed: Punctuated<InstructionArgDecl, Comma> =
            attr.parse_args_with(Punctuated::<InstructionArgDecl, Comma>::parse_terminated)?;
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
pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    expand_inner(attr, item, /* emit_struct */ true)
}

/// Public entry point for the `#[derive(Accounts)]` proc-macro derive.
///
/// Functionally identical to [`expand`], except the original input struct
/// is **not** re-emitted (the user already declared it themselves). Helper
/// attributes - `#[account(...)]`, `#[signer]`, `#[instruction(...)]`,
/// `#[validate]` - are still parsed off the input but cannot be stripped
/// in place because the struct is not under our attribute. We rely on the
/// `attributes(...)` declaration on the derive macro to silence the
/// compiler's "unknown attribute" check; the helpers are dropped from the
/// final compilation unit by `rustc` once all derives have run.
pub fn expand_for_derive(item: TokenStream) -> Result<TokenStream> {
    expand_inner(TokenStream::new(), item, /* emit_struct */ false)
}

fn expand_inner(attr: TokenStream, item: TokenStream, emit_struct: bool) -> Result<TokenStream> {
    let mut input: ItemStruct = parse2(item)?;
    let context_options = parse_context_options(attr, &mut input.attrs)?;

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
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
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
        reject_reference_wrapped_account(&field_name, &field_ty)?;
        let attr = parse_account_attr(&field.attrs)?;
        if attr.composite {
            // A composite field is a nested context, not a single account
            // slot: it must not also carry `#[account(...)]` / `#[signer]`
            // constraints. Those belong on the inner context's own fields
            // and would be silently dropped here.
            let has_slot_attrs = field
                .attrs
                .iter()
                .any(|a| a.path().is_ident("account") || a.path().is_ident("signer"));
            validate_composite_field(&field_name, &field_ty, has_slot_attrs)?;
        } else {
            validate_account_attr(&field_name, &attr)?;
            validate_optional_field(&field_name, &field_ty, &attr)?;
            validate_migrate_field(&field_name, &field_ty, &attr)?;
            if (!attr.mut_segments.is_empty()
                || !attr.read_segments.is_empty()
                || !attr.tail_segments.is_empty())
                && skips_layout_validation(&field_ty)
            {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "segment/tail accessors require a Hopper layout type, not a raw account view",
                ));
            }
        }
        field.attrs.retain(|attr| {
            !attr.path().is_ident("account")
                && !attr.path().is_ident("signer")
                && !attr.path().is_ident("composite")
        });
        ctx_fields.push(ContextField {
            name: field_name,
            ty: field_ty,
            attr,
            index: i,
            synthetic: None,
        });
    }

    // ── Composite (nested) contexts: options compose (v2) ─────────────
    //
    // A context that embeds a `#[composite]` field flattens the inner
    // context's slots in place. Since composite v2 the CONTAINER's
    // options compose across the nesting boundary instead of being
    // rejected (the v1 gate): `strict_writes` / `lamports(...)` compose
    // the authority write-set at const time — outer leaves at their
    // flattened const-expr offsets, each inner context's declared ranges
    // spliced with rebased indices (see the write-ranges emission below);
    // `event_cpi`'s two synthetic slots trail the flattened total at
    // const-expr indices; `emit_touch_map` needs no rebasing at all (the
    // emission is dispatcher-side and the touch log already records
    // flattened instruction slots by construction); and `auto_lifecycle`
    // drives the outer's own leaf helpers, whose slots — including every
    // sibling-role lookup (payer, system_program, close/sweep targets,
    // Metaplex roles) — are `__HOPPER_BASE + flattened-offset`
    // expressions. What stays restricted is the INNER side: an embedded
    // context must still be a plain validation context (no options, no
    // args, no lifecycle — see `__HOPPER_EMBEDDABLE` below).
    let has_composite = ctx_fields.iter().any(|cf| cf.attr.composite);

    // ── event_cpi: auto-append the two Anchor-parity trailing slots ──
    //
    // The author declares NOTHING: the macro appends the event-authority
    // PDA and the program's own account as real trailing account slots
    // (Anchor's `#[event_cpi]` appends the same two). They count toward
    // `ACCOUNT_COUNT`, are validated at bind (authority: PDA verify via
    // the runtime sha256 loop + bump capture; program: address pin to
    // `ctx.program_id()`), gain `_account()` accessors and schema
    // entries — but they are NOT struct fields, so the typed `accounts`
    // facade skips them (see `accounts_binding_fragments`). Appended
    // last so every user-declared field keeps its index.
    if context_options.event_cpi {
        for reserved in ["event_authority", "event_program"] {
            if ctx_fields.iter().any(|cf| cf.name == reserved) {
                return Err(syn::Error::new_spanned(
                    name,
                    format!(
                        "`event_cpi` auto-appends a trailing `{reserved}` account slot; \
                         rename the declared `{reserved}` field (the macro provides it)"
                    ),
                ));
            }
        }
        // The seed expression is carried on the attr for SCHEMA display
        // only (`bump` stays `None`, so the generic seeds+bump lowering
        // never fires); the actual verification is the dedicated
        // `verify_event_authority` check emitted in the validator loop,
        // which works on-chain (sha256 verify loop) and degrades
        // documented-ly on hosts (no sha256 syscall off-chain).
        ctx_fields.push(ContextField {
            name: format_ident!("event_authority"),
            ty: syn::parse_quote!(AccountView),
            attr: AccountAttr {
                seeds: Some(vec![syn::parse_quote!(
                    ::hopper::__runtime::cpi_event::EVENT_AUTHORITY_SEED
                )]),
                ..AccountAttr::default()
            },
            index: ctx_fields.len(),
            synthetic: Some(SyntheticFieldRole::EventAuthority),
        });
        // `address = *ctx.program_id()` reuses the plain stage-2 address
        // lowering: one compare, and the runtime needs this account in
        // the instruction to resolve the self-CPI target.
        ctx_fields.push(ContextField {
            name: format_ident!("event_program"),
            ty: syn::parse_quote!(AccountView),
            attr: AccountAttr {
                address: Some(syn::parse_quote!(*ctx.program_id())),
                ..AccountAttr::default()
            },
            index: ctx_fields.len(),
            synthetic: Some(SyntheticFieldRole::EventProgram),
        });
    }

    let accounts_binding = accounts_binding_fragments(name, &input.generics, &ctx_fields);
    let accounts_field_decl = accounts_binding.field_decl;
    let accounts_init_stmt = accounts_binding.init_stmt;
    let accounts_bound_field = accounts_binding.bound_field;

    // ── Flattened slot offsets (composite / nested contexts) ──────────
    //
    // `local_offsets[i]` is the base-0 flattened start slot of field `i`
    // WITHIN this context: a leaf field occupies one slot, a `#[composite]`
    // field occupies `<Inner>::ACCOUNT_COUNT` slots. For a composite-FREE
    // context every offset is the plain `usize` literal `cf.index`, so the
    // lowering below is byte-identical to the pre-composite one. Once a
    // composite appears, the following offsets become const expressions
    // (`1usize + <Inner>::ACCOUNT_COUNT + ...`) that const-fold at compile
    // time. `account_count_expr` is the matching flattened total.
    //
    // The RUNTIME absolute slot used inside per-field validators and bound
    // accessors is `__HOPPER_BASE + local_offset`, where `__HOPPER_BASE` is
    // a `const` generic that is `0` at the top level (so `0 + k` folds to
    // `k`, preserving the zero-cost top-level lowering) and a concrete
    // literal offset when this context is embedded. `__slot(i)` builds it.
    let hopper_base = format_ident!("__HOPPER_BASE");
    let (local_offsets, account_count_expr): (Vec<TokenStream>, TokenStream) = if !has_composite {
        let offsets = ctx_fields
            .iter()
            .map(|cf| {
                let i = cf.index;
                quote! { #i }
            })
            .collect();
        let len = ctx_fields.len();
        (offsets, quote! { #len })
    } else {
        let mut offsets = Vec::with_capacity(ctx_fields.len());
        let mut terms: Vec<TokenStream> = Vec::new();
        for cf in &ctx_fields {
            offsets.push(if terms.is_empty() {
                quote! { 0usize }
            } else {
                quote! { #(#terms)+* }
            });
            if cf.attr.composite {
                // Path form with generic args dropped: the outer's `'info`
                // is not in scope where these const expressions land
                // (`ACCOUNT_COUNT`, the flattened offsets, the delegation
                // turbofishes), so the lifetime must be elided.
                let inner_spec = composite_spec_ty(&cf.ty)?;
                terms.push(quote! { #inner_spec::ACCOUNT_COUNT });
            } else {
                terms.push(quote! { 1usize });
            }
        }
        let total = if terms.is_empty() {
            quote! { 0usize }
        } else {
            quote! { #(#terms)+* }
        };
        (offsets, total)
    };
    // Absolute runtime slot for the field at `ctx_fields` position `pos`
    // inside a `__HOPPER_BASE`-parametric scope (validators / accessors).
    let slot_abs = |pos: usize| -> TokenStream {
        let local = &local_offsets[pos];
        quote! { #hopper_base + #local }
    };

    // Generate per-field validation functions and collect check descriptions.
    let mut validation_stmts = Vec::new();
    // `bind()`'s copy of the validation sequence. Identical per-field
    // validator CALLS (no code duplication — the same fns `validate()`
    // composes), except the synthetic event-authority slot, whose call
    // is swapped for a direct `verify_event_authority` that BINDS the
    // bump into a local. `validate()` and `bind()` would otherwise each
    // run the sha256 verify loop — ~200+ CU per attempt on-chain, paid
    // twice per event-emitting instruction — and unlike user PDA fields
    // there is no `bump = expr` spelling to store it, so the fuse is the
    // only way an `event_cpi` bind pays for exactly one derivation.
    // Only consulted when an event authority exists; every other
    // context's `bind()` keeps delegating to `validate()`.
    let mut bind_validation_stmts = Vec::new();
    let mut per_field_validators = Vec::new();
    let mut check_descriptions: Vec<String> = Vec::new();
    // Lazy-migration pre-steps (`migrate(from = Old, with = path)`), one
    // per migrate field in declaration order. Spliced into `bind()` BEFORE
    // its validation fragment — never into `validate()` (read-only) —
    // so the account is upgraded in place before any validator sees it.
    let mut migration_stmts: Vec<TokenStream> = Vec::new();

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
    // Composite (nested) fields, collected in declaration order. Each is
    // lowered into: (1) an inner-context validation delegation at its
    // flattened offset (pushed into `validation_stmts` / `bind_validation_stmts`
    // right here so error precedence follows field order), (2) a nested
    // `Bumps` slot, (3) an inner-access method on the bound context, and
    // (4) a compile-time embeddability assertion. `(field, offset_expr)`.
    let mut composite_fields: Vec<(&ContextField, TokenStream)> = Vec::new();

    for cf in &ctx_fields {
        // Composite (nested) fields are not individual account slots: they
        // delegate to the inner context's own validators at the flattened
        // offset. `<Inner>::validate_at::<{offset}>(ctx)` runs the inner's
        // full validation with every slot rebased. A composite-container is
        // always top-level (base 0 — nesting a container inside another is
        // rejected via `__HOPPER_EMBEDDABLE`), so the offset is a concrete
        // const expression and the turbofish needs no `generic_const_exprs`.
        if cf.attr.composite {
            let inner_spec = composite_spec_ty(&cf.ty)?;
            let local = &local_offsets[cf.index];
            let offset_expr = quote! { #hopper_base + #local };
            composite_fields.push((cf, offset_expr.clone()));
            validation_stmts.push(quote! {
                #inner_spec::validate_at::<{ #offset_expr }>(ctx)?;
            });
            bind_validation_stmts.push(quote! {
                #inner_spec::validate_at::<{ #offset_expr }>(ctx)?;
            });
            continue;
        }
        let idx = cf.index;
        // Absolute runtime slot inside the `__HOPPER_BASE`-parametric
        // per-field validator: `__HOPPER_BASE + <flattened local offset>`.
        // `idx` (a plain `usize`) stays the base-0 field index used only in
        // human-readable descriptions and the `constraint` error code.
        // `let _ = &slot;` pins it as used even for a checkless raw-view
        // field, which emits an empty validator body.
        let slot = slot_abs(cf.index);
        let _ = &slot;
        let field_name = &cf.name;
        let validate_fn = format_ident!("validate_{}", field_name);
        let mut field_checks = Vec::new();
        // Where this field's check descriptions begin, so the optional
        // gate below can prepend its own entry after the fact.
        let desc_start = check_descriptions.len();

        // ── event_cpi synthetic authority (auto-appended) ──────────────
        //
        // The event-authority slot verifies against the PDA of
        // `EVENT_AUTHORITY_SEED` under the executing program id and
        // yields its bump. Verification runs through the dedicated
        // runtime helper rather than the generic `seeds`+`bump` lowering
        // because (a) on-chain the helper uses the sha256-only verify
        // loop (`find_and_verify_pda`, ~200 CU at bump 255 — the
        // cheapest verify path in the repo, no `create_program_address`
        // syscalls) and (b) the generic lowering's
        // `::hopper::pda::find_program_address` does not exist on host
        // targets, which would make every `event_cpi` context
        // host-uncompilable. The same helper backs the `Bumps` gather,
        // so `emit_event_cpi` signs with a stored bump.
        if cf.synthetic == Some(SyntheticFieldRole::EventAuthority) {
            field_checks.push(quote! {
                let _ = ::hopper::__runtime::cpi_event::verify_event_authority(
                    ctx.account(#slot)?,
                    ctx.program_id(),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be the event-authority PDA \
                 (seeds [b\"__hopper_event_authority\"], sha256 verify loop on-chain)",
                idx, field_name
            ));
            // The gather expression is a LOCAL read: `bind()`'s fused
            // validation sequence (see `bind_validation_stmts`) already
            // ran the one-and-only `verify_event_authority` and bound
            // its bump to this local, so the `Bumps` struct costs no
            // second derivation. (Bump gathers only ever appear inside
            // `bind()`, where the local is in scope.)
            bump_entries.push((field_name.clone(), quote! { __hopper_event_authority_bump }));
        }

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
        //
        // ── Optional accounts (Anchor ≥ 0.26 parity) ───────────────────
        //
        // An `Option<W>` field follows Anchor's optional-account calling
        // convention: an ABSENT account is passed as the executing
        // program's own id in that slot. Classification and check
        // emission below run against the INNER type `W`, and the wrap
        // just before validator assembly gates every accumulated check
        // for the field behind one `slot address != program id` compare.
        // `validate_optional_field` already rejected illegal shapes
        // (nested `Option`, lifecycle targets, segment lists,
        // non-wrapper inners), so `effective_ty` is a plain wrapper or
        // `AccountView` here.
        let option_inner = option_inner_type(&cf.ty);
        let is_optional = option_inner.is_some();
        let effective_ty: &Type = option_inner.unwrap_or(&cf.ty);
        let wrapper = classify_wrapper(effective_ty);
        let wrapper_is_signer = matches!(wrapper, Some(WrapperKind::Signer));
        let wrapper_is_init = matches!(wrapper, Some(WrapperKind::InitAccount { .. }));
        // The has_layout / layout_ty computation below resolves
        // `Account<'info, T>` → `T` so `load::<T>()` targets the
        // right layout.
        if let Some(WrapperKind::Program) = &wrapper {
            // Program<'info, P>. require address == P::ID + executable.
            // P is the last type arg of the path.
            if let Type::Path(TypePath { path, .. }) = effective_ty {
                if let Some(segment) = path.segments.last() {
                    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(program_ty) = args.args.iter().find_map(|arg| {
                            if let syn::GenericArgument::Type(t) = arg {
                                Some(t.clone())
                            } else {
                                None
                            }
                        }) {
                            field_checks.push(quote! {
                                if ctx.account(#slot)?.address()
                                    != &<#program_ty as ::hopper::__runtime::ProgramId>::ID
                                {
                                    return ::core::result::Result::Err(
                                        ::hopper::__runtime::ProgramError::IncorrectProgramId
                                    );
                                }
                                if !ctx.account(#slot)?.executable() {
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
        if let Some(WrapperKind::Interface { spec }) = &wrapper {
            field_checks.push(quote! {
                if !<#spec as ::hopper::__runtime::InterfaceSpec>::contains(
                    ctx.account(#slot)?.address()
                ) {
                    return ::core::result::Result::Err(
                        ::hopper::__runtime::ProgramError::IncorrectProgramId
                    );
                }
                if !ctx.account(#slot)?.executable() {
                    return ::core::result::Result::Err(
                        ::hopper::__runtime::ProgramError::InvalidAccountData
                    );
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be one of the declared interface programs (address set + executable pin)",
                idx, field_name
            ));
        }
        if let Some(WrapperKind::InterfaceAccount { inner }) = &wrapper {
            field_checks.push(quote! {
                let _ = ::hopper::prelude::InterfaceAccount::<#inner>::try_new(
                    ctx.account(#slot)?
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) owner must belong to its declared interface and match the Hopper layout header",
                idx, field_name
            ));
        }
        if let Some(WrapperKind::ExternalAccount { inner }) = &wrapper {
            field_checks.push(quote! {
                let _ = ::hopper::prelude::ExternalAccount::<#inner>::try_new(
                    ctx.account(#slot)?
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must satisfy its declared external account adapter",
                idx, field_name
            ));
        }
        if let Some(WrapperKind::SystemAccount) = &wrapper {
            field_checks.push(quote! {
                ctx.account(#slot)?.check_owned_by(&::hopper::prelude::SystemId::ID)?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be owned by the System Program",
                idx, field_name
            ));
        }

        // -- Stage 2: signer / mut / address / owner / layout -------------
        //
        // Signer and writable requirements lower to ONE fused packed-flags
        // compare (`expect_signer_writable`) instead of separate
        // `check_signer()` + `check_writable()` calls: the native backend
        // reads the four header flag bytes as a single u32, and with the
        // two literals below the whole check folds to `flags & MASK ==
        // MASK`. Precise errors are preserved by the helper's
        // failure-path fallback. (Innovation I10 — the fused-validation
        // shape competitor derives use, plus exact error codes.)

        let needs_signer = cf.attr.is_signer || wrapper_is_signer;
        let needs_writable =
            cf.attr.is_mut || !cf.attr.mut_segments.is_empty() || !cf.attr.tail_segments.is_empty();
        if needs_signer || needs_writable {
            field_checks.push(quote! {
                ctx.account(#slot)?.expect_signer_writable(#needs_signer, #needs_writable)?;
            });
            if needs_signer {
                check_descriptions.push(format!(
                    "accounts[{}] ({}) must be a signer",
                    idx, field_name
                ));
            }
            if needs_writable {
                check_descriptions.push(format!(
                    "accounts[{}] ({}) must be writable",
                    idx, field_name
                ));
            }
        }
        if cf.attr.executable {
            // Anchor-parity `executable` keyword. Routes through
            // AccountView::check_executable which returns an error
            // when the `executable` flag on the loader-provided
            // account header is unset.
            field_checks.push(quote! {
                ctx.account(#slot)?.check_executable()?;
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
                    // sysvar lazily - the check is explicit, not a
                    // heuristic.
                    field_checks.push(quote! {
                        ::hopper::hopper_runtime::rent::check_rent_exempt(
                            ctx.account(#slot)?,
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
            // `address = k @ MyError::X`: an @-named error resolves to
            // `(#err).into()` (feeding #[hopper::error_code] invariant
            // metadata); without `@` the generic error is unchanged.
            let addr_err = match &cf.attr.address_err {
                Some(e) => quote! { (#e).into() },
                None => quote! { ::hopper::__runtime::ProgramError::InvalidAccountData },
            };
            field_checks.push(quote! {
                if ctx.account(#slot)?.address() != &(#addr_expr) {
                    return ::core::result::Result::Err(#addr_err);
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) address must match `address = ...`",
                idx, field_name
            ));
        }
        let owner_expr = cf.attr.owner.clone();
        let owner_any = cf.attr.owner_any.clone();
        // Optional Anchor `owner = expr @ Err` override. When present the
        // owner check returns `(#err).into()` on mismatch; when absent the
        // generic `?`-propagated error path is emitted byte-unchanged.
        let owner_err = cf.attr.owner_err.clone();
        // Wrapper-aware layout handling:
        //   Account<'info, T>     → layout = T, has_layout = true
        //   InitAccount<'info, T> → has_layout = false at validate time
        //   Signer/Program/raw    → has_layout = false
        //   Plain T               → has_layout from skips_layout_validation
        let (has_layout, layout_ty): (bool, Option<Type>) = match &wrapper {
            Some(WrapperKind::Account { inner }) => (true, Some(inner.clone())),
            Some(WrapperKind::InitAccount { .. })
            | Some(WrapperKind::Signer)
            | Some(WrapperKind::Program)
            | Some(WrapperKind::Interface { .. })
            | Some(WrapperKind::InterfaceAccount { .. })
            | Some(WrapperKind::ExternalAccount { .. })
            | Some(WrapperKind::UncheckedAccount)
            | Some(WrapperKind::SystemAccount) => (false, None),
            // Unreachable: `wrapper` classifies `effective_ty`, which is
            // already the unwrapped inner type for `Option<..>` fields.
            Some(WrapperKind::Optional { .. }) => (false, None),
            None => {
                // `effective_ty` (not `cf.ty`) so an `Option<AccountView>`
                // field resolves like the raw view it wraps instead of
                // an opaque layout named `Option`. Required fields are
                // unchanged: `effective_ty` aliases `cf.ty` for them.
                let h = !skips_layout_validation(effective_ty);
                (h, if h { Some(effective_ty.clone()) } else { None })
            }
        };

        // For `init` accounts the account hasn't been created yet, so we
        // skip the owner+load step. the `init_{field}()` lifecycle
        // helper will allocate and write the header later. `init_if_needed`
        // is conditional: empty slots are allowed through binding for
        // creation, while nonempty slots are owner-checked and layout-checked
        // before the handler receives a typed wrapper. Other cases
        // (including `zero`) assume the account already exists. The same
        // reasoning applies when the field is typed as `InitAccount<T>`.
        let is_init_field = cf.attr.init || wrapper_is_init;
        if has_layout && cf.attr.init_if_needed && !is_init_field {
            let field_ty = layout_ty.as_ref().unwrap_or(&cf.ty);
            let owner_check = if !owner_any.is_empty() {
                let ids = owner_any.iter().map(|expr| quote! { &(#expr) });
                quote! {
                    __hopper_account.check_owned_by_any(&[ #(#ids),* ])?;
                }
            } else if let Some(expr) = &owner_expr {
                match &owner_err {
                    Some(e) => quote! {
                        if __hopper_account.check_owned_by(&(#expr)).is_err() {
                            return ::core::result::Result::Err((#e).into());
                        }
                    },
                    None => quote! {
                        __hopper_account.check_owned_by(&(#expr))?;
                    },
                }
            } else {
                quote! {
                    __hopper_account.check_owned_by(ctx.program_id())?;
                }
            };
            field_checks.push(quote! {
                let __hopper_account = ctx.account(#slot)?;
                if __hopper_account.data_len() > 0 {
                    #owner_check
                    let _ = __hopper_account.load::<#field_ty>()?;
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) may be empty for init_if_needed; if nonempty, owner matches and {} header is valid",
                idx,
                field_name,
                type_ident(field_ty)
                    .map(|i| i.to_string())
                    .unwrap_or_default()
            ));
        } else if has_layout && !is_init_field {
            let field_ty = layout_ty.as_ref().unwrap_or(&cf.ty);
            let owner_check = if !owner_any.is_empty() {
                let ids = owner_any.iter().map(|expr| quote! { &(#expr) });
                quote! {
                    ctx.account(#slot)?.check_owned_by_any(&[ #(#ids),* ])?;
                }
            } else if let Some(expr) = &owner_expr {
                match &owner_err {
                    Some(e) => quote! {
                        if ctx.account(#slot)?.check_owned_by(&(#expr)).is_err() {
                            return ::core::result::Result::Err((#e).into());
                        }
                    },
                    None => quote! {
                        ctx.account(#slot)?.check_owned_by(&(#expr))?;
                    },
                }
            } else {
                quote! {
                    ctx.account(#slot)?.check_owned_by(ctx.program_id())?;
                }
            };
            // `zero` fields are allocated but deliberately NOT yet stamped
            // with a layout header, so the typed `load` would reject them.
            // Owner is still pinned; the zero-discriminator check itself is
            // emitted with the lifecycle preconditions below.
            //
            // `migrate(from = Old, ...)` fields widen the header check to
            // EITHER version — "would bind accept this set?" semantics for
            // the read-only `validate()` surface. Each arm is the FULL
            // `validate_header` identity (disc / version / layout_id /
            // epoch), never a partial sniff, and the Old arm additionally
            // requires the allocation to already fit the New shape (the
            // same `required_len` gate `migrate_layout` enforces during
            // bind, since resizing is `realloc`'s job). Anything that is
            // neither version surfaces the New layout's own validation
            // error, unchanged.
            let load_check = if cf.attr.zero {
                TokenStream::new()
            } else if let Some((from_ty, _)) = &cf.attr.migrate {
                quote! {
                    {
                        let __hopper_migrate_data = ctx.account(#slot)?.try_borrow()?;
                        if let ::core::result::Result::Err(__hopper_new_err) =
                            <#field_ty as ::hopper::__runtime::LayoutContract>::validate_header(
                                &__hopper_migrate_data,
                            )
                        {
                            if <#from_ty as ::hopper::__runtime::LayoutContract>::validate_header(
                                &__hopper_migrate_data,
                            )
                            .is_err()
                                || __hopper_migrate_data.len()
                                    < <#field_ty as ::hopper::__runtime::LayoutContract>::required_len()
                            {
                                return ::core::result::Result::Err(__hopper_new_err);
                            }
                        }
                    }
                }
            } else {
                quote! { let _ = ctx.account(#slot)?.load::<#field_ty>()?; }
            };
            field_checks.push(quote! {
                #owner_check
                #load_check
            });
            check_descriptions.push(if cf.attr.zero {
                format!(
                    "accounts[{}] ({}) owner matches (layout load deferred: `zero` field is not yet initialized)",
                    idx, field_name
                )
            } else if let Some((from_ty, _)) = &cf.attr.migrate {
                format!(
                    "accounts[{}] ({}) owner matches; header is a valid {} — or a fully-valid {} \
                     lazy-migration source whose allocation already fits {} (bind migrates it in \
                     place before validation)",
                    idx,
                    field_name,
                    type_ident(field_ty)
                        .map(|i| i.to_string())
                        .unwrap_or_default(),
                    type_ident(from_ty)
                        .map(|i| i.to_string())
                        .unwrap_or_default(),
                    type_ident(field_ty)
                        .map(|i| i.to_string())
                        .unwrap_or_default()
                )
            } else {
                format!(
                    "accounts[{}] ({}) owner matches, valid {} header",
                    idx,
                    field_name,
                    type_ident(field_ty)
                        .map(|i| i.to_string())
                        .unwrap_or_default()
                )
            });
        } else if !has_layout {
            // For raw AccountView fields, still honor an explicit
            // `owner = expr` / `owner_any = [..]` even without a layout header.
            if !owner_any.is_empty() {
                let ids = owner_any.iter().map(|expr| quote! { &(#expr) });
                field_checks.push(quote! {
                    ctx.account(#slot)?.check_owned_by_any(&[ #(#ids),* ])?;
                });
                check_descriptions.push(format!(
                    "accounts[{}] ({}) owner must match one of `owner_any = [..]`",
                    idx, field_name
                ));
            } else if let Some(expr) = &owner_expr {
                field_checks.push(match &owner_err {
                    Some(e) => quote! {
                        if ctx.account(#slot)?.check_owned_by(&(#expr)).is_err() {
                            return ::core::result::Result::Err((#e).into());
                        }
                    },
                    None => quote! {
                        ctx.account(#slot)?.check_owned_by(&(#expr))?;
                    },
                });
                check_descriptions.push(format!(
                    "accounts[{}] ({}) owner must match `owner = ...`",
                    idx, field_name
                ));
            }
        }

        // -- Stage 3.5: lazy migration at bind (`migrate(from = Old, with = f)`) --
        //
        // Emitted into `bind()`'s pre-validation prologue (see
        // `migration_stmts`), NOT into this field's validator: `validate()`
        // is the read-only surface and must never write. The pre-step
        // checks — on a scoped read borrow, released before the migrate
        // call takes its own exclusive borrow — whether the slot still
        // holds a fully-valid OLD-layout header, and only then runs the
        // typed in-place `migrate_layout` (which re-verifies the Old
        // identity under its own borrow, checks the New shape fits, and
        // re-stamps the header LAST with flags preserved). An
        // already-migrated account fails the Old header check and skips
        // the call; anything else falls through to the normal New-layout
        // validation error below. `#slot` is the composite-aware absolute
        // slot (`__HOPPER_BASE + local_offset`), the same expression the
        // validators use, so a migrate leaf inside a composite CONTAINER
        // cranks at its flattened offset. (A migrate context can never be
        // the EMBEDDED side: it advertises `__HOPPER_EMBEDDABLE = false`,
        // because an outer bind runs inner validators, not inner bind
        // pre-steps.)
        if let Some((from_ty, with_path)) = &cf.attr.migrate {
            let field_ty = layout_ty.as_ref().unwrap_or(&cf.ty);
            migration_stmts.push(quote! {
                {
                    let __hopper_migrate_view = ctx.account(#slot)?;
                    let __hopper_migrate_is_old = {
                        let __hopper_migrate_data = __hopper_migrate_view.try_borrow()?;
                        <#from_ty as ::hopper::__runtime::LayoutContract>::validate_header(
                            &__hopper_migrate_data,
                        )
                        .is_ok()
                    };
                    if __hopper_migrate_is_old {
                        ::hopper::migration::migrate_layout::<#from_ty, #field_ty, _>(
                            __hopper_migrate_view,
                            #with_path,
                        )?;
                    }
                }
            });
        }

        // -- Stage 4a: typed-seeds sugar (`seeds_fn = Type::seeds(...)`) --
        //
        // PDA seed wiring: the user centralizes their PDA seed
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
                    let (expected, _bump) = ::hopper::pda::find_program_address(
                        __seed_slices,
                        #pda_program_expr,
                    );
                    if ctx.account(#slot)?.address() != &expected {
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
                        let (_, __b) = ::hopper::pda::find_program_address(
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
                        let (expected, _bump) = ::hopper::pda::find_program_address(
                            &[ #( AsRef::<[u8]>::as_ref(&(#seed_exprs)) ),* ],
                            #pda_program_expr,
                        );
                        if ctx.account(#slot)?.address() != &expected {
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
                        let expected = ::hopper::pda::create_program_address(
                            seeds_with_bump,
                            #pda_program_expr,
                        )?;
                        if ctx.account(#slot)?.address() != &expected {
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
                        let (_, __b) = ::hopper::pda::find_program_address(
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
        if cf.attr.init || cf.attr.init_if_needed {
            // Precondition: the account must be writable and, once the
            // lifecycle helper runs for an empty slot, owned by this
            // program. The helper itself handles CPI + header write.
            // For `init_if_needed`, nonempty slots have already taken
            // the owner+layout validation path above.
            field_checks.push(quote! {
                ctx.account(#slot)?.check_writable()?;
            });
            let lifecycle = if cf.attr.init_if_needed {
                "init_if_needed"
            } else {
                "init"
            };
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be writable ({} precondition)",
                idx, field_name, lifecycle
            ));
        }
        if cf.attr.realloc.is_some() {
            field_checks.push(quote! {
                ctx.account(#slot)?.check_writable()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be writable (realloc precondition)",
                idx, field_name
            ));
        }
        if cf.attr.zero {
            // `zero` — the account must be allocated but not yet
            // initialized as any Hopper layout. Every `#[hopper::state]`
            // layout compile-asserts `DISC != 0`, so a zero first byte
            // proves "no layout has been stamped here" — the same
            // re-initialization-attack guard as Anchor's `zero`
            // (discriminator-is-zero) constraint. An empty account (no
            // data) is rejected too: `zero` means *pre-allocated* and
            // zeroed, not absent.
            field_checks.push(quote! {
                {
                    let __hopper_data = ctx.account(#slot)?.try_borrow()?;
                    if __hopper_data.first().copied() != ::core::option::Option::Some(0u8) {
                        return ::core::result::Result::Err(
                            ::hopper::__runtime::ProgramError::AccountAlreadyInitialized
                        );
                    }
                }
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be allocated with a zero discriminator (uninitialized)",
                idx, field_name
            ));
        }
        if cf.attr.close.is_some() {
            field_checks.push(quote! {
                ctx.account(#slot)?.check_writable()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) must be writable (close precondition)",
                idx, field_name
            ));
            // The close target receives the drained lamports, and the SVM
            // rejects lamport changes on non-writable accounts — catch a
            // read-only destination at validate time instead of failing
            // the whole transaction at commit. The sibling resolves at
            // its flattened, base-parametric slot (composite-aware).
            if let Some(target) = &cf.attr.close {
                if let Some(target_idx) = ctx_fields.iter().position(|c| c.name == *target) {
                    let target_slot = slot_abs(target_idx);
                    field_checks.push(quote! {
                        ctx.account(#target_slot)?.check_writable()?;
                    });
                    check_descriptions.push(format!(
                        "accounts[{}] ({}) must be writable (receives lamports from `close = {}`)",
                        target_idx, target, field_name
                    ));
                }
            }
        }
        // `sweep = target`: same lamport-flow reasoning as `close` — the
        // target must be writable to receive the drained lamports. The
        // source's own writability is enforced through the `mut`
        // implication set at parse time. Flattened, base-parametric slot
        // (composite-aware), like every sibling-role lookup.
        if let Some(target) = &cf.attr.sweep {
            if let Some(target_idx) = ctx_fields.iter().position(|c| c.name == *target) {
                let target_slot = slot_abs(target_idx);
                field_checks.push(quote! {
                    ctx.account(#target_slot)?.check_writable()?;
                });
                check_descriptions.push(format!(
                    "accounts[{}] ({}) must be writable (receives lamports from `sweep = {}`)",
                    target_idx, target, field_name
                ));
            }
        }

        // -- Stage 5.5: has_one. field value must equal other account's key.
        // Runs after layout load so we can read the struct field.
        for (hi, target_ident) in cf.attr.has_one.iter().enumerate() {
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
            // `has_one` is a plain constraint (not lifecycle), so it can
            // appear on an embeddable context: the referenced sibling must
            // be resolved at the flattened, base-parametric slot.
            let target_slot = slot_abs(target_idx);
            let field_ty = layout_type_for_field(cf).ok_or_else(|| {
                syn::Error::new_spanned(
                    &cf.ty,
                    "has_one requires a Hopper layout field or Account<'info, T> wrapper",
                )
            })?;
            let target_field_ident = target_ident.clone();
            // `has_one = field @ MyError::X`: the @-named error replaces
            // the generic `InvalidAccountData`; without `@` the generic
            // error is emitted byte-unchanged.
            let has_one_err = match cf.attr.has_one_errs.get(hi).cloned().flatten() {
                Some(e) => quote! { (#e).into() },
                None => quote! { ::hopper::__runtime::ProgramError::InvalidAccountData },
            };
            field_checks.push(quote! {
                {
                    let view = ctx.account(#slot)?;
                    let layout = view.load::<#field_ty>()?;
                    let expected_key = ctx.account(#target_slot)?.address();
                    // Convention: the cross-referenced field on the
                    // layout must be named identically to the target
                    // account's field, and must coerce to an `Address`.
                    // Rewrap the field's bytes as an `Address` so the
                    // comparison hits the runtime's 4 x u64 word-compare
                    // `PartialEq` instead of a bytewise slice compare.
                    if ::hopper::__runtime::Address::new(
                        *::core::convert::AsRef::<[u8; 32]>::as_ref(&layout.#target_field_ident),
                    ) != *expected_key
                    {
                        return ::core::result::Result::Err(#has_one_err);
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
            // `constraint = expr @ MyError::X`: the @-named error replaces
            // the generic `Custom(0xC000 | idx)`; without `@` the generic
            // error is emitted byte-unchanged.
            let constraint_err = match cf.attr.constraint_errs.get(i).cloned().flatten() {
                Some(e) => quote! { (#e).into() },
                None => {
                    quote! { ::hopper::__runtime::ProgramError::Custom(0xc0_00 | (#idx as u32)) }
                }
            };
            field_checks.push(quote! {
                if !({ #expr }) {
                    return ::core::result::Result::Err(#constraint_err);
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
        let has_token_shape = cf.attr.token_mint.is_some() || cf.attr.token_authority.is_some();
        if has_token_shape || cf.attr.token_token_program.is_some() {
            let prog_expr = if let Some(tp) = &cf.attr.token_token_program {
                quote! { &(#tp) }
            } else {
                quote! { &::hopper::__runtime::token::TOKEN_PROGRAM_ID }
            };
            field_checks.push(quote! {
                ctx.account(#slot)?.check_owned_by(#prog_expr)?;
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
            // `token::mint = m @ MyError::X`: the @-named error replaces
            // the generic `require_token_mint` error; without `@` the
            // original `?`-propagated call is emitted byte-unchanged.
            field_checks.push(match &cf.attr.token_mint_err {
                Some(e) => quote! {
                    if ::hopper::__runtime::token::require_token_mint(
                        ctx.account(#slot)?,
                        &(#expected_mint),
                    ).is_err() {
                        return ::core::result::Result::Err((#e).into());
                    }
                },
                None => quote! {
                    ::hopper::__runtime::token::require_token_mint(
                        ctx.account(#slot)?,
                        &(#expected_mint),
                    )?;
                },
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) is a token account for the declared mint",
                idx, field_name
            ));
        }
        if let Some(expected_authority) = &cf.attr.token_authority {
            field_checks.push(match &cf.attr.token_authority_err {
                Some(e) => quote! {
                    if ::hopper::__runtime::token::require_token_owner_eq(
                        ctx.account(#slot)?,
                        &(#expected_authority),
                    ).is_err() {
                        return ::core::result::Result::Err((#e).into());
                    }
                },
                None => quote! {
                    ::hopper::__runtime::token::require_token_owner_eq(
                        ctx.account(#slot)?,
                        &(#expected_authority),
                    )?;
                },
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
                ctx.account(#slot)?.check_owned_by(#prog_expr)?;
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) carries ImmutableOwner extension",
                idx, field_name
            ));
        }
        if cf.attr.ext_cpi_guard {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_cpi_guard(
                    ctx.account(#slot)?,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) carries CpiGuard extension",
                idx, field_name
            ));
        }
        if cf.attr.ext_confidential_transfer_mint {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_confidential_transfer_mint(
                    ctx.account(#slot)?,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) carries ConfidentialTransferMint extension",
                idx, field_name
            ));
        }
        if cf.attr.ext_confidential_transfer_account {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_confidential_transfer_account(
                    ctx.account(#slot)?,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) carries ConfidentialTransferAccount extension",
                idx, field_name
            ));
        }
        if cf.attr.ext_scaled_ui_amount_config {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_scaled_ui_amount_config(
                    ctx.account(#slot)?,
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) carries ScaledUiAmountConfig extension",
                idx, field_name
            ));
        }
        if let Some(expected) = &cf.attr.ext_mint_close_authority {
            field_checks.push(quote! {
                ::hopper::__runtime::token_2022_ext::require_mint_close_authority(
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
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
                    ctx.account(#slot)?,
                    &(#expected),
                )?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) TransferFeeConfig withdraw_authority matches",
                idx, field_name
            ));
        }

        // Metaplex Token Metadata constraints. These are primarily
        // CPI-helper inputs, but validation still performs the cheap
        // data-shape checks up front so an oversized name/symbol/uri
        // fails in Hopper before issuing a Metaplex CPI.
        if let (Some(name_expr), Some(symbol_expr), Some(uri_expr), Some(sfbp_expr)) = (
            &cf.attr.metadata_name,
            &cf.attr.metadata_symbol,
            &cf.attr.metadata_uri,
            &cf.attr.metadata_seller_fee_basis_points,
        ) {
            field_checks.push(quote! {
                ::hopper::hopper_metaplex::DataV2::simple(
                    #name_expr,
                    #symbol_expr,
                    #uri_expr,
                    (#sfbp_expr) as u16,
                ).validate_for_context()?;
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) metadata data fits Metaplex name/symbol/uri limits",
                idx, field_name
            ));
        }

        if let Some(max_supply_expr) = &cf.attr.master_edition_max_supply {
            field_checks.push(quote! {
                let _max_supply: ::core::option::Option<u64> =
                    ::hopper::hopper_metaplex::IntoMasterEditionMaxSupply::into_master_edition_max_supply(
                        #max_supply_expr
                    );
            });
            check_descriptions.push(format!(
                "accounts[{}] ({}) master-edition max_supply is a valid u64/Option<u64> value",
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
                        format!(
                            "`dup = {}` must name a sibling field on the same context",
                            other
                        ),
                    )
                })?;
            // `dup` is a plain constraint (not lifecycle), so the aliased
            // sibling resolves at the flattened, base-parametric slot.
            let other_slot = slot_abs(other_idx);
            field_checks.push(quote! {
                if ctx.account(#slot)?.address() != ctx.account(#other_slot)?.address() {
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
                    #[allow(unexpected_cfgs)]
                    #[cfg(target_os = "solana")]
                    {
                        let (expected, _bump) =
                            ::hopper::hopper_associated_token::derive_ata_for_program(
                                &(#ata_auth),
                                &(#ata_mint),
                                #token_program_expr,
                            );
                        if ctx.account(#slot)?.address() != &expected {
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

        // ── Optional gate: absent ⇒ zero checks ────────────────────────
        //
        // Wrap EVERY check accumulated for an `Option<W>` field —
        // wrapper-derived role checks, signer/mut flags, owner + layout
        // load, PDA derivation, has_one, token::*/mint::* shapes,
        // custom constraints — in one presence test. Absence (slot
        // address == executing program id, Anchor's optional-account
        // convention) short-circuits before any of them runs; presence
        // runs the exact token-for-token checks the required form
        // emits. One address compare is the entire cost of absence.
        //
        // Bind-time bookkeeping deliberately stays OUTSIDE the gate:
        // the slot still counts toward ACCOUNT_COUNT, a declared `mut`
        // still publishes its (static) strict_writes WriteRange and
        // lamport-set entry, and `seeds = ...` bump gathering still
        // derives from the seed exprs (a pure computation that never
        // touches the account).
        if is_optional && !field_checks.is_empty() {
            let gated = ::core::mem::take(&mut field_checks);
            field_checks.push(quote! {
                if ctx.account(#slot)?.address() != ctx.program_id() {
                    #(#gated)*
                }
            });
            check_descriptions.insert(
                desc_start,
                format!(
                    "accounts[{}] ({}) is optional: when the slot carries the executing \
                     program's id it binds `None` and every following check for this field \
                     is skipped; when present, all of them run",
                    idx, field_name
                ),
            );
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
                ///
                /// Generic over `__HOPPER_BASE`, the flattened slot offset
                /// of the enclosing context. `0` at the top level (where
                /// `__HOPPER_BASE + idx` const-folds back to `idx`, so the
                /// lowering is byte-identical to a non-composite context)
                /// and a concrete offset when this context is embedded as a
                /// `#[composite]` field of another.
                #[inline(always)]
                #vis fn #validate_fn<const __HOPPER_BASE: usize>(
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
            // The base is threaded through as the const generic argument:
            // `__HOPPER_BASE` resolves to the generic param inside
            // `validate_at`, and to a `const __HOPPER_BASE: usize = 0` local
            // inside a composite-container's base-0 `validate`.
            validation_stmts.push(quote! {
                Self::#validate_fn::<#hopper_base>(ctx #arg_name_fragment)?;
            });
            // `bind()`'s sequence: same call — EXCEPT the synthetic
            // event authority, where the single fused verify replaces
            // the validator call and captures the bump for the `Bumps`
            // gather (same check, same helper, same error, same slot in
            // the ordering; just not run twice).
            if cf.synthetic == Some(SyntheticFieldRole::EventAuthority) {
                bind_validation_stmts.push(quote! {
                    let __hopper_event_authority_bump: u8 =
                        ::hopper::__runtime::cpi_event::verify_event_authority(
                            ctx.account(#slot)?,
                            ctx.program_id(),
                        )?;
                });
            } else {
                bind_validation_stmts.push(quote! {
                    Self::#validate_fn::<#hopper_base>(ctx #arg_name_fragment)?;
                });
            }
        }
    }

    let check_desc_literals: Vec<_> = check_descriptions.iter().map(|s| quote! { #s }).collect();
    let check_count = check_descriptions.len();

    // Generate segment accessor methods with const segment bindings.
    let mut accessors = Vec::new();

    for cf in &ctx_fields {
        // Composite fields expose the inner bound context through a
        // dedicated accessor emitted below, not the per-field slot
        // accessors.
        if cf.attr.composite {
            continue;
        }
        let field_name = &cf.name;
        // Base-parametric slot for accessors on the bound context (which
        // carries the `__HOPPER_BASE` const generic).
        let slot = slot_abs(cf.index);
        let layout_ty = layout_type_for_field(cf);
        let display_ty = layout_ty.as_ref().unwrap_or(&cf.ty);
        let type_ident = type_ident(display_ty)?;
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
                        format!(
                            "`sweep = {}` must name a sibling field on the same context",
                            target
                        ),
                    )
                })?;
            // A self-target would credit the drain back into the slot
            // being drained: the two-step move below computes the
            // credit from the pre-drain balance, so aliasing source
            // and target would mint lamports. Statically impossible
            // is better than a runtime footgun.
            if target_idx == cf.index {
                return Err(syn::Error::new_spanned(
                    target,
                    format!(
                        "`sweep = {}` cannot target its own field; name a different sibling",
                        target
                    ),
                ));
            }
            let sweep_fn = format_ident!("sweep_{}", field_name);
            // Sibling-role lookup at the flattened, base-parametric slot
            // (composite-aware): a target declared after a `#[composite]`
            // field lives past the inner context's flattened block.
            let target_slot = slot_abs(target_idx);
            accessors.push(quote! {
                /// Drain every lamport from this slot into the declared
                /// sweep target. Call in the happy path just before
                /// returning. Returns the drained amount.
                #[inline]
                #vis fn #sweep_fn(&self)
                    -> ::core::result::Result<u64, ::hopper::__runtime::ProgramError>
                {
                    let src = self.ctx.account(#slot)?;
                    let dst = self.ctx.account(#target_slot)?;
                    // Distinct field indices can still alias one
                    // account at runtime (duplicate metas). Crediting
                    // an alias with its own pre-drain balance would
                    // mint lamports, so refuse aliases outright — the
                    // same contract as `safe_close_unchecked`.
                    if src.address() == dst.address() {
                        return ::core::result::Result::Err(
                            ::hopper::__runtime::ProgramError::InvalidArgument,
                        );
                    }
                    let amount = src.lamports();
                    if amount == 0 {
                        return ::core::result::Result::Ok(0);
                    }
                    let new_dst = dst
                        .lamports()
                        .checked_add(amount)
                        .ok_or(::hopper::__runtime::ProgramError::ArithmeticOverflow)?;
                    // Both writes flow through the runtime lamport
                    // funnel (`try_set_lamports`), so a BLD-MUT gate
                    // observes the sweep: source and target are both in
                    // the macro's implied lamport permission set.
                    dst.try_set_lamports(new_dst)?;
                    src.try_set_lamports(0)?;
                    ::core::result::Result::Ok(amount)
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
                &::hopper::prelude::AccountView<'_>,
                ::hopper::__runtime::ProgramError,
            > {
                self.ctx.account(#slot)
            }
        });

        // Presence-aware accessor for `Option<...>` fields, so contexts
        // that do not qualify for the typed `accounts` facade (mixed
        // wrapper / layout / raw-view fields) still get a first-class
        // `None`-vs-`Some` read on the bound context.
        if option_inner_type(&cf.ty).is_some() {
            let account_opt_fn = format_ident!("{}_account_opt", field_name);
            accessors.push(quote! {
                /// Presence-aware raw view of the optional `#field_name`
                /// slot. Anchor's optional-account convention: an absent
                /// optional is passed as the executing program's own id,
                /// so this returns `None` exactly when `bind()` bound
                /// `None`. The unconditional `<field>_account()` accessor
                /// still returns the raw slot view either way.
                #[inline(always)]
                #vis fn #account_opt_fn(
                    &self,
                ) -> ::core::result::Result<
                    ::core::option::Option<&::hopper::prelude::AccountView<'_>>,
                    ::hopper::__runtime::ProgramError,
                > {
                    let view = self.ctx.account(#slot)?;
                    if view.address() == self.ctx.program_id() {
                        ::core::result::Result::Ok(::core::option::Option::None)
                    } else {
                        ::core::result::Result::Ok(::core::option::Option::Some(view))
                    }
                }
            });
        }

        if let Some(field_ty) = layout_ty.as_ref() {
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
                    self.ctx.account(#slot)?.load::<#field_ty>()
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
                    unsafe { self.ctx.account(#slot)?.raw_ref::<#field_ty>() }
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
                        self.ctx.account(#slot)?.load_mut::<#field_ty>()
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
                        unsafe { self.ctx.account(#slot)?.raw_mut::<#field_ty>() }
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
                        self.ctx.segment_mut::<__SegT>(#slot, abs_offset)
                    }
                });

                accessors.push(quote! {
                    /// Read-only segment escape: project an arbitrary
                    /// typed sub-slice of `#field_name`. The returned
                    /// [`SegRef`] is a RAII-leased guard that releases
                    /// the shared byte borrow on drop, so the same
                    /// account can be accessed in non-overlapping
                    /// segments sequentially within one instruction.
                    /// For pre-declared `read(...)` segments, prefer
                    /// the field-specific `<field>_<seg>_ref()`
                    /// accessor for type safety and clarity.
                    #[inline(always)]
                    #vis fn #segment_ref_fn<__SegT: ::hopper::__runtime::Pod>(
                        &mut self,
                        abs_offset: u32,
                    ) -> ::core::result::Result<
                        ::hopper::__runtime::SegRef<'_, __SegT>,
                        ::hopper::__runtime::ProgramError,
                    > {
                        self.ctx.segment_ref::<__SegT>(#slot, abs_offset)
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
        if let Some(field_ty) = layout_ty.as_ref() {
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
                            ::hopper::hopper_core::account::HEADER_LEN as u32 + <#field_ty>::#assoc_offset;
                        self.ctx.segment_mut::<#type_alias>(#slot, ABS_OFFSET)
                    }
                });
            }
        }

        // Generate read-only segment accessors.
        if let Some(field_ty) = layout_ty.as_ref() {
            for seg_name in &cf.attr.read_segments {
                let fn_name = format_ident!("{}_{}_ref", field_name, seg_name);
                let seg_upper = to_screaming_snake(seg_name);
                let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
                let type_alias = format_ident!("{}_{}_TYPE", type_upper, seg_upper);

                accessors.push(quote! {
                    /// Read-only access to the `#seg_name` segment of `#field_name`.
                    ///
                    /// Returns a [`SegRef`](::hopper::__runtime::SegRef) - a
                    /// RAII-leased guard that releases the shared byte borrow
                    /// on drop, allowing sequential non-overlapping reads
                    /// from the same account within one instruction.
                    #[inline(always)]
                    #vis fn #fn_name(
                        &mut self,
                    ) -> ::core::result::Result<
                        ::hopper::__runtime::SegRef<'_, #type_alias>,
                        ::hopper::__runtime::ProgramError,
                    > {
                        const ABS_OFFSET: u32 =
                            ::hopper::hopper_core::account::HEADER_LEN as u32 + <#field_ty>::#assoc_offset;
                        self.ctx.segment_ref::<#type_alias>(#slot, ABS_OFFSET)
                    }
                });
            }
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
        if cf.attr.composite {
            continue;
        }
        let field_name = &cf.name;
        let layout_ty = layout_type_for_field(cf);
        let field_ty = layout_ty.as_ref().unwrap_or(&cf.ty);
        // Base-parametric slot for lifecycle helpers on the bound context.
        // Lifecycle features make a context non-embeddable (base is always
        // 0), so `__HOPPER_BASE + idx` folds to `idx`; the slot keeps the
        // codegen uniform. `let _ = &slot;` pins it for fields with no
        // lifecycle attrs.
        let slot = slot_abs(cf.index);
        let _ = &slot;

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
            let space_expr = cf
                .attr
                .space
                .as_ref()
                .expect("validate_account_attr guarantees init/init_if_needed has space");
            // Sibling-role lookups at their flattened, base-parametric
            // slots (composite-aware): a payer / system_program declared
            // after a `#[composite]` field lives past the inner
            // context's flattened block, so the raw field position would
            // read the wrong account there.
            let payer_slot = slot_abs(payer_idx);
            let system_program_slot = slot_abs(system_program_idx);

            // ── PDA-aware creation (Batch 4 audit fix) ────────────────
            //
            // The System Program requires the created (or allocated +
            // assigned) account to SIGN. A fresh keypair signs the
            // transaction itself, but a PDA declared with `seeds = [...]`
            // can only sign via `invoke_signed` with its derivation
            // seeds + canonical bump. The pre-fix emission always used
            // the unsigned `invoke()`, so `#[account(init, seeds = ...)]`
            // validated the PDA and then failed the CPI at runtime.
            //
            // The bump comes from `self.bumps.<field>`: `bind()` gathered
            // it before any lifecycle helper can run, and for `init` the
            // attribute grammar guarantees `seeds` implies `bump`.
            // `seeds_fn` fields keep the unsigned path (their seed count
            // is not known at expansion time); non-PDA inits are
            // unaffected (`signers = &[]` is the old behavior).
            let init_invoke = if let Some(seeds) = &cf.attr.seeds {
                let seed_exprs: Vec<_> = seeds.iter().collect();
                quote! {
                    {
                        let __hopper_bump: u8 = self.bumps.#field_name;
                        let __hopper_seeds = [
                            #( ::hopper::__runtime::Seed::from(
                                ::core::convert::AsRef::<[u8]>::as_ref(&(#seed_exprs))
                            ), )*
                            ::hopper::__runtime::Seed::from(
                                ::core::slice::from_ref(&__hopper_bump)
                            ),
                        ];
                        ::hopper::hopper_init!(
                            payer,
                            account,
                            system_program,
                            self.ctx.program_id(),
                            #field_ty,
                            #space_expr,
                            signers = &[::hopper::__runtime::Signer::from(&__hopper_seeds[..])]
                        )
                    }
                }
            } else {
                quote! {
                    ::hopper::hopper_init!(
                        payer,
                        account,
                        system_program,
                        self.ctx.program_id(),
                        #field_ty,
                        #space_expr
                    )
                }
            };

            // Two emission shapes:
            //
            //   init            - call hopper_init! to create or allocate+assign
            //                     a zero-data account, then write the header.
            //
            //   init_if_needed  - skip the lifecycle CPI entirely
            //                     when the account already has data.
            //                     The account is then assumed to be
            //                     set up by a prior invocation; the
            //                     caller is responsible for verifying
            //                     the existing layout separately.
            let body = if is_if_needed {
                quote! {
                    let account = self.ctx.account(#slot)?;
                    if account.data_len() > 0 {
                        // Already allocated; nothing to do. Caller
                        // should still validate the layout via
                        // `<ctx>_load()` or equivalent.
                        return ::core::result::Result::Ok(());
                    }
                    let payer = self.ctx.account(#payer_slot)?;
                    let system_program = self.ctx.account(#system_program_slot)?;
                    #init_invoke
                }
            } else {
                quote! {
                    let payer = self.ctx.account(#payer_slot)?;
                    let account = self.ctx.account(#slot)?;
                    let system_program = self.ctx.account(#system_program_slot)?;
                    #init_invoke
                }
            };

            let doc = if is_if_needed {
                "Create or allocate+assign the account via System Program CPI if it doesn't exist yet (init_if_needed). \
                 If the account is already allocated (data_len > 0) the helper returns Ok(()) without \
                 touching lamports or data - caller is responsible for validating the existing layout."
            } else {
                "Create or allocate+assign the account via System Program CPI, zero-init its data, and write the Hopper header. \
                  Errors if the account already has data."
            };

            // Seed expressions may reference declared instruction args
            // (`seeds = [b"vault", nonce.to_le_bytes().as_ref()]`), so a
            // seeded init helper threads the args through its signature
            // exactly like the metadata CPI helpers do. Arg-less contexts
            // and unseeded inits keep the zero-arg shape.
            let init_arg_fragment = if has_instruction_args && cf.attr.seeds.is_some() {
                let aps = arg_params.clone();
                quote! { , #(#aps),* }
            } else {
                TokenStream::new()
            };

            accessors.push(quote! {
                #[doc = #doc]
                #[inline]
                #vis fn #init_fn(&self #init_arg_fragment) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    #body
                }
            });
        }

        if let (
            Some(name_expr),
            Some(symbol_expr),
            Some(uri_expr),
            Some(sfbp_expr),
            Some(mint_ident),
            Some(mint_authority_ident),
            Some(payer_ident),
            Some(update_authority_ident),
            Some(system_program_ident),
        ) = (
            &cf.attr.metadata_name,
            &cf.attr.metadata_symbol,
            &cf.attr.metadata_uri,
            &cf.attr.metadata_seller_fee_basis_points,
            &cf.attr.metadata_mint,
            &cf.attr.metadata_mint_authority,
            &cf.attr.metadata_payer,
            &cf.attr.metadata_update_authority,
            &cf.attr.metadata_system_program,
        ) {
            let create_fn = format_ident!("create_{}", field_name);
            let mint_idx = sibling_index(&ctx_fields, mint_ident, "metadata::mint")?;
            let mint_authority_idx = sibling_index(
                &ctx_fields,
                mint_authority_ident,
                "metadata::mint_authority",
            )?;
            let payer_idx = sibling_index(&ctx_fields, payer_ident, "metadata::payer")?;
            let update_authority_idx = sibling_index(
                &ctx_fields,
                update_authority_ident,
                "metadata::update_authority",
            )?;
            let system_program_idx = sibling_index(
                &ctx_fields,
                system_program_ident,
                "metadata::system_program",
            )?;
            // Every sibling role resolves at its flattened,
            // base-parametric slot (composite-aware).
            let mint_slot = slot_abs(mint_idx);
            let mint_authority_slot = slot_abs(mint_authority_idx);
            let payer_slot = slot_abs(payer_idx);
            let update_authority_slot = slot_abs(update_authority_idx);
            let system_program_slot = slot_abs(system_program_idx);
            let rent_expr = if let Some(rent_ident) = &cf.attr.metadata_rent {
                let rent_idx = sibling_index(&ctx_fields, rent_ident, "metadata::rent")?;
                let rent_slot = slot_abs(rent_idx);
                quote! { ::core::option::Option::Some(self.ctx.account(#rent_slot)?) }
            } else {
                quote! { ::core::option::Option::None }
            };
            let is_mutable_expr = if let Some(expr) = &cf.attr.metadata_is_mutable {
                quote! { (#expr) }
            } else {
                quote! { true }
            };
            let method_arg_fragment = if has_instruction_args {
                let aps = arg_params.clone();
                quote! { , #(#aps),* }
            } else {
                TokenStream::new()
            };

            accessors.push(quote! {
                /// Invoke Metaplex `CreateMetadataAccountV3` using the
                /// `metadata::*` accounts and data declared on this field.
                #[inline]
                #vis fn #create_fn(
                    &self
                    #method_arg_fragment
                ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    ::hopper::hopper_metaplex::CreateMetadataAccountV3 {
                        metadata: self.ctx.account(#slot)?,
                        mint: self.ctx.account(#mint_slot)?,
                        mint_authority: self.ctx.account(#mint_authority_slot)?,
                        payer: self.ctx.account(#payer_slot)?,
                        update_authority: self.ctx.account(#update_authority_slot)?,
                        system_program: self.ctx.account(#system_program_slot)?,
                        rent: #rent_expr,
                        data: ::hopper::hopper_metaplex::DataV2::simple(
                            #name_expr,
                            #symbol_expr,
                            #uri_expr,
                            (#sfbp_expr) as u16,
                        ),
                        is_mutable: #is_mutable_expr,
                    }.invoke()
                }
            });
        }

        if let (
            Some(max_supply_expr),
            Some(mint_ident),
            Some(metadata_ident),
            Some(update_authority_ident),
            Some(mint_authority_ident),
            Some(payer_ident),
            Some(token_program_ident),
            Some(system_program_ident),
        ) = (
            &cf.attr.master_edition_max_supply,
            &cf.attr.master_edition_mint,
            &cf.attr.master_edition_metadata,
            &cf.attr.master_edition_update_authority,
            &cf.attr.master_edition_mint_authority,
            &cf.attr.master_edition_payer,
            &cf.attr.master_edition_token_program,
            &cf.attr.master_edition_system_program,
        ) {
            let create_fn = format_ident!("create_{}", field_name);
            let mint_idx = sibling_index(&ctx_fields, mint_ident, "master_edition::mint")?;
            let metadata_idx =
                sibling_index(&ctx_fields, metadata_ident, "master_edition::metadata")?;
            let update_authority_idx = sibling_index(
                &ctx_fields,
                update_authority_ident,
                "master_edition::update_authority",
            )?;
            let mint_authority_idx = sibling_index(
                &ctx_fields,
                mint_authority_ident,
                "master_edition::mint_authority",
            )?;
            let payer_idx = sibling_index(&ctx_fields, payer_ident, "master_edition::payer")?;
            let token_program_idx = sibling_index(
                &ctx_fields,
                token_program_ident,
                "master_edition::token_program",
            )?;
            let system_program_idx = sibling_index(
                &ctx_fields,
                system_program_ident,
                "master_edition::system_program",
            )?;
            // Every sibling role resolves at its flattened,
            // base-parametric slot (composite-aware).
            let mint_slot = slot_abs(mint_idx);
            let metadata_slot = slot_abs(metadata_idx);
            let update_authority_slot = slot_abs(update_authority_idx);
            let mint_authority_slot = slot_abs(mint_authority_idx);
            let payer_slot = slot_abs(payer_idx);
            let token_program_slot = slot_abs(token_program_idx);
            let system_program_slot = slot_abs(system_program_idx);
            let rent_expr = if let Some(rent_ident) = &cf.attr.master_edition_rent {
                let rent_idx = sibling_index(&ctx_fields, rent_ident, "master_edition::rent")?;
                let rent_slot = slot_abs(rent_idx);
                quote! { ::core::option::Option::Some(self.ctx.account(#rent_slot)?) }
            } else {
                quote! { ::core::option::Option::None }
            };
            let method_arg_fragment = if has_instruction_args {
                let aps = arg_params.clone();
                quote! { , #(#aps),* }
            } else {
                TokenStream::new()
            };

            accessors.push(quote! {
                /// Invoke Metaplex `CreateMasterEditionV3` using the
                /// `master_edition::*` accounts and max_supply declared
                /// on this field.
                #[inline]
                #vis fn #create_fn(
                    &self
                    #method_arg_fragment
                ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    let __max_supply: ::core::option::Option<u64> =
                        ::hopper::hopper_metaplex::IntoMasterEditionMaxSupply::into_master_edition_max_supply(
                            #max_supply_expr
                        );
                    ::hopper::hopper_metaplex::CreateMasterEditionV3 {
                        edition: self.ctx.account(#slot)?,
                        mint: self.ctx.account(#mint_slot)?,
                        update_authority: self.ctx.account(#update_authority_slot)?,
                        mint_authority: self.ctx.account(#mint_authority_slot)?,
                        payer: self.ctx.account(#payer_slot)?,
                        metadata: self.ctx.account(#metadata_slot)?,
                        token_program: self.ctx.account(#token_program_slot)?,
                        system_program: self.ctx.account(#system_program_slot)?,
                        rent: #rent_expr,
                        max_supply: __max_supply,
                    }.invoke()
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
            // Sibling-role lookup at the flattened, base-parametric slot
            // (composite-aware).
            let close_target_slot = slot_abs(close_target_idx);
            accessors.push(quote! {
                /// Drain lamports from `#field_name` into the declared
                /// close target and mark the data for reclaim. Uses the
                /// sentinel-protected close path so a double-close (via
                /// a re-entered instruction) is detected rather than
                /// silently zeroing a reused account.
                #[inline]
                #vis fn #close_fn(&self) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                    let account = self.ctx.account(#slot)?;
                    let destination = self.ctx.account(#close_target_slot)?;
                    ::hopper::hopper_close!(account, destination, self.ctx.program_id())
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
                                format!(
                                    "realloc_payer `{}`: no field named `{}` in this context",
                                    p_ident, p_ident
                                ),
                            )
                        })?;
                    // Flattened, base-parametric slot (composite-aware).
                    let p_slot = slot_abs(p_idx);
                    Ok::<_, syn::Error>(quote! { Some(self.ctx.account(#p_slot)?) })
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
                    let account = self.ctx.account(#slot)?;
                    let new_len: usize = (#realloc_expr) as usize;
                    let old_len = account.data_len() as usize;
                    let payer = #payer_path.ok_or(::hopper::__runtime::ProgramError::InvalidArgument)?;
                    ::hopper::hopper_core::account::safe_realloc(
                        account,
                        new_len,
                        payer,
                        self.ctx.program_id(),
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
        if cf.attr.composite {
            continue;
        }
        let Some(field_ty) = layout_type_for_field(cf) else {
            continue;
        };
        if !cf.attr.is_mut && cf.attr.mut_segments.is_empty() {
            continue;
        }

        let field_name = &cf.name;
        // Receipt scopes run at the top level (base 0); the flattened
        // local offset IS the absolute slot. `begin_receipt_scope` /
        // `finish` are not `__HOPPER_BASE`-parametric.
        let slot = local_offsets[cf.index].clone();
        let receipt_field_name = format_ident!("{}_receipt", field_name);
        let layout_ident = type_ident(&field_ty)?;

        receipt_scope_fields.push(quote! {
            #receipt_field_name: ::hopper::receipt::StateReceipt<SNAP>,
        });

        receipt_begin_inits.push(quote! {
            #receipt_field_name: {
                let account = ctx.account(#slot)?;
                let data = account.try_borrow()?;
                ::hopper::receipt::StateReceipt::<SNAP>::begin(
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
                let account = ctx.account(#slot)?;
                let data = account.try_borrow()?;
                self.#receipt_field_name.commit_with_segments(&data, &[#(#segment_pairs),*]);
                self.#receipt_field_name.set_invariants(invariants_passed, invariants_checked);
                // If a failure was recorded for this instruction, stamp the
                // receipt *before* emission so off-chain consumers can map the
                // code → invariant name via the program's ErrorRegistry.
                if let ::core::option::Option::Some((__hp_code, __hp_idx, __hp_stage)) = failure {
                    self.#receipt_field_name.set_failure(__hp_code, __hp_idx, __hp_stage);
                }
                ::hopper::receipt::emit_receipt(&self.#receipt_field_name.to_bytes())?;
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
    //
    // A `#[composite]` field is NOT one account: it is
    // `<Inner>::ACCOUNT_COUNT` flattened slots. Its schema entry is
    // therefore not a literal descriptor but a SPLICE of the inner
    // context's own `SCHEMA_METADATA.accounts` (evaluated at compile
    // time below), so the published descriptor list stays exactly one
    // entry per flattened slot — audit-grade coverage never drops to a
    // single opaque row. Inner descriptors are spliced VERBATIM: their
    // `name`s are the inner context's field names (slot order, not
    // name, is the descriptor key; tooling that wants the grouping
    // recurses via the inner context's own SCHEMA_METADATA).
    enum SchemaEntry {
        /// One leaf slot: a ready descriptor literal.
        Leaf(TokenStream),
        /// A nested context occupying `<Inner>::ACCOUNT_COUNT` slots;
        /// carries the generics-dropped spec path (`composite_spec_ty`),
        /// usable in const positions where the outer `'info` is absent.
        Composite(TokenStream),
    }
    let mut account_schema_entries: Vec<SchemaEntry> = Vec::with_capacity(ctx_fields.len());
    for cf in &ctx_fields {
        if cf.attr.composite {
            account_schema_entries.push(SchemaEntry::Composite(composite_spec_ty(&cf.ty)?));
            continue;
        }
        account_schema_entries.push(build_leaf_schema_entry(cf));
    }

    fn build_leaf_schema_entry(cf: &ContextField) -> SchemaEntry {
        {
            let name_lit = cf.name.to_string();
            let layout_ty = layout_type_for_field(cf);
            // `Option<W>` publishes the INNER wrapper's name as its
            // kind (`Signer`, `Program`, ... or the layout type for
            // `Option<Account<'info, T>>`) plus `optional: true`, so
            // manifest consumers see the role, not the literal
            // `Option` ident.
            let display_ty: &Type = layout_ty
                .as_ref()
                .unwrap_or_else(|| option_inner_type(&cf.ty).unwrap_or(&cf.ty));
            let kind_lit = type_ident(display_ty)
                .map(|i| i.to_string())
                .unwrap_or_else(|_| "AccountView".to_string());
            let layout_lit = if layout_ty.is_some() {
                kind_lit.clone()
            } else {
                String::new()
            };
            let writable = cf.attr.is_mut
                || !cf.attr.mut_segments.is_empty()
                || !cf.attr.tail_segments.is_empty();
            // Signer-ness must match what `validate()` actually enforces:
            // the `#[signer]` / `#[account(signer)]` attribute OR the
            // type-level `Signer<'info>` wrapper (the fused
            // `expect_signer_writable` check treats both identically).
            // Publishing only the attribute form under-reported every
            // wrapper-declared signer in SCHEMA_METADATA — and through
            // it in every generated manifest row.
            let effective_ty: &Type = option_inner_type(&cf.ty).unwrap_or(&cf.ty);
            let signer = cf.attr.is_signer
                || matches!(classify_wrapper(effective_ty), Some(WrapperKind::Signer));
            let optional = option_inner_type(&cf.ty).is_some();
            let seeds_lits: Vec<String> = cf
                .attr
                .seeds
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|e| quote!(#e).to_string())
                .collect();
            let has_one_lits: Vec<String> = cf.attr.has_one.iter().map(|i| i.to_string()).collect();
            let lifecycle_path = if cf.attr.init {
                quote! { ::hopper::hopper_schema::accounts::AccountLifecycle::Init }
            } else if cf.attr.init_if_needed {
                quote! { ::hopper::hopper_schema::accounts::AccountLifecycle::InitIfNeeded }
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

            SchemaEntry::Leaf(quote! {
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
            })
        }
    }

    // Composite-free contexts publish the inline literal slice —
    // byte-identical to the pre-composite emission. A context with a
    // composite field publishes a compile-time-COMPOSED array instead:
    // one descriptor per flattened slot, leaves as literals, each
    // composite spliced from the inner context's SCHEMA_METADATA, with
    // a const assert pinning the composed length to ACCOUNT_COUNT so a
    // descriptor/slot mismatch can never ship again.
    let has_composite_schema = account_schema_entries
        .iter()
        .any(|e| matches!(e, SchemaEntry::Composite(_)));
    // Length lives at MODULE level (mangled like `__HOPPER_{Name}_WRITE_RANGES`)
    // because an array length is an anonymous const, where neither `Self`
    // nor the outer `'info` may appear; the spec paths inside it are
    // generics-dropped, so the const is fully concrete.
    let schema_accounts_len_ident = format_ident!("__HOPPER_{}_SCHEMA_ACCOUNTS_LEN", name);
    let (schema_len_module_item, schema_support_items, schema_accounts_expr): (
        TokenStream,
        TokenStream,
        TokenStream,
    ) = if !has_composite_schema {
        let lits: Vec<&TokenStream> = account_schema_entries
            .iter()
            .map(|e| match e {
                SchemaEntry::Leaf(lit) => lit,
                SchemaEntry::Composite(_) => unreachable!("guarded by has_composite_schema"),
            })
            .collect();
        (
            TokenStream::new(),
            TokenStream::new(),
            quote! { &[ #( #lits ),* ] },
        )
    } else {
        let len_terms: Vec<TokenStream> = account_schema_entries
            .iter()
            .map(|e| match e {
                SchemaEntry::Leaf(_) => quote! { 1usize },
                SchemaEntry::Composite(spec) => quote! {
                    #spec::SCHEMA_METADATA.accounts.len()
                },
            })
            .collect();
        let fill_stmts: Vec<TokenStream> = account_schema_entries
            .iter()
            .map(|e| match e {
                SchemaEntry::Leaf(lit) => quote! {
                    __out[__n] = #lit;
                    __n += 1;
                },
                SchemaEntry::Composite(spec) => quote! {
                    {
                        let __inner = #spec::SCHEMA_METADATA.accounts;
                        let mut __i = 0;
                        while __i < __inner.len() {
                            __out[__n] = __inner[__i];
                            __n += 1;
                            __i += 1;
                        }
                    }
                },
            })
            .collect();
        let module_item = quote! {
            /// Number of flattened account slots described by the
            /// context's `SCHEMA_METADATA` — leaves count one,
            /// `#[composite]` fields count the inner context's full
            /// descriptor set.
            #[doc(hidden)]
            #[allow(non_upper_case_globals)]
            #vis const #schema_accounts_len_ident: usize = 0 #( + #len_terms )*;
        };
        let impl_item = quote! {
            /// Compile-time-composed per-slot descriptors: leaf
            /// literals in declaration order with each composite
            /// field spliced (verbatim) from the inner context's
            /// own `SCHEMA_METADATA.accounts`.
            #[doc(hidden)]
            pub const __HOPPER_SCHEMA_ACCOUNTS: [::hopper::hopper_schema::accounts::ContextAccountDescriptor;
                #schema_accounts_len_ident] = {
                // Descriptor count and flattened slot count are the
                // same contract; a mismatch is a macro bug and must
                // fail the build, not ship a lying manifest.
                assert!(
                    #schema_accounts_len_ident == Self::ACCOUNT_COUNT,
                    "composite SCHEMA_METADATA slot count must equal ACCOUNT_COUNT"
                );
                const __EMPTY: ::hopper::hopper_schema::accounts::ContextAccountDescriptor =
                    ::hopper::hopper_schema::accounts::ContextAccountDescriptor {
                        name: "",
                        kind: "",
                        writable: false,
                        signer: false,
                        layout_ref: "",
                        policy_ref: "",
                        seeds: &[],
                        optional: false,
                        lifecycle: ::hopper::hopper_schema::accounts::AccountLifecycle::Existing,
                        payer: "",
                        init_space: 0u32,
                        has_one: &[],
                        expected_address: "",
                        expected_owner: "",
                    };
                let mut __out = [__EMPTY; #schema_accounts_len_ident];
                let mut __n = 0;
                #( #fill_stmts )*
                let _ = __n;
                __out
            };
        };
        (
            module_item,
            impl_item,
            quote! { &Self::__HOPPER_SCHEMA_ACCOUNTS },
        )
    };

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

    // ── bind()'s validation entry ──────────────────────────────────────
    //
    // Almost every context binds by delegating to `validate()`. The
    // exception is `event_cpi`: its synthetic authority slot needs a
    // bump that only the sha256 verify loop can produce, and running
    // that loop inside `validate()` AND again in the bump gather would
    // charge every event-emitting instruction twice (~200+ CU per
    // attempt on-chain). So an event-authority context's `bind()` runs
    // the SAME per-field validators in the SAME order — no duplicated
    // check bodies, no reordered error precedence — with the authority's
    // call swapped for one fused verify that binds the bump to a local
    // the gather then reads. `validate()` itself is untouched, so
    // standalone validate-only callers still get every check.
    let has_event_authority = ctx_fields
        .iter()
        .any(|cf| cf.synthetic == Some(SyntheticFieldRole::EventAuthority));
    let bind_validate_fragment: TokenStream = if has_event_authority {
        quote! {
            ctx.require_accounts(Self::ACCOUNT_COUNT)?;
            #(#bind_validation_stmts)*
        }
    } else {
        quote! { Self::#top_validate_ident(ctx #top_arg_name_fragment)?; }
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
    let mut bumps_field_defs: Vec<TokenStream> = bump_entries
        .iter()
        .map(|(ident, _)| quote! { pub #ident: u8, })
        .collect();
    let mut bumps_gather_stmts: Vec<TokenStream> = bump_entries
        .iter()
        .map(|(ident, expr)| quote! { __hopper_bumps.#ident = #expr; })
        .collect();
    let mut bumps_registry_entries: Vec<TokenStream> = bump_entries
        .iter()
        .map(|(ident, _)| {
            let s = ident.to_string();
            quote! { #s }
        })
        .collect();
    // Nested bumps for `#[composite]` fields (Anchor parity: the outer
    // `Bumps` struct carries the inner context's `Bumps` as a field, so
    // `ctx.bumps().<inner>.<leaf>` reads a nested PDA bump). The inner's
    // bumps are gathered at the flattened offset by its generated
    // `__hopper_gather_bumps_at` (only emitted on embeddable contexts).
    for (cf, offset_expr) in &composite_fields {
        let field_name = &cf.name;
        let inner_spec = composite_spec_ty(&cf.ty)?;
        let inner_bumps_ty = composite_bumps_ty(&cf.ty)?;
        let s = field_name.to_string();
        bumps_field_defs.push(quote! { pub #field_name: #inner_bumps_ty, });
        bumps_gather_stmts.push(quote! {
            __hopper_bumps.#field_name =
                #inner_spec::__hopper_gather_bumps_at::<{ #offset_expr }>(ctx)?;
        });
        bumps_registry_entries.push(quote! { #s });
    }

    // ── Innovation I12: strict_writes → static WritePolicy ────────────
    //
    // The context's own declarations already carve the write surface:
    // `mut` grants the whole account, `mut(seg, ...)` grants exact field
    // ranges, and the lifecycle attrs (init / realloc / close) rewrite
    // the account wholesale. Under `strict_writes` those declarations
    // stop being documentation and become a `static` write-set the
    // runtime enforces at every Context-mediated write acquire. The
    // range expressions below are the *same* const arithmetic the
    // generated segment accessors use, so a declared accessor can never
    // be refused by the policy compiled from its own declaration.
    //
    // BLD-I24: the range set is emitted ONCE, as a module-level const,
    // and every consumer reads that const: the runtime `WritePolicy`
    // installed by `bind()`, the `WRITE_RANGES` associated const, and
    // `SCHEMA_METADATA.write_ranges`. Routing all three through one
    // const makes the published (manifest/IDL) write-set byte-identical
    // to the enforced one by construction — they cannot drift. The
    // const lives at module scope (not inside `bind()`, not on the
    // impl) so the function-local `static WritePolicy` can reference it
    // even when the context struct carries generic lifetimes.
    let strict_writes_enabled = context_options.strict_writes;
    // The context name is embedded VERBATIM (not case-folded): struct
    // idents are unique per module, so the raw name keeps the generated
    // const collision-free. A screaming-snake fold is lossy (`AbC` and
    // `Ab_c` both fold to `AB_C`) and would turn two legal context names
    // into a duplicate-definition error inside hidden macro output.
    let write_ranges_const_ident = format_ident!("__HOPPER_{}_WRITE_RANGES", name);
    // BLD-MUT: whether the context declared the lamport dimension.
    // `mutation_complete` is claimed ONLY for `strict_writes` +
    // `lamports(...)` — a bare `strict_writes` context leaves lamports
    // ungoverned (the pre-BLD-MUT passthrough) and stays incomplete, so
    // adopting the framework can never retroactively refuse lamport
    // writes an already-deployed program performs.
    let lamports_declared = context_options.lamports.is_some();
    let mutation_complete = strict_writes_enabled && lamports_declared;

    // ── Declared-range classification (composite v2) ───────────────────
    //
    // The context's `mut` / `mut(seg, ...)` / lifecycle declarations are
    // classified ONCE into position-keyed entries, then rendered into
    // whichever token shape each consumer needs:
    //
    //   1. the hidden `__HOPPER_DECLARED_WRITE_RANGES` associated const —
    //      emitted on every composite-FREE context regardless of
    //      `strict_writes`, because an embedding OUTER's `strict_writes`
    //      must be able to splice the inner's declared structure at const
    //      time even though an embeddable inner can never enable
    //      `strict_writes` itself (the const carries NO authority; only
    //      the outer's opt-in confers it);
    //   2. the composite-free `strict_writes` authority const — the
    //      legacy `#idx_u8`-literal lowering, byte-identical to the
    //      pre-composite emission;
    //   3. the composite container's compile-time-COMPOSED authority
    //      array — outer leaves at const-expr flattened offsets, each
    //      inner context's declared const spliced with rebased indices.
    enum DeclaredRange {
        /// Whole-account grant (plain `mut` or an init / init_if_needed /
        /// realloc / close lifecycle rewrite) on the field at this
        /// `ctx_fields` position.
        Whole(usize),
        /// One exact `mut(seg)` range on the field at this position,
        /// resolved through the `#[hopper::state]` constants.
        Segment {
            pos: usize,
            field_ty: Type,
            seg_name: String,
        },
        /// An OPEN-ENDED growable `Seq<T>` tail range (from `tail(seg)`) on
        /// the field at this position: `[HEADER_LEN + <Ty>::SEG_OFFSET,
        /// +inf)`. Lowers to `WriteRange::tail_from`, so the whole tail
        /// region is writable and grows without re-declaration, while the
        /// fixed head stays protected (an open tail range starting past the
        /// head is not a whole-account grant, so CPI delegation stays
        /// refused). Composes with Feature 1's realloc carve-out: the same
        /// field carries `realloc` to grow the tail.
        Tail {
            pos: usize,
            field_ty: Type,
            seg_name: String,
        },
    }
    let mut declared_ranges: Vec<DeclaredRange> = Vec::new();
    // Field positions carrying a whole-account data grant (used to dedupe
    // the implied writable-CPI-meta delegation grants below: init payer,
    // Metaplex helper roles).
    let mut whole_account_positions: Vec<usize> = Vec::new();
    for cf in &ctx_fields {
        if cf.attr.composite {
            continue;
        }
        // A field combining `realloc` with explicit `mut(seg, ...)`
        // segments is NOT a whole-account handler grant. `realloc` is a
        // BIND-time lifecycle: it resizes the account and tops up rent
        // lamports, both of which happen OUTSIDE the handler's governed
        // write surface -- `safe_realloc` resizes and moves lamports
        // without crossing the `Context` byte-range gate, and the
        // implied-lamport scan below keeps the account in the lamport set
        // by keying off `realloc.is_some()` directly (not this
        // classification). The declared `mut(seg)` ranges are what the
        // HANDLER may write. Classifying such a field `Whole` here would
        // silently widen its enforced/published surface to the entire
        // account and discard the segment scoping -- the strict_writes +
        // realloc silent-degrade bug. So a realloc'd field that ALSO
        // declares segments falls through to the `Segment` arm; `realloc`
        // ALONE (no `mut(seg)`) keeps `Whole`, carrying `mut` semantics on
        // its own.
        //
        // NOTE (parser reality): `realloc = ...` sets `is_mut = true` at
        // parse time (a realloc'd account is writable), so `is_mut` cannot
        // distinguish a *bare* `mut` from the realloc-implied one -- a
        // realloc field always presents `is_mut == true`. We therefore
        // special-case realloc explicitly rather than subtracting it from
        // the `is_mut` term. `init` / `init_if_needed` / `close` keep
        // `Whole` even alongside segments: those lifecycles (re)write or
        // destroy the entire account at bind, so scoping the handler
        // surface to a segment would be meaningless there.
        // `tail(seg)` also scopes the field: a growable-tail grant is an
        // open-ended range past the head, never whole-account.
        let has_scoped_segments =
            !cf.attr.mut_segments.is_empty() || !cf.attr.tail_segments.is_empty();
        let realloc_scoped_by_segments = cf.attr.realloc.is_some() && has_scoped_segments;
        let whole_account = !realloc_scoped_by_segments
            && (cf.attr.is_mut
                || cf.attr.init
                || cf.attr.init_if_needed
                || cf.attr.realloc.is_some()
                || cf.attr.close.is_some());
        if whole_account {
            whole_account_positions.push(cf.index);
            declared_ranges.push(DeclaredRange::Whole(cf.index));
            continue;
        }
        if !has_scoped_segments {
            continue;
        }
        // `mut(seg, ...)` / `tail(seg)` fields: one range per declared
        // segment, resolved through the `#[hopper::state]` constants.
        let field_ty = layout_type_for_field(cf).unwrap_or_else(|| cf.ty.clone());
        for seg_name in &cf.attr.mut_segments {
            declared_ranges.push(DeclaredRange::Segment {
                pos: cf.index,
                field_ty: field_ty.clone(),
                seg_name: seg_name.clone(),
            });
        }
        for seg_name in &cf.attr.tail_segments {
            declared_ranges.push(DeclaredRange::Tail {
                pos: cf.index,
                field_ty: field_ty.clone(),
                seg_name: seg_name.clone(),
            });
        }
    }

    // Consumer 2: the composite-free authority ranges, byte-identical to
    // the pre-composite lowering (`#idx_u8` literals, alias-typed sizes).
    // The writable-CPI delegation extras are appended after the lamport
    // scan below, in grant order, exactly as before.
    let mut range_exprs: Vec<TokenStream> = Vec::new();
    if strict_writes_enabled && !has_composite {
        for dr in &declared_ranges {
            range_exprs.push(match dr {
                DeclaredRange::Whole(pos) => {
                    let idx_u8 = *pos as u8;
                    quote! {
                        ::hopper::__runtime::write_policy::WriteRange::whole_account(#idx_u8)
                    }
                }
                DeclaredRange::Segment {
                    pos,
                    field_ty,
                    seg_name,
                } => {
                    let idx_u8 = *pos as u8;
                    let type_ident = type_ident(field_ty)?;
                    let type_upper = to_screaming_snake(&type_ident.to_string());
                    let seg_upper = to_screaming_snake(seg_name);
                    let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
                    let type_alias = format_ident!("{}_{}_TYPE", type_upper, seg_upper);
                    quote! {
                        ::hopper::__runtime::write_policy::WriteRange::new(
                            #idx_u8,
                            ::hopper::hopper_core::account::HEADER_LEN as u32
                                + <#field_ty>::#assoc_offset,
                            ::core::mem::size_of::<#type_alias>() as u32,
                        )
                    }
                }
                DeclaredRange::Tail {
                    pos,
                    field_ty,
                    seg_name,
                } => {
                    let idx_u8 = *pos as u8;
                    let seg_upper = to_screaming_snake(seg_name);
                    let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
                    quote! {
                        ::hopper::__runtime::write_policy::WriteRange::tail_from(
                            #idx_u8,
                            ::hopper::hopper_core::account::HEADER_LEN as u32
                                + <#field_ty>::#assoc_offset,
                        )
                    }
                }
            });
        }
    }

    // Consumer 1: the always-on hidden declared-range const. Assoc-const
    // spellings only (`<Ty>::SEG_OFFSET` / `<Ty>::SEG_SIZE`) so the const
    // resolves wherever the field type resolves — unlike the legacy
    // alias-typed spelling above, it must not impose a name-in-scope
    // requirement on contexts that never asked for `strict_writes`.
    // Values are identical (`{SEG}_SIZE` is `size_of` of the same field
    // type). Indices are LOCAL (base-0 within this context): the
    // embedding outer rebases them by the composite's flattened offset.
    let declared_write_ranges_item: TokenStream = if has_composite {
        // A composite CONTAINER cannot itself be embedded (nesting is
        // single-level, refused via `__HOPPER_EMBEDDABLE`), so nothing
        // ever splices its declared set — skip the const instead of
        // emitting a second composed array nobody can reference.
        TokenStream::new()
    } else {
        let mut declared_lits: Vec<TokenStream> = Vec::new();
        for dr in &declared_ranges {
            declared_lits.push(match dr {
                DeclaredRange::Whole(pos) => {
                    let idx_u8 = *pos as u8;
                    quote! {
                        ::hopper::__runtime::write_policy::WriteRange::whole_account(#idx_u8)
                    }
                }
                DeclaredRange::Segment {
                    pos,
                    field_ty,
                    seg_name,
                } => {
                    let idx_u8 = *pos as u8;
                    let seg_upper = to_screaming_snake(seg_name);
                    let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
                    let assoc_size = format_ident!("{}_SIZE", seg_upper);
                    quote! {
                        ::hopper::__runtime::write_policy::WriteRange::new(
                            #idx_u8,
                            ::hopper::hopper_core::account::HEADER_LEN as u32
                                + <#field_ty>::#assoc_offset,
                            <#field_ty>::#assoc_size
                        )
                    }
                }
                DeclaredRange::Tail {
                    pos,
                    field_ty,
                    seg_name,
                } => {
                    let idx_u8 = *pos as u8;
                    let seg_upper = to_screaming_snake(seg_name);
                    let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
                    quote! {
                        ::hopper::__runtime::write_policy::WriteRange::tail_from(
                            #idx_u8,
                            ::hopper::hopper_core::account::HEADER_LEN as u32
                                + <#field_ty>::#assoc_offset,
                        )
                    }
                }
            });
        }
        quote! {
            /// Raw declared write-range structure of this context — the
            /// ranges its `mut` / `mut(seg, ...)` / lifecycle
            /// declarations describe, at LOCAL (base-0) account indices,
            /// emitted regardless of `strict_writes` and carrying **no
            /// authority** on their own (`WRITE_RANGES` stays empty
            /// without the opt-in). Exists so a `strict_writes` context
            /// embedding this one as a `#[composite]` field can splice
            /// the inner structure into its composed policy at const
            /// time with each index rebased by the flattened offset.
            #[doc(hidden)]
            pub const __HOPPER_DECLARED_WRITE_RANGES:
                &'static [::hopper::__runtime::write_policy::WriteRange] = &[
                    #(#declared_lits),*
                ];
        }
    };

    // ── BLD-MUT: lamport permission set ────────────────────────────────
    //
    // Explicit `lamports(field, ...)` names PLUS the implied lifecycle
    // set. The implication mirrors what the generated lifecycle helpers
    // *actually do* with lamports (so declared helpers keep working
    // under the gate, and the published set is true, not convenient):
    //
    //   - whole-account `mut` / `init` / `realloc` / `close` fields:
    //     full control of the account includes its balance (init
    //     credits it, realloc tops it up, close drains it);
    //   - `init` payer: debited by `CreateAccount` / the top-up
    //     `Transfer` inside `hopper_init!`;
    //   - `realloc` payer (`realloc_payer`): debited by
    //     `safe_realloc`'s rent top-up;
    //   - `close = target` destination and `sweep = target` target:
    //     credited with the drained balance;
    //   - `sweep` source: drained;
    //   - Metaplex helper roles (`metadata::*` / `master_edition::*`):
    //     the generated `create_<field>()` methods CPI into Metaplex
    //     with **writable** metas on the created metadata/edition PDA
    //     (this field), the payer, and — master edition only — the
    //     mint. The payer is debited and the created PDA credited by
    //     the inner System CPI, so all of them move lamports.
    //
    // Additionally, every account the macro's own helpers hand
    // **writable** to a CPI callee (the `init` payer to the System
    // Program; the Metaplex roles above) is unbounded delegation of
    // both dimensions — so each also receives a whole-account data
    // range (published in WRITE_RANGES; the delegation is real, the
    // set states it; `check_lamport_delegation` demands both). These
    // extra ranges are emitted only under `lamports(...)`, keeping
    // bare `strict_writes` output byte-identical to pre-BLD-MUT.
    let lamport_accounts_const_ident = format_ident!("__HOPPER_{}_LAMPORT_ACCOUNTS", name);
    // Both sets are collected as FIELD POSITIONS (not `u8` indices) so
    // one scan serves both renderings: `u8` literals composite-free
    // (byte-identical to the pre-composite lowering — positions ARE the
    // indices there), const-expr flattened offsets in a container.
    let mut lamport_positions: Vec<usize> = Vec::new();
    // Writable-CPI-meta delegation extras (init payer, Metaplex roles):
    // whole-account ranges appended AFTER the declared set, in grant
    // order, exactly like the pre-composite lowering.
    let mut delegable_extra_positions: Vec<usize> = Vec::new();
    if mutation_complete {
        let push_pos = |v: &mut Vec<usize>, pos: usize| {
            if !v.contains(&pos) {
                v.push(pos);
            }
        };
        if let Some(named) = &context_options.lamports {
            for ident in named {
                let idx = sibling_index(&ctx_fields, ident, "lamports")?;
                if ctx_fields[idx].attr.composite {
                    return Err(syn::Error::new_spanned(
                        ident,
                        format!(
                            "`lamports({ident})` names a `#[composite]` field: the lamport \
                             dimension grants only the outer context's own account fields. An \
                             account inside an embedded context cannot be granted lamport \
                             permission from the outer (an embeddable inner carries no \
                             lifecycle or `lamports(...)` of its own) — flatten the inner \
                             context into this one if one of its accounts must move lamports."
                        ),
                    ));
                }
                push_pos(&mut lamport_positions, idx);
            }
        }
        for cf in &ctx_fields {
            let whole_account = cf.attr.is_mut
                || cf.attr.init
                || cf.attr.init_if_needed
                || cf.attr.realloc.is_some()
                || cf.attr.close.is_some();
            if whole_account {
                push_pos(&mut lamport_positions, cf.index);
            }
            if cf.attr.sweep.is_some() {
                push_pos(&mut lamport_positions, cf.index);
            }
            if (cf.attr.init || cf.attr.init_if_needed) && cf.attr.payer.is_some() {
                let payer_ident = cf.attr.payer.as_ref().unwrap();
                let payer_idx = sibling_index(&ctx_fields, payer_ident, "init payer")?;
                // Writable CPI hand-off to the System Program: the payer
                // needs lamport permission AND the whole-account data
                // grant.
                grant_cpi_delegable(
                    payer_idx,
                    &mut lamport_positions,
                    &mut whole_account_positions,
                    &mut delegable_extra_positions,
                );
            }
            // Generated Metaplex CPI helpers: `create_<field>()` hands
            // writable metas to Metaplex — CreateMetadataAccountV3
            // marks the metadata PDA (this field) and the payer
            // writable; CreateMasterEditionV3 marks the edition PDA
            // (this field), the mint, and the payer writable (see the
            // account tables in `hopper-metaplex::instructions`).
            // Every writable meta is a both-dimension delegation, so
            // each of these accounts gets lamports + a whole-account
            // range — otherwise the gate refuses the helper's own CPI
            // at runtime and the published helper is unusable.
            if metadata_cpi_helper_declared(&cf.attr) {
                let payer_ident = cf.attr.metadata_payer.as_ref().unwrap();
                let payer_idx = sibling_index(&ctx_fields, payer_ident, "metadata::payer")?;
                for idx in [cf.index, payer_idx] {
                    grant_cpi_delegable(
                        idx,
                        &mut lamport_positions,
                        &mut whole_account_positions,
                        &mut delegable_extra_positions,
                    );
                }
            }
            if master_edition_cpi_helper_declared(&cf.attr) {
                let mint_ident = cf.attr.master_edition_mint.as_ref().unwrap();
                let mint_idx = sibling_index(&ctx_fields, mint_ident, "master_edition::mint")?;
                let payer_ident = cf.attr.master_edition_payer.as_ref().unwrap();
                let payer_idx = sibling_index(&ctx_fields, payer_ident, "master_edition::payer")?;
                for idx in [cf.index, mint_idx, payer_idx] {
                    grant_cpi_delegable(
                        idx,
                        &mut lamport_positions,
                        &mut whole_account_positions,
                        &mut delegable_extra_positions,
                    );
                }
            }
            if cf.attr.realloc.is_some() {
                if let Some(payer_ident) = &cf.attr.realloc_payer {
                    let payer_idx = sibling_index(&ctx_fields, payer_ident, "realloc_payer")?;
                    push_pos(&mut lamport_positions, payer_idx);
                }
            }
            if let Some(target) = &cf.attr.close {
                let target_idx = sibling_index(&ctx_fields, target, "close target")?;
                push_pos(&mut lamport_positions, target_idx);
            }
            if let Some(target) = &cf.attr.sweep {
                let target_idx = sibling_index(&ctx_fields, target, "sweep target")?;
                push_pos(&mut lamport_positions, target_idx);
            }
        }
        // Field positions are monotone in flattened offset, so sorting
        // positions sorts the published indices in both renderings.
        lamport_positions.sort_unstable();
    }
    // The composite-free delegation extras join the authority ranges
    // AFTER the declared set, in grant order (the pre-composite shape).
    if strict_writes_enabled && !has_composite {
        for pos in &delegable_extra_positions {
            let idx_u8 = *pos as u8;
            range_exprs.push(quote! {
                ::hopper::__runtime::write_policy::WriteRange::whole_account(#idx_u8)
            });
        }
    }
    let lamport_index_lits: Vec<TokenStream> = lamport_positions
        .iter()
        .map(|&pos| {
            if has_composite {
                // Flattened local offset, a const expression. The `as u8`
                // is bounded by the composed array's `<= 256` assert —
                // `mutation_complete` implies `strict_writes`, so the
                // composed array (and its eagerly-evaluated assert)
                // always exists alongside this list.
                let local = &local_offsets[pos];
                quote! { (#local) as u8 }
            } else {
                let idx = pos as u8;
                quote! { #idx }
            }
        })
        .collect();
    // Single source of truth for the lamport permission set, mirroring
    // the write-ranges const: the runtime `WritePolicy`, the
    // `LAMPORT_ACCOUNTS` associated const, and
    // `SCHEMA_METADATA.lamport_accounts` all read this one const.
    let lamport_accounts_const_item = quote! {
        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        #vis const #lamport_accounts_const_ident: &[u8] = &[
            #(#lamport_index_lits),*
        ];
    };
    // Consumer 3: the composite container's authority set under
    // `strict_writes` is a compile-time-COMPOSED module-level array —
    // the same const-eval copy-loop pattern as `__HOPPER_SCHEMA_ACCOUNTS`
    // (outer leaves in declaration order, each `#[composite]` field
    // spliced from the inner context's `__HOPPER_DECLARED_WRITE_RANGES`
    // with every account index rebased by the composite's flattened base
    // offset, delegation extras appended last). Module level (not on the
    // impl) because `bind()`'s function-local `static WritePolicy` must
    // reference the result with no generics in scope; every path inside
    // is generics-dropped, so the consts are fully concrete.
    let write_ranges_len_ident = format_ident!("__HOPPER_{}_WRITE_RANGES_LEN", name);
    let write_ranges_arr_ident = format_ident!("__HOPPER_{}_WRITE_RANGES_ARR", name);
    let (composed_write_ranges_items, write_ranges_value): (TokenStream, TokenStream) =
        if !(strict_writes_enabled && has_composite) {
            (TokenStream::new(), quote! { &[ #(#range_exprs),* ] })
        } else {
            let mut len_terms: Vec<TokenStream> = Vec::new();
            let mut fill_stmts: Vec<TokenStream> = Vec::new();
            for cf in &ctx_fields {
                let local = &local_offsets[cf.index];
                if cf.attr.composite {
                    let inner_spec = composite_spec_ty(&cf.ty)?;
                    len_terms.push(quote! {
                        #inner_spec::__HOPPER_DECLARED_WRITE_RANGES.len()
                    });
                    fill_stmts.push(quote! {
                        {
                            // Inner context spliced with every account
                            // index rebased by the composite's flattened
                            // base offset. The inner's offsets/sizes are
                            // copied VERBATIM — an inner `mut(seg)` lease
                            // is enforceable from the outer gate exactly
                            // as it would be standalone.
                            let __inner = #inner_spec::__HOPPER_DECLARED_WRITE_RANGES;
                            let __base: usize = #local;
                            let mut __i = 0;
                            while __i < __inner.len() {
                                let mut __r = __inner[__i];
                                let __abs = __base + __r.account_index as usize;
                                ::core::assert!(
                                    __abs < 256,
                                    "composite write-range rebase exceeds the u8 \
                                     account-index space of WritePolicy"
                                );
                                __r.account_index = __abs as u8;
                                __out[__n] = __r;
                                __n += 1;
                                __i += 1;
                            }
                        }
                    });
                    continue;
                }
                // Same realloc+segments carve-out as the composite-free
                // classification above (see the long note there): a
                // realloc'd field that also declares `mut(seg)` is scoped
                // to its segments, not whole-account. `realloc` sets
                // `is_mut` at parse time, so it must be special-cased
                // explicitly.
                let has_scoped_segments =
                    !cf.attr.mut_segments.is_empty() || !cf.attr.tail_segments.is_empty();
                let realloc_scoped_by_segments = cf.attr.realloc.is_some() && has_scoped_segments;
                let whole_account = !realloc_scoped_by_segments
                    && (cf.attr.is_mut
                        || cf.attr.init
                        || cf.attr.init_if_needed
                        || cf.attr.realloc.is_some()
                        || cf.attr.close.is_some());
                if whole_account {
                    len_terms.push(quote! { 1usize });
                    fill_stmts.push(quote! {
                        __out[__n] =
                            ::hopper::__runtime::write_policy::WriteRange::whole_account(
                                (#local) as u8
                            );
                        __n += 1;
                    });
                    continue;
                }
                if !has_scoped_segments {
                    continue;
                }
                // Assoc-const spellings (like the declared const above):
                // the composed array lives at module scope, where only
                // path-resolved constants are guaranteed to be in scope.
                let field_ty = layout_type_for_field(cf).unwrap_or_else(|| cf.ty.clone());
                for seg_name in &cf.attr.mut_segments {
                    let seg_upper = to_screaming_snake(seg_name);
                    let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
                    let assoc_size = format_ident!("{}_SIZE", seg_upper);
                    len_terms.push(quote! { 1usize });
                    fill_stmts.push(quote! {
                        __out[__n] = ::hopper::__runtime::write_policy::WriteRange::new(
                            (#local) as u8,
                            ::hopper::hopper_core::account::HEADER_LEN as u32
                                + <#field_ty>::#assoc_offset,
                            <#field_ty>::#assoc_size
                        );
                        __n += 1;
                    });
                }
                for seg_name in &cf.attr.tail_segments {
                    let seg_upper = to_screaming_snake(seg_name);
                    let assoc_offset = format_ident!("{}_OFFSET", seg_upper);
                    len_terms.push(quote! { 1usize });
                    fill_stmts.push(quote! {
                        __out[__n] = ::hopper::__runtime::write_policy::WriteRange::tail_from(
                            (#local) as u8,
                            ::hopper::hopper_core::account::HEADER_LEN as u32
                                + <#field_ty>::#assoc_offset,
                        );
                        __n += 1;
                    });
                }
            }
            // Writable-CPI delegation extras, appended after the declared
            // set in grant order — same shape as the composite-free path.
            for pos in &delegable_extra_positions {
                let local = &local_offsets[*pos];
                len_terms.push(quote! { 1usize });
                fill_stmts.push(quote! {
                    __out[__n] = ::hopper::__runtime::write_policy::WriteRange::whole_account(
                        (#local) as u8
                    );
                    __n += 1;
                });
            }
            let items = quote! {
                /// Number of composed write ranges: one per outer leaf
                /// grant plus each inner context's full declared set.
                #[doc(hidden)]
                #[allow(non_upper_case_globals)]
                #vis const #write_ranges_len_ident: usize = 0usize #( + #len_terms )*;

                /// Compile-time-composed write-range set for a composite
                /// container under `strict_writes`: outer leaves at their
                /// flattened const-expr indices, inner contexts spliced
                /// from `__HOPPER_DECLARED_WRITE_RANGES` with rebased
                /// indices, delegation extras last. Referenced by the
                /// module-level authority slice below (and through it by
                /// the installed `WritePolicy`, `WRITE_RANGES`, and
                /// `SCHEMA_METADATA`), so published == enforced holds in
                /// the composed world too.
                #[doc(hidden)]
                #[allow(non_upper_case_globals)]
                #vis const #write_ranges_arr_ident:
                    [::hopper::__runtime::write_policy::WriteRange; #write_ranges_len_ident] = {
                    // WritePolicy addresses accounts with `u8`; a
                    // flattened context wider than that index space
                    // cannot be strictly gated. This assert also bounds
                    // every `(offset) as u8` cast in the fill below and
                    // in the composed lamport list (module consts are
                    // evaluated eagerly, so it can never be skipped).
                    ::core::assert!(
                        (#account_count_expr) <= 256,
                        "composite context exceeds the u8 account-index space of WritePolicy"
                    );
                    let mut __out =
                        [::hopper::__runtime::write_policy::WriteRange::new(0, 0, 0);
                            #write_ranges_len_ident];
                    let mut __n = 0usize;
                    #(#fill_stmts)*
                    ::core::assert!(
                        __n == #write_ranges_len_ident,
                        "composed write-range fill must cover the declared length"
                    );
                    __out
                };
            };
            (items, quote! { &#write_ranges_arr_ident })
        };

    // Empty (and carrying no authority) unless `strict_writes` is on,
    // mirroring `InstructionDescriptor.write_ranges` semantics. With a
    // `#[composite]` field the strict set is the composed array above;
    // composite-free output stays byte-identical to the pre-composite
    // inline-literal lowering.
    let write_ranges_const_item = quote! {
        #[doc(hidden)]
        // The verbatim context name keeps the ident injective; consts
        // are conventionally SCREAMING so silence the case lint.
        #[allow(non_upper_case_globals)]
        #vis const #write_ranges_const_ident:
            &[::hopper::__runtime::write_policy::WriteRange] = #write_ranges_value;
    };
    // Under the lamport dimension the policy carries both dimensions and
    // `bind()` additionally installs the instruction-scoped lamport gate;
    // the returned RAII guard is stored on the bound context so the gate
    // lives exactly as long as the bound instruction scope. The install
    // is fallible (gate-store capacity/occupancy, `0xD1__` error page)
    // and fails the bind loudly via `?` rather than truncating the
    // governed set or sharing another gate's slot. The `static` is
    // emitted at function-statement level (no wrapping block) so the
    // guard binding stays in scope for the bound-struct constructor.
    let write_policy_install_stmt: TokenStream = if mutation_complete {
        quote! {
            static __HOPPER_WRITE_POLICY:
                ::hopper::__runtime::write_policy::WritePolicy =
                ::hopper::__runtime::write_policy::WritePolicy::with_lamports(
                    #write_ranges_const_ident,
                    #lamport_accounts_const_ident,
                );
            ctx.set_write_policy(&__HOPPER_WRITE_POLICY);
            let __hopper_lamport_gate =
                ::hopper::__runtime::write_policy::try_install_lamport_gate(
                    ctx.accounts(),
                    &__HOPPER_WRITE_POLICY,
                )?;
        }
    } else if strict_writes_enabled {
        quote! {
            {
                static __HOPPER_WRITE_POLICY:
                    ::hopper::__runtime::write_policy::WritePolicy =
                    ::hopper::__runtime::write_policy::WritePolicy::new(
                        #write_ranges_const_ident,
                    );
                ctx.set_write_policy(&__HOPPER_WRITE_POLICY);
            }
        }
    } else {
        TokenStream::new()
    };
    // Bound-struct plumbing for the gate guard (empty unless the lamport
    // dimension was declared).
    let lamport_gate_field_decl: TokenStream = if mutation_complete {
        quote! {
            #[doc(hidden)]
            __hopper_lamport_gate:
                ::hopper::__runtime::write_policy::LamportGateGuard<'a>,
        }
    } else {
        TokenStream::new()
    };
    let lamport_gate_bound_field: TokenStream = if mutation_complete {
        quote! { __hopper_lamport_gate, }
    } else {
        TokenStream::new()
    };
    // BLD-MUT steering: a mutation-complete context exposes the GATED
    // lamport transfer as a first-class bound-context method, so the
    // discoverable spelling under `lamports(...)` is the one whose
    // mutation the gate can see. No new codegen surface style: it is a
    // fixed-name thin delegation like the unconditional `account()` /
    // `program_id()` methods, emitted conditionally exactly like the
    // gate-guard field above. It delegates to
    // `hopper_runtime::transfer_lamports`, which runs the substrate
    // helper's arithmetic through the gated `native_boundary` funnel
    // (both sides checked BEFORE any balance change). The generated
    // lifecycle helpers already route through that same funnel:
    // `sweep_*` calls `try_set_lamports` directly, `close_*` goes via
    // `safe_close_with_sentinel` -> `move_all_lamports`, and
    // `realloc_*` via `safe_realloc` — all `try_set_lamports` inside.
    let gated_transfer_method: TokenStream = if mutation_complete {
        quote! {
            /// Gate-checked lamport transfer between two accounts of
            /// this instruction (BLD-MUT). Delegates to
            /// `hopper_runtime::transfer_lamports`: both sides are
            /// checked against this context's declared lamport set
            /// **before** any balance changes (refusal is
            /// `Custom(0xD000 | account_index)`), insufficient funds /
            /// overflow are checked before either side is applied, and
            /// a same-address transfer is a balance-checked net zero.
            /// Prefer this over the substrate
            /// `batch::transfer_lamports`, which bypasses the lamport
            /// gate by design.
            #[inline]
            #vis fn transfer_lamports(
                &self,
                from: &::hopper::prelude::AccountView<'_>,
                to: &::hopper::prelude::AccountView<'_>,
                amount: u64,
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                ::hopper::__runtime::transfer_lamports(from, to, amount)
            }
        }
    } else {
        TokenStream::new()
    };

    let auto_lifecycle_stmts: Vec<TokenStream> = ctx_fields
        .iter()
        .flat_map(|cf| {
            let should_auto = context_options.auto_lifecycle || cf.attr.auto_lifecycle;
            if !should_auto {
                return Vec::new();
            }

            let field_name = &cf.name;
            let mut calls = Vec::new();
            if cf.attr.init || cf.attr.init_if_needed {
                let init_fn = format_ident!("init_{}", field_name);
                // Seeded init helpers take the declared instruction args
                // (their seed expressions may reference them); bind's body
                // has those bindings in scope as parameters.
                if has_instruction_args && cf.attr.seeds.is_some() {
                    let names = arg_names.clone();
                    calls.push(quote! { __hopper_bound.#init_fn(#(#names),*)?; });
                } else {
                    calls.push(quote! { __hopper_bound.#init_fn()?; });
                }
            }
            if cf.attr.realloc.is_some() {
                let realloc_fn = format_ident!("realloc_{}", field_name);
                calls.push(quote! { __hopper_bound.#realloc_fn()?; });
            }
            if cf.attr.close.is_some() {
                let close_fn = format_ident!("close_{}", field_name);
                calls.push(quote! { __hopper_bound.#close_fn()?; });
            }
            calls
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
    // the user's source. Skip re-emitting it - emitting twice would be a
    // duplicate-definition error. When called from `#[hopper::context]`
    // we keep the original passthrough since attribute macros own the
    // item they decorate.
    let original_struct: TokenStream = if emit_struct {
        quote! { #input }
    } else {
        TokenStream::new()
    };

    // ── Innovation I7: opt-in self-describing transactions ────────────
    //
    // Under `emit_touch_map` this context advertises `EMIT_TOUCH_MAP =
    // true` as a public associated const (emitted below on the spec
    // type). The DISPATCHER — which alone can see the handler's `Result`
    // — reads that const on the Ok path and, only then, calls
    // `Context::finish_with_touch_map()` to emit the instruction's
    // cumulative touch map as one `sol_log_data` record. That is what
    // makes the emit fire ONLY on success: a handler that returns `Err`
    // (via `?`, `require!`, a failed invariant, an access-control gate,
    // …) short-circuits before the dispatcher reaches the finish call, so
    // a failed, rolled-back instruction never advertises Write ranges for
    // state it did not keep.
    //
    // A `Drop` would be wrong here: Rust runs drop glue on EVERY scope
    // exit — the Ok return AND every early `?`/`Err` return — and a Drop
    // hook cannot observe the handler's `Result`, so it could not be
    // Ok-only (adversarial review, CONFIRMED P2).
    //
    // The dispatcher spells the call as a `const`-guarded `if`, so for
    // every context whose `EMIT_TOUCH_MAP` is `false` (the default) the
    // call is dead-code-eliminated to zero instructions. The runtime
    // helper additionally no-ops when hopper-runtime's `touch-map`
    // feature is off, so even an opted-in context pays nothing on a build
    // without that feature (the feature gate lives in the runtime helper,
    // not in macro-emitted `#[cfg]`).
    let emit_touch_map_flag: bool = context_options.emit_touch_map;

    // ── Self-CPI events: the bound-context one-liner ───────────────────
    //
    // Under `event_cpi`, the bound context gains the fixed-name
    // `emit_event_cpi(&event)` delegation (the same conditional
    // fixed-name pattern as the BLD-MUT `transfer_lamports` method):
    // encode `[0xE0, 0x1E, tag, payload]` via the runtime's zero-alloc
    // encoder, then self-invoke through the checked `invoke_signed`
    // tier with the event-authority signer seeds. The bump comes from
    // `self.bumps.event_authority` — gathered once at bind by the same
    // verify call that validated the PDA — so the emit itself derives
    // nothing.
    let emit_event_cpi_method: TokenStream = if context_options.event_cpi {
        // The synthetic fields are the two trailing slots. The authority
        // resolves at its FLATTENED local offset — the plain field index
        // composite-free (byte-identical to the pre-composite lowering),
        // a const-expr sum past a `#[composite]` field — so the emit
        // reads the correct trailing slot in both worlds.
        let authority_idx = &local_offsets[account_count - 2];
        quote! {
            /// Emit a `#[hopper::event]` as an authenticated self-CPI so
            /// indexers read it from the transaction's inner-instruction
            /// metadata (which RPC nodes do not truncate, unlike logs).
            ///
            /// Wire format: `[0xE0, 0x1E, tag, payload]` — 3 bytes of
            /// instruction-data overhead per event vs Anchor
            /// `emit_cpi!`'s 16 (8-byte instruction tag + 8-byte event
            /// discriminator). The self-CPI is signed by this context's
            /// auto-appended event-authority PDA using the bump captured
            /// at bind; the generated program dispatcher authenticates
            /// the event in the reserved `[0xE0, 0x1E]` sink before
            /// accepting it.
            #[inline]
            #vis fn emit_event_cpi<E: ::hopper::__runtime::cpi_event::CpiEvent>(
                &self,
                event: &E,
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                let __hopper_authority = self.ctx.account(__HOPPER_BASE + #authority_idx)?;
                let mut __hopper_buf =
                    [0u8; 3 + ::hopper::__runtime::cpi_event::MAX_EVENT_PAYLOAD];
                let __hopper_len = ::hopper::__runtime::cpi_event::encode_event_cpi(
                    <E as ::hopper::__runtime::cpi_event::CpiEvent>::TAG,
                    ::hopper::__runtime::cpi_event::CpiEvent::payload_bytes(event),
                    &mut __hopper_buf,
                )
                .ok_or(::hopper::__runtime::ProgramError::InvalidInstructionData)?;
                let __hopper_bump: [u8; 1] = [self.bumps.event_authority];
                let __hopper_seeds: [&[u8]; 2] = [
                    ::hopper::__runtime::cpi_event::EVENT_AUTHORITY_SEED,
                    &__hopper_bump,
                ];
                ::hopper::__runtime::cpi_event::invoke_event_cpi(
                    self.ctx.program_id(),
                    __hopper_authority,
                    &__hopper_buf[..__hopper_len],
                    &__hopper_seeds,
                )
            }
        }
    } else {
        TokenStream::new()
    };
    let event_cpi_flag: bool = context_options.event_cpi;

    // ── Composite (nested) contexts: embeddability + base-parametric API ──
    //
    // A context is EMBEDDABLE (can be a `#[composite]` field of another)
    // only when validating it is the WHOLE of binding it: no
    // `#[instruction(...)]` args (they can't be threaded through the
    // outer's bind), no `strict_writes` / `lamports(...)` /
    // `emit_touch_map` / `event_cpi` (an outer composite bind runs the
    // inner's validators only — it would never install the inner's
    // policy/gate, run its fused authority verify, or reach its
    // dispatcher consts, so an inner opt-in would be silently inert;
    // note the OUTER may declare all of these since composite v2 — they
    // compose across the boundary), no lifecycle / lazy-migration /
    // Metaplex-CPI helpers (bind-time writes and CPI surfaces live in
    // the inner's own `bind()`, which an outer composite bind never
    // invokes, so embedding one would silently stop e.g. the migrate
    // crank), and — because nesting is single-level — no `#[composite]`
    // field of its own. Others assert this const before embedding, so a
    // non-embeddable inner is a clean compile error.
    let has_lifecycle = ctx_fields.iter().any(|cf| {
        cf.attr.init
            || cf.attr.init_if_needed
            || cf.attr.zero
            || cf.attr.close.is_some()
            || cf.attr.realloc.is_some()
            || cf.attr.sweep.is_some()
            || cf.attr.migrate.is_some()
            || metadata_cpi_helper_declared(&cf.attr)
            || master_edition_cpi_helper_declared(&cf.attr)
    });
    let embeddable = !has_composite
        && !has_instruction_args
        && !context_options.strict_writes
        && context_options.lamports.is_none()
        && !context_options.emit_touch_map
        && !context_options.event_cpi
        && !context_options.auto_lifecycle
        && !has_lifecycle;

    // Inner-access methods on the bound context: one per `#[composite]`
    // field, returning the inner context's bound view rebased to its
    // flattened slot offset. The outer is always top-level (base 0 — a
    // container is not itself embeddable), so the offset is the concrete
    // `local_offsets[..]` and the turbofish is a concrete const expression.
    // The view is reconstructed WITHOUT re-validation (the outer bind
    // already validated the inner) by reborrowing the raw context.
    let mut composite_access_methods: Vec<TokenStream> = Vec::new();
    let mut composite_embeddable_asserts: Vec<TokenStream> = Vec::new();
    for (cf, _offset_expr) in &composite_fields {
        let field_name = &cf.name;
        let inner_spec = composite_spec_ty(&cf.ty)?;
        let inner_bound_ty = composite_bound_ty(&cf.ty)?;
        let local = &local_offsets[cf.index];
        let doc = format!(
            "Access the embedded `{}` context (a `#[composite]` nested context) as its own \
             bound context, rebased to its flattened slot offset. Reborrows the raw context, \
             so the returned view is tied to this `&mut` borrow; read nested PDA bumps through \
             `self.bumps().{}`.",
            field_name, field_name
        );
        composite_access_methods.push(quote! {
            #[doc = #doc]
            #[inline(always)]
            #vis fn #field_name(
                &mut self,
            ) -> ::core::result::Result<
                #inner_bound_ty<'_, 'a, { #local }>,
                ::hopper::__runtime::ProgramError,
            > {
                #inner_spec::__hopper_view_at::<{ #local }>(
                    &mut *self.ctx,
                    self.bumps.#field_name,
                )
            }
        });
        let inner_str = type_ident(&cf.ty)
            .map(|i| i.to_string())
            .unwrap_or_else(|_| quote!(#inner_spec).to_string());
        let msg = format!(
            "composite field `{}`: the inner context `{}` is not embeddable in Hopper v1. A \
             `#[composite]` inner context must be a plain validation context — no \
             `#[instruction(...)]` args, no `strict_writes` / `lamports(...)` / `emit_touch_map` \
             / `event_cpi` options, no `init` / `init_if_needed` / `zero` / `close` / `realloc` \
             / `sweep` / `migrate` (or Metaplex-CPI) lifecycle, and no nested `#[composite]` \
             field of its own. Flatten it into the outer context, or split it into a separate \
             instruction.",
            field_name, inner_str
        );
        composite_embeddable_asserts.push(quote! {
            const _: () = ::core::assert!(#inner_spec::__HOPPER_EMBEDDABLE, #msg);
        });
    }

    // The top-level validate/bind surface. Composite-free contexts expose
    // a base-parametric `validate_at` / `bind_at` (real logic) plus zero-cost
    // base-0 forwarders under the public `validate` / `bind` names, and the
    // `__hopper_gather_bumps_at` / `__hopper_view_at` hooks an outer uses to
    // embed them. A composite CONTAINER stays base-0 (a `const __HOPPER_BASE:
    // usize = 0` puts the base in scope for the per-field turbofishes and the
    // inner delegations) and emits no `_at` surface — nesting a container
    // inside another is rejected via `__HOPPER_EMBEDDABLE`.
    // `bumps_gather_stmts` is interpolated twice in the embeddable arm
    // (`bind_at` and `__hopper_gather_bumps_at`); quote consumes a `Vec`
    // repetition, so clone one copy for the second site. Same for the
    // lazy-migration pre-steps, spliced into whichever bind arm is built.
    let bumps_gather_stmts_hook = bumps_gather_stmts.clone();
    let migration_stmts_at = migration_stmts.clone();
    // Only EMBEDDABLE contexts get the base-parametric `_at` surface and the
    // embed hooks. A non-embeddable context — a composite container, or one
    // that opted into args / strict_writes / event_cpi / lifecycle — stays
    // base-0: those features bind bind-local state (`__hopper_lamport_gate`,
    // the fused event-authority bump) that only the full `bind` establishes,
    // so a stripped-down `__hopper_view_at` / `__hopper_gather_bumps_at`
    // could not reference it. Nesting such a context is refused by the
    // `__HOPPER_EMBEDDABLE` assertion at the outer's expansion.
    let validate_bind_fns: TokenStream = if embeddable {
        let bind_validation_body: TokenStream = if has_event_authority {
            quote! {
                ctx.require_accounts(__HOPPER_BASE + Self::ACCOUNT_COUNT)?;
                #(#bind_validation_stmts)*
            }
        } else {
            quote! { Self::validate_at::<__HOPPER_BASE>(ctx #top_arg_name_fragment)?; }
        };
        quote! {
            #(#per_field_validators)*

            /// Validate this context's accounts at a flattened base offset.
            ///
            /// `__HOPPER_BASE` is `0` at the top level (and every
            /// `__HOPPER_BASE + idx` const-folds back to `idx`, so the
            /// lowering is byte-identical to a non-composite context) and a
            /// concrete offset when this context is embedded as a
            /// `#[composite]` field of another. The public `validate` /
            /// `validate_with_args` forwards here with `0`.
            #[inline]
            pub fn validate_at<const __HOPPER_BASE: usize>(
                ctx: &::hopper::prelude::Context<'_>
                #top_arg_param_fragment
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                ctx.require_accounts(__HOPPER_BASE + Self::ACCOUNT_COUNT)?;
                #(#validation_stmts)*
                Ok(())
            }

            #[inline(always)]
            pub fn #top_validate_ident(
                ctx: &::hopper::prelude::Context<'_>
                #top_arg_param_fragment
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                Self::validate_at::<0>(ctx #top_arg_name_fragment)
            }

            /// Bind this context at a flattened base offset (see
            /// [`validate_at`](Self::validate_at)). The public `bind` /
            /// `bind_with_args` forwards here with `0`.
            #[inline]
            pub fn bind_at<'ctx, 'a, const __HOPPER_BASE: usize>(
                ctx: &'ctx mut ::hopper::prelude::Context<'a>
                #top_arg_param_fragment
            ) -> ::core::result::Result<
                #bound_name<'ctx, 'a, __HOPPER_BASE>,
                ::hopper::__runtime::ProgramError,
            > {
                // Lazy-migration pre-steps run BEFORE the validation
                // fragment so validators see the upgraded account. Empty
                // by construction today (a migrate field forces the
                // non-embeddable arm); spliced here so "bind migrates
                // before validating" holds structurally in every arm.
                #(#migration_stmts_at)*
                #bind_validation_body
                #write_policy_install_stmt
                let mut __hopper_bumps = <#bumps_name as ::core::default::Default>::default();
                #( #bumps_gather_stmts )*
                #accounts_init_stmt
                let __hopper_bound = #bound_name {
                    ctx,
                    #accounts_bound_field
                    #lamport_gate_bound_field
                    bumps: __hopper_bumps,
                };
                #(#auto_lifecycle_stmts)*
                #user_validate_call
                Ok(__hopper_bound)
            }

            #[inline(always)]
            pub fn #top_bind_ident<'ctx, 'a>(
                ctx: &'ctx mut ::hopper::prelude::Context<'a>
                #top_arg_param_fragment
            ) -> ::core::result::Result<#bound_name<'ctx, 'a>, ::hopper::__runtime::ProgramError> {
                Self::bind_at::<0>(ctx #top_arg_name_fragment)
            }

            /// Gather this context's PDA bumps at a flattened base offset
            /// WITHOUT re-validation, so an outer context can populate the
            /// nested `Bumps` slot of a `#[composite]` field after the inner
            /// has already been validated during the outer's `validate`.
            #[doc(hidden)]
            #[inline]
            pub fn __hopper_gather_bumps_at<const __HOPPER_BASE: usize>(
                ctx: &::hopper::prelude::Context<'_>,
            ) -> ::core::result::Result<#bumps_name, ::hopper::__runtime::ProgramError> {
                // `ctx` is unused when this context declares no PDA seeds.
                let _ = &ctx;
                let mut __hopper_bumps = <#bumps_name as ::core::default::Default>::default();
                #( #bumps_gather_stmts_hook )*
                Ok(__hopper_bumps)
            }

            /// Reconstruct the bound context at a flattened base offset
            /// WITHOUT re-running the full validation (already validated
            /// during the outer bind). The inner bumps are supplied by the
            /// caller. Fallible only because rebuilding the typed `accounts`
            /// facade re-runs the cheap wrapper role checks (`try_new`).
            #[doc(hidden)]
            #[inline]
            pub fn __hopper_view_at<'ctx, 'a, const __HOPPER_BASE: usize>(
                ctx: &'ctx mut ::hopper::prelude::Context<'a>,
                __hopper_bumps: #bumps_name,
            ) -> ::core::result::Result<
                #bound_name<'ctx, 'a, __HOPPER_BASE>,
                ::hopper::__runtime::ProgramError,
            > {
                #accounts_init_stmt
                ::core::result::Result::Ok(#bound_name {
                    ctx,
                    #accounts_bound_field
                    #lamport_gate_bound_field
                    bumps: __hopper_bumps,
                })
            }
        }
    } else {
        quote! {
            #(#per_field_validators)*

            /// Validate the account slice against this context spec.
            ///
            /// Runs each per-field validator and each `#[composite]` inner
            /// context's `validate_at` at the flattened offset, in
            /// declaration order (so error precedence follows field order).
            #[inline]
            pub fn #top_validate_ident(
                ctx: &::hopper::prelude::Context<'_>
                #top_arg_param_fragment
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                // A composite container is always top-level; the `const`
                // puts the base in scope for the per-field turbofishes and
                // the inner-context delegations (all concrete const exprs).
                const __HOPPER_BASE: usize = 0;
                ctx.require_accounts(Self::ACCOUNT_COUNT)?;
                #(#validation_stmts)*
                Ok(())
            }

            /// Bind a raw Hopper context into the typed proc-macro wrapper.
            #[inline]
            pub fn #top_bind_ident<'ctx, 'a>(
                ctx: &'ctx mut ::hopper::prelude::Context<'a>
                #top_arg_param_fragment
            ) -> ::core::result::Result<#bound_name<'ctx, 'a>, ::hopper::__runtime::ProgramError> {
                const __HOPPER_BASE: usize = 0;
                // Lazy-migration pre-steps (`migrate(from = Old, with = f)`)
                // fire BEFORE the validation fragment: a slot still holding
                // a fully-valid OLD-layout header is migrated in place, so
                // the New-layout validators below see the upgraded account.
                // A migration error propagates as the instruction error
                // (the runtime then rolls the transaction back), matching
                // `migrate_layout`'s atomicity contract.
                #(#migration_stmts)*
                #bind_validate_fragment
                #write_policy_install_stmt
                let mut __hopper_bumps = <#bumps_name as ::core::default::Default>::default();
                #( #bumps_gather_stmts )*
                #accounts_init_stmt
                let __hopper_bound = #bound_name {
                    ctx,
                    #accounts_bound_field
                    #lamport_gate_bound_field
                    bumps: __hopper_bumps,
                };
                #(#auto_lifecycle_stmts)*
                #user_validate_call
                Ok(__hopper_bound)
            }
        }
    };

    let expanded = quote! {
        // Emit the original struct unchanged (attribute macro path only).
        #original_struct

        // Single source of truth for this context's declared byte-range
        // write-set (BLD-I24). Referenced by the runtime `WritePolicy`
        // that `bind()` installs under `strict_writes`, by the
        // `WRITE_RANGES` associated const, and by
        // `SCHEMA_METADATA.write_ranges`, so the scheduler-legible
        // published set is byte-identical to the enforced set. For a
        // composite container under `strict_writes` the slice points at
        // the compile-time-composed array emitted just below it.
        #write_ranges_const_item
        #composed_write_ranges_items
        #schema_len_module_item

        // BLD-MUT: single source of truth for the lamport permission
        // set (explicit `lamports(...)` + implied lifecycle roles).
        // Backs the runtime `WritePolicy`'s lamport dimension,
        // `LAMPORT_ACCOUNTS`, and `SCHEMA_METADATA.lamport_accounts`.
        #lamport_accounts_const_item

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

        // The bound context is generic over `__HOPPER_BASE`, the flattened
        // slot offset of the fields this context owns within the whole
        // instruction. It defaults to `0` — the top-level case — so every
        // existing spelling `#bound_name<'ctx, 'a>` keeps working and every
        // accessor's `__HOPPER_BASE + idx` const-folds back to `idx` (the
        // pre-composite, zero-cost lowering). When this context is embedded
        // as a `#[composite]` field, the outer binds it at a concrete
        // offset and `__HOPPER_BASE + idx` folds to the correct absolute
        // slot at monomorphization — no runtime add.
        #vis struct #bound_name<'ctx, 'a, const __HOPPER_BASE: usize = 0> {
            ctx: &'ctx mut ::hopper::prelude::Context<'a>,
            #accounts_field_decl
            #lamport_gate_field_decl
            pub bumps: #bumps_name,
        }

        #vis struct #receipt_scope_name<const SNAP: usize> {
            #(#receipt_scope_fields)*
        }

        impl #impl_generics #name #ty_generics #where_clause {
            /// Number of accounts this context requires.
            pub const ACCOUNT_COUNT: usize = #account_count_expr;
            pub const RECEIPT_EXPECTED: bool = #receipt_expected;
            pub const MUTABLE_ACCOUNT_COUNT: usize = #mutable_account_count;

            /// Innovation I7: whether this context opts into
            /// self-describing transactions (`#[hopper::context(emit_touch_map)]`).
            ///
            /// `true` only when the author opted in. The generated
            /// dispatcher reads this const on the handler's **Ok** path
            /// and, only then, calls `Context::finish_with_touch_map()` —
            /// so the touch-map `sol_log_data` record is emitted
            /// exclusively on success, never on an `Err`/rolled-back
            /// instruction. `false` (the default) makes the dispatcher's
            /// `const`-guarded call dead-code-eliminate to zero
            /// instructions, so a context that did not opt in pays
            /// nothing.
            pub const EMIT_TOUCH_MAP: bool = #emit_touch_map_flag;

            /// Whether this context opts into self-CPI events
            /// (`#[hopper::context(event_cpi)]`).
            ///
            /// `true` only when the author opted in: the two trailing
            /// event accounts are then auto-appended (they are the last
            /// two of [`ACCOUNT_COUNT`](Self::ACCOUNT_COUNT)) and the
            /// bound context exposes `emit_event_cpi(&event)`. The
            /// generated `#[hopper::program]` dispatcher ORs this const
            /// across its typed contexts to decide — at compile time —
            /// whether the reserved `[0xE0, 0x1E]` event-sink arm is
            /// live; `false` everywhere makes that guard dead-code-
            /// eliminate, so programs without the feature pay nothing.
            pub const EVENT_CPI: bool = #event_cpi_flag;

            /// Number of individual validation checks performed.
            pub const VALIDATION_CHECK_COUNT: usize = #check_count;

            /// Human-readable descriptions of every validation check.
            ///
            /// Inspect this constant (or use `hopper compile --emit rust`) to
            /// see exactly what `validate()` enforces. nothing is hidden.
            pub const VALIDATION_CHECKS: &'static [&'static str] = &[
                #(#check_desc_literals),*
            ];

            #schema_support_items

            /// Full Anchor-grade schema metadata: lifecycle role, PDA
            /// seeds, `has_one` edges, `payer`/`space` for init,
            /// `address`/`owner` pins. everything the audit's
            /// Stage 2.5 closure asks client generators and IDL tools
            /// to consume without re-parsing source. The `const`
            /// guarantees it's available at compile time too. For a
            /// context embedding `#[composite]` fields the descriptor
            /// list is compile-time-composed to one entry per FLATTENED
            /// slot (inner contexts spliced verbatim), length-pinned to
            /// `ACCOUNT_COUNT` by a const assert.
            pub const SCHEMA_METADATA: ::hopper::hopper_schema::accounts::ContextDescriptor =
                ::hopper::hopper_schema::accounts::ContextDescriptor {
                    name: #ctx_name_lit,
                    accounts: #schema_accounts_expr,
                    policies: &[],
                    receipts_expected: #receipt_expected,
                    mutation_classes: &[],
                    strict_writes: #strict_writes_enabled,
                    write_ranges: #write_ranges_const_ident,
                    mutation_complete: #mutation_complete,
                    lamport_accounts: #lamport_accounts_const_ident,
                };

            /// Whether this context was compiled with `strict_writes`.
            ///
            /// When `true`, [`WRITE_RANGES`](Self::WRITE_RANGES) is the
            /// complete, runtime-enforced write surface: `bind()` installs
            /// a `WritePolicy` built from the very same const, so any
            /// Context-mediated write outside the set fails at acquisition
            /// time. When `false`, the range set is empty and carries no
            /// authority.
            pub const STRICT_WRITES: bool = #strict_writes_enabled;

            /// Declared byte-range write-set for this context, compiled
            /// from its `mut` / `mut(seg, ...)` / lifecycle declarations.
            ///
            /// This slice, `SCHEMA_METADATA.write_ranges`, and the runtime
            /// `WritePolicy` installed by `bind()` under `strict_writes`
            /// all read the same generated const, so what schedulers and
            /// indexers see published is byte-identical to what the
            /// runtime enforces. Wire it into a manifest
            /// `InstructionDescriptor` as
            /// `write_ranges: MyCtx::WRITE_RANGES` (with
            /// `strict_writes: MyCtx::STRICT_WRITES`) — no hand-authored
            /// offsets.
            pub const WRITE_RANGES:
                &'static [::hopper::__runtime::write_policy::WriteRange] =
                #write_ranges_const_ident;

            #declared_write_ranges_item

            /// Whether this context's declared write set covers **both**
            /// mutation dimensions — data byte ranges AND lamports
            /// (BLD-MUT). `true` only for `strict_writes` +
            /// `lamports(...)`: `bind()` then installs a lamport gate and
            /// the runtime refuses any lamport mutation or writable CPI
            /// hand-off outside [`LAMPORT_ACCOUNTS`](Self::LAMPORT_ACCOUNTS).
            /// A bare `strict_writes` context leaves lamports ungoverned
            /// and is deliberately NOT mutation-complete. Wire into a
            /// manifest `InstructionDescriptor` as
            /// `mutation_complete: MyCtx::MUTATION_COMPLETE,
            /// lamport_accounts: MyCtx::LAMPORT_ACCOUNTS`.
            pub const MUTATION_COMPLETE: bool = #mutation_complete;

            /// Account indices permitted to have their lamports mutated:
            /// explicit `lamports(...)` names plus the implied lifecycle
            /// set (whole-`mut` accounts, init account + payer, close
            /// account + destination, realloc account + payer, sweep
            /// account + target, and the writable metas of generated
            /// Metaplex helpers: created metadata/edition PDA, payer,
            /// master-edition mint). The same generated const backs the
            /// enforced `WritePolicy` and `SCHEMA_METADATA`, so published
            /// and enforced sets cannot drift. Empty (no authority)
            /// unless [`MUTATION_COMPLETE`](Self::MUTATION_COMPLETE).
            pub const LAMPORT_ACCOUNTS: &'static [u8] = #lamport_accounts_const_ident;

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

            // ── Per-field validators + top-level validate/bind ───────────
            //
            // Composite-free contexts get a base-parametric `validate_at` /
            // `bind_at` plus zero-cost base-0 forwarders (`validate` /
            // `bind`, or the `_with_args` variants) and the embed hooks
            // `__hopper_gather_bumps_at` / `__hopper_view_at`. A composite
            // CONTAINER stays base-0 and delegates to each inner context's
            // `validate_at` at the flattened offset. Built above so the two
            // shapes share the per-field validators without duplication.
            #validate_bind_fns

            /// Whether this context may be embedded as a `#[composite]`
            /// field of another (see the embeddability rules on
            /// `#[composite]`). Others assert this before nesting, so a
            /// non-embeddable inner is a clean compile-time error.
            #[doc(hidden)]
            pub const __HOPPER_EMBEDDABLE: bool = #embeddable;

            #[inline]
            pub fn begin_receipt_scope<const SNAP: usize>(
                ctx: &::hopper::prelude::ScopedContext<'_, '_>,
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
                ctx: &::hopper::prelude::ScopedContext<'_, '_>,
                invariants_passed: bool,
                invariants_checked: u16,
                failure: ::core::option::Option<(
                    u32,
                    u8,
                    ::hopper::receipt::FailureStage,
                )>,
            ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
                #(#receipt_finish_blocks)*
                Ok(())
            }
        }

        impl<'ctx, 'a, const __HOPPER_BASE: usize> #bound_name<'ctx, 'a, __HOPPER_BASE> {
            /// Borrow-scoped access to the underlying raw Hopper context.
            ///
            /// Account references returned through this value are tied to the
            /// borrow of the generated context, not to the raw instruction
            /// lifetime. Use `raw_unchecked()` only when an audited escape
            /// from that restriction is required.
            #[inline(always)]
            #vis fn raw(&mut self) -> ::hopper::prelude::ScopedContext<'_, 'a> {
                ::hopper::prelude::ScopedContext::new(self.ctx)
            }

            /// Direct access to the underlying raw Hopper context.
            ///
            /// # Safety
            ///
            /// Returning `&mut Context<'a>` exposes methods whose account
            /// references are parameterized by the raw instruction lifetime.
            /// Callers must not return, store, or otherwise leak those
            /// references beyond the generated context/instruction scope.
            #[inline(always)]
            #vis unsafe fn raw_unchecked(&mut self) -> &mut ::hopper::prelude::Context<'a> {
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
                &::hopper::prelude::AccountView<'_>,
                ::hopper::__runtime::ProgramError,
            > {
                self.ctx.account(index)
            }

            #[inline(always)]
            #vis fn account_mut(
                &self,
                index: usize,
            ) -> ::core::result::Result<
                &::hopper::prelude::AccountView<'_>,
                ::hopper::__runtime::ProgramError,
            > {
                self.ctx.account_mut(index)
            }

            #[inline(always)]
            #vis fn remaining_accounts(
                &self,
            ) -> ::hopper::hopper_runtime::remaining::RemainingAccounts<'_> {
                self.ctx.remaining_accounts_strict(#account_count_expr)
            }

            #[inline(always)]
            #vis fn remaining_accounts_passthrough(
                &self,
            ) -> ::hopper::hopper_runtime::remaining::RemainingAccounts<'_> {
                self.ctx.remaining_accounts_passthrough(#account_count_expr)
            }

            #[inline(always)]
            #vis fn remaining_typed(
                &self,
            ) -> ::hopper::hopper_runtime::remaining::RemainingTyped<'_> {
                self.ctx.remaining_accounts_typed(#account_count_expr)
            }

            #[inline(always)]
            #vis fn remaining_lazy(
                &self,
            ) -> ::hopper::hopper_runtime::remaining::RemainingLazy<'_> {
                self.ctx.remaining_accounts_lazy(#account_count_expr)
            }

            #[inline(always)]
            #vis fn remaining_accounts_raw(&self) -> &[::hopper::prelude::AccountView<'_>] {
                self.ctx.remaining_accounts(#account_count_expr)
            }

            #gated_transfer_method

            #emit_event_cpi_method

            // --- Composite (nested) context accessors ---
            #(#composite_access_methods)*

            // --- Generated segment accessors ---
            #(#accessors)*
        }

        // Compile-time embeddability guard for each `#[composite]` field:
        // a non-embeddable inner context fails HERE with an actionable
        // message rather than deeper in the generated delegation.
        #(#composite_embeddable_asserts)*
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
/// After a constraint's value has been parsed, peek for Anchor's
/// `@ CustomError` error-override token and, if present, consume the
/// `@` and parse the trailing error expression.
///
/// Anchor spells a per-constraint error as
/// `has_one = authority @ MyError::WrongAuth`,
/// `constraint = x == y @ MyError::Bad`, `address = k @ MyError::BadAddr`,
/// etc. `@` (`Token![@]`) is never a valid continuation of an
/// expression, so the preceding `Expr`/`Ident` parse always stops exactly
/// before it — leaving `input` positioned on the `@` for this peek.
/// Returns `None` when no `@` follows, so callers keep their existing
/// generic error path byte-for-byte unchanged.
fn parse_opt_at_error(input: ParseStream) -> Result<Option<Expr>> {
    if input.peek(Token![@]) {
        let _at: Token![@] = input.parse()?;
        Ok(Some(input.parse()?))
    } else {
        Ok(None)
    }
}

fn parse_account_attr(attrs: &[Attribute]) -> Result<AccountAttr> {
    let mut result = AccountAttr::default();

    for attr in attrs {
        if attr.path().is_ident("signer") {
            result.is_signer = true;
            continue;
        }

        // `#[composite]` bare marker (parallel to `#[signer]`): the field
        // is a nested context. It must be a `Meta::Path` (no argument
        // list); `#[composite(...)]` is rejected so the surface stays
        // exactly the Anchor-adjacent bare form.
        if attr.path().is_ident("composite") {
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                return Err(syn::Error::new_spanned(
                    attr,
                    "`#[composite]` is a bare marker and takes no arguments; \
                     the nested context's own `#[account(...)]` constraints live \
                     on its fields, not on the composite field",
                ));
            }
            result.composite = true;
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
                    ("confidential_transfer", "mint") => {
                        result.ext_confidential_transfer_mint = true;
                        Ok(())
                    }
                    ("confidential_transfer", "account") => {
                        result.ext_confidential_transfer_account = true;
                        Ok(())
                    }
                    ("scaled_ui_amount", "config") => {
                        result.ext_scaled_ui_amount_config = true;
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
                        result.token_mint_err = parse_opt_at_error(meta.input)?;
                        Ok(())
                    }
                    ("token", "authority") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.token_authority = Some(expr);
                        result.token_authority_err = parse_opt_at_error(meta.input)?;
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
                    ("metadata", "name") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.metadata_name = Some(expr);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "symbol") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.metadata_symbol = Some(expr);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "uri") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.metadata_uri = Some(expr);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "seller_fee_basis_points") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.metadata_seller_fee_basis_points = Some(expr);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "is_mutable") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.metadata_is_mutable = Some(expr);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "mint") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.metadata_mint = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "mint_authority") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.metadata_mint_authority = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "payer") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.metadata_payer = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "update_authority") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.metadata_update_authority = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "system_program") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.metadata_system_program = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("metadata", "rent") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.metadata_rent = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "max_supply") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.master_edition_max_supply = Some(expr);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "mint") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.master_edition_mint = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "metadata") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.master_edition_metadata = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "update_authority") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.master_edition_update_authority = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "mint_authority") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.master_edition_mint_authority = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "payer") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.master_edition_payer = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "token_program") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.master_edition_token_program = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "system_program") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.master_edition_system_program = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("master_edition", "rent") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.master_edition_rent = Some(ident);
                        result.is_mut = true;
                        Ok(())
                    }
                    ("seeds", "program") => {
                        let expr: Expr = meta.value()?.parse()?;
                        result.seeds_program = Some(expr);
                        Ok(())
                    }
                    // Anchor colon spelling `realloc::payer = p`. Pure
                    // alias of the underscore `realloc_payer = p` field;
                    // both spellings resolve to the same slot so an Anchor
                    // program ports mechanically without a rewrite.
                    ("realloc", "payer") => {
                        let ident: Ident = meta.value()?.parse()?;
                        result.realloc_payer = Some(ident);
                        Ok(())
                    }
                    // Anchor colon spelling `realloc::zero = true`. Alias
                    // of the underscore `realloc_zero`; accepts the bare
                    // form (meaning true) exactly like its sibling.
                    ("realloc", "zero") => {
                        if meta.input.peek(Token![=]) {
                            let lit: syn::LitBool = meta.value()?.parse()?;
                            result.realloc_zero = lit.value;
                        } else {
                            result.realloc_zero = true;
                        }
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
                    ("extensions", "cpi_guard") => {
                        result.ext_cpi_guard = true;
                        Ok(())
                    }
                    ("extensions", "confidential_transfer_mint") => {
                        result.ext_confidential_transfer_mint = true;
                        Ok(())
                    }
                    ("extensions", "confidential_transfer_account") => {
                        result.ext_confidential_transfer_account = true;
                        Ok(())
                    }
                    ("extensions", "scaled_ui_amount_config") => {
                        result.ext_scaled_ui_amount_config = true;
                        Ok(())
                    }
                    _ => Err(meta.error(format!(
                        "unrecognized nested account attribute `{ns}::{key}`. \
                         accepted namespaces: token::{{mint,authority,token_program}}, \
                         mint::{{authority,decimals,freeze_authority,token_program}}, \
                         associated_token::{{mint,authority,token_program}}, \
                         metadata::{{name,symbol,uri,seller_fee_basis_points,is_mutable,mint,mint_authority,payer,update_authority,system_program,rent}}, \
                         master_edition::{{max_supply,mint,metadata,update_authority,mint_authority,payer,token_program,system_program,rent}}, \
                         seeds::{{program}}, realloc::{{payer,zero}}",
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
                "tail" => {
                    // `tail(seq_field)` — declare a growable `Seq<T>` tail
                    // as writable via an open-ended `tail_from` range. Marks
                    // the field writable (like `mut(seg)`); pair with
                    // `realloc` to grow it.
                    if meta.input.peek(syn::token::Paren) {
                        let content;
                        syn::parenthesized!(content in meta.input);
                        let segments: Punctuated<Ident, Comma> =
                            content.parse_terminated(Ident::parse, Token![,])?;
                        for seg in segments {
                            result.tail_segments.push(seg.to_string());
                        }
                    } else {
                        return Err(meta.error(
                            "`tail` requires a named tail segment, for example `tail(members)`",
                        ));
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
                "auto" | "auto_lifecycle" => {
                    result.auto_lifecycle = true;
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
                    // Anchor `has_one = field @ Err`: capture the optional
                    // per-constraint error, kept parallel to `has_one`.
                    result.has_one_errs.push(parse_opt_at_error(meta.input)?);
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
                    // `sweep` drains this account's lamports, which the SVM
                    // only permits on writable accounts — the documented
                    // "implies `mut`" contract, enforced.
                    result.is_mut = true;
                    Ok(())
                }
                "migrate" => {
                    // `migrate(from = OldLayout, with = path::to::transform)`.
                    // Both keys are mandatory; every malformed spelling names
                    // the expected shape so the fix is copy-pasteable.
                    const MIGRATE_SHAPE: &str =
                        "expected `migrate(from = OldLayout, with = path::to::transform)`";
                    if result.migrate.is_some() {
                        return Err(meta.error("`migrate(...)` may only be set once per field"));
                    }
                    if !meta.input.peek(syn::token::Paren) {
                        return Err(meta.error(format!(
                            "malformed `migrate` attribute: {MIGRATE_SHAPE}"
                        )));
                    }
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let mut from_ty: Option<Type> = None;
                    let mut with_path: Option<syn::Path> = None;
                    while !content.is_empty() {
                        let key: Ident = content.parse()?;
                        let _: Token![=] = content.parse()?;
                        match key.to_string().as_str() {
                            "from" => {
                                if from_ty.is_some() {
                                    return Err(syn::Error::new_spanned(
                                        key,
                                        format!("`migrate(...)` sets `from` twice: {MIGRATE_SHAPE}"),
                                    ));
                                }
                                from_ty = Some(content.parse()?);
                            }
                            "with" => {
                                if with_path.is_some() {
                                    return Err(syn::Error::new_spanned(
                                        key,
                                        format!("`migrate(...)` sets `with` twice: {MIGRATE_SHAPE}"),
                                    ));
                                }
                                with_path = Some(content.parse()?);
                            }
                            other => {
                                return Err(syn::Error::new_spanned(
                                    key,
                                    format!("unknown `migrate` key `{other}`: {MIGRATE_SHAPE}"),
                                ));
                            }
                        }
                        if content.peek(Token![,]) {
                            let _: Token![,] = content.parse()?;
                        }
                    }
                    let Some(from_ty) = from_ty else {
                        return Err(meta.error(format!(
                            "`migrate(...)` is missing `from = OldLayout`: {MIGRATE_SHAPE}"
                        )));
                    };
                    let Some(with_path) = with_path else {
                        return Err(meta.error(format!(
                            "`migrate(...)` is missing `with = path::to::transform`: {MIGRATE_SHAPE}"
                        )));
                    };
                    result.migrate = Some((from_ty, with_path));
                    Ok(())
                }
                "owner" => {
                    let expr: Expr = meta.value()?.parse()?;
                    result.owner = Some(expr);
                    result.owner_err = parse_opt_at_error(meta.input)?;
                    Ok(())
                }
                "owner_any" => {
                    let array: syn::ExprArray = meta.value()?.parse()?;
                    if array.elems.is_empty() {
                        return Err(meta.error(
                            "owner_any requires at least one program id, e.g. owner_any = [token::ID, token_2022::ID]",
                        ));
                    }
                    result.owner_any = array.elems.into_iter().collect();
                    Ok(())
                }
                "address" => {
                    let expr: Expr = meta.value()?.parse()?;
                    result.address = Some(expr);
                    result.address_err = parse_opt_at_error(meta.input)?;
                    Ok(())
                }
                "constraint" => {
                    let expr: Expr = meta.value()?.parse()?;
                    result.constraint.push(expr);
                    // Anchor `constraint = expr @ Err`: capture the optional
                    // per-guard error, kept parallel to `constraint`.
                    result
                        .constraint_errs
                        .push(parse_opt_at_error(meta.input)?);
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
    if attr.owner.is_some() && !attr.owner_any.is_empty() {
        return Err(syn::Error::new_spanned(
            field_name,
            "use either `owner = expr` or `owner_any = [..]`, not both",
        ));
    }
    if attr.init || attr.init_if_needed {
        let kw = if attr.init_if_needed {
            "init_if_needed"
        } else {
            "init"
        };
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
        // A PDA of *another* program cannot sign its own creation from
        // this program, so `init` + `seeds::program` can never succeed
        // at runtime. Reject at compile time instead.
        if attr.seeds_program.is_some() {
            return Err(syn::Error::new_spanned(
                field_name,
                format!(
                    "#[account({}, seeds::program = ...)] cannot be created here: only the owning \
                     program can sign for its PDA. Create the account through that program's own \
                     instruction, then reference it without `{}`.",
                    kw, kw
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

    let metadata_data_any = attr.metadata_name.is_some()
        || attr.metadata_symbol.is_some()
        || attr.metadata_uri.is_some()
        || attr.metadata_seller_fee_basis_points.is_some()
        || attr.metadata_is_mutable.is_some();
    let metadata_data_complete = attr.metadata_name.is_some()
        && attr.metadata_symbol.is_some()
        && attr.metadata_uri.is_some()
        && attr.metadata_seller_fee_basis_points.is_some();
    if metadata_data_any && !metadata_data_complete {
        return Err(syn::Error::new_spanned(
            field_name,
            "metadata constraints require `metadata::name`, `metadata::symbol`, `metadata::uri`, and `metadata::seller_fee_basis_points` together",
        ));
    }
    let metadata_cpi_any = attr.metadata_mint.is_some()
        || attr.metadata_mint_authority.is_some()
        || attr.metadata_payer.is_some()
        || attr.metadata_update_authority.is_some()
        || attr.metadata_system_program.is_some()
        || attr.metadata_rent.is_some();
    if metadata_cpi_any
        && (!metadata_data_complete
            || attr.metadata_mint.is_none()
            || attr.metadata_mint_authority.is_none()
            || attr.metadata_payer.is_none()
            || attr.metadata_update_authority.is_none()
            || attr.metadata_system_program.is_none())
    {
        return Err(syn::Error::new_spanned(
            field_name,
            "metadata CPI helpers require metadata::{mint,mint_authority,payer,update_authority,system_program,name,symbol,uri,seller_fee_basis_points}; `metadata::rent` is optional",
        ));
    }

    let master_edition_any = attr.master_edition_max_supply.is_some()
        || attr.master_edition_mint.is_some()
        || attr.master_edition_metadata.is_some()
        || attr.master_edition_update_authority.is_some()
        || attr.master_edition_mint_authority.is_some()
        || attr.master_edition_payer.is_some()
        || attr.master_edition_token_program.is_some()
        || attr.master_edition_system_program.is_some()
        || attr.master_edition_rent.is_some();
    if (metadata_data_any || metadata_cpi_any) && master_edition_any {
        return Err(syn::Error::new_spanned(
            field_name,
            "declare `metadata::*` and `master_edition::*` on separate account fields",
        ));
    }
    if master_edition_any
        && (attr.master_edition_max_supply.is_none()
            || attr.master_edition_mint.is_none()
            || attr.master_edition_metadata.is_none()
            || attr.master_edition_update_authority.is_none()
            || attr.master_edition_mint_authority.is_none()
            || attr.master_edition_payer.is_none()
            || attr.master_edition_token_program.is_none()
            || attr.master_edition_system_program.is_none())
    {
        return Err(syn::Error::new_spanned(
            field_name,
            "master_edition helpers require master_edition::{max_supply,mint,metadata,update_authority,mint_authority,payer,token_program,system_program}; `master_edition::rent` is optional",
        ));
    }
    Ok(())
}

/// Whether this field declares the complete `metadata::*` surface that
/// makes `generate` emit the `create_<field>()` CreateMetadataAccountV3
/// CPI helper. Must stay in lock-step with the emission tuple in
/// `generate` and the completeness validation in `validate_account_attr`
/// — the BLD-MUT implied lamport union keys off this predicate, and a
/// drift would republish an incomplete (dishonest) permission set.
fn metadata_cpi_helper_declared(attr: &AccountAttr) -> bool {
    attr.metadata_name.is_some()
        && attr.metadata_symbol.is_some()
        && attr.metadata_uri.is_some()
        && attr.metadata_seller_fee_basis_points.is_some()
        && attr.metadata_mint.is_some()
        && attr.metadata_mint_authority.is_some()
        && attr.metadata_payer.is_some()
        && attr.metadata_update_authority.is_some()
        && attr.metadata_system_program.is_some()
}

/// Whether this field declares the complete `master_edition::*` surface
/// that makes `generate` emit the `create_<field>()`
/// CreateMasterEditionV3 CPI helper. Same lock-step contract as
/// [`metadata_cpi_helper_declared`].
fn master_edition_cpi_helper_declared(attr: &AccountAttr) -> bool {
    attr.master_edition_max_supply.is_some()
        && attr.master_edition_mint.is_some()
        && attr.master_edition_metadata.is_some()
        && attr.master_edition_update_authority.is_some()
        && attr.master_edition_mint_authority.is_some()
        && attr.master_edition_payer.is_some()
        && attr.master_edition_token_program.is_some()
        && attr.master_edition_system_program.is_some()
}

/// Grant the field at `pos` the permission pair a **writable CPI meta**
/// requires under the BLD-MUT gate: lamport permission plus a
/// whole-account data range. `check_lamport_delegation` demands both,
/// because handing an account writable to a callee is unbounded
/// delegation of both mutation dimensions. Dedupes against grants
/// already implied by lifecycle roles or named explicitly in
/// `lamports(...)`. Operates on FIELD POSITIONS (not `u8` indices) so
/// the caller can render the resulting extra whole-account ranges —
/// recorded in grant order in `delegable_extra_positions` — in either
/// token shape: `u8` literals composite-free, const-expr flattened
/// offsets in a composite container.
fn grant_cpi_delegable(
    pos: usize,
    lamport_positions: &mut Vec<usize>,
    whole_account_positions: &mut Vec<usize>,
    delegable_extra_positions: &mut Vec<usize>,
) {
    if !lamport_positions.contains(&pos) {
        lamport_positions.push(pos);
    }
    if !whole_account_positions.contains(&pos) {
        whole_account_positions.push(pos);
        delegable_extra_positions.push(pos);
    }
}

fn sibling_index(ctx_fields: &[ContextField], ident: &Ident, role: &str) -> Result<usize> {
    ctx_fields
        .iter()
        .position(|field| field.name == *ident)
        .ok_or_else(|| {
            syn::Error::new_spanned(
                ident,
                format!(
                    "{} references `{}`, but no sibling context field has that name",
                    role, ident
                ),
            )
        })
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

fn reject_reference_wrapped_account(field_name: &Ident, ty: &Type) -> Result<()> {
    let Type::Reference(reference) = ty else {
        return Ok(());
    };

    let Some(wrapper) = classify_wrapper(reference.elem.as_ref()) else {
        return Ok(());
    };

    match wrapper {
        WrapperKind::Account { inner } => {
            let inner = type_ident(&inner)
                .map(|ident| ident.to_string())
                .unwrap_or_else(|_| "T".to_string());
            let account_ty = format!("Account<'info, {}>", inner);
            let hint = if reference.mutability.is_some() {
                format!("#[account(mut)] pub {}: {}", field_name, account_ty)
            } else {
                format!("pub {}: {}", field_name, account_ty)
            };
            let reason = if reference.mutability.is_some() {
                "mutable access is enforced by Hopper account-data guards, not by `&mut` on the wrapper"
            } else {
                "the wrapper is already a borrowed role view over the account"
            };
            Err(syn::Error::new_spanned(
                ty,
                format!("Use `{}` in Hopper; {}.", hint, reason),
            ))
        }
        WrapperKind::InitAccount { inner } => {
            let inner = type_ident(&inner)
                .map(|ident| ident.to_string())
                .unwrap_or_else(|_| "T".to_string());
            Err(syn::Error::new_spanned(
                ty,
                format!(
                    "Use `pub {}: InitAccount<'info, {}>` with the appropriate `#[account(init, ...)]` attributes in Hopper; do not wrap Hopper account wrappers in references.",
                    field_name, inner
                ),
            ))
        }
        WrapperKind::Interface { .. }
        | WrapperKind::InterfaceAccount { .. }
        | WrapperKind::ExternalAccount { .. } => Err(syn::Error::new_spanned(
            ty,
            "Hopper interface/external wrappers are value role wrappers; remove `&` or `&mut` from this field type and put mutability in `#[account(...)]` attributes.",
        )),
        WrapperKind::Signer
        | WrapperKind::Program
        | WrapperKind::UncheckedAccount
        | WrapperKind::SystemAccount => Err(syn::Error::new_spanned(
            ty,
            "Hopper account wrappers are value role wrappers; remove `&` or `&mut` from this field type and put mutability in `#[account(...)]` attributes.",
        )),
        WrapperKind::Optional { .. } => Err(syn::Error::new_spanned(
            ty,
            "Hopper optional accounts are value fields; remove `&` or `&mut` and declare the field as a plain `Option<Wrapper>`.",
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
                        | "SystemAccount"
                        | "ProgramRef"
                        | "Program"
                        | "Interface"
                        | "InterfaceAccount"
                        | "ExternalAccount"
                )
            })
            .unwrap_or(false),
        _ => false,
    }
}

/// Audit Stage 2.3: classify wrapper types so the context macro can
/// auto-derive the appropriate checks from the type name alone.
#[derive(Clone)]
#[allow(dead_code)]
enum WrapperKind {
    /// `Signer<'info>`. emit `check_signer`.
    Signer,
    /// `Program<'info, P>`. emit `check_address == P::ID` and
    /// `check_executable`. Layout validation skipped.
    Program,
    /// `Interface<'info, I>`. emit address-in-interface-set and executable.
    Interface { spec: Type },
    /// `UncheckedAccount<'info>`. no type-derived validation.
    UncheckedAccount,
    /// `SystemAccount<'info>`. emit owner == System Program.
    SystemAccount,
    /// `Account<'info, T>`. emit `check_owned_by(program_id)` +
    /// `load::<T>()` using `T` as the layout.
    Account { inner: Type },
    /// `InitAccount<'info, T>`. skip pre-instruction layout check
    /// (account doesn't exist yet); the `init_{field}` lifecycle
    /// helper will create + initialise it.
    InitAccount { inner: Type },
    /// `InterfaceAccount<'info, T>`. emit owner-in-interface-set plus
    /// cross-program Hopper layout validation.
    InterfaceAccount { inner: Type },
    /// `ExternalAccount<'info, T>`. emit adapter validation without Hopper
    /// header validation.
    ExternalAccount { inner: Type },
    /// `Option<W>` where `W` is a supported wrapper. Anchor's
    /// optional-account convention (Anchor ≥ 0.26): an ABSENT optional
    /// is passed as the executing program's own id in that slot. Bind
    /// yields `None` for that field and skips every check attached to
    /// it; a PRESENT slot runs the inner wrapper's full checks and
    /// binds `Some(inner)`. Absence costs one address compare.
    ///
    /// Duplicate-address note: only fields *declared* `Option<..>` take
    /// this branch. A required `Program<'info, Self>`-style field whose
    /// pinned address happens to equal the executing program id binds
    /// exactly as before — the presence test never runs for required
    /// fields, so the program's own id in a required slot cannot be
    /// mistaken for absence.
    Optional { inner: Box<WrapperKind> },
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
        "Interface" => {
            let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
                return None;
            };
            let spec = args.args.iter().find_map(|arg| {
                if let syn::GenericArgument::Type(ty) = arg {
                    Some(ty.clone())
                } else {
                    None
                }
            })?;
            Some(WrapperKind::Interface { spec })
        }
        "UncheckedAccount" => Some(WrapperKind::UncheckedAccount),
        "SystemAccount" => Some(WrapperKind::SystemAccount),
        "Account" | "InitAccount" | "InterfaceAccount" | "ExternalAccount" => {
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
            } else if name == "InitAccount" {
                Some(WrapperKind::InitAccount { inner })
            } else if name == "ExternalAccount" {
                Some(WrapperKind::ExternalAccount { inner })
            } else {
                Some(WrapperKind::InterfaceAccount { inner })
            }
        }
        "Option" => {
            // `Option<W>` — Anchor-parity optional account. Classify
            // the inner wrapper form; the expansion entry point has
            // already rejected illegal shapes (nested Option, lifecycle
            // targets, non-wrapper inners) with targeted errors via
            // `validate_optional_field`, so consumers of this kind only
            // ever see a legal inner. `Option<AccountView>` returns
            // `None` here on purpose: raw views are not role wrappers
            // and never participate in the typed `accounts` facade —
            // their attribute checks are still presence-gated through
            // `option_inner_type` in the main expansion loop.
            let inner_ty = option_inner_type(ty)?;
            let inner = classify_wrapper(inner_ty)?;
            Some(WrapperKind::Optional {
                inner: Box::new(inner),
            })
        }
        _ => None,
    }
}

/// Extract `T` from an `Option<T>` context-field type. Returns `None`
/// for every non-`Option` type. Matches on the last path segment so the
/// `core::option::Option<T>` / `std::option::Option<T>` spellings work
/// the same as the bare `Option<T>` prelude form.
fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return None;
    };
    let segment = path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| {
        if let syn::GenericArgument::Type(ty) = arg {
            Some(ty)
        } else {
            None
        }
    })
}

/// Whether the type is a raw `AccountView` (the only supported
/// non-wrapper inner for `Option<...>` context fields).
fn is_account_view(ty: &Type) -> bool {
    match ty {
        Type::Path(TypePath { path, .. }) => path
            .segments
            .last()
            .map(|segment| segment.ident == "AccountView")
            .unwrap_or(false),
        _ => false,
    }
}

/// Expansion-time legality of `Option<...>` context fields (Anchor's
/// optional-accounts convention).
///
/// Accepts `Option<W>` where `W` is a supported role wrapper
/// (`Account<'info, T>`, `Signer<'info>`, `UncheckedAccount<'info>`,
/// `SystemAccount<'info>`, `Program<'info, P>`, `Interface<'info, I>`,
/// `InterfaceAccount<'info, T>`, `ExternalAccount<'info, T>`) or a raw
/// `AccountView<'info>`. Everything else is rejected here with an
/// actionable message instead of falling through to the opaque-layout
/// path (the pre-optional failure mode: `Option<T>` silently bound as a
/// layout type named `Option`).
fn validate_optional_field(field_name: &Ident, ty: &Type, attr: &AccountAttr) -> Result<()> {
    let Some(inner) = option_inner_type(ty) else {
        return Ok(());
    };
    if option_inner_type(inner).is_some() {
        return Err(syn::Error::new_spanned(
            ty,
            "nested `Option<Option<...>>` is not supported: an account slot is either present \
             or absent. Declare the field as a single `Option<Wrapper>`.",
        ));
    }
    match classify_wrapper(inner) {
        Some(WrapperKind::InitAccount { .. }) => {
            return Err(syn::Error::new_spanned(
                ty,
                "`Option<InitAccount<...>>` is not supported: optional accounts cannot be \
                 lifecycle targets. Make the field required, or declare it \
                 `Option<Account<'info, T>>` and create the account in a separate instruction.",
            ));
        }
        Some(_) => {}
        None if is_account_view(inner) => {}
        None => {
            return Err(syn::Error::new_spanned(
                ty,
                "unsupported `Option<...>` context field: optional accounts support \
                 `Option<Account<'info, T>>`, `Option<Signer<'info>>`, \
                 `Option<UncheckedAccount<'info>>`, `Option<SystemAccount<'info>>`, \
                 `Option<Program<'info, P>>`, `Option<Interface<'info, I>>`, \
                 `Option<InterfaceAccount<'info, T>>`, `Option<ExternalAccount<'info, T>>`, \
                 and `Option<AccountView<'info>>`. For a Hopper layout type, wrap it as \
                 `Option<Account<'info, T>>`.",
            ));
        }
    }
    // Lifecycle attributes rewrite the slot (create / resize / drain /
    // zero-check), and the generated helpers assume the account is
    // unconditionally present. Reject the combination at expansion time
    // rather than fail the CPI at runtime — Anchor 1.x restricts several
    // of these combos on optional accounts the same way.
    let lifecycle = [
        (attr.init, "init"),
        (attr.init_if_needed, "init_if_needed"),
        (attr.zero, "zero"),
        (attr.close.is_some(), "close"),
        (attr.realloc.is_some(), "realloc"),
        (attr.sweep.is_some(), "sweep"),
    ];
    if let Some((_, kw)) = lifecycle.iter().find(|(on, _)| *on) {
        return Err(syn::Error::new_spanned(
            field_name,
            format!(
                "`{kw}` cannot target the optional account `{field_name}`: lifecycle \
                 operations (init / init_if_needed / zero / close / realloc / sweep) require \
                 the account to be unconditionally present. Make the field required."
            ),
        ));
    }
    // Segment accessors project const offsets through the layout type
    // and are emitted unconditionally on the bound context; an absent
    // slot has no layout bytes to project. Whole-account `mut` remains
    // supported (the strict_writes range set is static; an absent
    // account simply never writes).
    if !attr.mut_segments.is_empty()
        || !attr.read_segments.is_empty()
        || !attr.tail_segments.is_empty()
    {
        return Err(syn::Error::new_spanned(
            field_name,
            format!(
                "`mut(...)`/`read(...)`/`tail(...)` segment lists are not supported on the \
                 optional account `{field_name}`; use whole-account `mut` and access the layout \
                 through the bound `Option` field."
            ),
        ));
    }
    Ok(())
}

/// Expansion-time legality of `migrate(from = ..., with = ...)` fields
/// (lazy migration at bind).
///
/// A migration is a bind-time WRITE that rewrites an existing account of
/// the old layout into the field's (new) layout, so it composes only
/// with fields that (a) are writable, (b) unconditionally present, and
/// (c) carry a Hopper layout to migrate INTO. Every violation is a
/// compile error with the fix spelled out; none degrade to a runtime
/// surprise.
fn validate_migrate_field(field_name: &Ident, ty: &Type, attr: &AccountAttr) -> Result<()> {
    let Some((from_ty, _)) = &attr.migrate else {
        return Ok(());
    };
    // Optional accounts: an absent slot has nothing to migrate, and the
    // pre-step (like every lifecycle write) assumes unconditional
    // presence. Same restriction class as init/close/realloc.
    if option_inner_type(ty).is_some() {
        return Err(syn::Error::new_spanned(
            field_name,
            format!(
                "`migrate(...)` cannot target the optional account `{field_name}`: an absent \
                 slot has nothing to migrate. Make the field required."
            ),
        ));
    }
    // Lifecycle combos: those attrs create/resize/drain/zero-check the
    // slot, while a migration rewrites an EXISTING account of the OLD
    // layout — the two contracts are mutually exclusive on one field.
    let lifecycle = [
        (attr.init, "init"),
        (attr.init_if_needed, "init_if_needed"),
        (attr.zero, "zero"),
        (attr.close.is_some(), "close"),
        (attr.realloc.is_some(), "realloc"),
        (attr.sweep.is_some(), "sweep"),
    ];
    if let Some((_, kw)) = lifecycle.iter().find(|(on, _)| *on) {
        return Err(syn::Error::new_spanned(
            field_name,
            format!(
                "`migrate(...)` cannot be combined with `{kw}` on `{field_name}`: lifecycle \
                 attributes create/resize/drain the slot, while a lazy migration rewrites an \
                 EXISTING account of the old layout. Declare the migration on a plain \
                 `#[account(mut, migrate(...))]` field."
            ),
        ));
    }
    // Resolve the layout the field migrates INTO. `InitAccount` is the
    // type-directed `init` spelling (rejected for the same reason as the
    // attr); every other wrapper carries no migratable Hopper layout.
    let layout_ty: Type = match classify_wrapper(ty) {
        Some(WrapperKind::Account { inner }) => inner,
        Some(WrapperKind::InitAccount { .. }) => {
            return Err(syn::Error::new_spanned(
                field_name,
                format!(
                    "`migrate(...)` cannot target `InitAccount<..>` `{field_name}`: a \
                     freshly-created account starts at the new layout and has nothing to \
                     migrate. Use `Account<'info, NewLayout>`."
                ),
            ));
        }
        Some(_) => {
            return Err(syn::Error::new_spanned(
                field_name,
                format!(
                    "`migrate(...)` on `{field_name}` requires a Hopper layout field: declare \
                     the field as `Account<'info, NewLayout>` or as a plain `#[hopper::state]` \
                     layout type."
                ),
            ));
        }
        None => {
            if skips_layout_validation(ty) {
                return Err(syn::Error::new_spanned(
                    field_name,
                    format!(
                        "`migrate(...)` on `{field_name}` requires a Hopper layout field: \
                         declare the field as `Account<'info, NewLayout>` or as a plain \
                         `#[hopper::state]` layout type."
                    ),
                ));
            }
            ty.clone()
        }
    };
    // Writability: the migration rewrites the account bytes at bind.
    // Required explicitly (never implied) so the context's declared
    // write set stays an honest description of what bind may touch.
    if !attr.is_mut {
        return Err(syn::Error::new_spanned(
            field_name,
            format!(
                "`migrate(...)` requires `mut` on `{field_name}`: a lazy migration rewrites \
                 the account bytes at bind. Add `mut` to the same `#[account(...)]` list."
            ),
        ));
    }
    // Same-spelling source/target is the compile-catchable form of a
    // non-forward migration (the runtime's `New::VERSION > Old::VERSION`
    // guard would refuse it on every bind at runtime; differently-spelled
    // paths to the same type still fall through to that guard).
    if quote!(#from_ty).to_string() == quote!(#layout_ty).to_string() {
        return Err(syn::Error::new_spanned(
            field_name,
            format!(
                "`migrate(from = ...)` on `{field_name}`: migration source must be a different \
                 layout version — `from` names the OLD layout, the field's type is the NEW one \
                 (e.g. `migrate(from = VaultV1, with = ...)` on `Account<'info, VaultV2>`)."
            ),
        ));
    }
    Ok(())
}

/// Expansion-time legality of a `#[composite]` field (nested context).
///
/// A composite field's type must be another context struct
/// (`#[derive(Accounts)]` / `#[hopper::context]`), embedded so its account
/// slots flatten in place. v1 rejects, with actionable messages:
/// - `Option<Inner>` composites (an inner context is present-or-absent as a
///   unit — out of v1 scope),
/// - `#[account(...)]` / `#[signer]` constraints on the composite field
///   itself (they belong on the inner context's fields),
/// - wrapper types (`Account<..>`, `Signer`, `Program<..>`, …) and raw
///   `AccountView`: those are single account slots, not nested contexts.
fn validate_composite_field(field_name: &Ident, ty: &Type, has_slot_attrs: bool) -> Result<()> {
    if option_inner_type(ty).is_some() {
        return Err(syn::Error::new_spanned(
            ty,
            "`Option<..>` composite contexts are not supported in v1: a nested context is \
             embedded as a fixed block of account slots, not an optional one. Make the \
             composite field required, or gate presence with a separate instruction.",
        ));
    }
    if has_slot_attrs {
        return Err(syn::Error::new_spanned(
            field_name,
            format!(
                "the composite field `{field_name}` is a nested context, not a single \
                 account slot: it cannot carry `#[account(...)]` or `#[signer]` \
                 constraints. Move those constraints onto the inner context's own fields."
            ),
        ));
    }
    if classify_wrapper(ty).is_some() || is_account_view(ty) {
        return Err(syn::Error::new_spanned(
            ty,
            "a `#[composite]` field's type must be another context struct \
             (`#[derive(Accounts)]` / `#[hopper::context]`), not a role wrapper \
             (`Account<..>`, `Signer`, `Program<..>`, …) or a raw `AccountView`. \
             A single account slot needs no `#[composite]` marker.",
        ));
    }
    Ok(())
}

/// Build the inner context's generated `Bumps` type path for a
/// `#[composite]` field. `Foo<'info>` → `FooBumps`, `crate::m::Bar` →
/// `crate::m::BarBumps` (module qualification preserved, generic args
/// dropped — the `Bumps` struct is non-generic). The nested context type
/// is already validated to be a plain path by `validate_composite_field`.
fn composite_bumps_ty(ty: &Type) -> Result<TokenStream> {
    match ty {
        Type::Path(TypePath { qself: None, path }) => {
            let mut path = path.clone();
            let last = path.segments.last_mut().ok_or_else(|| {
                syn::Error::new_spanned(ty, "composite context type must be a named path")
            })?;
            last.ident = format_ident!("{}Bumps", last.ident);
            last.arguments = syn::PathArguments::None;
            Ok(quote! { #path })
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "composite context type must be a named path (e.g. `Inner` or `module::Inner`)",
        )),
    }
}

/// Build the inner context's spec-type PATH (generic args dropped) for a
/// `#[composite]` field, e.g. `Foo<'info>` → `Foo`. Used to call the
/// generated associated fns (`Foo::validate_at::<{..}>(ctx)`) in path form
/// so the elided lifetime is inferred — the outer's `'info` is not in scope
/// inside the generated `validate` / `bind`.
fn composite_spec_ty(ty: &Type) -> Result<TokenStream> {
    match ty {
        Type::Path(TypePath { qself: None, path }) => {
            let mut path = path.clone();
            let last = path.segments.last_mut().ok_or_else(|| {
                syn::Error::new_spanned(ty, "composite context type must be a named path")
            })?;
            last.arguments = syn::PathArguments::None;
            Ok(quote! { #path })
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "composite context type must be a named path (e.g. `Inner` or `module::Inner`)",
        )),
    }
}

/// Build the inner context's generated bound-context type path for a
/// `#[composite]` field. `Foo<'info>` → `FooCtx`, `crate::m::Bar` →
/// `crate::m::BarCtx` (module qualification preserved, generic args
/// dropped — the caller re-applies the `<'_, 'a, { OFFSET }>` arguments).
fn composite_bound_ty(ty: &Type) -> Result<TokenStream> {
    match ty {
        Type::Path(TypePath { qself: None, path }) => {
            let mut path = path.clone();
            let last = path.segments.last_mut().ok_or_else(|| {
                syn::Error::new_spanned(ty, "composite context type must be a named path")
            })?;
            last.ident = format_ident!("{}Ctx", last.ident);
            last.arguments = syn::PathArguments::None;
            Ok(quote! { #path })
        }
        _ => Err(syn::Error::new_spanned(
            ty,
            "composite context type must be a named path (e.g. `Inner` or `module::Inner`)",
        )),
    }
}

fn accounts_binding_fragments(
    name: &Ident,
    generics: &syn::Generics,
    ctx_fields: &[ContextField],
) -> AccountsBindingFragments {
    let has_only_lifetime_generics = generics
        .params
        .iter()
        .all(|param| matches!(param, GenericParam::Lifetime(_)));
    // Synthetic (auto-appended) fields are account slots, not struct
    // fields: the facade constructs the USER's struct, so only
    // author-declared fields participate — both in the all-wrappers
    // eligibility test and in the constructor. A context whose declared
    // fields are all wrappers keeps its facade when `event_cpi` appends
    // the two raw trailing slots.
    let user_fields: Vec<&ContextField> = ctx_fields
        .iter()
        .filter(|field| field.synthetic.is_none())
        .collect();
    let all_fields_are_wrappers = user_fields
        .iter()
        .all(|field| classify_wrapper(&field.ty).is_some());

    if user_fields.is_empty() || !has_only_lifetime_generics || !all_fields_are_wrappers {
        return AccountsBindingFragments {
            field_decl: TokenStream::new(),
            init_stmt: TokenStream::new(),
            bound_field: TokenStream::new(),
        };
    }

    let generic_args: Vec<TokenStream> = generics.params.iter().map(|_| quote! { 'a }).collect();
    let accounts_ty = if generic_args.is_empty() {
        quote! { #name }
    } else {
        quote! { #name<#(#generic_args),*> }
    };

    let field_inits = user_fields.iter().map(|field| {
        let field_name = &field.name;
        let idx = field.index;
        let expr = wrapper_init_expr(&classify_wrapper(&field.ty).expect("checked above"), idx);
        quote! { #field_name: #expr }
    });

    AccountsBindingFragments {
        field_decl: quote! { pub accounts: #accounts_ty, },
        init_stmt: quote! {
            let __hopper_accounts: #accounts_ty = #name {
                #(#field_inits),*
            };
        },
        bound_field: quote! { accounts: __hopper_accounts, },
    }
}

/// The construction expression for one bound wrapper field. `bind()`
/// runs strictly AFTER `validate()`, so `try_new` constructors re-verify
/// only cheap role invariants and `new_unchecked` constructors rely on
/// the per-field validator having already proven owner + layout.
fn wrapper_init_expr(kind: &WrapperKind, idx: usize) -> TokenStream {
    match kind {
        WrapperKind::Signer => quote! {
            ::hopper::prelude::Signer::try_new(ctx.account(__HOPPER_BASE + #idx)?)?
        },
        WrapperKind::Program => quote! {
            ::hopper::prelude::Program::try_new(ctx.account(__HOPPER_BASE + #idx)?)?
        },
        WrapperKind::Interface { spec } => quote! {
            ::hopper::prelude::Interface::<#spec>::try_new(ctx.account(__HOPPER_BASE + #idx)?)?
        },
        WrapperKind::UncheckedAccount => quote! {
            unsafe {
                ::hopper::prelude::UncheckedAccount::new_unchecked(ctx.account(__HOPPER_BASE + #idx)?)
            }
        },
        WrapperKind::SystemAccount => quote! {
            ::hopper::prelude::SystemAccount::try_new(ctx.account(__HOPPER_BASE + #idx)?)?
        },
        WrapperKind::Account { .. } => quote! {
            unsafe {
                ::hopper::prelude::Account::new_unchecked(ctx.account(__HOPPER_BASE + #idx)?)
            }
        },
        WrapperKind::InitAccount { .. } => quote! {
            unsafe {
                ::hopper::prelude::InitAccount::new_unchecked(ctx.account(__HOPPER_BASE + #idx)?)
            }
        },
        WrapperKind::InterfaceAccount { .. } => quote! {
            unsafe {
                ::hopper::prelude::InterfaceAccount::new_unchecked(ctx.account(__HOPPER_BASE + #idx)?)
            }
        },
        WrapperKind::ExternalAccount { .. } => quote! {
            unsafe {
                ::hopper::prelude::ExternalAccount::new_unchecked(ctx.account(__HOPPER_BASE + #idx)?)
            }
        },
        WrapperKind::Optional { inner } => {
            let inner_expr = wrapper_init_expr(inner, idx);
            // Anchor optional-account convention: an absent optional is
            // passed as the executing program's own id in the slot. One
            // address compare selects `None`; anything else takes the
            // exact construction path the required form emits.
            // `validate()` gated this field's checks behind the same
            // compare, so a present slot has already passed every check
            // by the time this runs.
            quote! {
                if ctx.account(__HOPPER_BASE + #idx)?.address() == ctx.program_id() {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(#inner_expr)
                }
            }
        }
    }
}

fn layout_type_for_field(field: &ContextField) -> Option<Type> {
    match classify_wrapper(&field.ty) {
        Some(WrapperKind::Account { inner }) | Some(WrapperKind::InitAccount { inner }) => {
            Some(inner)
        }
        // `Option<Account<'info, T>>` resolves to `T` so `has_one`
        // sources, typed accessors, and the schema `layout_ref` see the
        // real layout; the emitted checks and loads stay fallible /
        // presence-gated. Non-layout optional wrappers carry no layout,
        // exactly like their required forms.
        Some(WrapperKind::Optional { inner }) => match *inner {
            WrapperKind::Account { inner } | WrapperKind::InitAccount { inner } => Some(inner),
            _ => None,
        },
        Some(WrapperKind::Signer)
        | Some(WrapperKind::Program)
        | Some(WrapperKind::Interface { .. })
        | Some(WrapperKind::InterfaceAccount { .. })
        | Some(WrapperKind::ExternalAccount { .. })
        | Some(WrapperKind::UncheckedAccount)
        | Some(WrapperKind::SystemAccount) => None,
        None => {
            // `Option<AccountView>` classifies as no wrapper but must
            // not fall through to the opaque-layout path: a raw view
            // (optional or not) has no Hopper layout.
            if option_inner_type(&field.ty).is_some() || skips_layout_validation(&field.ty) {
                None
            } else {
                Some(field.ty.clone())
            }
        }
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
        // - that would be a duplicate definition once the user's
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
    fn init_lifecycle_passes_explicit_space_to_hopper_init() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Initialize<'info> {
                #[account(mut)]
                pub payer: Signer<'info>,

                #[account(init, payer = payer, space = AuditState::ALLOC_SPACE)]
                pub state: InitAccount<'info, AuditState>,

                pub system_program: Program<'info, System>,
            }
        };

        let derived = expand_for_derive(item).expect("derive expand ok");
        let s = derived.to_string();
        let call_idx = s
            .find("hopper_init")
            .expect("generated init helper should call hopper_init!");
        let call_tail = &s[call_idx..];
        assert!(
            call_tail.contains("AuditState :: ALLOC_SPACE"),
            "init helper must pass the explicit `space =` expression into hopper_init!: {s}"
        );
    }

    /// BLD-MUT: the lamport dimension lowers explicit `lamports(...)`
    /// names PLUS the implied lifecycle roles, and an init payer (handed
    /// writable to the System Program CPI) additionally receives a
    /// whole-account data range so the CPI delegation gate admits it.
    #[test]
    fn lamports_option_lowers_explicit_and_implied_permission_set() {
        let attr: TokenStream = quote! { strict_writes, lamports(recipient) };
        let item: TokenStream = quote! {
            pub struct Funding<'info> {
                pub payer: Signer<'info>,

                #[account(init, payer = payer, space = FundState::ALLOC_SPACE)]
                pub state: InitAccount<'info, FundState>,

                pub recipient: AccountView,

                pub system_program: Program<'info, System>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();

        // Complete: both dimensions declared.
        assert!(
            s.contains("MUTATION_COMPLETE : bool = true"),
            "lamports(...) context must publish MUTATION_COMPLETE = true: {s}"
        );
        // Permission set: payer (0, implied by init), state (1, init),
        // recipient (2, explicit) — sorted, deduped.
        assert!(
            s.contains("0u8 , 1u8 , 2u8"),
            "lamport set must be payer+state+recipient: {s}"
        );
        // The init payer is not `mut`, so its whole-account delegation
        // range is the extra one emitted for the System Program CPI.
        assert!(
            s.contains("whole_account (0u8)"),
            "init payer must receive a whole-account delegation range: {s}"
        );
        assert!(
            s.contains("whole_account (1u8)"),
            "init account keeps its whole-account range: {s}"
        );
        // Both dimensions are wired into one policy + the ambient gate.
        assert!(
            s.contains("with_lamports"),
            "bind must build the two-dimension policy: {s}"
        );
        assert!(
            s.contains("install_lamport_gate"),
            "bind must install the instruction-scoped lamport gate: {s}"
        );
    }

    /// A bare strict_writes context must stay byte-identical to the
    /// pre-BLD-MUT lowering: no gate install, incomplete, empty set.
    #[test]
    fn bare_strict_writes_context_stays_incomplete_and_ungated() {
        let attr: TokenStream = quote! { strict_writes };
        let item: TokenStream = quote! {
            pub struct Plain<'info> {
                #[account(mut)]
                pub vault: Account<'info, VaultState>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();
        assert!(s.contains("MUTATION_COMPLETE : bool = false"), "got: {s}");
        assert!(!s.contains("install_lamport_gate"), "got: {s}");
        assert!(!s.contains("with_lamports"), "got: {s}");
    }

    #[test]
    fn lamports_without_strict_writes_is_rejected() {
        let attr: TokenStream = quote! { lamports(payer) };
        let item: TokenStream = quote! {
            pub struct Loose<'info> {
                pub payer: Signer<'info>,
            }
        };
        let err = expand(attr, item).expect_err("lamports requires strict_writes");
        assert!(err.to_string().contains("strict_writes"), "got: {err}");
    }

    /// I7 opt-in: `emit_touch_map` advertises `EMIT_TOUCH_MAP = true` as
    /// a public associated const on the spec type. The dispatcher reads
    /// that const on the handler's Ok path to decide whether to emit the
    /// touch-map record — the const is the whole macro-side surface now.
    /// Crucially, the CONTEXT macro must NOT generate a `Drop` (that
    /// would emit on every scope exit, including `?`/`Err` returns — the
    /// CONFIRMED P2) and must NOT itself call `finish_with_touch_map`;
    /// the finish call lives in the dispatcher (program.rs) on the Ok
    /// path only.
    #[test]
    fn emit_touch_map_opt_in_advertises_the_const_and_no_drop() {
        let attr: TokenStream = quote! { emit_touch_map };
        let item: TokenStream = quote! {
            pub struct Report<'info> {
                #[account(mut)]
                pub vault: Account<'info, VaultState>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();
        assert!(
            s.contains("const EMIT_TOUCH_MAP : bool = true"),
            "opt-in must advertise EMIT_TOUCH_MAP = true: {s}"
        );
        assert!(
            !s.contains(":: core :: ops :: Drop for ReportCtx"),
            "opt-in must NOT generate a Drop (P2: Drop fires on Err too): {s}"
        );
        // The actual finish CALL (`ctx.finish_with_touch_map()`) belongs
        // in the dispatcher, not the context macro. The const's doc
        // comment names `Context::finish_with_touch_map()` in prose, so
        // match the call's receiver form to avoid tripping on the doc.
        assert!(
            !s.contains("ctx . finish_with_touch_map"),
            "the finish call belongs in the dispatcher, not the context macro: {s}"
        );
    }

    /// I7 opt-in composes with `strict_writes` and, like every other
    /// context option, is accepted in the same attribute list — still via
    /// the const, still no `Drop`.
    #[test]
    fn emit_touch_map_composes_with_strict_writes() {
        let attr: TokenStream = quote! { strict_writes, emit_touch_map };
        let item: TokenStream = quote! {
            pub struct ReportStrict<'info> {
                #[account(mut)]
                pub vault: Account<'info, VaultState>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();
        assert!(
            s.contains("const EMIT_TOUCH_MAP : bool = true"),
            "emit_touch_map must still advertise the const alongside strict_writes: {s}"
        );
        assert!(
            !s.contains(":: core :: ops :: Drop for ReportStrictCtx"),
            "no Drop must be generated under strict_writes either: {s}"
        );
    }

    /// Without the opt-in, the const is `false` (so the dispatcher's
    /// const-guarded call dead-code-eliminates), and NOTHING else
    /// touch-map-related is generated: no `Drop`, no `finish` call. This
    /// pins the "did not opt in pays nothing" guarantee for every context.
    #[test]
    fn no_emit_touch_map_opt_in_defaults_const_false_and_no_drop() {
        let item: TokenStream = quote! {
            pub struct Plain<'info> {
                #[account(mut)]
                pub vault: Account<'info, VaultState>,
            }
        };
        let expanded = expand(TokenStream::new(), item).expect("expand ok");
        let s = expanded.to_string();
        assert!(
            s.contains("const EMIT_TOUCH_MAP : bool = false"),
            "no opt-in must default EMIT_TOUCH_MAP = false: {s}"
        );
        // No actual finish CALL in the context macro (the const's doc
        // names the method in prose; match the call's receiver form).
        assert!(
            !s.contains("ctx . finish_with_touch_map"),
            "no opt-in must emit no touch-map finish call: {s}"
        );
        assert!(
            !s.contains("Drop for PlainCtx"),
            "no opt-in must generate no Drop for the bound context: {s}"
        );
    }

    /// `event_cpi` appends the two Anchor-parity trailing account slots
    /// WITHOUT the author declaring them: `ACCOUNT_COUNT` grows by 2,
    /// both slots get per-field validators (authority: PDA verify via
    /// the runtime helper; program: `address == ctx.program_id()` pin),
    /// the authority's bump lands on the `Bumps` struct, both get
    /// `_account()` accessors, the spec advertises `EVENT_CPI = true`,
    /// and the bound context gains the `emit_event_cpi` one-liner.
    #[test]
    fn event_cpi_appends_validates_and_exposes_the_two_trailing_accounts() {
        let attr: TokenStream = quote! { event_cpi };
        let item: TokenStream = quote! {
            pub struct Deposit<'info> {
                #[account(mut)]
                pub vault: Account<'info, VaultState>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();

        // One user field + two appended slots.
        assert!(
            s.contains("ACCOUNT_COUNT : usize = 3"),
            "event_cpi must append two trailing account slots: {s}"
        );
        assert!(
            s.contains("const EVENT_CPI : bool = true"),
            "opt-in must advertise EVENT_CPI = true: {s}"
        );

        // Both slots are validated at bind.
        assert!(
            s.contains("fn validate_event_authority"),
            "the appended authority slot must get a validator: {s}"
        );
        assert!(
            s.contains("verify_event_authority"),
            "authority validation must run the runtime PDA verify helper: {s}"
        );
        assert!(
            s.contains("fn validate_event_program"),
            "the appended program slot must get a validator: {s}"
        );
        assert!(
            s.contains("(* ctx . program_id ())"),
            "the program slot must be pinned to ctx.program_id(): {s}"
        );

        // The authority bump is captured at bind for the CPI signer.
        assert!(
            s.contains("__hopper_bumps . event_authority ="),
            "the authority bump must be gathered onto the Bumps struct: {s}"
        );

        // Both slots are exposed for the emit call.
        assert!(
            s.contains("fn event_authority_account"),
            "the authority slot must get an accessor: {s}"
        );
        assert!(
            s.contains("fn event_program_account"),
            "the program slot must get an accessor: {s}"
        );

        // The one-liner: encode + stored-bump self-invoke.
        assert!(
            s.contains("fn emit_event_cpi"),
            "the bound context must expose emit_event_cpi: {s}"
        );
        assert!(
            s.contains("encode_event_cpi") && s.contains("invoke_event_cpi"),
            "emit_event_cpi must encode the wire format and self-invoke: {s}"
        );
        assert!(
            s.contains("self . bumps . event_authority"),
            "emit_event_cpi must sign with the bind-captured bump: {s}"
        );
    }

    /// A context whose declared fields are all wrapper types keeps its
    /// typed `accounts` facade when `event_cpi` appends the two raw
    /// trailing slots: the facade mirrors the USER's struct only, so the
    /// synthetic slots must be skipped by the constructor, not counted
    /// against wrapper-eligibility.
    #[test]
    fn event_cpi_keeps_the_wrapper_accounts_facade() {
        let attr: TokenStream = quote! { event_cpi };
        let item: TokenStream = quote! {
            pub struct Ping<'info> {
                pub payer: Signer<'info>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();
        assert!(
            s.contains("pub accounts :"),
            "the all-wrappers facade must survive the appended raw slots: {s}"
        );
        // The facade constructor mirrors the USER's struct only: slice
        // the `= Ping { ... }` initializer out of the expansion and
        // require it to construct `payer` but neither synthetic slot.
        let ctor_start = s.find("= Ping {").expect("facade constructor present");
        let ctor = &s[ctor_start..];
        let ctor = &ctor[..ctor.find('}').expect("constructor closes")];
        assert!(
            ctor.contains("payer"),
            "the facade constructor must build the declared field: {ctor}"
        );
        assert!(
            !ctor.contains("event_authority") && !ctor.contains("event_program"),
            "the facade constructor must not name the synthetic slots: {ctor}"
        );
        assert!(
            s.contains("ACCOUNT_COUNT : usize = 3"),
            "the two slots still count toward ACCOUNT_COUNT: {s}"
        );
    }

    /// Without the opt-in, nothing event-related is generated: the const
    /// defaults to `false`, no accounts are appended, and no emit method
    /// or event validators exist. This pins the "did not opt in pays
    /// nothing" guarantee.
    #[test]
    fn no_event_cpi_opt_in_appends_nothing() {
        let item: TokenStream = quote! {
            pub struct Plain<'info> {
                #[account(mut)]
                pub vault: Account<'info, VaultState>,
            }
        };
        let expanded = expand(TokenStream::new(), item).expect("expand ok");
        let s = expanded.to_string();
        assert!(
            s.contains("const EVENT_CPI : bool = false"),
            "no opt-in must default EVENT_CPI = false: {s}"
        );
        assert!(
            s.contains("ACCOUNT_COUNT : usize = 1"),
            "no opt-in must not append account slots: {s}"
        );
        // The EVENT_CPI const's doc prose names the method and the
        // dispatcher marker, so match generated CODE forms only: the
        // method definition, the appended validators/accessors, and the
        // bump gather must all be absent.
        assert!(
            !s.contains("fn emit_event_cpi"),
            "no opt-in must not emit the event-CPI method: {s}"
        );
        assert!(
            !s.contains("fn validate_event_authority") && !s.contains("fn validate_event_program"),
            "no opt-in must not append event-account validators: {s}"
        );
        assert!(
            !s.contains("__hopper_bumps . event_authority"),
            "no opt-in must not gather an event-authority bump: {s}"
        );
        assert!(
            !s.contains("verify_event_authority"),
            "no opt-in must not call the event-authority verifier: {s}"
        );
    }

    /// `event_cpi` composes with the other context options; the appended
    /// slots are read-only so a strict write set is unchanged.
    #[test]
    fn event_cpi_composes_with_strict_writes() {
        let attr: TokenStream = quote! { strict_writes, event_cpi };
        let item: TokenStream = quote! {
            pub struct Report<'info> {
                #[account(mut)]
                pub vault: Account<'info, VaultState>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();
        assert!(
            s.contains("const EVENT_CPI : bool = true"),
            "event_cpi must compose with strict_writes: {s}"
        );
        assert!(
            s.contains("const STRICT_WRITES : bool = true"),
            "strict_writes must survive the composition: {s}"
        );
        assert!(
            s.contains("ACCOUNT_COUNT : usize = 3"),
            "the appended slots must still count: {s}"
        );
    }

    /// A declared field named like a reserved auto-appended slot is
    /// rejected with an error that names the collision, instead of
    /// silently generating duplicate accessors/validators.
    #[test]
    fn event_cpi_rejects_reserved_field_names() {
        let attr: TokenStream = quote! { event_cpi };
        let item: TokenStream = quote! {
            pub struct Clash<'info> {
                pub event_authority: AccountView<'info>,
            }
        };
        let err = expand(attr, item).expect_err("reserved name must be rejected");
        assert!(
            err.to_string().contains("event_authority"),
            "error must name the colliding reserved field: {err}"
        );
    }

    /// An unknown context option is still rejected, and the error text
    /// advertises `emit_touch_map` as a supported option so a typo points
    /// the author at the real name.
    #[test]
    fn unknown_option_error_lists_emit_touch_map() {
        let attr: TokenStream = quote! { emit_touchmap };
        let item: TokenStream = quote! {
            pub struct Typo<'info> {
                pub payer: Signer<'info>,
            }
        };
        let err = expand(attr, item).expect_err("unknown option must be rejected");
        assert!(
            err.to_string().contains("emit_touch_map"),
            "error must advertise the emit_touch_map option: {err}"
        );
    }

    /// BLD-MUT completeness over the macro's own CPI surface: the
    /// generated Metaplex helpers hand writable metas to the callee —
    /// CreateMetadataAccountV3 marks the metadata PDA and the payer
    /// writable; CreateMasterEditionV3 marks the edition PDA, the mint,
    /// and the payer writable. A `lamports(...)` context must imply
    /// lamport permission AND a whole-account delegation range for
    /// every one of them, or the gate refuses the helper's CPI at
    /// runtime while the manifest still claims mutation completeness.
    #[test]
    fn metaplex_helpers_imply_cpi_delegation_roles() {
        let attr: TokenStream = quote! { strict_writes, lamports(fee_sink) };
        let item: TokenStream = quote! {
            pub struct MintNft<'info> {
                #[account(signer)]
                pub authority: AccountView<'static>,

                pub mint: AccountView<'static>,

                #[account(
                    metadata::mint = mint,
                    metadata::mint_authority = authority,
                    metadata::payer = authority,
                    metadata::update_authority = authority,
                    metadata::system_program = system_program,
                    metadata::name = "Name",
                    metadata::symbol = "SYM",
                    metadata::uri = "https://example.com/nft.json",
                    metadata::seller_fee_basis_points = 500,
                )]
                pub metadata: AccountView<'static>,

                #[account(
                    master_edition::mint = mint,
                    master_edition::metadata = metadata,
                    master_edition::update_authority = authority,
                    master_edition::mint_authority = authority,
                    master_edition::payer = authority,
                    master_edition::token_program = token_program,
                    master_edition::system_program = system_program,
                    master_edition::max_supply = 0u64,
                )]
                pub master_edition: AccountView<'static>,

                pub fee_sink: AccountView<'static>,
                pub token_program: AccountView<'static>,
                pub system_program: AccountView<'static>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();

        // Lamport set (sorted, deduped): payer/authority (0, writable
        // meta of both helpers), mint (1, writable meta of the master-
        // edition helper), metadata (2, created PDA), master_edition
        // (3, created PDA), fee_sink (4, explicit). token_program and
        // system_program (5, 6) are read-only metas: NOT implied.
        assert!(
            s.contains("0u8 , 1u8 , 2u8 , 3u8 , 4u8"),
            "lamport set must cover every writable Metaplex CPI meta plus the explicit name: {s}"
        );
        // Every writable CPI meta additionally carries the whole-account
        // delegation range `check_lamport_delegation` demands.
        for idx in ["0u8", "1u8", "2u8", "3u8"] {
            assert!(
                s.contains(&format!("whole_account ({idx})")),
                "writable Metaplex CPI meta {idx} must receive a whole-account delegation range: {s}"
            );
        }
        // The explicit lamports(...) name is lamports-only: no implied
        // data grant, and read-only metas get nothing.
        for idx in ["4u8", "5u8", "6u8"] {
            assert!(
                !s.contains(&format!("whole_account ({idx})")),
                "account {idx} is not a writable CPI meta and must not get a data grant: {s}"
            );
        }
    }

    /// `sweep = target` lowers through the real fallible lamport API
    /// (`try_set_lamports`, the runtime funnel the BLD-MUT gate hooks)
    /// — the pre-fix emission called a nonexistent
    /// `try_borrow_mut_lamports` and could never compile — and both
    /// sweep roles land in the implied lamport permission set.
    #[test]
    fn sweep_helper_lowers_through_the_lamport_funnel() {
        let attr: TokenStream = quote! { strict_writes, lamports() };
        let item: TokenStream = quote! {
            pub struct Cleanup<'info> {
                #[account(sweep = treasury)]
                pub fees: AccountView<'static>,

                pub treasury: AccountView<'static>,
            }
        };
        let expanded = expand(attr, item).expect("expand ok");
        let s = expanded.to_string();
        assert!(
            s.contains("sweep_fees"),
            "sweep field must emit its helper: {s}"
        );
        assert!(
            s.contains("try_set_lamports"),
            "sweep helper must move lamports through the runtime funnel: {s}"
        );
        assert!(
            !s.contains("try_borrow_mut_lamports"),
            "sweep helper must not call the nonexistent borrow-lamports API: {s}"
        );
        // Implied roles: source (0, sweep implies mut) + target (1).
        assert!(
            s.contains("0u8 , 1u8"),
            "sweep source and target must be in the implied lamport set: {s}"
        );
    }

    /// A sweep targeting its own field would credit the drained amount
    /// back into the slot being drained (minting lamports in the
    /// two-step move) — rejected at expansion time.
    #[test]
    fn sweep_targeting_its_own_field_is_rejected() {
        let item: TokenStream = quote! {
            pub struct SelfSweep<'info> {
                #[account(sweep = fees)]
                pub fees: AccountView<'static>,
            }
        };
        let err = expand(TokenStream::new(), item).expect_err("self-sweep must be rejected");
        assert!(
            err.to_string().contains("cannot target its own field"),
            "got: {err}"
        );
    }

    #[test]
    fn lamports_naming_an_unknown_field_is_rejected() {
        let attr: TokenStream = quote! { strict_writes, lamports(ghost) };
        let item: TokenStream = quote! {
            pub struct Haunted<'info> {
                pub payer: Signer<'info>,
            }
        };
        let err = expand(attr, item).expect_err("unknown lamports field");
        assert!(err.to_string().contains("ghost"), "got: {err}");
    }

    #[test]
    fn context_auto_lifecycle_bind_calls_helpers_in_declaration_order() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            #[accounts(auto_lifecycle)]
            pub struct Lifecycle<'info> {
                #[account(mut)]
                pub payer: Signer<'info>,

                #[account(init, payer = payer, space = FirstState::INIT_SPACE)]
                pub first: InitAccount<'info, FirstState>,

                #[account(realloc = SecondState::NEW_LEN, realloc_payer = payer, realloc_zero = true)]
                pub second: Account<'info, SecondState>,

                #[account(close = payer)]
                pub third: Account<'info, ThirdState>,

                pub system_program: Program<'info, System>,
            }
        };

        let derived = expand_for_derive(item).expect("derive expand ok");
        let s = derived.to_string();
        let init_idx = s
            .find("__hopper_bound . init_first")
            .expect("auto lifecycle should call init_first in bind");
        let realloc_idx = s
            .find("__hopper_bound . realloc_second")
            .expect("auto lifecycle should call realloc_second in bind");
        let close_idx = s
            .find("__hopper_bound . close_third")
            .expect("auto lifecycle should call close_third in bind");
        assert!(
            init_idx < realloc_idx && realloc_idx < close_idx,
            "auto lifecycle calls must follow field declaration order: {s}"
        );
    }

    #[test]
    fn field_auto_lifecycle_keeps_other_fields_explicit() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Mixed<'info> {
                #[account(mut)]
                pub payer: Signer<'info>,

                #[account(init, payer = payer, space = AutoState::INIT_SPACE, auto)]
                pub auto_state: InitAccount<'info, AutoState>,

                #[account(init, payer = payer, space = ExplicitState::INIT_SPACE)]
                pub explicit_state: InitAccount<'info, ExplicitState>,

                pub system_program: Program<'info, System>,
            }
        };

        let derived = expand_for_derive(item).expect("derive expand ok");
        let s = derived.to_string();
        assert!(
            s.contains("__hopper_bound . init_auto_state"),
            "field-level auto should call only the opted-in lifecycle helper: {s}"
        );
        assert!(
            !s.contains("__hopper_bound . init_explicit_state"),
            "non-auto field must stay explicit by default: {s}"
        );
    }

    #[test]
    fn init_if_needed_allows_empty_but_validates_nonempty_layout_before_binding() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Upsert<'info> {
                #[account(mut)]
                pub payer: Signer<'info>,

                #[account(init_if_needed, payer = payer, space = AuditState::ALLOC_SPACE)]
                pub state: Account<'info, AuditState>,

                pub system_program: Program<'info, System>,
            }
        };

        let derived = expand_for_derive(item).expect("derive expand ok");
        let s = derived.to_string();
        assert!(
            s.contains("if __hopper_account . data_len () > 0"),
            "init_if_needed validation must branch on the existing data length: {s}"
        );
        assert!(
            s.contains("__hopper_account . load :: < AuditState > ()"),
            "nonempty init_if_needed accounts must be layout-checked before binding: {s}"
        );
        assert!(
            s.contains("Account :: new_unchecked"),
            "validated init_if_needed Account wrapper should still bind through the generated facade: {s}"
        );
    }

    #[test]
    fn external_account_wrapper_validates_without_hopper_layout() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct ReadOracle<'info> {
                pub oracle: ExternalAccount<'info, PythPrice>,
            }
        };

        let derived = expand_for_derive(item).expect("derive expand ok");
        let s = derived.to_string();
        assert!(
            s.contains("ExternalAccount :: < PythPrice > :: try_new"),
            "ExternalAccount fields must validate through their adapter: {s}"
        );
        assert!(
            s.contains("ExternalAccount :: new_unchecked"),
            "ExternalAccount fields must bind into the generated accounts facade: {s}"
        );
        assert!(
            !s.contains("load :: < PythPrice >"),
            "ExternalAccount must not be treated as a Hopper-header layout: {s}"
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

    // ── DX-CONSTRAINTS Finding 1: `@ CustomError` on constraints ────────
    //
    // Anchor lets a constraint name the error it raises:
    // `has_one = authority @ MyError::WrongAuth`,
    // `constraint = x == y @ MyError::Bad`, `address = k @ MyError::BadAddr`,
    // `owner = p @ MyError::BadOwner`, `token::mint = m @ MyError::BadMint`.
    // Hopper captures the trailing error and lowers it through `.into()`,
    // so an @-named error resolves against `#[hopper::error_code]`
    // invariant metadata instead of a generic Custom/InvalidAccountData
    // code.

    /// (a) `has_one` / `constraint` / `address` / `owner` / `token::mint`
    /// with `@ MyError::X` each emit that error path. We assert the
    /// generated tokens reference the named error and lower through
    /// `.into()`, and that the generic `Custom(0xC000)` fallback is gone
    /// for the guard that named its own error.
    #[test]
    fn at_error_binds_custom_errors_on_constraints() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct WithErrors<'info> {
                #[account(address = EXPECTED @ MyError::BadAddr)]
                pub cfg: AccountView<'info>,

                #[account(owner = ORACLE_OWNER @ MyError::BadOwner)]
                pub oracle: AccountView<'info>,

                #[account(token::mint = MINT @ MyError::BadMint)]
                pub token_acct: AccountView<'info>,

                #[account(
                    has_one = authority @ MyError::WrongAuth,
                    constraint = layout_ok @ MyError::Disabled,
                )]
                pub vault: Account<'info, VaultState>,

                pub authority: AccountView<'info>,
            }
        };
        let s = expand_for_derive(item)
            .expect("derive expand ok")
            .to_string();

        assert!(
            s.contains("MyError :: BadAddr"),
            "address @-error must be emitted: {s}"
        );
        assert!(
            s.contains("MyError :: BadOwner"),
            "owner @-error must be emitted: {s}"
        );
        assert!(
            s.contains("MyError :: BadMint"),
            "token::mint @-error must be emitted: {s}"
        );
        assert!(
            s.contains("MyError :: WrongAuth"),
            "has_one @-error must be emitted: {s}"
        );
        assert!(
            s.contains("MyError :: Disabled"),
            "constraint @-error must be emitted: {s}"
        );
        // Every @-error resolves through `.into()` so it can carry
        // invariant-tagged error_code metadata.
        assert!(
            s.contains(". into ()"),
            "@-errors must lower through .into(): {s}"
        );
        // The only `constraint` guard named its own error, so the generic
        // Custom fallback must not appear for it.
        assert!(
            !s.contains("Custom (0xc0_00"),
            "an @-named constraint replaces the generic Custom code: {s}"
        );
    }

    /// (b) Without `@`, the generic error paths are emitted byte-for-byte
    /// as before: `InvalidAccountData` for address/has_one, the
    /// `Custom(0xC000 | idx)` code for constraint, and never an
    /// `.into()`-bound custom error.
    #[test]
    fn constraints_without_at_keep_generic_errors_unchanged() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct NoErrors<'info> {
                #[account(address = EXPECTED)]
                pub cfg: AccountView<'info>,

                #[account(
                    has_one = authority,
                    constraint = layout_ok,
                )]
                pub vault: Account<'info, VaultState>,

                pub authority: AccountView<'info>,
            }
        };
        let s = expand_for_derive(item)
            .expect("derive expand ok")
            .to_string();

        assert!(
            s.contains(":: hopper :: __runtime :: ProgramError :: InvalidAccountData"),
            "address/has_one without @ keep the generic InvalidAccountData: {s}"
        );
        assert!(
            s.contains("Custom (0xc0_00"),
            "constraint without @ keeps the generic Custom(0xC000 | idx) code: {s}"
        );
        // No @-error anywhere means no custom-error binding leaked in.
        assert!(
            !s.contains("MyError"),
            "a constraint set with no @ must not reference any custom error: {s}"
        );
    }

    // ── DX-CONSTRAINTS Finding 2: `realloc::payer` / `realloc::zero` ────

    /// (c) The Anchor colon spelling `realloc::payer` / `realloc::zero`
    /// parses identically to the underscore `realloc_payer` /
    /// `realloc_zero`: the two contexts expand byte-for-byte the same.
    #[test]
    fn realloc_colon_matches_underscore_spelling() {
        let underscore: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Grow<'info> {
                #[account(mut)]
                pub payer: Signer<'info>,

                #[account(realloc = NEW_LEN, realloc_payer = payer, realloc_zero = true)]
                pub data: AccountView<'info>,
            }
        };
        let colon: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Grow<'info> {
                #[account(mut)]
                pub payer: Signer<'info>,

                #[account(realloc = NEW_LEN, realloc::payer = payer, realloc::zero = true)]
                pub data: AccountView<'info>,
            }
        };
        let a = expand_for_derive(underscore)
            .expect("underscore realloc expand ok")
            .to_string();
        let b = expand_for_derive(colon)
            .expect("colon realloc expand ok")
            .to_string();
        assert_eq!(
            a, b,
            "realloc::payer / realloc::zero must expand identically to realloc_payer / realloc_zero"
        );
    }

    /// (d) A full, pre-existing context that uses none of the new syntax
    /// still expands successfully and keeps every generic error path
    /// intact — the additive changes leave existing programs unchanged.
    #[test]
    fn existing_full_context_expands_unchanged() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Settle<'info> {
                #[account(mut, signer)]
                pub authority: Signer<'info>,

                #[account(
                    mut,
                    has_one = authority,
                    constraint = layout_ok,
                    owner = PROGRAM_ID,
                )]
                pub vault: Account<'info, VaultState>,

                #[account(address = TREASURY)]
                pub treasury: AccountView<'info>,

                pub system_program: Program<'info, System>,
            }
        };
        let s = expand_for_derive(item)
            .expect("derive expand ok")
            .to_string();

        // Generic error surfaces are all present and no custom-error
        // binding was introduced.
        assert!(
            s.contains(":: hopper :: __runtime :: ProgramError :: InvalidAccountData"),
            "generic InvalidAccountData path must remain: {s}"
        );
        assert!(
            s.contains("Custom (0xc0_00"),
            "generic Custom constraint code must remain: {s}"
        );
        assert!(
            s.contains("check_owned_by (& (PROGRAM_ID)) ?"),
            "owner check without @ must stay a plain `?` propagation: {s}"
        );
        assert!(
            !s.contains("MyError") && !s.contains(". into ()"),
            "a context with no @ must not gain any custom-error binding: {s}"
        );
    }

    // ── Optional accounts (Anchor ≥ 0.26 `Option<...>` parity) ──────────
    //
    // Semantics under test: an absent optional account is passed as THE
    // PROGRAM'S OWN ID in its slot. Binding then yields `None` and skips
    // every check attached to the field; a present slot runs the exact
    // checks the required form emits and binds `Some(inner)`.

    /// Slice the expanded output down to one generated fn's body: from
    /// `fn <name>` to the next `fn ` occurrence. Scopes the containment
    /// assertions below to the validator under test instead of the whole
    /// expansion.
    fn fn_window<'a>(s: &'a str, fn_name: &str) -> &'a str {
        let needle = format!("fn {fn_name}");
        let start = s
            .find(&needle)
            .unwrap_or_else(|| panic!("missing `{needle}` in: {s}"));
        let tail = &s[start + needle.len()..];
        let end = tail.find("fn ").unwrap_or(tail.len());
        &s[start..start + needle.len() + end]
    }

    /// Every supported `Option<W>` wrapper form is recognized: the typed
    /// facade binds each optional slot through the one-address-compare
    /// presence gate (None when the slot carries the program id, Some
    /// otherwise), required fields never gain a gate, and the
    /// presence-aware accessor is emitted for optional fields only.
    #[test]
    fn optional_wrapper_forms_bind_none_or_some_per_slot_address() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Route<'info> {
                pub payer: Signer<'info>,
                pub a: Option<Account<'info, VaultState>>,
                pub b: Option<Signer<'info>>,
                pub c: Option<UncheckedAccount<'info>>,
                pub d: Option<SystemAccount<'info>>,
                pub e: Option<Program<'info, System>>,
                pub f: Option<InterfaceAccount<'info, VaultState>>,
                pub g: Option<Interface<'info, VaultSpec>>,
                pub h: Option<ExternalAccount<'info, PythPrice>>,
            }
        };
        let s = expand_for_derive(item)
            .expect("derive expand ok")
            .to_string();

        for idx in 1..=8usize {
            let gate = format!(
                "if ctx . account (__HOPPER_BASE + {idx}usize) ? . address () == ctx . program_id ()"
            );
            assert!(
                s.contains(&gate),
                "missing facade presence gate for optional slot {idx}: {s}"
            );
        }
        assert!(
            s.contains("Option :: None"),
            "facade must bind None for an absent optional: {s}"
        );
        assert!(
            s.contains("Option :: Some"),
            "facade must bind Some for a present optional: {s}"
        );
        // The required signer at slot 0 must never be presence-gated.
        assert!(
            !s.contains(
                "if ctx . account (__HOPPER_BASE + 0usize) ? . address () == ctx . program_id ()"
            ),
            "required fields must not be presence-gated: {s}"
        );
        // Presence-aware accessor: optional fields only.
        assert!(
            s.contains("fn a_account_opt") && s.contains("fn h_account_opt"),
            "optional fields must get a `<field>_account_opt` accessor: {s}"
        );
        assert!(
            !s.contains("fn payer_account_opt"),
            "required fields must not get a `<field>_account_opt` accessor: {s}"
        );
    }

    /// The load-bearing guarantee: EVERY check attached to an optional
    /// field (mut, owner, layout load, PDA seeds, has_one, constraint)
    /// lives INSIDE the presence gate — an absent optional performs one
    /// address compare and ZERO checks, a present one performs ALL of
    /// them — while the required sibling's validator carries the same
    /// checks with no gate.
    #[test]
    fn absent_optional_skips_every_check_present_runs_all_inside_the_gate() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct TipRoute<'info> {
                pub payer: Signer<'info>,

                #[account(
                    mut,
                    has_one = payer,
                    constraint = referral_enabled,
                    seeds = [b"referral"],
                    bump,
                )]
                pub referral: Option<Account<'info, RefState>>,

                #[account(mut, has_one = payer, constraint = referral_enabled)]
                pub vault: Account<'info, RefState>,
            }
        };
        let s = expand_for_derive(item)
            .expect("derive expand ok")
            .to_string();

        let gate =
            "if ctx . account (__HOPPER_BASE + 1usize) ? . address () != ctx . program_id ()";
        let w = fn_window(&s, "validate_referral");
        let gate_at = w
            .find(gate)
            .unwrap_or_else(|| panic!("optional validator missing the presence gate: {w}"));
        assert_eq!(
            w.matches(gate).count(),
            1,
            "absence must cost exactly one address compare: {w}"
        );
        for check in [
            "expect_signer_writable", // mut
            "check_owned_by",         // default owner pin
            "load :: < RefState >",   // layout header load
            "find_program_address",   // PDA derivation (seeds + bump)
            "InvalidSeeds",           // PDA mismatch error path
            "layout . payer",         // has_one field read
            "referral_enabled",       // custom constraint expr
        ] {
            let at = w
                .find(check)
                .unwrap_or_else(|| panic!("optional validator missing `{check}`: {w}"));
            assert!(
                at > gate_at,
                "`{check}` must be INSIDE the presence gate: {w}"
            );
        }

        // The required sibling runs the same checks with no gate.
        let wv = fn_window(&s, "validate_vault");
        assert!(
            !wv.contains("!= ctx . program_id ()"),
            "required field must not be presence-gated: {wv}"
        );
        for check in [
            "expect_signer_writable",
            "check_owned_by",
            "load :: < RefState >",
            "layout . payer",
            "referral_enabled",
        ] {
            assert!(
                wv.contains(check),
                "required validator missing `{check}`: {wv}"
            );
        }

        // The PDA bump is still gathered for the Bumps struct (a pure
        // seed-derivation that never touches the account, so it stays
        // outside the gate), and the docs const names the skip contract.
        assert!(
            s.contains("__hopper_bumps . referral ="),
            "optional PDA fields still gather their bump: {s}"
        );
        assert!(
            s.contains("is optional: when the slot carries"),
            "VALIDATION_CHECKS must document the optional skip contract: {s}"
        );
    }

    /// `Option<AccountView>` — the raw-view spelling — is supported on
    /// the attribute/accessor path: its attribute checks are
    /// presence-gated and the presence-aware accessor is emitted. Raw
    /// views never participate in the typed `accounts` facade,
    /// optional or not.
    #[test]
    fn optional_raw_account_view_gates_attribute_checks() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Raw<'info> {
                #[account(mut, owner = FEE_OWNER)]
                pub fee_sink: Option<AccountView<'info>>,

                #[signer]
                pub authority: AccountView<'info>,
            }
        };
        let s = expand_for_derive(item)
            .expect("derive expand ok")
            .to_string();

        let w = fn_window(&s, "validate_fee_sink");
        let gate =
            "if ctx . account (__HOPPER_BASE + 0usize) ? . address () != ctx . program_id ()";
        let gate_at = w
            .find(gate)
            .unwrap_or_else(|| panic!("raw-view optional missing the presence gate: {w}"));
        for check in ["expect_signer_writable", "check_owned_by (& (FEE_OWNER))"] {
            let at = w
                .find(check)
                .unwrap_or_else(|| panic!("raw-view optional missing `{check}`: {w}"));
            assert!(
                at > gate_at,
                "`{check}` must be INSIDE the presence gate: {w}"
            );
        }
        assert!(
            s.contains("fn fee_sink_account_opt"),
            "raw-view optional must get the presence-aware accessor: {s}"
        );
        // No opaque-layout misbinding: nothing loads a layout named
        // `Option`, and raw views never form the typed facade.
        assert!(
            !s.contains("load :: < Option"),
            "Option<AccountView> must not be treated as a layout type: {s}"
        );
        assert!(
            !s.contains("__hopper_accounts"),
            "raw-view fields never form the typed accounts facade: {s}"
        );
    }

    /// Nested `Option<Option<..>>` is a compile error with a clear
    /// message — a slot is either present or absent.
    #[test]
    fn nested_option_option_is_rejected() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Bad<'info> {
                pub x: Option<Option<Signer<'info>>>,
            }
        };
        let err = expand_for_derive(item).expect_err("nested Option must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("nested `Option<Option<...>>`"), "got: {msg}");
    }

    /// Optional fields cannot be lifecycle targets this pass: every
    /// lifecycle keyword is rejected at expansion time with an error
    /// naming the limitation.
    #[test]
    fn optional_lifecycle_targets_are_rejected() {
        let cases: [(TokenStream, &str); 6] = [
            (
                quote! { #[account(init, payer = payer, space = RefState::LEN)] },
                "init",
            ),
            (
                quote! { #[account(init_if_needed, payer = payer, space = RefState::LEN)] },
                "init_if_needed",
            ),
            (quote! { #[account(zero)] }, "zero"),
            (quote! { #[account(mut, close = payer)] }, "close"),
            (
                quote! { #[account(realloc = RefState::LEN, realloc_payer = payer, realloc_zero = true)] },
                "realloc",
            ),
            (quote! { #[account(sweep = payer)] }, "sweep"),
        ];
        for (account_attr, kw) in cases {
            let item: TokenStream = quote! {
                #[derive(Accounts)]
                pub struct Bad<'info> {
                    #[account(mut)]
                    pub payer: Signer<'info>,

                    #account_attr
                    pub target: Option<Account<'info, RefState>>,

                    pub system_program: Program<'info, System>,
                }
            };
            let err = expand_for_derive(item)
                .expect_err(&format!("`{kw}` on an optional account must be rejected"));
            let msg = err.to_string();
            assert!(
                msg.contains(&format!("`{kw}` cannot target the optional account")),
                "{kw}: got {msg}"
            );
        }
    }

    /// `Option<InitAccount<...>>` is the type-directed spelling of a
    /// lifecycle target and is rejected the same way.
    #[test]
    fn optional_init_account_wrapper_is_rejected() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Bad<'info> {
                pub target: Option<InitAccount<'info, RefState>>,
            }
        };
        let err = expand_for_derive(item).expect_err("Option<InitAccount> must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("`Option<InitAccount<...>>` is not supported"),
            "got: {msg}"
        );
    }

    /// A plain layout type inside `Option<...>` is rejected with the
    /// actionable `Option<Account<'info, T>>` suggestion instead of
    /// silently binding as an opaque layout named `Option` (the
    /// pre-optional failure mode).
    #[test]
    fn optional_plain_layout_field_is_rejected() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Bad<'info> {
                pub jar: Option<RefState>,
            }
        };
        let err = expand_for_derive(item).expect_err("Option<PlainLayout> must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("unsupported `Option<...>` context field")
                && msg.contains("Option<Account<'info, T>>"),
            "got: {msg}"
        );
    }

    /// Segment lists project const offsets unconditionally; on an
    /// optional field they are rejected in favor of whole-account `mut`.
    #[test]
    fn optional_segment_lists_are_rejected() {
        let item: TokenStream = quote! {
            #[derive(Accounts)]
            pub struct Bad<'info> {
                #[account(mut(balance))]
                pub jar: Option<Account<'info, RefState>>,
            }
        };
        let err = expand_for_derive(item).expect_err("segment lists on optional must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("segment lists are not supported on the optional"),
            "got: {msg}"
        );
    }

    /// Interaction rules that must stay STATIC in the face of runtime
    /// absence: an optional `mut` field still declares its strict_writes
    /// WriteRange and its lamport-set entry (the policy is compiled, not
    /// negotiated per-transaction; an absent account simply never
    /// writes), and the schema publishes `optional: true` with the INNER
    /// wrapper kind.
    #[test]
    fn optional_mut_keeps_static_write_set_and_publishes_schema_optionality() {
        let attr: TokenStream = quote! { strict_writes, lamports(referral) };
        let item: TokenStream = quote! {
            pub struct Pay<'info> {
                #[account(mut)]
                pub referral: Option<Account<'info, RefState>>,

                pub tipper: Option<Signer<'info>>,

                pub payer: Signer<'info>,
            }
        };
        let s = expand(attr, item).expect("expand ok").to_string();

        // strict_writes: the optional mut field's whole-account range is
        // still DECLARED at its slot index.
        assert!(
            s.contains("WriteRange :: whole_account (0u8)"),
            "optional mut field must still declare its static WriteRange: {s}"
        );
        // lamports(referral) + the whole-account `mut` implication both
        // resolve to slot 0 in the static lamport set.
        assert!(
            s.contains("__HOPPER_Pay_LAMPORT_ACCOUNTS : & [u8] = & [0u8]"),
            "optional field named in lamports(...) must stay in the static set: {s}"
        );
        // Schema: inner kind + optional: true for both wrapper shapes;
        // the required field publishes optional: false.
        assert!(
            s.contains("name : \"referral\" , kind : \"RefState\""),
            "Option<Account<T>> must publish the layout kind: {s}"
        );
        assert!(
            s.contains("name : \"tipper\" , kind : \"Signer\""),
            "Option<Signer> must publish the inner wrapper kind: {s}"
        );
        assert_eq!(
            s.matches("optional : true").count(),
            2,
            "exactly the two Option fields publish optional: true: {s}"
        );
        assert!(
            s.contains("optional : false"),
            "required fields keep optional: false: {s}"
        );
    }
}

// ── Lazy migration at bind (`migrate(from = Old, with = path)`) ────────
//
// Expansion-level pins for the typed cross-version migration crank: the
// parsed attribute emits the bind pre-step (and ONLY into bind — the
// read-only `validate()` never writes), `validate()`'s layout-header
// check lowers to the either-version form, and every illegal spelling /
// combination is a compile error with an actionable message.
#[cfg(test)]
mod migrate_attr_tests {
    use super::*;

    /// Slice the expanded output down to one generated fn's body: from
    /// `fn <name>` to the next `fn ` occurrence (same helper shape as the
    /// optional-accounts tests above).
    fn fn_window<'a>(s: &'a str, fn_name: &str) -> &'a str {
        let needle = format!("fn {fn_name}");
        let start = s
            .find(&needle)
            .unwrap_or_else(|| panic!("missing `{needle}` in: {s}"));
        let tail = &s[start + needle.len()..];
        let end = tail.find("fn ").unwrap_or(tail.len());
        &s[start..start + needle.len() + end]
    }

    /// The canonical migrating context used across these tests.
    fn touch_item() -> TokenStream {
        quote! {
            pub struct Touch<'info> {
                pub authority: Signer<'info>,

                #[account(mut, migrate(from = VaultV1, with = crate::migrations::v1_to_v2))]
                pub vault: Account<'info, VaultV2>,
            }
        }
    }

    /// `bind()` gains the pre-step — old-header probe on a scoped read
    /// borrow, then the typed in-place `migrate_layout` — spliced BEFORE
    /// the validation fragment, so validators see the upgraded account.
    /// `validate()` and the per-field validators never contain the
    /// migrate call: the read-only surface never writes.
    #[test]
    fn migrate_emits_the_bind_pre_step_before_validation() {
        let s = expand(TokenStream::new(), touch_item())
            .expect("expand ok")
            .to_string();

        let w = fn_window(&s, "bind");
        let migrate_at = w
            .find("migrate_layout :: < VaultV1 , VaultV2 , _ >")
            .unwrap_or_else(|| {
                panic!("bind must call the typed in-place migrate for (Old, New): {w}")
            });
        let validate_at = w
            .find("Self :: validate (ctx) ?")
            .unwrap_or_else(|| panic!("bind must still delegate to validate(): {w}"));
        assert!(
            migrate_at < validate_at,
            "the migration pre-step must run BEFORE bind's validation fragment: {w}"
        );
        // The probe is the FULL Old identity on a read borrow that is
        // scoped (inner block) so it drops before migrate_layout takes
        // its own exclusive borrow.
        assert!(
            w.contains("try_borrow ()"),
            "the old-header probe must use a read borrow: {w}"
        );
        assert!(
            w.contains(
                "< VaultV1 as :: hopper :: __runtime :: LayoutContract > :: validate_header"
            ),
            "the probe must be the FULL Old validate_header, never a partial sniff: {w}"
        );
        // Exactly one migrate call in the whole expansion: bind's
        // pre-step. Neither validate() nor any per-field validator may
        // carry it (they are the read-only surface).
        assert_eq!(
            s.matches("migrate_layout").count(),
            1,
            "migrate_layout must appear exactly once (bind's pre-step): {s}"
        );
        assert!(
            !fn_window(&s, "validate (").contains("migrate_layout"),
            "validate() must never write: {s}"
        );
    }

    /// `validate()`'s layout-header check for the migrate field lowers to
    /// the either-version form: valid New, or fully-valid Old that
    /// already fits the New shape ("would bind accept this set?"). Both
    /// arms are the complete `validate_header`; the plain `load::<New>`
    /// is replaced; the owner pin stays; anything that is neither
    /// version surfaces the New layout's own error.
    #[test]
    fn migrate_validate_lowers_the_either_version_header_check() {
        let s = expand(TokenStream::new(), touch_item())
            .expect("expand ok")
            .to_string();

        let w = fn_window(&s, "validate_vault");
        assert!(
            w.contains(
                "< VaultV2 as :: hopper :: __runtime :: LayoutContract > :: validate_header"
            ),
            "the New arm must be the full validate_header: {w}"
        );
        assert!(
            w.contains(
                "< VaultV1 as :: hopper :: __runtime :: LayoutContract > :: validate_header"
            ),
            "the Old arm must be the full validate_header: {w}"
        );
        assert!(
            w.contains("< VaultV2 as :: hopper :: __runtime :: LayoutContract > :: required_len"),
            "the Old arm must also require the allocation to fit the New shape: {w}"
        );
        assert!(
            w.contains("Err (__hopper_new_err)"),
            "neither-version accounts must surface the New layout's own error, unchanged: {w}"
        );
        assert!(
            !w.contains("load :: < VaultV2 >"),
            "the plain New-only load must be replaced by the either-version check: {w}"
        );
        assert!(
            w.contains("check_owned_by"),
            "the owner pin is unchanged for migrate fields: {w}"
        );
        // The published check description documents the crank honestly.
        assert!(
            s.contains("lazy-migration source"),
            "VALIDATION_CHECKS must document the either-version acceptance: {s}"
        );
    }

    /// Both attribute forms — `#[hopper::context]` and
    /// `#[derive(Accounts)]` — share the expansion path, so both emit the
    /// pre-step and the either-version lowering.
    #[test]
    fn migrate_expands_in_both_attribute_forms() {
        let attr_form = expand(TokenStream::new(), touch_item())
            .expect("attr expand ok")
            .to_string();
        let derive_form = expand_for_derive(touch_item())
            .expect("derive expand ok")
            .to_string();
        for s in [&attr_form, &derive_form] {
            assert!(
                s.contains("migrate_layout :: < VaultV1 , VaultV2 , _ >"),
                "both forms must emit the bind pre-step: {s}"
            );
            assert!(
                s.contains(
                    "< VaultV1 as :: hopper :: __runtime :: LayoutContract > :: validate_header"
                ),
                "both forms must emit the either-version validate lowering: {s}"
            );
        }
    }

    /// A migrate context is NOT embeddable: the pre-step only runs in its
    /// own `bind()`, which an outer `#[composite]` bind never invokes, so
    /// embedding is refused at compile time (via the existing
    /// `__HOPPER_EMBEDDABLE` assertion) instead of silently skipping the
    /// crank.
    #[test]
    fn migrate_context_advertises_not_embeddable() {
        let s = expand(TokenStream::new(), touch_item())
            .expect("expand ok")
            .to_string();
        assert!(
            s.contains("__HOPPER_EMBEDDABLE : bool = false"),
            "a migrate context must refuse composite embedding: {s}"
        );
    }

    /// A migrate LEAF inside a composite CONTAINER is legal and cranks at
    /// its composite-aware flattened slot (`__HOPPER_BASE + const-sum`),
    /// the same expression the validators use.
    #[test]
    fn migrate_leaf_in_a_composite_container_uses_the_flattened_slot() {
        let item: TokenStream = quote! {
            pub struct Operate<'info> {
                pub payer: Signer<'info>,

                #[composite]
                pub check: VaultCheck<'info>,

                #[account(mut, migrate(from = VaultV1, with = crate::migrations::v1_to_v2))]
                pub vault: Account<'info, VaultV2>,
            }
        };
        let s = expand(TokenStream::new(), item)
            .expect("expand ok")
            .to_string();
        let w = fn_window(&s, "bind");
        assert!(
            w.contains("migrate_layout :: < VaultV1 , VaultV2 , _ >"),
            "the container's bind must carry the leaf's pre-step: {w}"
        );
        assert!(
            w.contains("account (__HOPPER_BASE + 1usize + VaultCheck :: ACCOUNT_COUNT)"),
            "the pre-step must address the composite-aware flattened slot: {w}"
        );
    }

    // ── Compile-error paths (every one actionable) ──────────────────────

    /// `migrate(...)` without `mut` on the same field is rejected: a
    /// migration writes.
    #[test]
    fn migrate_without_mut_is_rejected() {
        let item: TokenStream = quote! {
            pub struct Touch<'info> {
                #[account(migrate(from = VaultV1, with = crate::migrations::v1_to_v2))]
                pub vault: Account<'info, VaultV2>,
            }
        };
        let err = expand(TokenStream::new(), item).expect_err("migrate requires mut");
        let msg = err.to_string();
        assert!(msg.contains("requires `mut`"), "got: {msg}");
        assert!(msg.contains("rewrites the account bytes"), "got: {msg}");
    }

    /// Every lifecycle keyword is mutually exclusive with `migrate(...)`
    /// on one field: lifecycle attrs create/resize/drain the slot, a
    /// migration rewrites an existing old-layout account.
    #[test]
    fn migrate_lifecycle_combos_are_rejected() {
        let cases: [(TokenStream, &str); 6] = [
            (
                quote! { #[account(init, payer = payer, space = VaultV2::LEN, migrate(from = VaultV1, with = m))] },
                "init",
            ),
            (
                quote! { #[account(init_if_needed, payer = payer, space = VaultV2::LEN, migrate(from = VaultV1, with = m))] },
                "init_if_needed",
            ),
            (
                quote! { #[account(mut, zero, migrate(from = VaultV1, with = m))] },
                "zero",
            ),
            (
                quote! { #[account(mut, close = payer, migrate(from = VaultV1, with = m))] },
                "close",
            ),
            (
                quote! { #[account(mut, realloc = VaultV2::LEN, realloc_payer = payer, realloc_zero = true, migrate(from = VaultV1, with = m))] },
                "realloc",
            ),
            (
                quote! { #[account(mut, sweep = payer, migrate(from = VaultV1, with = m))] },
                "sweep",
            ),
        ];
        for (account_attr, kw) in cases {
            let item: TokenStream = quote! {
                pub struct Touch<'info> {
                    #[account(mut)]
                    pub payer: Signer<'info>,

                    #account_attr
                    pub vault: Account<'info, VaultV2>,

                    pub system_program: Program<'info, System>,
                }
            };
            let err = expand(TokenStream::new(), item)
                .expect_err(&format!("`{kw}` + migrate must be rejected"));
            let msg = err.to_string();
            assert!(
                msg.contains(&format!("cannot be combined with `{kw}`")),
                "{kw}: got {msg}"
            );
        }
    }

    /// `Option<..>` fields cannot migrate: an absent slot has nothing to
    /// rewrite.
    #[test]
    fn migrate_on_an_optional_field_is_rejected() {
        let item: TokenStream = quote! {
            pub struct Touch<'info> {
                #[account(mut, migrate(from = VaultV1, with = crate::migrations::v1_to_v2))]
                pub vault: Option<Account<'info, VaultV2>>,
            }
        };
        let err = expand(TokenStream::new(), item).expect_err("optional migrate must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("cannot target the optional account"),
            "got: {msg}"
        );
    }

    /// A `#[composite]` field is a nested context, not an account slot:
    /// it cannot carry `migrate(...)` (or any `#[account(...)]`
    /// constraint) — the existing composite guard fires.
    #[test]
    fn migrate_on_a_composite_field_is_rejected() {
        let item: TokenStream = quote! {
            pub struct Touch<'info> {
                #[composite]
                #[account(mut, migrate(from = VaultV1, with = crate::migrations::v1_to_v2))]
                pub check: VaultCheck<'info>,
            }
        };
        let err = expand(TokenStream::new(), item).expect_err("composite migrate must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("cannot carry `#[account(...)]`"), "got: {msg}");
    }

    /// `InitAccount<..>` is the type-directed `init` spelling and is
    /// rejected like the attribute form.
    #[test]
    fn migrate_on_an_init_account_wrapper_is_rejected() {
        let item: TokenStream = quote! {
            pub struct Touch<'info> {
                #[account(mut, migrate(from = VaultV1, with = crate::migrations::v1_to_v2))]
                pub vault: InitAccount<'info, VaultV2>,
            }
        };
        let err =
            expand(TokenStream::new(), item).expect_err("InitAccount migrate must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("cannot target `InitAccount<..>`"),
            "got: {msg}"
        );
    }

    /// Migration needs a Hopper layout to migrate INTO: role wrappers and
    /// raw views are rejected with the `Account<'info, NewLayout>` hint.
    #[test]
    fn migrate_on_a_non_layout_field_is_rejected() {
        for field_ty in [quote! { Signer<'info> }, quote! { AccountView }] {
            let item: TokenStream = quote! {
                pub struct Touch<'info> {
                    #[account(mut, migrate(from = VaultV1, with = crate::migrations::v1_to_v2))]
                    pub vault: #field_ty,
                }
            };
            let err =
                expand(TokenStream::new(), item).expect_err("non-layout migrate must be rejected");
            let msg = err.to_string();
            assert!(msg.contains("requires a Hopper layout field"), "got: {msg}");
        }
    }

    /// `from` naming the field's own layout is the compile-catchable form
    /// of a non-forward migration.
    #[test]
    fn migrate_from_the_same_layout_is_rejected() {
        let item: TokenStream = quote! {
            pub struct Touch<'info> {
                #[account(mut, migrate(from = VaultV2, with = crate::migrations::v1_to_v2))]
                pub vault: Account<'info, VaultV2>,
            }
        };
        let err = expand(TokenStream::new(), item).expect_err("same-type migrate must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("migration source must be a different layout version"),
            "got: {msg}"
        );
    }

    /// Every malformed spelling names the expected shape so the fix is
    /// copy-pasteable.
    #[test]
    fn migrate_malformed_spellings_are_rejected_naming_the_shape() {
        let cases: [TokenStream; 5] = [
            // Missing `with`.
            quote! { #[account(mut, migrate(from = VaultV1))] },
            // Missing `from`.
            quote! { #[account(mut, migrate(with = crate::migrations::v1_to_v2))] },
            // Empty list.
            quote! { #[account(mut, migrate())] },
            // Bare word.
            quote! { #[account(mut, migrate)] },
            // Unknown key.
            quote! { #[account(mut, migrate(source = VaultV1, with = m))] },
        ];
        for account_attr in cases {
            let item: TokenStream = quote! {
                pub struct Touch<'info> {
                    #account_attr
                    pub vault: Account<'info, VaultV2>,
                }
            };
            let err = expand(TokenStream::new(), item)
                .expect_err("malformed migrate spelling must be rejected");
            let msg = err.to_string();
            assert!(
                msg.contains("migrate(from = OldLayout, with = path::to::transform)"),
                "the error must name the expected shape, got: {msg}"
            );
        }
        // `migrate = path` (name-value instead of a list) is also refused.
        let item: TokenStream = quote! {
            pub struct Touch<'info> {
                #[account(mut, migrate = v1_to_v2)]
                pub vault: Account<'info, VaultV2>,
            }
        };
        let err =
            expand(TokenStream::new(), item).expect_err("name-value migrate must be rejected");
        assert!(
            err.to_string()
                .contains("migrate(from = OldLayout, with = path::to::transform)"),
            "got: {err}"
        );
    }
}

#[cfg(test)]
mod composite_v2_tests {
    //! Composite v2: the container's options compose across the nesting
    //! boundary. These tests pin the composed lowering — rebased write
    //! ranges, const-expr synthetic slots, the spliced schema — and that
    //! the formerly-gated option combinations now expand.

    use super::*;

    /// Slice the expanded output down to one generated fn's body: from
    /// `fn <name>` to the next `fn ` occurrence.
    fn fn_window<'a>(s: &'a str, fn_name: &str) -> &'a str {
        let needle = format!("fn {fn_name}");
        let start = s
            .find(&needle)
            .unwrap_or_else(|| panic!("missing `{needle}` in: {s}"));
        let tail = &s[start + needle.len()..];
        let end = tail.find("fn ").unwrap_or(tail.len());
        &s[start..start + needle.len() + end]
    }

    /// Every composite-free context now publishes the hidden
    /// `__HOPPER_DECLARED_WRITE_RANGES` const — the splice source for an
    /// embedding outer — with LOCAL indices and assoc-const spellings,
    /// independent of `strict_writes` (no authority implied: the
    /// authority const stays empty without the opt-in).
    #[test]
    fn composite_free_context_publishes_declared_ranges_without_strict_writes() {
        let item: TokenStream = quote! {
            pub struct VaultCheck<'info> {
                pub authority: Signer<'info>,

                #[account(mut(balance))]
                pub vault: Account<'info, Vault>,
            }
        };
        let s = expand(TokenStream::new(), item)
            .expect("expand ok")
            .to_string();
        assert!(
            s.contains("pub const __HOPPER_DECLARED_WRITE_RANGES"),
            "the declared const must be emitted without strict_writes: {s}"
        );
        assert!(
            s.contains(
                "WriteRange :: new (1u8 , :: hopper :: hopper_core :: account :: HEADER_LEN \
                 as u32 + < Vault > :: BALANCE_OFFSET , < Vault > :: BALANCE_SIZE)"
            ),
            "the declared range must use local indices + assoc-const spellings: {s}"
        );
        // No strict_writes: the authority const stays empty and no
        // policy is installed.
        assert!(
            s.contains("__HOPPER_VaultCheck_WRITE_RANGES : & [:: hopper :: __runtime :: write_policy :: WriteRange] = & [] ;"),
            "without strict_writes the authority const must stay empty: {s}"
        );
        assert!(
            !s.contains("set_write_policy"),
            "no policy install without strict_writes: {s}"
        );
    }

    /// The v1 gate is gone: `strict_writes` on a composite container
    /// expands, and the authority const is the compile-time-composed
    /// array — outer leaves at flattened const-expr indices, the inner
    /// context spliced from its declared const with rebased indices.
    #[test]
    fn composite_strict_writes_composes_rebased_write_ranges() {
        let attr: TokenStream = quote! { strict_writes };
        let item: TokenStream = quote! {
            pub struct Guarded<'info> {
                #[account(mut)]
                pub ledger: Account<'info, Ledger>,

                #[composite]
                pub check: VaultCheck<'info>,

                #[account(mut(balance))]
                pub tail: Account<'info, Vault>,
            }
        };
        let s = expand(attr, item).expect("expand ok").to_string();

        // Length composes: outer leaf grants count 1 each, the composite
        // contributes the inner's full declared set.
        assert!(
            s.contains(
                "const __HOPPER_Guarded_WRITE_RANGES_LEN : usize = 0usize + 1usize \
                 + VaultCheck :: __HOPPER_DECLARED_WRITE_RANGES . len () + 1usize"
            ),
            "composed length must sum leaves and the inner declared set: {s}"
        );
        // Outer leaf BEFORE the composite keeps its flattened (== plain)
        // index as a const-expr cast.
        assert!(
            s.contains("WriteRange :: whole_account ((0usize) as u8)"),
            "pre-composite leaf must keep its flattened index: {s}"
        );
        // Outer leaf AFTER the composite is rebased past the inner block:
        // exact token match on the composed const-expr index.
        assert!(
            s.contains(
                "WriteRange :: new ((1usize + VaultCheck :: ACCOUNT_COUNT) as u8 , \
                 :: hopper :: hopper_core :: account :: HEADER_LEN as u32 \
                 + < Vault > :: BALANCE_OFFSET , < Vault > :: BALANCE_SIZE)"
            ),
            "post-composite leaf range must carry the rebased const-expr index: {s}"
        );
        // The splice loop rebases the inner's indices by the composite's
        // flattened base offset.
        assert!(
            s.contains("let __inner = VaultCheck :: __HOPPER_DECLARED_WRITE_RANGES"),
            "the inner declared const must be spliced: {s}"
        );
        assert!(
            s.contains("let __base : usize = 1usize")
                && s.contains("__r . account_index = __abs as u8"),
            "the splice must rebase inner indices by the flattened base: {s}"
        );
        // The installed policy reads the composed const (same single
        // source of truth as composite-free contexts; the trailing comma
        // is the install stmt's long-standing token shape).
        assert!(
            s.contains("WritePolicy :: new (__HOPPER_Guarded_WRITE_RANGES ,)"),
            "bind must install the composed set: {s}"
        );
        assert!(
            s.contains("= & __HOPPER_Guarded_WRITE_RANGES_ARR"),
            "the authority slice must point at the composed array: {s}"
        );
        // A container is not itself embeddable, so it publishes no
        // declared const of its own.
        assert!(
            !s.contains("__HOPPER_DECLARED_WRITE_RANGES :"),
            "a container must not publish a declared const: {s}"
        );
    }

    /// `strict_writes, lamports(...)` on a container: the lamport list is
    /// rendered at flattened const-expr indices and the two-dimension
    /// policy + gate install survive the composition.
    #[test]
    fn composite_lamports_renders_flattened_const_expr_indices() {
        let attr: TokenStream = quote! { strict_writes, lamports(fee_sink) };
        let item: TokenStream = quote! {
            pub struct Funded<'info> {
                pub payer: Signer<'info>,

                #[composite]
                pub check: VaultCheck<'info>,

                #[account(mut)]
                pub vault: Account<'info, Vault>,

                pub fee_sink: AccountView,
            }
        };
        let s = expand(attr, item).expect("expand ok").to_string();
        assert!(
            s.contains("MUTATION_COMPLETE : bool = true"),
            "the composed context must stay mutation-complete: {s}"
        );
        // vault (mut, position 2) and fee_sink (explicit, position 3)
        // both sit past the composite: their published indices are the
        // flattened const exprs, sorted by field position.
        assert!(
            s.contains(
                "(1usize + VaultCheck :: ACCOUNT_COUNT) as u8 , \
                 (1usize + VaultCheck :: ACCOUNT_COUNT + 1usize) as u8"
            ),
            "the lamport set must carry rebased const-expr indices: {s}"
        );
        assert!(
            s.contains("with_lamports") && s.contains("try_install_lamport_gate"),
            "the two-dimension policy and gate install must survive: {s}"
        );
    }

    /// `lamports(...)` cannot name the composite field itself — the
    /// dimension grants outer leaves only, and the refusal says what to
    /// do instead.
    #[test]
    fn lamports_naming_a_composite_field_is_rejected() {
        let attr: TokenStream = quote! { strict_writes, lamports(check) };
        let item: TokenStream = quote! {
            pub struct Funded<'info> {
                pub payer: Signer<'info>,

                #[composite]
                pub check: VaultCheck<'info>,
            }
        };
        let err = expand(attr, item).expect_err("lamports(composite) must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("names a `#[composite]` field"), "got: {msg}");
        assert!(msg.contains("flatten the inner context"), "got: {msg}");
    }

    /// `event_cpi` on a container: the two synthetic slots trail the
    /// FLATTENED set at const-expr indices — the fused bind verify, the
    /// emit method, and `ACCOUNT_COUNT` all use the composed expressions.
    #[test]
    fn composite_event_cpi_places_synthetic_slots_at_const_expr_trailing_indices() {
        let attr: TokenStream = quote! { event_cpi };
        let item: TokenStream = quote! {
            pub struct Emitting<'info> {
                pub payer: Signer<'info>,

                #[composite]
                pub check: VaultCheck<'info>,
            }
        };
        let s = expand(attr, item).expect("expand ok").to_string();
        // Flattened total: payer + inner + the two trailing synthetics.
        assert!(
            s.contains(
                "ACCOUNT_COUNT : usize = 1usize + VaultCheck :: ACCOUNT_COUNT + 1usize + 1usize"
            ),
            "the synthetic slots must extend the flattened total: {s}"
        );
        // The fused bind-time verify addresses the authority at its
        // const-expr trailing offset.
        let bind = fn_window(&s, "bind");
        assert!(
            bind.contains(
                "verify_event_authority (ctx . account \
                 (__HOPPER_BASE + 1usize + VaultCheck :: ACCOUNT_COUNT) ?"
            ),
            "bind's fused verify must use the const-expr trailing slot: {bind}"
        );
        // The emit one-liner reads the same const-expr slot.
        let emit = fn_window(&s, "emit_event_cpi");
        assert!(
            emit.contains("account (__HOPPER_BASE + 1usize + VaultCheck :: ACCOUNT_COUNT)"),
            "emit_event_cpi must address the const-expr trailing slot: {emit}"
        );
        // The schema splice covers the synthetics per flattened slot.
        assert!(
            s.contains("__HOPPER_Emitting_SCHEMA_ACCOUNTS_LEN"),
            "the composed schema length const must exist: {s}"
        );
        assert!(
            s.contains("name : \"event_authority\"") && s.contains("name : \"event_program\""),
            "the schema splice must include both synthetic slots: {s}"
        );
    }

    /// `emit_touch_map` and `auto_lifecycle` on a container now expand:
    /// the const is advertised, and bind auto-runs the outer leaf's
    /// lifecycle helper (whose slots are composite-aware).
    #[test]
    fn composite_emit_touch_map_and_auto_lifecycle_expand() {
        let attr: TokenStream = quote! { emit_touch_map };
        let item: TokenStream = quote! {
            pub struct Traced<'info> {
                #[composite]
                pub check: VaultCheck<'info>,

                #[account(mut)]
                pub tail: Account<'info, Vault>,
            }
        };
        let s = expand(attr, item).expect("expand ok").to_string();
        assert!(
            s.contains("EMIT_TOUCH_MAP : bool = true"),
            "the touch-map const must be advertised on a container: {s}"
        );

        let attr: TokenStream = quote! { auto_lifecycle };
        let item: TokenStream = quote! {
            pub struct AutoRun<'info> {
                #[account(mut)]
                pub payer: Signer<'info>,

                #[composite]
                pub check: VaultCheck<'info>,

                #[account(init, payer = payer, space = Vault::LEN)]
                pub vault: InitAccount<'info, Vault>,

                pub system_program: Program<'info, System>,
            }
        };
        let s = expand(attr, item).expect("expand ok").to_string();
        let bind = fn_window(&s, "bind");
        assert!(
            bind.contains("__hopper_bound . init_vault () ? ;"),
            "auto_lifecycle must call the leaf's init helper from bind: {bind}"
        );
        // The helper's sibling-role lookups are composite-aware: the
        // created account and the system_program sit past the composite,
        // at rebased const-expr slots.
        let init = fn_window(&s, "init_vault");
        assert!(
            init.contains("account (__HOPPER_BASE + 1usize + VaultCheck :: ACCOUNT_COUNT)"),
            "the init helper must address the flattened account slot: {init}"
        );
        assert!(
            init.contains(
                "account (__HOPPER_BASE + 1usize + VaultCheck :: ACCOUNT_COUNT + 1usize)"
            ),
            "the init helper must address the flattened system_program slot: {init}"
        );
    }

    /// The inner-side rules did NOT loosen: an inner context carrying an
    /// option still advertises `__HOPPER_EMBEDDABLE = false` (the
    /// embedding site's assert then refuses it); the plain validation
    /// context stays embeddable.
    #[test]
    fn inner_side_embeddability_rules_are_unchanged() {
        for attr in [
            quote! { strict_writes },
            quote! { emit_touch_map },
            quote! { event_cpi },
        ] {
            let item: TokenStream = quote! {
                pub struct Inner<'info> {
                    #[account(mut)]
                    pub vault: Account<'info, Vault>,
                }
            };
            let s = expand(attr, item).expect("expand ok").to_string();
            assert!(
                s.contains("__HOPPER_EMBEDDABLE : bool = false"),
                "an option-carrying context must stay non-embeddable: {s}"
            );
        }
        // Control: the plain validation context stays embeddable.
        let item: TokenStream = quote! {
            pub struct Inner<'info> {
                #[account(mut)]
                pub vault: Account<'info, Vault>,
            }
        };
        let s = expand(TokenStream::new(), item)
            .expect("expand ok")
            .to_string();
        assert!(
            s.contains("__HOPPER_EMBEDDABLE : bool = true"),
            "a plain validation context must stay embeddable: {s}"
        );
    }
}
