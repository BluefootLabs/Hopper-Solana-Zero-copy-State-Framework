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
    parse2, Attribute, Expr, FnArg, GenericArgument, Ident, Item, ItemFn, ItemMod, Lit, Pat,
    Path, PathArguments, Result, Type, TypePath,
};

/// A discovered instruction handler.
struct Handler {
    discriminator: u8,
    fn_name: Ident,
    binding: ContextBinding,
    arg_types: Vec<Type>,
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

pub fn expand(_attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
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

fn prepare_handler(function: &mut ItemFn) -> Result<Option<Handler>> {
    if !function
        .attrs
        .iter()
        .any(|attr| attr_has_name(attr, "instruction"))
    {
        return Ok(None);
    }

    let modifiers = extract_handler_modifiers(&mut function.attrs)?;

    let discriminator = extract_instruction_discriminator(&function.attrs)?.ok_or_else(|| {
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

/// Extract the discriminator from `#[instruction(N)]`.
fn extract_instruction_discriminator(attrs: &[Attribute]) -> Result<Option<u8>> {
    for attr in attrs {
        if !attr_has_name(attr, "instruction") {
            continue;
        }
        let disc: Lit = attr.parse_args()?;
        match disc {
            Lit::Int(lit_int) => {
                let val: u8 = lit_int.base10_parse()?;
                return Ok(Some(val));
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    disc,
                    "instruction discriminator must be an integer literal",
                ));
            }
        }
    }
    Ok(None)
}
