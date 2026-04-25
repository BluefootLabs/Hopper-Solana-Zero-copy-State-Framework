use {
    super::super::{
        attrs::AccountFieldAttrs,
        field_kind::{strip_ref, type_base_name, FieldFlags, FieldKind},
    },
    crate::helpers::extract_generic_inner_type,
    quote::quote,
    syn::{Ident, Type},
};

#[derive(Copy, Clone)]
pub(super) enum TokenProgramResolution {
    ExplicitOnly,
    FallbackRequired,
}

impl TokenProgramResolution {
    pub(super) fn fallback_to_single_field(self) -> bool {
        matches!(self, TokenProgramResolution::FallbackRequired)
    }

    pub(super) fn require_account_field(self) -> bool {
        matches!(self, TokenProgramResolution::FallbackRequired)
    }
}

/// Find a field by type base name or Program<T>/Interface<T> wrapper. Returns
/// the field ident if found.
fn find_field_by_type<'a>(
    fields: &'a syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    type_names: &[&str],
) -> Option<&'a Ident> {
    for field in fields.iter() {
        let ty = strip_ref(&field.ty);

        if let Some(base) = type_base_name(ty) {
            if type_names.contains(&base.as_str()) {
                return field.ident.as_ref();
            }
        }

        match FieldKind::classify(ty) {
            FieldKind::Program { inner_ty } | FieldKind::Interface { inner_ty } => {
                if let Some(inner_base) = type_base_name(inner_ty) {
                    if type_names.contains(&inner_base.as_str()) {
                        return field.ident.as_ref();
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn is_token_program_field(field: &syn::Field) -> bool {
    match FieldKind::classify(strip_ref(&field.ty)) {
        FieldKind::Program { inner_ty } | FieldKind::Interface { inner_ty } => {
            type_base_name(inner_ty)
                .as_deref()
                .is_some_and(|name| matches!(name, "Token" | "Token2022" | "TokenInterface"))
        }
        _ => false,
    }
}

fn resolve_explicit_token_program_field<'a>(
    fields: &'a syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    selector: &Ident,
    selector_attr: &str,
) -> Result<&'a Ident, proc_macro::TokenStream> {
    let field = fields
        .iter()
        .find(|f| f.ident.as_ref() == Some(selector))
        .ok_or_else(|| -> proc_macro::TokenStream {
            syn::Error::new_spanned(
                selector,
                format!("`{selector_attr}` references unknown field `{selector}`"),
            )
            .to_compile_error()
            .into()
        })?;

    if !is_token_program_field(field) {
        return Err(syn::Error::new_spanned(
            selector,
            format!(
                "`{selector_attr}` must reference `Program<Token>`, `Program<Token2022>`, or \
                 `Interface<TokenInterface>`"
            ),
        )
        .to_compile_error()
        .into());
    }

    Ok(field
        .ident
        .as_ref()
        .expect("account field must have an identifier"))
}

/// Count how many fields match the given type names (checking Program<T> and
/// Interface<T> wrappers). Used to detect ambiguous token program fields.
fn count_fields_by_type(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    type_names: &[&str],
) -> usize {
    let mut count = 0;
    for field in fields.iter() {
        let ty = strip_ref(&field.ty);
        let matched = match FieldKind::classify(ty) {
            FieldKind::Program { inner_ty } | FieldKind::Interface { inner_ty } => {
                type_base_name(inner_ty).is_some_and(|b| type_names.contains(&b.as_str()))
            }
            _ => false,
        };
        if matched {
            count += 1;
        }
    }
    count
}

/// Find a field by name.
pub(super) fn find_field_by_name<'a>(
    fields: &'a syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    name: &str,
) -> Option<&'a Ident> {
    fields
        .iter()
        .find(|f| f.ident.as_ref().is_some_and(|i| i == name))
        .and_then(|f| f.ident.as_ref())
}

/// Find a field whose type is `Account<T>` (or `&Account<T>` / `&mut
/// Account<T>`) where `T` matches one of the given inner type names.
fn find_field_by_account_inner_type<'a>(
    fields: &'a syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    inner_type_names: &[&str],
) -> Option<&'a Ident> {
    for field in fields.iter() {
        let ty = strip_ref(&field.ty);
        if let FieldKind::Account { inner_ty } = FieldKind::classify(ty) {
            if let Some(base) = type_base_name(inner_ty) {
                if inner_type_names.contains(&base.as_str()) {
                    return field.ident.as_ref();
                }
            }
        }
    }
    None
}

