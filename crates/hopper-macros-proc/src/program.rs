//! `#[hopper_program]`, instruction dispatch codegen.
//!
//! Annotate a module to generate a `process_instruction` entry point that
//! dispatches based on the first byte of instruction data (discriminator).
//!
//! ```ignore
//! #[hopper_program]
//! mod vault {
//!     #[instruction(0)]
//!     fn initialize(ctx: &mut Context<'_>) -> ProgramResult { ... }
//!
//!     #[instruction(1)]
//!     fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult { ... }
//!
//!     // Context declared with `#[instruction(amount: u64, nonce: u8)]`:
//!     // `ctx_args = 2` forwards the first two decoded args into
//!     // `Swap::bind_with_args(ctx, amount, nonce)?` so seed / constraint
//!     // expressions in the context struct can reference them by name.
//!     #[instruction(2, ctx_args = 2)]
//!     fn swap(ctx: Context<Swap>, amount: u64, nonce: u8) -> ProgramResult { ... }
//! }
//! ```

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse2, punctuated::Punctuated, spanned::Spanned, Attribute, Expr, ExprLit, FnArg,
    GenericArgument, Ident, Item, ItemFn, ItemMod, Lit, Meta, Pat, Path, PathArguments, Result,
    Token, Type, TypePath,
};

/// A discovered instruction handler.
///
/// The discriminator is a `Vec<u8>` with length 1 for the legacy
/// single-byte form and length `N` (up to 8) for the multi-byte form.
/// Anchor programs use 8-byte SHA-256 prefixes; Quasar lets the author
/// pick any length. Hopper caps at 8 bytes because longer prefixes
/// cost CU at the dispatcher with no real uniqueness benefit past a
/// decent hash. Dispatch matches on the prefix of instruction_data.
#[derive(Debug)]
struct Handler {
    discriminator: Vec<u8>,
    fn_name: Ident,
    binding: ContextBinding,
    arg_types: Vec<Type>,
    instruction_policy: InstructionPolicyArgs,
}

#[derive(Default)]
struct HandlerModifiers {
    pipeline: bool,
    receipt: bool,
    invariants: Vec<InvariantSpec>,
}

/// A single `#[invariant(...)]` attribute on a handler.
///
/// Supports two forms:
/// - `#[invariant(cond)]`. bare condition, a violation returns
///   `ProgramError::InvalidAccountData` and leaves the receipt's
///   failure payload empty.
/// - `#[invariant(cond, err = MyError::Variant)]`. typed error
///   variant (declared via `#[hopper::error]`). On violation the
///   handler returns `ProgramError::Custom(MyError::Variant.code())`
///   and the receipt is stamped with the variant's code, its index
///   in `INVARIANT_TABLE`, and `FailureStage::Invariant`, closing the
///   on-chain → off-chain invariant chain.
struct InvariantSpec {
    condition: Expr,
    error_variant: Option<Expr>,
}

#[derive(Debug)]
enum ContextBinding {
    Raw,
    Typed { spec: Path },
}

/// Parsed `#[hopper::program(...)]` attribute arguments. Stored as
/// explicit per-field `Option<bool>` so a caller can supply partial
/// overrides (`#[hopper::program(strict = false)]` still implies the
/// other defaults from `HopperProgramPolicy::default_policy()`).
///
/// Named shorthand (`strict`, `raw`, `sealed`) pre-fills every field
/// to the matching `HopperProgramPolicy::*` constant; an explicit
/// `name = value` after the shorthand overrides individual levers.
#[derive(Default)]
struct ProgramPolicyArgs {
    strict: Option<bool>,
    enforce_token_checks: Option<bool>,
    allow_unsafe: Option<bool>,
}

impl ProgramPolicyArgs {
    /// Resolve unset levers from `HopperProgramPolicy::STRICT`.
    fn strict(&self) -> bool {
        self.strict.unwrap_or(true)
    }
    fn enforce_token_checks(&self) -> bool {
        self.enforce_token_checks.unwrap_or(true)
    }
    fn allow_unsafe(&self) -> bool {
        self.allow_unsafe.unwrap_or(true)
    }
}

