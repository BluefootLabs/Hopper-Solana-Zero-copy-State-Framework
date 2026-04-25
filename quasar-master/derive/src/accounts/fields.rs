//! Per-field codegen for `#[derive(Accounts)]`.
//!
//! Each field in the accounts struct produces: parsing code (account extraction
//! from the accounts slice), validation code (constraint checks), and PDA seed
//! reconstruction. This module is the largest in the derive crate because each
//! field attribute combination produces distinct codegen paths.

#[path = "fields_process.rs"]
mod process;
#[path = "fields_support.rs"]
mod support;

pub(super) use process::process_fields;
use {
    super::{
        attrs::AccountFieldAttrs,
        field_kind::{strip_ref, FieldFlags, FieldKind},
    },
    crate::helpers::extract_generic_inner_type,
    syn::Ident,
};

/// Info for a single `#[account(close = dest)]` field.
pub(super) struct CloseFieldInfo {
    pub field: Ident,
    pub destination: Ident,
    /// For token/mint types, CPI close requires the token program and
    /// authority.
    pub cpi_close: Option<CpiCloseInfo>,
}

pub(super) struct CpiCloseInfo {
    pub token_program: Ident,
    pub authority: Ident,
}

/// Info for a single `#[account(sweep = receiver)]` field.
pub(super) struct SweepFieldInfo {
    pub field: Ident,
    pub receiver: Ident,
    pub mint: Ident,
    pub authority: Ident,
    pub token_program: Ident,
}

pub(super) struct ProcessedFields {
    pub field_constructs: Vec<proc_macro2::TokenStream>,
    pub field_checks: Vec<proc_macro2::TokenStream>,
    pub bump_init_vars: Vec<proc_macro2::TokenStream>,
    pub bump_struct_fields: Vec<proc_macro2::TokenStream>,
    pub bump_struct_inits: Vec<proc_macro2::TokenStream>,
    pub seeds_methods: Vec<proc_macro2::TokenStream>,
    pub seed_addr_captures: Vec<proc_macro2::TokenStream>,
    pub field_attrs: Vec<AccountFieldAttrs>,
    pub init_pda_checks: Vec<proc_macro2::TokenStream>,
    pub init_blocks: Vec<proc_macro2::TokenStream>,
    pub close_fields: Vec<CloseFieldInfo>,
    pub sweep_fields: Vec<SweepFieldInfo>,
    pub needs_rent: bool,
    /// If the struct has a `Sysvar<Rent>` field, its ident. Used to avoid
    /// the `sol_get_rent_sysvar` syscall when a rent account is available.
    pub rent_sysvar_field: Option<Ident>,
}

/// Compute the [`FieldFlags`] for a field (non-optional variant).
pub(super) fn compute_field_flags(
    field: &syn::Field,
    attrs: &super::attrs::AccountFieldAttrs,
    is_ref_mut: bool,
) -> FieldFlags {
    let ty = strip_ref(&field.ty);
    let kind = FieldKind::classify(ty);
    FieldFlags::compute(&kind, attrs, is_ref_mut)
}

/// Compute the expected u32 header value for a field based on its attributes
/// and type.
///
/// Returns a u32 in little-endian byte order:
/// - Byte 0: borrow_state (always 0xFF for no-dup)
/// - Byte 1: is_signer (1 if required, 0 otherwise)
/// - Byte 2: is_writable (1 if required, 0 otherwise)
/// - Byte 3: executable (1 if required, 0 otherwise)
pub(super) fn compute_header_expected(
    field: &syn::Field,
    attrs: &super::attrs::AccountFieldAttrs,
    is_ref_mut: bool,
) -> u32 {
    let effective_ty = extract_generic_inner_type(&field.ty, "Option").unwrap_or(&field.ty);
    let ty = strip_ref(effective_ty);
    let kind = FieldKind::classify(ty);
    FieldFlags::compute(&kind, attrs, is_ref_mut).header_constant()
}