pub(super) struct DetectedFields<'a> {
    pub system_program: Option<&'a Ident>,
    pub token_program: Option<&'a Ident>,
    pub token_program_count: usize,
    pub associated_token_program: Option<&'a Ident>,
    pub metadata_program: Option<&'a Ident>,
    pub metadata_account: Option<&'a Ident>,
    pub master_edition_account: Option<&'a Ident>,
    pub payer: Option<&'a Ident>,
    pub realloc_payer: Option<&'a Ident>,
    pub mint_authority: Option<&'a Ident>,
    pub update_authority: Option<&'a Ident>,
    pub rent_sysvar: Option<&'a Ident>,
}

impl<'a> DetectedFields<'a> {
    pub(super) fn detect(
        fields: &'a syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
        field_attrs: &[AccountFieldAttrs],
    ) -> Self {
        let system_program = find_field_by_type(fields, &["System"]);
        let token_program = find_field_by_type(fields, &["Token", "Token2022", "TokenInterface"]);
        let token_program_count =
            count_fields_by_type(fields, &["Token", "Token2022", "TokenInterface"]);
        let associated_token_program = find_field_by_type(fields, &["AssociatedTokenProgram"]);
        let metadata_program = find_field_by_type(fields, &["MetadataProgram"]);

        let metadata_account = find_field_by_account_inner_type(fields, &["MetadataAccount"]);
        let master_edition_account =
            find_field_by_account_inner_type(fields, &["MasterEditionAccount"]);

        let explicit_payer = field_attrs.iter().find_map(|a| a.payer.as_ref());
        let payer = explicit_payer
            .and_then(|name| find_field_by_name(fields, &name.to_string()))
            .or_else(|| find_field_by_name(fields, "payer"));

        let explicit_realloc_payer = field_attrs.iter().find_map(|a| a.realloc_payer.as_ref());
        let realloc_payer = explicit_realloc_payer
            .and_then(|name| find_field_by_name(fields, &name.to_string()))
            .or_else(|| {
                field_attrs
                    .iter()
                    .find_map(|a| a.payer.as_ref())
                    .and_then(|name| find_field_by_name(fields, &name.to_string()))
            })
            .or_else(|| find_field_by_name(fields, "payer"));

        let mint_authority = find_field_by_name(fields, "mint_authority")
            .or_else(|| find_field_by_name(fields, "authority"));
        let update_authority = find_field_by_name(fields, "update_authority").or(mint_authority);

        let rent_sysvar = fields.iter().find_map(|field| {
            let ty = strip_ref(&field.ty);
            if let FieldKind::Sysvar { inner_ty } = FieldKind::classify(ty) {
                if type_base_name(inner_ty).as_deref() == Some("Rent") {
                    return field.ident.as_ref();
                }
            }
            None
        });

        Self {
            system_program,
            token_program,
            token_program_count,
            associated_token_program,
            metadata_program,
            metadata_account,
            master_edition_account,
            payer,
            realloc_payer,
            mint_authority,
            update_authority,
            rent_sysvar,
        }
    }

    pub(super) fn require(
        field: Option<&'a Ident>,
        msg: &str,
    ) -> Result<&'a Ident, proc_macro::TokenStream> {
        field.ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), msg)
                .to_compile_error()
                .into()
        })
    }
}