/// Parsed per-handler `#[instruction(N, unsafe_memory, skip_token_checks, ctx_args = K)]`
/// flags. Bare flag form only: `unsafe_memory` equivalent to
/// `unsafe_memory = true`.
///
/// `ctx_args = K` ties the handler to a typed context spec that was
/// declared with `#[instruction(name: Type, ...)]`. It tells the
/// dispatch codegen: "the first `K` decoded args belong to the context
/// binder. thread them to `bind_with_args(...)`. and *all* decoded
/// args (including those K) are still forwarded to the handler
/// function". This is the program-side half of Audit Task #20: it
/// closes the gap where a `#[hopper::context]` with instruction args
/// couldn't compose with the `ctx: Context<MyAccounts>` sugar because
/// the args-less `bind(...)` is intentionally not emitted for such
/// specs.
///
/// Validation rules applied later in `prepare_handler`:
/// - `ctx_args > 0` requires the leading parameter to be a typed
///   context (not `&mut Context<'_>`).
/// - `ctx_args` must be ≤ the number of handler value args, otherwise
///   the decoder would be asked for more than exist.
/// - `ctx_args` defaults to `0`, preserving the legacy behavior where
///   dispatch calls `bind(ctx)?` on argument-free context specs.
#[derive(Default, Clone, Copy, Debug)]
struct InstructionPolicyArgs {
    unsafe_memory: bool,
    skip_token_checks: bool,
    ctx_args: u8,
}

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let policy = parse_program_policy(attr)?;
    let mut input: ItemMod = parse2(item.clone()).map_err(|_| {
        syn::Error::new_spanned(
            &item,
            "hopper_program expects a module, e.g. `mod vault { ... }`",
        )
    })?;

    let mut handlers: Vec<Handler> = Vec::new();
    if input.content.is_none() {
        return Err(syn::Error::new_spanned(
            &input,
            "hopper_program requires an inline module body",
        ));
    }
    let (_, items) = input.content.as_mut().expect("checked above");

    for module_item in items.iter_mut() {
        if let Item::Fn(method) = module_item {
            if let Some(mut handler) = prepare_handler(method)? {
                method.attrs.retain(|attr| !attr.path().is_ident("instruction"));
                handler.fn_name = method.sig.ident.clone();
                handlers.push(handler);
            }
        }
    }

    if handlers.is_empty() {
        return Err(syn::Error::new_spanned(
            &input,
            "hopper_program requires at least one #[instruction(N)] function",
        ));
    }

    // Sort for deterministic codegen AND correct dispatch ordering:
    // longer discriminators first so a 4-byte prefix can't be shadowed
    // by a 1-byte prefix it happens to start with. Within a length,
    // sort by bytes so the emitted `if` chain is stable.
    handlers.sort_by(|a, b| {
        b.discriminator
            .len()
            .cmp(&a.discriminator.len())
            .then_with(|| a.discriminator.cmp(&b.discriminator))
    });

    // Exact-duplicate detection. Two handlers with the same bytes is
    // always a bug, no matter the length.
    for i in 0..handlers.len() {
        for j in (i + 1)..handlers.len() {
            if handlers[i].discriminator == handlers[j].discriminator {
                return Err(syn::Error::new_spanned(
                    &input,
                    format!(
                        "duplicate instruction discriminator {:02x?}: `{}` and `{}`",
                        handlers[i].discriminator,
                        handlers[i].fn_name,
                        handlers[j].fn_name,
                    ),
                ));
            }
        }
    }

    // Prefix-shadow detection. If handler A's discriminator is a
    // strict prefix of handler B's, then B's bytes coming in would
    // match A's prefix first (in the sorted-by-length-desc emission,
    // B runs first, which is correct ordering, but the program author
    // probably didn't intend it). Flag it so the author either
    // lengthens A or shortens B.
    for i in 0..handlers.len() {
        for j in 0..handlers.len() {
            if i == j {
                continue;
            }
            let short = &handlers[i].discriminator;
            let long = &handlers[j].discriminator;
            if short.len() < long.len() && long.starts_with(short) {
                return Err(syn::Error::new_spanned(
                    &input,
                    format!(
                        "instruction discriminator {:02x?} (`{}`) is a prefix of {:02x?} (`{}`). Either lengthen the shorter one or shorten the longer one so the dispatcher can distinguish them unambiguously.",
                        short, handlers[i].fn_name, long, handlers[j].fn_name,
                    ),
                ));
            }
        }
    }

    // Fast path: every discriminator is exactly one byte. Emit the
    // dense `match data[0] { b => handler, ... }` form so the compiler
    // keeps the jump-table optimization. Anchor programs that never
    // touched the multi-byte syntax see byte-for-byte identical
    // codegen to the pre-multi-byte Hopper.
    let all_single_byte = handlers.iter().all(|h| h.discriminator.len() == 1);

    // Slow path entries for the multi-byte case. Emitted as an
    // ordered `if data.starts_with(&[...]) { ... } else if ...` chain
    // because `match` in Rust cannot branch on a variable-length slice
    // prefix.
    let dispatch_body = if all_single_byte {
        let match_arms: Vec<TokenStream> = handlers
            .iter()
            .map(|h| {
                let byte = h.discriminator[0];
                let invocation = handler_invocation(h);
                quote! { #byte => #invocation, }
            })
            .collect();
        quote! {
            if data.is_empty() {
                return ::core::result::Result::Err(
                    ::hopper::__runtime::ProgramError::InvalidInstructionData,
                );
            }
            match data[0] {
                #(#match_arms)*
                _ => ::core::result::Result::Err(
                    ::hopper::__runtime::ProgramError::InvalidInstructionData,
                ),
            }
        }
    } else {
        // Prefix-match chain, longest first.
        let arms: Vec<TokenStream> = handlers
            .iter()
            .map(|h| {
                let bytes = &h.discriminator;
                let byte_lits: Vec<u8> = bytes.clone();
                let invocation = handler_invocation(h);
                quote! {
                    if data.starts_with(&[ #(#byte_lits),* ]) {
                        return #invocation;
                    }
                }
            })
            .collect();
        quote! {
            #(#arms)*
            ::core::result::Result::Err(
                ::hopper::__runtime::ProgramError::InvalidInstructionData,
            )
        }
    };

    // Apply per-handler policy: when the program disallows unsafe and
    // the handler does not opt back in via `unsafe_memory`, emit
    // `#[deny(unsafe_code)]` on the handler. Surfacing it as an
    // attribute (rather than wrapping the function body) keeps the
    // policy visible in `cargo expand` output.
    if !policy.allow_unsafe() {
        for module_item in items.iter_mut() {
            let Item::Fn(method) = module_item else {
                continue;
            };
            let Some(handler) = handlers
                .iter()
                .find(|h| h.fn_name == method.sig.ident)
            else {
                continue;
            };
            if handler.instruction_policy.unsafe_memory {
                continue;
            }
            method.attrs.push(syn::parse_quote!(#[deny(unsafe_code)]));
        }
    }

    // Emit per-handler `<HANDLER>_POLICY: HopperInstructionPolicy`
    // constants so downstream code can branch on them at compile time
    // the same way it branches on `HOPPER_PROGRAM_POLICY`.
    for handler in handlers.iter() {
        let const_name = format_ident!("{}_POLICY", handler.fn_name.to_string().to_uppercase());
        let unsafe_memory = handler.instruction_policy.unsafe_memory;
        let skip_token_checks = handler.instruction_policy.skip_token_checks;
        let ctx_args = handler.instruction_policy.ctx_args;
        items.push(syn::parse_quote! {
            #[allow(non_upper_case_globals, dead_code)]
            pub const #const_name: ::hopper::__runtime::HopperInstructionPolicy =
                ::hopper::__runtime::HopperInstructionPolicy {
                    unsafe_memory: #unsafe_memory,
                    skip_token_checks: #skip_token_checks,
                    ctx_args: #ctx_args,
                };
        });
    }

    // Emit the program-level policy const at module scope so handlers
    // can consult it via `super::HOPPER_PROGRAM_POLICY.<lever>`.
    let strict = policy.strict();
    let enforce_token_checks = policy.enforce_token_checks();
    let allow_unsafe = policy.allow_unsafe();
    items.push(syn::parse_quote! {
        #[allow(dead_code)]
        pub const HOPPER_PROGRAM_POLICY: ::hopper::__runtime::HopperProgramPolicy =
            ::hopper::__runtime::HopperProgramPolicy {
                strict: #strict,
                enforce_token_checks: #enforce_token_checks,
                allow_unsafe: #allow_unsafe,
            };
    });

    items.push(syn::parse_quote! {
        #[inline]
        pub fn process_instruction(
            ctx: &mut ::hopper::prelude::Context<'_>,
        ) -> ::core::result::Result<(), ::hopper::__runtime::ProgramError> {
            let data = ctx.instruction_data();
            #dispatch_body
        }
    });

    let expanded = quote! { #input };

    Ok(expanded)
}

/// Parse `#[hopper::program(...)]` attribute args.
///
/// Accepts:
/// - empty args: defaults to `HopperProgramPolicy::STRICT`
/// - bare shorthand: `strict` | `raw` | `sealed`
/// - explicit levers: `strict = bool`, `enforce_token_checks = bool`, `allow_unsafe = bool`
/// - any combination (shorthand sets defaults; explicit levers override)
fn parse_program_policy(attr: TokenStream) -> Result<ProgramPolicyArgs> {
    let mut policy = ProgramPolicyArgs::default();
    if attr.is_empty() {
        return Ok(policy);
    }

    let metas: Punctuated<Meta, Token![,]> =
        syn::parse::Parser::parse2(Punctuated::<Meta, Token![,]>::parse_terminated, attr)?;

    for meta in metas {
        match meta {
            Meta::Path(path) => match path_ident(&path)?.as_str() {
                "strict" => {
                    policy.strict.get_or_insert(true);
                    policy.enforce_token_checks.get_or_insert(true);
                    policy.allow_unsafe.get_or_insert(true);
                }
                "sealed" => {
                    policy.strict.get_or_insert(true);
                    policy.enforce_token_checks.get_or_insert(true);
                    policy.allow_unsafe.get_or_insert(false);
                    // Explicit `sealed` locks allow_unsafe off even if
                    // defaulting left it unset as true.
                    policy.allow_unsafe = Some(false);
                }
                "raw" => {
                    policy.strict = Some(false);
                    policy.enforce_token_checks = Some(false);
                    policy.allow_unsafe.get_or_insert(true);
                }
                other => {
                    return Err(syn::Error::new(
                        path.span(),
                        format!(
                            "unknown program policy shorthand `{other}`; expected `strict`, `sealed`, or `raw`",
                        ),
                    ));
                }
            },
            Meta::NameValue(nv) => {
                let name = path_ident(&nv.path)?;
                let value = expect_bool_lit(&nv.value)?;
                match name.as_str() {
                    "strict" => policy.strict = Some(value),
                    "enforce_token_checks" => policy.enforce_token_checks = Some(value),
                    "allow_unsafe" => policy.allow_unsafe = Some(value),
                    other => {
                        return Err(syn::Error::new(
                            nv.path.span(),
                            format!(
                                "unknown program policy lever `{other}`; expected `strict`, `enforce_token_checks`, or `allow_unsafe`",
                            ),
                        ));
                    }
                }
            }
            Meta::List(list) => {
                return Err(syn::Error::new(
                    list.span(),
                    "hopper::program policy expects bare flags or `name = bool` pairs",
                ));
            }
        }
    }

    Ok(policy)
}

fn path_ident(path: &Path) -> Result<String> {
    path.get_ident()
        .map(|ident| ident.to_string())
        .ok_or_else(|| syn::Error::new(path.span(), "expected a bare identifier"))
}

fn expect_bool_lit(expr: &Expr) -> Result<bool> {
    if let Expr::Lit(ExprLit { lit: Lit::Bool(b), .. }) = expr {
        Ok(b.value)
    } else {
        Err(syn::Error::new(
            expr.span(),
            "expected a boolean literal (`true` or `false`)",
        ))
    }
}

/// Extract a u8 literal for `ctx_args = K`. The cap matches the
/// Hopper-wide 255-item limit already used by `#[hopper::error]`
/// receipts and SchemaExport positional slots. Anything larger almost
/// certainly indicates a typo rather than a 256-arg handler.
fn expect_u8_lit(expr: &Expr) -> Result<u8> {
    if let Expr::Lit(ExprLit { lit: Lit::Int(int), .. }) = expr {
        int.base10_parse::<u8>().map_err(|_| {
            syn::Error::new(
                expr.span(),
                "`ctx_args` must fit in a u8 (0..=255)",
            )
        })
    } else {
        Err(syn::Error::new(
            expr.span(),
            "expected an integer literal for `ctx_args`",
        ))
    }
}

fn prepare_handler(function: &mut ItemFn) -> Result<Option<Handler>> {
    if !function
        .attrs
        .iter()
        .any(|attr| attr_has_name(attr, "instruction"))
    {
        return Ok(None);
    }

    let modifiers = extract_handler_modifiers(&mut function.attrs)?;

    let (discriminator, instruction_policy) =
        extract_instruction_attribute(&function.attrs)?.ok_or_else(|| {
            syn::Error::new_spanned(
                &function.sig,
                "hopper_program requires #[instruction(N)] on each generated handler",
            )
        })?;

    if function.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "hopper_program handlers must start with either `ctx: &mut Context<'_>` or `ctx: Context<MyAccounts>`",
        ));
    }

    let mut inputs = function.sig.inputs.iter_mut();
    let first = inputs.next().expect("checked above");
    let binding = classify_context_binding(first)?;

    let mut arg_types = Vec::new();
    for input in inputs {
        match input {
            FnArg::Typed(pat_type) => arg_types.push((*pat_type.ty).clone()),
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "hopper_program does not support methods; use free functions inside the module",
                ));
            }
        }
    }

    // `ctx_args = K` is only valid when the handler's leading arg is a
    // typed `Context<MyAccounts>`. the `&mut Context<'_>` raw form
    // doesn't have a `bind_with_args` to route to. And K must not
    // exceed the handler's own value-arg count, because we decode one
    // arg per slot and the first K go to the binder.
    let ctx_args = instruction_policy.ctx_args as usize;
    if ctx_args > 0 {
        if matches!(binding, ContextBinding::Raw) {
            return Err(syn::Error::new_spanned(
                &function.sig,
                "`ctx_args = ...` requires a typed `Context<MyAccounts>` parameter, not `&mut Context<'_>`",
            ));
        }
        if ctx_args > arg_types.len() {
            return Err(syn::Error::new_spanned(
                &function.sig,
                format!(
                    "`ctx_args = {ctx_args}` exceeds the handler's {} value argument(s)",
                    arg_types.len(),
                ),
            ));
        }
    }

    apply_handler_modifiers(function, &binding, &modifiers)?;

    Ok(Some(Handler {
        discriminator,
        fn_name: function.sig.ident.clone(),
        binding,
        arg_types,
        instruction_policy,
    }))
}

