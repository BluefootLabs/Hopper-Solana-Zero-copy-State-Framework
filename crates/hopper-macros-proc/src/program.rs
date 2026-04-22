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
struct Handler {
    discriminator: u8,
    fn_name: Ident,
    binding: ContextBinding,
    arg_types: Vec<Type>,
    instruction_policy: InstructionPolicyArgs,
}

#[derive(Default)]
struct HandlerModifiers {
    pipeline: bool,
    receipt: bool,
    invariants: Vec<Expr>,
}

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

/// Parsed per-handler `#[instruction(N, unsafe_memory, skip_token_checks)]`
/// flags. Bare flag form only: `unsafe_memory` equivalent to
/// `unsafe_memory = true`.
#[derive(Default, Clone, Copy)]
struct InstructionPolicyArgs {
    unsafe_memory: bool,
    skip_token_checks: bool,
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

    // Sort by discriminator for deterministic codegen.
    handlers.sort_by_key(|h| h.discriminator);

    // Check for duplicate discriminators.
    for pair in handlers.windows(2) {
        if pair[0].discriminator == pair[1].discriminator {
            return Err(syn::Error::new_spanned(
                &input,
                format!(
                    "duplicate instruction discriminator {}: `{}` and `{}`",
                    pair[0].discriminator,
                    pair[0].fn_name,
                    pair[1].fn_name,
                ),
            ));
        }
    }

    // Generate match arms.
    let match_arms: Vec<_> = handlers
        .iter()
        .map(|h| {
            let disc = h.discriminator;
            let invocation = handler_invocation(h);
            quote! {
                #disc => #invocation,
            }
        })
        .collect();

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
        items.push(syn::parse_quote! {
            #[allow(non_upper_case_globals, dead_code)]
            pub const #const_name: ::hopper::__runtime::HopperInstructionPolicy =
                ::hopper::__runtime::HopperInstructionPolicy {
                    unsafe_memory: #unsafe_memory,
                    skip_token_checks: #skip_token_checks,
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
            if data.is_empty() {
                return Err(::hopper::__runtime::ProgramError::InvalidInstructionData);
            }
            match data[0] {
                #(#match_arms)*
                _ => Err(::hopper::__runtime::ProgramError::InvalidInstructionData),
            }
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
    let ctx_expr = match &handler.binding {
        ContextBinding::Raw => quote! { ctx },
        ContextBinding::Typed { spec } => quote! { #spec::bind(ctx)? },
    };

    if handler.arg_types.is_empty() {
        return quote! { #fn_name(#ctx_expr) };
    }

    let arg_idents: Vec<_> = (0..handler.arg_types.len())
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

    quote! {{
        let mut __hopper_decoder = ::hopper::__macro_support::Decoder::new(&data[1..]);
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
            modifiers.invariants.push(attr.parse_args::<Expr>()?);
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
            __hopper_receipt_scope.finish(#raw_ctx, __hopper_invariants_passed, __hopper_invariants_checked)?;
        }
    } else {
        TokenStream::new()
    };

    let invariant_checks: Vec<_> = modifiers
        .invariants
        .iter()
        .map(|expr| {
            quote! {
                if __hopper_modifier_error.is_none() {
                    let __hopper_invariant_value = (|| -> ::core::result::Result<bool, ::hopper::__runtime::ProgramError> {
                        Ok(#expr)
                    })()?;
                    __hopper_invariants_checked = __hopper_invariants_checked.saturating_add(1);
                    if !__hopper_invariant_value {
                        __hopper_invariants_passed = false;
                        __hopper_modifier_error = Some(::hopper::__runtime::ProgramError::InvalidAccountData);
                    }
                }
            }
        })
        .collect();

    function.block = Box::new(syn::parse_quote!({
        #pipeline_checks
        #receipt_begin
        let mut __hopper_invariants_passed = true;
        let mut __hopper_invariants_checked: u16 = 0;
        let __hopper_result = (|| #original_block)();

        match __hopper_result {
            Ok(__hopper_value) => {
                let mut __hopper_modifier_error: ::core::option::Option<::hopper::__runtime::ProgramError> = None;
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
/// `#[instruction(N)]` or `#[instruction(N, unsafe_memory, skip_token_checks)]`.
///
/// The first positional argument is always the discriminator literal.
/// Trailing bare identifiers (`unsafe_memory`, `skip_token_checks`)
/// set the matching bit on `HopperInstructionPolicy`. `name = bool`
/// pairs are also accepted for symmetry with `#[hopper::program(...)]`.
fn extract_instruction_attribute(
    attrs: &[Attribute],
) -> Result<Option<(u8, InstructionPolicyArgs)>> {
    for attr in attrs {
        if !attr_has_name(attr, "instruction") {
            continue;
        }

        // Parse the attribute argument list as a comma-separated
        // sequence of `Meta` items so we get a single idiomatic
        // representation for both the shorthand `#[instruction(1)]`
        // and the extended `#[instruction(1, unsafe_memory)]` forms.
        // An `Expr::Lit` is wrapped in `Meta::Path` when bare (`1`
        // parses as a path in syn's grammar only after custom lexing),
        // so we fall back to a manual token walk that accepts either
        // a leading literal or a leading meta.
        let tokens = match &attr.meta {
            syn::Meta::List(list) => list.tokens.clone(),
            _ => {
                return Err(syn::Error::new_spanned(
                    attr,
                    "hopper_program requires #[instruction(N, ...flags)]",
                ));
            }
        };

        // First token must be an integer literal (the discriminator).
        // We parse it directly, then parse the rest as `Meta` items.
        use syn::parse::{ParseStream, Parser};

        let parser = |input: ParseStream| -> Result<(u8, InstructionPolicyArgs)> {
            let disc_lit: Lit = input.parse()?;
            let disc = match disc_lit {
                Lit::Int(lit_int) => lit_int.base10_parse::<u8>()?,
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "instruction discriminator must be an integer literal",
                    ));
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
                                    "unknown instruction policy flag `{other}`; expected `unsafe_memory` or `skip_token_checks`",
                                ),
                            ));
                        }
                    },
                    Meta::NameValue(nv) => {
                        let name = path_ident(&nv.path)?;
                        let value = expect_bool_lit(&nv.value)?;
                        match name.as_str() {
                            "unsafe_memory" => policy.unsafe_memory = value,
                            "skip_token_checks" => policy.skip_token_checks = value,
                            other => {
                                return Err(syn::Error::new(
                                    nv.path.span(),
                                    format!(
                                        "unknown instruction policy lever `{other}`",
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