pub(super) fn resolve_token_program_field<'a>(
    fields: &'a syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    detected: &DetectedFields<'a>,
    field_name: &Ident,
    selector: Option<&Ident>,
    selector_attr: &str,
    consumer_desc: &str,
    resolution: TokenProgramResolution,
) -> Result<Option<&'a Ident>, proc_macro::TokenStream> {
    if let Some(selector) = selector {
        return Ok(Some(resolve_explicit_token_program_field(
            fields,
            selector,
            selector_attr,
        )?));
    }

    if detected.token_program_count > 1 {
        return Err(syn::Error::new_spanned(
            field_name,
            format!(
                "Multiple token program fields detected. `{selector_attr} = <field>` is required \
                 for {consumer_desc} when more than one token program field is present."
            ),
        )
        .to_compile_error()
        .into());
    }

    if resolution.fallback_to_single_field() {
        if resolution.require_account_field() {
            return Ok(Some(DetectedFields::require(
                detected.token_program,
                &format!(
                    "{consumer_desc} requires a token program field (Program<Token>, \
                     Program<Token2022>, or Interface<TokenInterface>)"
                ),
            )?));
        }
        return Ok(detected.token_program);
    }

    if resolution.require_account_field() {
        return Ok(Some(DetectedFields::require(
            detected.token_program,
            &format!(
                "{consumer_desc} requires a token program field (Program<Token>, \
                 Program<Token2022>, or Interface<TokenInterface>)"
            ),
        )?));
    }

    Ok(None)
}