fn handler_invocation(handler: &Handler) -> TokenStream {
    let fn_name = &handler.fn_name;
    let ctx_args = handler.instruction_policy.ctx_args as usize;

    // Fast path: no instruction args at all, and the context (if any)
    // is bound via the legacy `bind(ctx)?`. preserve byte-for-byte the
    // prior codegen shape so existing programs see no regression.
    if handler.arg_types.is_empty() {
        let ctx_expr = match &handler.binding {
            ContextBinding::Raw => quote! { ctx },
            ContextBinding::Typed { spec } => quote! { #spec::bind(ctx)? },
        };
        return quote! { #fn_name(#ctx_expr) };
    }

    let arg_idents: Vec<Ident> = (0..handler.arg_types.len())
        .map(|index| format_ident!("__hopper_arg_{index}"))
        .collect();
    let decode_stmts: Vec<_> = handler
        .arg_types
        .iter()
        .zip(arg_idents.iter())
        .map(|(arg_ty, arg_ident)| {
            quote! {
                let #arg_ident: #arg_ty =
                    <#arg_ty as ::hopper::__macro_support::DecodeInstructionArg>::decode(
                        &mut __hopper_decoder,
                    )?;
            }
        })
        .collect();

    // Build the context expression. `ctx_args == 0` keeps the legacy
    // `bind(ctx)?` (or raw `ctx`). `ctx_args > 0` routes to
    // `bind_with_args(ctx, arg0, ..., arg{K-1})?`. the same decoded
    // values are *also* reused as handler arguments, so the pattern
    // matches Anchor's "seeds refer to an arg the handler also sees"
    // ergonomics but with real typed bindings.
    let ctx_expr = match (&handler.binding, ctx_args) {
        (ContextBinding::Raw, _) => quote! { ctx },
        (ContextBinding::Typed { spec }, 0) => quote! { #spec::bind(ctx)? },
        (ContextBinding::Typed { spec }, k) => {
            let binder_args = &arg_idents[..k];
            quote! { #spec::bind_with_args(ctx, #(#binder_args),*)? }
        }
    };

    // Skip the discriminator prefix so the arg decoder starts at the
    // first payload byte. `disc_len` is the source of truth; it is
    // computed from the parsed bytes (1 for single-byte, N for
    // multi-byte). Single-byte programs keep the exact old offset.
    let disc_len = handler.discriminator.len();
    quote! {{
        let mut __hopper_decoder = ::hopper::__macro_support::Decoder::new(&data[#disc_len..]);
        #(#decode_stmts)*
        __hopper_decoder.finish()?;
        #fn_name(#ctx_expr, #(#arg_idents),*)
    }}
}

fn classify_context_binding(arg: &mut FnArg) -> Result<ContextBinding> {
    let FnArg::Typed(pat_type) = arg else {
        return Err(syn::Error::new_spanned(
            arg,
            "hopper_program handlers must use a typed context parameter, not a method receiver",
        ));
    };

    if is_raw_context_ref(&pat_type.ty) {
        return Ok(ContextBinding::Raw);
    }

    if let Some(spec) = extract_typed_context_spec(&pat_type.ty)? {
        let bound_ty = bind_type_for(&spec)?;
        pat_type.ty = Box::new(bound_ty);
        mark_pattern_mutable(&mut pat_type.pat)?;
        return Ok(ContextBinding::Typed { spec });
    }

    Err(syn::Error::new_spanned(
        &pat_type.ty,
        "hopper_program handlers must start with either `ctx: &mut Context<'_>` or `ctx: Context<MyAccounts>`",
    ))
}

/// Parse an `#[invariant(...)]` attribute on a handler.
///
/// Accepts:
/// - `#[invariant(expr)]`. condition only.
/// - `#[invariant(expr, err = MyError::Variant)]`. condition plus
///   typed error variant. The variant must come from an enum decorated
///   with `#[hopper::error]` so that `.code()` and `.invariant_idx()`
///   exist on it at expansion time.
///
/// Additional positional args or repeated `err =` keys are rejected
/// so mistakes surface early rather than silently emit stray code.
fn parse_invariant_attr(attr: &Attribute) -> Result<InvariantSpec> {
    let args = attr.parse_args_with(Punctuated::<Expr, Token![,]>::parse_terminated)?;
    let mut items = args.into_iter();
    let condition = items.next().ok_or_else(|| {
        syn::Error::new_spanned(attr, "#[invariant(...)] requires a boolean condition")
    })?;

    let mut error_variant: Option<Expr> = None;
    for extra in items {
        if let Expr::Assign(ref assign) = extra {
            if let Expr::Path(ref left) = *assign.left {
                if left.path.is_ident("err") {
                    if error_variant.is_some() {
                        return Err(syn::Error::new_spanned(
                            &extra,
                            "#[invariant(...)]: `err = ...` may only be set once",
                        ));
                    }
                    error_variant = Some((*assign.right).clone());
                    continue;
                }
            }
        }
        return Err(syn::Error::new_spanned(
            extra,
            "#[invariant(cond[, err = MyError::Variant])] supports only `err = ...` after the condition",
        ));
    }

    Ok(InvariantSpec { condition, error_variant })
}

fn extract_handler_modifiers(attrs: &mut Vec<Attribute>) -> Result<HandlerModifiers> {
    let mut modifiers = HandlerModifiers::default();
    let mut retained = Vec::with_capacity(attrs.len());

    for attr in attrs.drain(..) {
        if attr_has_name(&attr, "pipeline") {
            modifiers.pipeline = true;
            continue;
        }
        if attr_has_name(&attr, "receipt") {
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                return Err(syn::Error::new_spanned(
                    attr,
                    "receipt does not take arguments yet; use bare #[receipt]",
                ));
            }
            modifiers.receipt = true;
            continue;
        }
        if attr_has_name(&attr, "invariant") {
            modifiers.invariants.push(parse_invariant_attr(&attr)?);
            continue;
        }
        retained.push(attr);
    }

    *attrs = retained;
    Ok(modifiers)
}

fn apply_handler_modifiers(
    function: &mut ItemFn,
    binding: &ContextBinding,
    modifiers: &HandlerModifiers,
) -> Result<()> {
    if !modifiers.pipeline && !modifiers.receipt && modifiers.invariants.is_empty() {
        return Ok(());
    }

    let ctx_ident = context_ident(function)?;
    let raw_ctx = raw_context_expr(&ctx_ident, binding);
    let original_block = function.block.clone();

    if modifiers.receipt && matches!(binding, ContextBinding::Raw) {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "#[receipt] currently requires a typed Hopper context so receipt segments can be derived from #[hopper_context]",
        ));
    }

    let pipeline_checks = if modifiers.pipeline {
        quote! {
            #raw_ctx.require_unique_writable_accounts()?;
            #raw_ctx.require_unique_signer_accounts()?;
        }
    } else {
        TokenStream::new()
    };

    let receipt_begin = if modifiers.receipt {
        match binding {
            ContextBinding::Typed { spec } => quote! {
                let __hopper_receipt_scope = #spec::begin_receipt_scope::<256>(#raw_ctx)?;
            },
            ContextBinding::Raw => TokenStream::new(),
        }
    } else {
        TokenStream::new()
    };

    let receipt_finish = if modifiers.receipt {
        quote! {
            __hopper_receipt_scope.finish(
                #raw_ctx,
                __hopper_invariants_passed,
                __hopper_invariants_checked,
                __hopper_failure_info,
            )?;
        }
    } else {
        TokenStream::new()
    };

    // Build the failure-capture block for each invariant. Two flavors:
    //
    // 1. Bare condition (`#[invariant(cond)]`). on violation we preserve
    //    the legacy behavior: `ProgramError::InvalidAccountData`, no
    //    failure payload recorded.
    //
    // 2. Typed-error form (`#[invariant(cond, err = MyError::Variant)]`)
    //. on violation we extract `.code()` and `.invariant_idx()` from
    //    the variant (methods emitted by `#[hopper::error]`), stamp the
    //    receipt's failure slot when a receipt is in scope, and return
    //    `ProgramError::Custom(code)`. This is what makes the invariant
    //    name visible to off-chain consumers without a hand-written
    //    lookup table.
    //
    // When `#[receipt]` is not on the handler, the failure stamp has
    // nowhere to go and we skip the assignment. that also avoids an
    // `unused_assignments` warning from the generated code.
    let receipt_enabled = modifiers.receipt;
    let invariant_checks: Vec<_> = modifiers
        .invariants
        .iter()
        .map(|spec| {
            let cond = &spec.condition;
            let on_fail = match &spec.error_variant {
                Some(err) => {
                    let stamp = if receipt_enabled {
                        quote! {
                            __hopper_failure_info = ::core::option::Option::Some((
                                __hopper_err_code,
                                __hopper_err_idx,
                                ::hopper::prelude::FailureStage::Invariant,
                            ));
                        }
                    } else {
                        TokenStream::new()
                    };
                    quote! {
                        let __hopper_err_variant = #err;
                        let __hopper_err_code: u32 = __hopper_err_variant.code();
                        let __hopper_err_idx: u8 = __hopper_err_variant.invariant_idx();
                        #stamp
                        __hopper_modifier_error = ::core::option::Option::Some(
                            ::hopper::__runtime::ProgramError::Custom(__hopper_err_code),
                        );
                    }
                }
                None => quote! {
                    __hopper_modifier_error = ::core::option::Option::Some(
                        ::hopper::__runtime::ProgramError::InvalidAccountData,
                    );
                },
            };
            quote! {
                if __hopper_modifier_error.is_none() {
                    let __hopper_invariant_value = (|| -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError> {
                        Ok(#cond)
                    })()?;
                    __hopper_invariants_checked = __hopper_invariants_checked.saturating_add(1);
                    if !__hopper_invariant_value {
                        __hopper_invariants_passed = false;
                        #on_fail
                    }
                }
            }
        })
        .collect();

    // Only declare `__hopper_failure_info` when a receipt scope will
    // consume it, otherwise the generated code would have an unused
    // variable. The variable must exist in scope for any
    // `__hopper_failure_info = Some(...)` assignment the checks emit,
    // so its declaration gates off the typed-error stamp above.
    let failure_info_decl = if receipt_enabled {
        quote! {
            let mut __hopper_failure_info: ::core::option::Option<(
                u32,
                u8,
                ::hopper::prelude::FailureStage,
            )> = None;
        }
    } else {
        TokenStream::new()
    };

    function.block = Box::new(syn::parse_quote!({
        #pipeline_checks
        #receipt_begin
        let mut __hopper_invariants_passed = true;
        let mut __hopper_invariants_checked: u16 = 0;
        let __hopper_result = (|| #original_block)();

        match __hopper_result {
            Ok(__hopper_value) => {
                let mut __hopper_modifier_error: ::core::option::Option<::hopper::__runtime::ProgramError> = None;
                #failure_info_decl
                #(#invariant_checks)*
                #receipt_finish
                if let ::core::option::Option::Some(__hopper_error) = __hopper_modifier_error {
                    Err(__hopper_error)
                } else {
                    Ok(__hopper_value)
                }
            }
            Err(__hopper_error) => Err(__hopper_error),
        }
    }));

    Ok(())
}

fn context_ident(function: &ItemFn) -> Result<Ident> {
    let Some(first) = function.sig.inputs.first() else {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "hopper_program handlers require a leading context parameter",
        ));
    };
    let FnArg::Typed(pat_type) = first else {
        return Err(syn::Error::new_spanned(
            first,
            "hopper_program handlers must use a simple context identifier when execution modifiers are present",
        ));
    };
    let Pat::Ident(ident) = pat_type.pat.as_ref() else {
        return Err(syn::Error::new_spanned(
            pat_type.pat.as_ref(),
            "execution modifiers require a simple identifier like `ctx` for the first parameter",
        ));
    };
    Ok(ident.ident.clone())
}

fn raw_context_expr(ctx_ident: &Ident, binding: &ContextBinding) -> TokenStream {
    match binding {
        ContextBinding::Raw => quote! { #ctx_ident },
        ContextBinding::Typed { .. } => quote! { #ctx_ident.raw() },
    }
}

fn attr_has_name(attr: &Attribute, name: &str) -> bool {
    attr.path()
        .segments
        .last()
        .map(|segment| segment.ident == name)
        .unwrap_or(false)
}

fn is_raw_context_ref(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    if reference.mutability.is_none() {
        return false;
    }
    is_context_path(reference.elem.as_ref())
}

fn is_context_path(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Path(TypePath { qself: None, path })
            if path.segments.last().map(|segment| segment.ident == "Context").unwrap_or(false)
    )
}

fn extract_typed_context_spec(ty: &Type) -> Result<Option<Path>> {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return Ok(None);
    };
    let Some(last) = path.segments.last() else {
        return Ok(None);
    };
    if last.ident != "Context" {
        return Ok(None);
    }

    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return Err(syn::Error::new_spanned(
            last,
            "typed Hopper handlers use `Context<MyAccounts>`",
        ));
    };

    let mut spec = None;
    for arg in &args.args {
        if let GenericArgument::Type(Type::Path(type_path)) = arg {
            if spec.is_some() {
                return Err(syn::Error::new_spanned(
                    arg,
                    "typed Hopper handlers accept exactly one context type argument",
                ));
            }
            spec = Some(type_path.path.clone());
        }
    }

    spec.map(Some).ok_or_else(|| {
        syn::Error::new_spanned(
            args,
            "typed Hopper handlers require a path type, e.g. `Context<Deposit>`",
        )
    })
}

fn bind_type_for(spec: &Path) -> Result<Type> {
    let mut bound = spec.clone();
    let Some(last) = bound.segments.last_mut() else {
        return Err(syn::Error::new_spanned(spec, "expected a concrete context type path"));
    };
    if !matches!(last.arguments, PathArguments::None) {
        return Err(syn::Error::new_spanned(
            last,
            "typed Hopper contexts must name the generated context struct directly",
        ));
    }
    last.ident = format_ident!("{}Ctx", last.ident);

    Ok(syn::parse_quote! { #bound<'_, '_> })
}

fn mark_pattern_mutable(pattern: &mut Box<Pat>) -> Result<()> {
    let Pat::Ident(ident) = pattern.as_mut() else {
        return Err(syn::Error::new_spanned(
            pattern.as_ref(),
            "typed Hopper context parameters must use a simple identifier pattern",
        ));
    };
    if ident.mutability.is_none() {
        ident.mutability = Some(Default::default());
    }
    Ok(())
}

/// Extract the discriminator plus any per-handler policy flags from
/// `#[instruction(N)]`, `#[instruction(discriminator = [0x1a, 0xf4])]`,
/// or either plus trailing `unsafe_memory` / `skip_token_checks` /
/// `ctx_args = K` levers.
///
/// Two discriminator forms are accepted:
///
/// - **Single byte.** `#[instruction(3)]` parses as a `u8` literal.
///   Dispatch emits a classic `match data[0]` so the compiler keeps
///   its jump-table optimization. This is the legacy shape and the
///   recommended one for programs with ≤256 instructions.
/// - **Multi-byte.** `#[instruction(discriminator = [0x1a, 0xf4, 0x3c, 0x2d])]`
///   parses as a byte-array literal (up to 8 bytes). Dispatch emits
///   a `data.starts_with(&[...])` chain ordered longest-prefix-first.
///   Used for Anchor-style 8-byte SHA-256 discriminators, namespaced
///   program ports, or any multi-program interop surface where a
///   single byte cannot be guaranteed unique across ABIs.
///
/// Returns the discriminator as `Vec<u8>` so both forms feed the
/// same downstream dispatch/codegen paths.
fn extract_instruction_attribute(
    attrs: &[Attribute],
) -> Result<Option<(Vec<u8>, InstructionPolicyArgs)>> {
    for attr in attrs {
        if !attr_has_name(attr, "instruction") {
            continue;
        }

        let tokens = match &attr.meta {
            syn::Meta::List(list) => list.tokens.clone(),
            _ => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "hopper_program requires #[instruction(N, ...flags)] or #[instruction(discriminator = [bytes], ...flags)]",
                ));
            }
        };

        use syn::parse::{ParseStream, Parser};

        let parser = |input: ParseStream| -> Result<(Vec<u8>, InstructionPolicyArgs)> {
            // Peek to decide single-byte vs multi-byte form. A leading
            // `discriminator` identifier signals the array form; anything
            // else (integer literal, bare flag, nothing) falls through to
            // the single-byte / legacy path.
            let disc: Vec<u8> = if input.peek(syn::Ident)
                && input.fork().parse::<Ident>().map(|i| i == "discriminator").unwrap_or(false)
            {
                let _: Ident = input.parse()?;
                let _: Token![=] = input.parse()?;
                let arr: syn::ExprArray = input.parse()?;
                if arr.elems.is_empty() {
                    return Err(syn::Error::new_spanned(
                        &arr,
                        "discriminator array must not be empty",
                    ));
                }
                if arr.elems.len() > 8 {
                    return Err(syn::Error::new_spanned(
                        &arr,
                        "discriminator array may be at most 8 bytes; longer prefixes cost CU at the dispatcher with no meaningful uniqueness benefit",
                    ));
                }
                let mut bytes = Vec::with_capacity(arr.elems.len());
                for elem in &arr.elems {
                    match elem {
                        Expr::Lit(ExprLit { lit: Lit::Int(int), .. }) => {
                            bytes.push(int.base10_parse::<u8>().map_err(|_| {
                                syn::Error::new_spanned(
                                    elem,
                                    "discriminator bytes must fit in u8 (0..=255)",
                                )
                            })?);
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(
                                elem,
                                "discriminator array entries must be u8 integer literals",
                            ));
                        }
                    }
                }
                bytes
            } else {
                let disc_lit: Lit = input.parse()?;
                match disc_lit {
                    Lit::Int(lit_int) => vec![lit_int.base10_parse::<u8>()?],
                    other => {
                        return Err(syn::Error::new(
                            other.span(),
                            "instruction discriminator must be an integer literal or `discriminator = [bytes]`",
                        ));
                    }
                }
            };

            let mut policy = InstructionPolicyArgs::default();
            while input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
                if input.is_empty() {
                    break;
                }
                let meta: Meta = input.parse()?;
                match meta {
                    Meta::Path(path) => match path_ident(&path)?.as_str() {
                        "unsafe_memory" => policy.unsafe_memory = true,
                        "skip_token_checks" => policy.skip_token_checks = true,
                        other => {
                            return Err(syn::Error::new(
                                path.span(),
                                format!(
                                    "unknown instruction policy flag `{other}`; expected `unsafe_memory`, `skip_token_checks`, or `ctx_args = K`",
                                ),
                            ));
                        }
                    },
                    Meta::NameValue(nv) => {
                        let name = path_ident(&nv.path)?;
                        match name.as_str() {
                            "unsafe_memory" => {
                                policy.unsafe_memory = expect_bool_lit(&nv.value)?;
                            }
                            "skip_token_checks" => {
                                policy.skip_token_checks = expect_bool_lit(&nv.value)?;
                            }
                            // `ctx_args = K`. forward the first K decoded
                            // instruction args to the typed context's
                            // `bind_with_args(...)`. A u8 cap matches the
                            // Hopper runtime's schema-wide 255-item limit
                            // (same cap as error variants) and sidesteps
                            // any argument-count collision with a huge
                            // handler signature.
                            "ctx_args" => {
                                let count = expect_u8_lit(&nv.value)?;
                                policy.ctx_args = count;
                            }
                            other => {
                                return Err(syn::Error::new(
                                    nv.path.span(),
                                    format!(
                                        "unknown instruction policy lever `{other}`; expected `unsafe_memory`, `skip_token_checks`, or `ctx_args`",
                                    ),
                                ));
                            }
                        }
                    }
                    Meta::List(list) => {
                        return Err(syn::Error::new(
                            list.span(),
                            "instruction policy expects bare flags or `name = bool` pairs",
                        ));
                    }
                }
            }
            Ok((disc, policy))
        };

        let (disc, policy) = parser.parse2(tokens)?;
        return Ok(Some((disc, policy)));
    }
    Ok(None)
}