/// Resolve the token program address expression for a non-init field based on
/// its wrapper type.
pub(super) fn resolve_token_program_addr(
    effective_ty: &Type,
    token_program_field: Option<&Ident>,
) -> proc_macro2::TokenStream {
    if let Some(tp) = token_program_field {
        return quote! { #tp.to_account_view().address() };
    }

    let underlying = strip_ref(effective_ty);
    if let FieldKind::Account { inner_ty } = FieldKind::classify(underlying) {
        if let Some(name) = type_base_name(inner_ty) {
            match name.as_str() {
                "Token" | "Mint" => return quote! { &quasar_spl::SPL_TOKEN_ID },
                "Token2022" | "Mint2022" => return quote! { &quasar_spl::TOKEN_2022_ID },
                _ => {}
            }
        }
    }

    let tp = token_program_field
        .expect("InterfaceAccount with token/ata attrs requires a token program field");
    quote! { #tp.to_account_view().address() }
}

/// Validate attribute combinations on a single field. Returns a compile error
/// on conflict.
pub(super) fn validate_field_attrs(
    field: &syn::Field,
    field_name: &Ident,
    attrs: &AccountFieldAttrs,
    kind: &FieldKind,
    _flags: &FieldFlags,
) -> Result<(), proc_macro::TokenStream> {
    macro_rules! reject {
        ($cond:expr, $msg:expr) => {
            if $cond {
                return Err(syn::Error::new_spanned(field_name, $msg)
                    .to_compile_error()
                    .into());
            }
        };
    }

    let is_init = attrs.is_init || attrs.init_if_needed;
    let is_optional = extract_generic_inner_type(&field.ty, "Option").is_some();

    reject!(
        is_init && attrs.close.is_some(),
        "#[account(init)] and #[account(close)] cannot be used on the same field"
    );
    reject!(
        attrs.is_init && attrs.init_if_needed,
        "#[account(init)] and #[account(init_if_needed)] are mutually exclusive"
    );
    reject!(
        attrs.realloc.is_some() && is_init,
        "#[account(realloc)] and #[account(init)] cannot be used on the same field"
    );
    reject!(
        attrs.realloc.is_some() && kind.is_token_or_mint(),
        "#[account(realloc)] cannot be used on token or mint accounts — their size is fixed by \
         the token program"
    );
    reject!(
        attrs.realloc.is_some() && !matches!(kind, FieldKind::Account { .. }),
        "#[account(realloc)] is only valid on Account<T> fields"
    );
    reject!(
        attrs.realloc.is_some() && is_optional,
        "#[account(realloc)] cannot be used on Option<Account<T>> fields"
    );
    reject!(
        attrs.close.is_some() && kind.inner_name_matches(&["Mint", "Mint2022"]),
        "#[account(close)] cannot be used on mint accounts. Mint closing is not supported through \
         the token-account close path."
    );

    reject!(
        attrs.sweep.is_some() && !kind.is_token_account(),
        "#[account(sweep)] is only valid on token accounts, not mint accounts"
    );
    reject!(
        attrs.sweep.is_some() && attrs.token_mint.is_none(),
        "#[account(sweep)] requires `token::mint` (needed for transfer_checked decimals)"
    );
    reject!(
        attrs.sweep.is_some() && attrs.token_authority.is_none(),
        "#[account(sweep)] requires `token::authority` (needed for transfer signer)"
    );

    reject!(
        attrs.token_mint.is_some() && attrs.associated_token_mint.is_some(),
        "`token::*` and `associated_token::*` cannot be used on the same field"
    );
    reject!(
        attrs.seeds.is_some() && attrs.associated_token_mint.is_some(),
        "`seeds` and `associated_token::*` cannot be used on the same field"
    );

    reject!(
        attrs.payer.is_some() && !is_init,
        "`payer` requires `init` or `init_if_needed`"
    );
    reject!(
        attrs.space.is_some() && !is_init,
        "`space` requires `init` or `init_if_needed`"
    );

    reject!(
        attrs.token_mint.is_some() != attrs.token_authority.is_some(),
        "`token::mint` and `token::authority` must both be specified"
    );
    reject!(
        attrs.associated_token_mint.is_some() != attrs.associated_token_authority.is_some(),
        "`associated_token::mint` and `associated_token::authority` must both be specified"
    );
    reject!(
        attrs.associated_token_token_program.is_some() && attrs.associated_token_mint.is_none(),
        "`associated_token::token_program` requires `associated_token::mint` and \
         `associated_token::authority`"
    );
    reject!(
        attrs.token_token_program.is_some()
            && attrs.token_mint.is_none()
            && attrs.sweep.is_none()
            && attrs.close.is_none(),
        "`token::token_program` requires `token::mint`/`token::authority`, `sweep`, or token \
         account `close`"
    );
    reject!(
        attrs.mint_token_program.is_some()
            && attrs.mint_decimals.is_none()
            && attrs.master_edition_max_supply.is_none(),
        "`mint::token_program` requires `mint::decimals` or `master_edition::max_supply`"
    );
    reject!(
        attrs.realloc_payer.is_some() && attrs.realloc.is_none(),
        "`realloc::payer` requires `realloc`"
    );

    let has_metadata = attrs.metadata_name.is_some()
        || attrs.metadata_symbol.is_some()
        || attrs.metadata_uri.is_some()
        || attrs.metadata_seller_fee_basis_points.is_some()
        || attrs.metadata_is_mutable.is_some();
    reject!(
        has_metadata && !is_init,
        "`metadata::*` attributes require `init` or `init_if_needed`"
    );
    reject!(
        has_metadata
            && (attrs.metadata_name.is_none()
                || attrs.metadata_symbol.is_none()
                || attrs.metadata_uri.is_none()),
        "`metadata::name`, `metadata::symbol`, and `metadata::uri` must all be specified"
    );

    reject!(
        attrs.master_edition_max_supply.is_some() && !is_init,
        "`master_edition::max_supply` requires `init` or `init_if_needed`"
    );
    reject!(
        attrs.master_edition_max_supply.is_some() && attrs.metadata_name.is_none(),
        "`master_edition::max_supply` requires `metadata::name`, `metadata::symbol`, and \
         `metadata::uri`"
    );

    if attrs.dup {
        let has_doc = field.attrs.iter().any(|a| a.path().is_ident("doc"));
        reject!(
            !has_doc,
            "#[account(dup)] requires a /// CHECK: <reason> doc comment explaining why this \
             account is safe to use as a duplicate."
        );
    }

    Ok(())
}