// ─────────────────────────────────────────────────────────────────────
// Regression tests for `#[instruction(N, ctx_args = K, ...)]` parsing
// and `handler_invocation` routing. These live in the proc-macro crate
// itself so they exercise `extract_instruction_attribute` /
// `handler_invocation` with real `syn` inputs, catching grammar-level
// regressions (parser rejects `ctx_args`, wrong error message shape,
// wrong bind target) before the framework-wide build ever runs.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod ctx_args_tests {
    use super::*;
    use syn::parse_quote;

    fn extract(attr: Attribute) -> (Vec<u8>, InstructionPolicyArgs) {
        extract_instruction_attribute(&[attr])
            .expect("parse succeeded")
            .expect("attr present")
    }

    #[test]
    fn ctx_args_zero_is_default_when_absent() {
        let (disc, pol) = extract(parse_quote!(#[instruction(0)]));
        assert_eq!(disc, vec![0u8]);
        assert_eq!(pol.ctx_args, 0);
        assert!(!pol.unsafe_memory);
        assert!(!pol.skip_token_checks);
    }

    #[test]
    fn single_byte_discriminator_becomes_one_element_vec() {
        let (disc, _) = extract(parse_quote!(#[instruction(42)]));
        assert_eq!(disc, vec![42u8]);
    }

    #[test]
    fn multi_byte_discriminator_parses_to_byte_vec() {
        let (disc, _) = extract(parse_quote!(
            #[instruction(discriminator = [0x1a, 0xf4, 0x3c, 0x2d])]
        ));
        assert_eq!(disc, vec![0x1a, 0xf4, 0x3c, 0x2d]);
    }

    #[test]
    fn multi_byte_rejects_empty_array() {
        let res = extract_instruction_attribute(&[parse_quote!(
            #[instruction(discriminator = [])]
        )]);
        let err = res.expect_err("empty array should error");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn multi_byte_rejects_oversize_array() {
        let res = extract_instruction_attribute(&[parse_quote!(
            #[instruction(discriminator = [1, 2, 3, 4, 5, 6, 7, 8, 9])]
        )]);
        let err = res.expect_err("9-byte array should error");
        assert!(err.to_string().contains("8 bytes"));
    }

    #[test]
    fn multi_byte_accepts_flags_after_array() {
        let (disc, pol) = extract(parse_quote!(
            #[instruction(discriminator = [0xde, 0xad, 0xbe, 0xef], unsafe_memory, ctx_args = 1)]
        ));
        assert_eq!(disc, vec![0xde, 0xad, 0xbe, 0xef]);
        assert!(pol.unsafe_memory);
        assert_eq!(pol.ctx_args, 1);
    }

    #[test]
    fn ctx_args_is_threaded_from_attribute() {
        let (_, pol) = extract(parse_quote!(#[instruction(1, ctx_args = 2)]));
        assert_eq!(pol.ctx_args, 2);
    }

    #[test]
    fn ctx_args_coexists_with_other_flags() {
        let (_, pol) = extract(parse_quote!(
            #[instruction(3, unsafe_memory, ctx_args = 1, skip_token_checks)]
        ));
        assert!(pol.unsafe_memory);
        assert!(pol.skip_token_checks);
        assert_eq!(pol.ctx_args, 1);
    }

    #[test]
    fn ctx_args_rejects_non_integer_literal() {
        let result = extract_instruction_attribute(&[
            parse_quote!(#[instruction(0, ctx_args = "two")]),
        ]);
        let err = result.expect_err("should reject string literal");
        let msg = err.to_string();
        assert!(
            msg.contains("integer literal"),
            "unexpected error message: {msg}",
        );
    }

    #[test]
    fn ctx_args_rejects_u8_overflow() {
        let result = extract_instruction_attribute(&[
            parse_quote!(#[instruction(0, ctx_args = 256)]),
        ]);
        let err = result.expect_err("should reject 256");
        let msg = err.to_string();
        assert!(msg.contains("u8"), "unexpected error message: {msg}");
    }

    #[test]
    fn unknown_flag_lists_ctx_args_in_suggestion() {
        let result = extract_instruction_attribute(&[
            parse_quote!(#[instruction(0, unknown_flag)]),
        ]);
        let err = result.expect_err("should reject unknown flag");
        let msg = err.to_string();
        assert!(
            msg.contains("ctx_args"),
            "diagnostic must mention ctx_args: {msg}",
        );
    }

    // Parse a handler function body end-to-end through `prepare_handler`
    // to verify the combined ctx_args + typed context validation path.
    fn run_prepare(mut function: syn::ItemFn) -> Result<Handler> {
        prepare_handler(&mut function).map(|o| o.expect("handler discovered"))
    }

    #[test]
    fn ctx_args_without_typed_context_errors_clearly() {
        let f: syn::ItemFn = parse_quote! {
            #[instruction(0, ctx_args = 1)]
            fn handler(ctx: &mut Context<'_>, amount: u64) -> ProgramResult { Ok(()) }
        };
        let err = run_prepare(f).expect_err("should reject raw context + ctx_args");
        let msg = err.to_string();
        assert!(msg.contains("typed"), "want 'typed' in error: {msg}");
    }

    #[test]
    fn ctx_args_exceeding_arity_errors_clearly() {
        let f: syn::ItemFn = parse_quote! {
            #[instruction(0, ctx_args = 3)]
            fn handler(ctx: Context<Swap>, amount: u64) -> ProgramResult { Ok(()) }
        };
        let err = run_prepare(f).expect_err("should reject ctx_args > arity");
        let msg = err.to_string();
        assert!(msg.contains("exceeds"), "want 'exceeds' in error: {msg}");
    }

    #[test]
    fn ctx_args_valid_typed_context_builds() {
        let f: syn::ItemFn = parse_quote! {
            #[instruction(0, ctx_args = 2)]
            fn handler(ctx: Context<Swap>, amount: u64, nonce: u8) -> ProgramResult { Ok(()) }
        };
        let h = run_prepare(f).expect("should accept typed context + matching ctx_args");
        assert_eq!(h.instruction_policy.ctx_args, 2);
        assert!(matches!(h.binding, ContextBinding::Typed { .. }));
        assert_eq!(h.arg_types.len(), 2);
    }

    #[test]
    fn handler_invocation_threads_ctx_args_to_bind_with_args() {
        let h = Handler {
            discriminator: vec![0u8],
            fn_name: format_ident!("swap"),
            binding: ContextBinding::Typed { spec: parse_quote!(Swap) },
            arg_types: vec![
                parse_quote!(u64),
                parse_quote!(u8),
                parse_quote!(::core::primitive::bool),
            ],
            instruction_policy: InstructionPolicyArgs {
                unsafe_memory: false,
                skip_token_checks: false,
                ctx_args: 2,
            },
        };
        let raw = handler_invocation(&h).to_string();
        // Normalize whitespace. `quote`'s pretty-printer inserts spaces
        // between every token, so raw `.contains` against a readable
        // snippet is brittle. We collapse runs of whitespace and drop
        // space around grouping punctuation before substring-matching
        // the semantic shape.
        let out: String = raw
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(" (", "(")
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(") ", ")")
            .replace(" ,", ",")
            .replace(", ", ",")
            .replace(" ::", "::")
            .replace(":: ", "::");

        assert!(
            out.contains("Swap::bind_with_args(ctx,__hopper_arg_0,__hopper_arg_1)?"),
            "missing bind_with_args call: {out}",
        );
        assert!(out.contains("swap("), "handler name should appear: {out}");
        assert!(
            out.contains("__hopper_arg_2"),
            "third arg should reach handler: {out}",
        );
        // Extra belt-and-suspenders: the legacy bind(ctx)? MUST NOT
        // appear here. that would silently bypass the args-aware
        // validation path.
        assert!(
            !out.contains("Swap::bind(ctx)"),
            "legacy bind(ctx)? should not appear alongside ctx_args: {out}",
        );
    }

    #[test]
    fn handler_invocation_without_ctx_args_preserves_legacy_bind() {
        let h = Handler {
            discriminator: vec![0u8],
            fn_name: format_ident!("deposit"),
            binding: ContextBinding::Typed { spec: parse_quote!(Deposit) },
            arg_types: vec![parse_quote!(u64)],
            instruction_policy: InstructionPolicyArgs::default(),
        };
        let raw = handler_invocation(&h).to_string();
        let out: String = raw
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(" (", "(")
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(") ", ")")
            .replace(" ,", ",")
            .replace(" ::", "::")
            .replace(":: ", "::");

        assert!(
            out.contains("Deposit::bind(ctx)?"),
            "legacy bind should still be used when ctx_args = 0: {out}",
        );
        assert!(
            !out.contains("bind_with_args"),
            "bind_with_args must not appear when ctx_args = 0: {out}",
        );
    }

    #[test]
    fn handler_invocation_raw_ctx_never_calls_bind() {
        let h = Handler {
            discriminator: vec![0u8],
            fn_name: format_ident!("raw"),
            binding: ContextBinding::Raw,
            arg_types: vec![],
            instruction_policy: InstructionPolicyArgs::default(),
        };
        let out = handler_invocation(&h).to_string();
        assert!(
            !out.contains("bind"),
            "raw ctx handler must not reference any bind*: {out}",
        );
        assert!(out.contains("raw (ctx)"), "raw ctx dispatch expected: {out}");
    }
}
