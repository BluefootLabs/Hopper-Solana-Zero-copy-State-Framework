//! `#[hopper_state]`. contract-aware zero-copy layout codegen.
//!
//! The canonical proc-macro path must participate in the same runtime,
//! schema, and receipt pipeline as hand-written Hopper layouts. This macro
//! therefore emits more than a `SegmentMap`: it generates field metadata,
//! layout fingerprints, typed load helpers, and schema export hooks.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use sha2::{Digest, Sha256};
use syn::{parse::Parser, parse2, Attribute, Fields, ItemStruct, LitInt, Result};

#[derive(Clone)]
struct StateOptions {
    disc: Option<u8>,
    version: u8,
    /// Audit innovation I5 (hybrid serialization). When set, the
    /// layout emits tail-access helpers that read/write a
    /// length-prefixed dynamic payload at offset
    /// `HEADER_LEN + BODY_SIZE`.
    dynamic_tail: Option<syn::Type>,
}

impl Default for StateOptions {
    fn default() -> Self {
        Self {
            disc: None,
            version: 1,
            dynamic_tail: None,
        }
    }
}

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let options = parse_state_options(attr)?;
    let dynamic_tail = options.dynamic_tail.clone();
    let input: ItemStruct = parse2(item)?;
    let name = &input.ident;
    let vis = &input.vis;

    if !has_repr_c(&input.attrs) {
        return Err(syn::Error::new_spanned(
            &input,
            "hopper_state requires #[repr(C)] so segment offsets and typed loads stay stable",
        ));
    }

    let fields = match &input.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &input,
                "hopper_state requires a struct with named fields",
            ))
        }
    };

    let mut segment_entries = Vec::new();
    let mut module_items = Vec::new();
    let mut inherent_items = Vec::new();
    let mut field_name_literals = Vec::new();
    let mut field_type_literals = Vec::new();
    let mut field_types = Vec::new();
    let mut running_offset = quote! { 0u32 };

    let struct_name_upper = to_screaming_snake(&name.to_string());

    for field in fields.iter() {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let field_ty = &field.ty;
        let field_name_upper = to_screaming_snake(&field_name_str);
        let current_offset = running_offset.clone();

        field_name_literals.push(syn::LitStr::new(&field_name_str, field_name.span()));
        field_type_literals.push(syn::LitStr::new(
            &field_ty.to_token_stream().to_string().replace(' ', ""),
            field_name.span(),
        ));
        field_types.push(field_ty.clone());

        segment_entries.push(quote! {
            ::hopper::hopper_core::segment_map::StaticSegment::new(
                #field_name_str,
                #current_offset,
                core::mem::size_of::<#field_ty>() as u32,
            )
        });

        let const_name = format_ident!("{}_{}_OFFSET", struct_name_upper, field_name_upper);
        let const_abs_name = format_ident!("{}_{}_ABS_OFFSET", struct_name_upper, field_name_upper);
        let const_size_name = format_ident!("{}_{}_SIZE", struct_name_upper, field_name_upper);
        let const_type_name = format_ident!("{}_{}_TYPE", struct_name_upper, field_name_upper);
        let assoc_offset_name = format_ident!("{}_OFFSET", field_name_upper);
        let assoc_abs_offset_name = format_ident!("{}_ABS_OFFSET", field_name_upper);
        let assoc_size_name = format_ident!("{}_SIZE", field_name_upper);

        // Body-relative offset + account-absolute offset for each field.
        //
        // `*_OFFSET` is the field's offset within the layout body (0-indexed
        // from the start of the layout's `#[repr(C)]` struct). That's the
        // value `ctx.segment_mut::<T>(..)` historically expected with the
        // 16-byte header pre-added.
        //
        // `*_ABS_OFFSET` folds in `HEADER_LEN` so the caller can pass it
        // directly to any segment accessor that expects a buffer-absolute
        // offset. avoiding `HEADER_LEN + Vault::AUTHORITY_OFFSET`
        // boilerplate at every call site.
        module_items.push(quote! {
            #vis const #const_name: u32 = #current_offset;
            #vis const #const_abs_name: u32 =
                ::hopper::hopper_core::account::HEADER_LEN as u32 + #current_offset;
            #vis const #const_size_name: u32 = core::mem::size_of::<#field_ty>() as u32;
            #vis type #const_type_name = #field_ty;
        });

        inherent_items.push(quote! {
            #vis const #assoc_offset_name: u32 = #current_offset;
            #vis const #assoc_abs_offset_name: u32 =
                ::hopper::hopper_core::account::HEADER_LEN as u32 + #current_offset;
            #vis const #assoc_size_name: u32 = core::mem::size_of::<#field_ty>() as u32;
        });

        running_offset = quote! {
            #current_offset + core::mem::size_of::<#field_ty>() as u32
        };
    }

    let body_size = running_offset.clone();
    let version = options.version;
    let layout_id = layout_id_bytes(name, version, fields);
    // Default discriminator: first byte of the layout_id fingerprint.
    // If that byte is zero (1-in-256 chance given SHA-256 uniformity)
    // we fall through to the first non-zero byte so the compile-time
    // "disc != 0" fence never fires spuriously. This mirrors Quasar's
    // `validate_discriminator_not_zero()` but in a forgiving form:
    // the user never needs to set `disc = ...` explicitly unless they
    // want a specific wire value.
    let disc = options.disc.unwrap_or_else(|| {
        for byte in layout_id.iter() {
            if *byte != 0 {
                return *byte;
            }
        }
        // All-zero SHA-256 first 8 bytes is astronomically improbable;
        // if we ever hit it the 1-byte fallback is still non-zero.
        1u8
    });
    let layout_id_tokens = byte_array_literal(&layout_id);
    let field_count = field_name_literals.len();

    // ── Audit I5: hybrid-serialization tail helpers ──────────────────
    //
    // When the user writes `#[hopper::state(dynamic_tail = MyTail)]`,
    // emit `tail_len`, `tail_read`, `tail_write`, and a
    // `TAIL_PREFIX_OFFSET` constant on the layout's inherent impl.
    // The offset points to the `u32 LE` length prefix that precedes
    // the dynamic payload. exactly the position defined in
    // `crates/hopper-runtime/src/tail.rs`.
    //
    // Layouts without `dynamic_tail` emit an empty token stream and
    // pay zero cost: the fast path stays strictly zero-copy.
    let dynamic_tail_methods = if let Some(tail_ty) = &dynamic_tail {
        quote! {
            /// This layout opts in to the Hopper hybrid-serialization
            /// tail (audit innovation I5). Fixed body remains zero-copy;
            /// tail access is explicit via the `tail_*` helpers below.
            pub const HAS_DYNAMIC_TAIL: bool = true;

            /// Byte offset of the tail's `u32 LE` length prefix. The
            /// payload starts at `TAIL_PREFIX_OFFSET + 4`. Layouts
            /// without a dynamic tail do not emit this constant.
            pub const TAIL_PREFIX_OFFSET: usize = Self::LEN;

            /// Read the tail's length prefix.
            #[inline]
            pub fn tail_len(data: &[u8]) -> ::core::result::Result<
                u32,
                ::hopper::__runtime::ProgramError,
            > {
                ::hopper::__runtime::read_tail_len(data, Self::TAIL_PREFIX_OFFSET)
            }

            /// Decode and return the dynamic tail as `#tail_ty`.
            ///
            /// The full encoded length (from the u32 prefix) must be
            /// consumed exactly by `#tail_ty::decode`. trailing bytes
            /// indicate a malformed encoding and are rejected.
            #[inline]
            pub fn tail_read(data: &[u8]) -> ::core::result::Result<
                #tail_ty,
                ::hopper::__runtime::ProgramError,
            > {
                ::hopper::__runtime::read_tail::<#tail_ty>(data, Self::TAIL_PREFIX_OFFSET)
            }

            /// Encode `tail` in place and update the u32 length prefix.
            /// Returns the number of bytes written (excluding the prefix).
            /// Caller is responsible for ensuring the account has enough
            /// room (call `resize` first when growing).
            #[inline]
            pub fn tail_write(
                data: &mut [u8],
                tail: &#tail_ty,
            ) -> ::core::result::Result<
                usize,
                ::hopper::__runtime::ProgramError,
            > {
                ::hopper::__runtime::write_tail::<#tail_ty>(
                    data,
                    Self::TAIL_PREFIX_OFFSET,
                    tail,
                )
            }
        }
    } else {
        quote! {
            /// This layout has no dynamic tail. The constant is emitted
            /// unconditionally so callers can branch on it at compile time.
            pub const HAS_DYNAMIC_TAIL: bool = false;
        }
    };

    let expanded = quote! {
        #input

        // ── Compile-time safety fence ──────────────────────────────────
        // Mirrors Quasar's alignment/padding/zero-discriminator asserts
        // plus Hopper's own size invariant. All four checks fire at
        // type-check time, so malformed layouts never reach link time.
        const _: () = {
            assert!(
                core::mem::align_of::<#name>() == 1,
                "hopper_state layouts must use alignment-1 field types such as WireU64 or TypedAddress",
            );
            assert!(
                core::mem::size_of::<#name>() == ((#body_size) as usize),
                "hopper_state layouts must be #[repr(C)] with no implicit padding",
            );
            assert!(
                core::mem::size_of::<#name>() > 0,
                "hopper_state layouts must have at least one field; zero-sized overlays project to dangling pointers",
            );
            assert!(
                #disc != 0,
                "hopper_state discriminator must be non-zero: a zero discriminator cannot be distinguished from an uninitialized account",
            );
        };

        #(#module_items)*

        impl #name {
            #(#inherent_items)*

            pub const BODY_SIZE: usize = core::mem::size_of::<Self>();
            pub const LEN: usize = ::hopper::hopper_core::account::HEADER_LEN + Self::BODY_SIZE;
            pub const DISC: u8 = #disc;
            pub const VERSION: u8 = #version;
            pub const LAYOUT_ID: [u8; 8] = #layout_id_tokens;

            #[inline(always)]
            pub fn overlay(
                data: &[u8],
            ) -> ::core::result::Result<&Self, ::hopper::__runtime::ProgramError> {
                ::hopper::hopper_core::account::pod_from_bytes::<Self>(data)
            }

            #[inline(always)]
            pub fn overlay_mut(
                data: &mut [u8],
            ) -> ::core::result::Result<&mut Self, ::hopper::__runtime::ProgramError> {
                ::hopper::hopper_core::account::pod_from_bytes_mut::<Self>(data)
            }

            #[inline(always)]
            pub fn load<'a>(
                account: &'a ::hopper::prelude::AccountView,
                program_id: &::hopper::prelude::Address,
            ) -> ::core::result::Result<
                ::hopper::__runtime::Ref<'a, Self>,
                ::hopper::__runtime::ProgramError,
            > {
                account.check_owned_by(program_id)?;
                account.load::<Self>()
            }

            #[inline(always)]
            pub fn load_mut<'a>(
                account: &'a ::hopper::prelude::AccountView,
                program_id: &::hopper::prelude::Address,
            ) -> ::core::result::Result<
                ::hopper::__runtime::RefMut<'a, Self>,
                ::hopper::__runtime::ProgramError,
            > {
                account.check_owned_by(program_id)?.check_writable()?;
                account.load_mut::<Self>()
            }

            #[inline(always)]
            #[deprecated(since = "0.2.0", note = "renamed to load_cross_program()")]
            pub fn load_foreign<'a>(
                account: &'a ::hopper::prelude::AccountView,
                expected_owner: &::hopper::prelude::Address,
            ) -> ::core::result::Result<
                ::hopper::__runtime::Ref<'a, Self>,
                ::hopper::__runtime::ProgramError,
            > {
                Self::load_cross_program(account, expected_owner)
            }

            #[inline(always)]
            pub fn load_cross_program<'a>(
                account: &'a ::hopper::prelude::AccountView,
                expected_owner: &::hopper::prelude::Address,
            ) -> ::core::result::Result<
                ::hopper::__runtime::Ref<'a, Self>,
                ::hopper::__runtime::ProgramError,
            > {
                account.check_owned_by(expected_owner)?;
                account.load::<Self>()
            }

            #dynamic_tail_methods
        }

        // Bytemuck-backed field-level Pod proof (Hopper Safety Audit
        // Must-Fix #4 / #5).
        //
        // The three `unsafe impl`s alone would be rubber stamps. the
        // compiler does not inspect fields when a bare `unsafe impl
        // bytemuck::Pod for T` is emitted. We therefore pair them with
        // a per-field `__FieldPodProof<T: bytemuck::Pod + Zeroable>`
        // instantiation. Each field type is forced through that bound;
        // a `bool`, `char`, reference, or non-`bytemuck::Pod` nested
        // struct fails the trait bound *on the field*, not at some
        // distant `segment_ref::<T>()` call site.
        #[doc(hidden)]
        const _: () = {
            struct __FieldPodProof<
                T: ::hopper::__runtime::__hopper_native::bytemuck::Pod
                    + ::hopper::__runtime::__hopper_native::bytemuck::Zeroable,
            >(::core::marker::PhantomData<T>);
            #(
                #[allow(dead_code)]
                const _: __FieldPodProof<#field_types> =
                    __FieldPodProof(::core::marker::PhantomData);
            )*
        };

        unsafe impl ::hopper::__runtime::__hopper_native::bytemuck::Zeroable for #name {}
        unsafe impl ::hopper::__runtime::__hopper_native::bytemuck::Pod for #name {}
        unsafe impl ::hopper::hopper_core::account::Pod for #name {}

        impl ::hopper::hopper_core::account::FixedLayout for #name {
            const SIZE: usize = core::mem::size_of::<Self>();
        }

        impl ::hopper::hopper_core::segment_map::SegmentMap for #name {
            const SEGMENTS: &'static [::hopper::hopper_core::segment_map::StaticSegment] = &[
                #(#segment_entries),*
            ];
        }

        impl ::hopper::hopper_core::field_map::FieldMap for #name {
            const FIELDS: &'static [::hopper::hopper_core::field_map::FieldInfo] = {
                const FIELD_COUNT: usize = #field_count;
                const NAMES: [&str; FIELD_COUNT] = [#(#field_name_literals),*];
                const SIZES: [usize; FIELD_COUNT] = [#(core::mem::size_of::<#field_types>()),*];
                const FIELDS: [::hopper::hopper_core::field_map::FieldInfo; FIELD_COUNT] = {
                    let mut result = [::hopper::hopper_core::field_map::FieldInfo::new("", 0, 0); FIELD_COUNT];
                    let mut offset = ::hopper::hopper_core::account::HEADER_LEN;
                    let mut index = 0;
                    while index < FIELD_COUNT {
                        result[index] = ::hopper::hopper_core::field_map::FieldInfo::new(
                            NAMES[index],
                            offset,
                            SIZES[index],
                        );
                        offset += SIZES[index];
                        index += 1;
                    }
                    result
                };
                &FIELDS
            };
        }

        impl ::hopper::hopper_runtime::LayoutContract for #name {
            const DISC: u8 = #name::DISC;
            const VERSION: u8 = #name::VERSION;
            const LAYOUT_ID: [u8; 8] = #name::LAYOUT_ID;
            const SIZE: usize = #name::LEN;
        }

        impl ::hopper::hopper_schema::SchemaExport for #name {
            fn layout_manifest() -> ::hopper::hopper_schema::LayoutManifest {
                const FIELD_COUNT: usize = #field_count;
                const NAMES: [&str; FIELD_COUNT] = [#(#field_name_literals),*];
                const TYPES: [&str; FIELD_COUNT] = [#(#field_type_literals),*];
                const SIZES: [u16; FIELD_COUNT] = [#(core::mem::size_of::<#field_types>() as u16),*];
                const FIELDS: [::hopper::hopper_schema::FieldDescriptor; FIELD_COUNT] = {
                    let mut result = [::hopper::hopper_schema::FieldDescriptor {
                        name: "",
                        canonical_type: "",
                        size: 0,
                        offset: 0,
                        intent: ::hopper::hopper_schema::FieldIntent::Custom,
                    }; FIELD_COUNT];
                    let mut offset = ::hopper::hopper_core::account::HEADER_LEN as u16;
                    let mut index = 0;
                    while index < FIELD_COUNT {
                        result[index] = ::hopper::hopper_schema::FieldDescriptor {
                            name: NAMES[index],
                            canonical_type: TYPES[index],
                            size: SIZES[index],
                            offset,
                            intent: ::hopper::hopper_schema::FieldIntent::Custom,
                        };
                        offset += SIZES[index];
                        index += 1;
                    }
                    result
                };

                ::hopper::hopper_schema::LayoutManifest {
                    name: stringify!(#name),
                    version: #name::VERSION,
                    disc: #name::DISC,
                    layout_id: #name::LAYOUT_ID,
                    total_size: #name::LEN,
                    field_count: FIELD_COUNT,
                    fields: &FIELDS,
                }
            }
        }
    };

    Ok(expanded)
}

fn parse_state_options(attr: TokenStream) -> Result<StateOptions> {
    if attr.is_empty() {
        return Ok(StateOptions::default());
    }

    let mut options = StateOptions::default();
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("disc") {
            let value: LitInt = meta.value()?.parse()?;
            options.disc = Some(value.base10_parse()?);
            return Ok(());
        }
        if meta.path.is_ident("version") {
            let value: LitInt = meta.value()?.parse()?;
            options.version = value.base10_parse()?;
            return Ok(());
        }
        if meta.path.is_ident("dynamic_tail") {
            let ty: syn::Type = meta.value()?.parse()?;
            options.dynamic_tail = Some(ty);
            return Ok(());
        }
        Err(meta.error("unsupported hopper_state option; expected `disc = N`, `version = N`, or `dynamic_tail = T`"))
    });

    parser.parse2(attr)?;
    Ok(options)
}

fn has_repr_c(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("repr") {
            return false;
        }

        let mut has_c = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("C") {
                has_c = true;
            }
            Ok(())
        });
        has_c
    })
}

/// Build the 8-byte wire fingerprint for a layout.
///
/// The Hopper Safety Audit flagged the pre-fix algorithm. hashing
/// raw Rust token strings. as source-spelling-dependent and therefore
/// unsuitable as a long-term ABI identity primitive. A rename of
/// `foo::bar::WireU64` to `crate::WireU64`, or a swap of the generic
/// parameter on `TypedAddress<Authority>` → `TypedAddress<Token>`,
/// changed the fingerprint even though the wire layout was identical.
///
/// The audit-compliant algorithm this function implements normalizes
/// each field's type to a **canonical wire stem**:
///
/// - `Type::Path`. the last `::`-separated path segment only
///   (`foo::bar::WireU64` → `WireU64`), with generic parameters
///   stripped (`TypedAddress<Authority>` → `TypedAddress`). This
///   makes path re-exports and phantom-only generic changes ABI-
///   invisible, which they should be.
/// - `Type::Array`. `arr_<elem_stem>_<len>` so `[u8; 32]` becomes
///   `arr_u8_32`. stable against spelling variants.
/// - Anything else. fall back to the normalized token string so a
///   non-path type still produces a deterministic value.
///
/// The canonical descriptor format then feeds into SHA-256:
///
/// ```text
/// hopper:wire:v2|S:<StructName>|V:<Version>|
///   f0:<field_name>:<wire_stem>|f1:...|...
/// ```
///
/// `hopper:wire:v2` is the fingerprint-algorithm version marker. If
/// the descriptor format itself ever changes, bump this tag and the
/// old fingerprints stay distinguishable.
fn layout_id_bytes(
    name: &syn::Ident,
    version: u8,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> [u8; 8] {
    let mut input = format!("hopper:wire:v2|S:{}|V:{}", name, version);
    for (idx, field) in fields.iter().enumerate() {
        let field_name = field.ident.as_ref().expect("named fields only");
        let stem = canonical_wire_stem(&field.ty);
        input.push_str(&format!("|f{}:{}:{}", idx, field_name, stem));
    }

    let digest = Sha256::digest(input.as_bytes());
    let mut layout_id = [0u8; 8];
    layout_id.copy_from_slice(&digest[..8]);
    layout_id
}

/// Normalize a Rust type token to a canonical wire stem.
///
/// See [`layout_id_bytes`] for the full contract. The normalization
/// is deliberately lossy on purely-cosmetic differences (paths,
/// phantom generics) and lossless on anything that actually shifts
/// the wire shape.
fn canonical_wire_stem(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(type_path) => {
            if let Some(last) = type_path.path.segments.last() {
                // `TypedAddress<Authority>` → `TypedAddress`.
                // Phantom-only generic parameters shouldn't be part
                // of the wire ABI fingerprint because they don't
                // affect the byte layout.
                last.ident.to_string()
            } else {
                "unknown_path".to_string()
            }
        }
        syn::Type::Array(arr) => {
            let elem = canonical_wire_stem(&arr.elem);
            // `[u8; 32]` → `arr_u8_32`. Normalize so `[u8; 32]`,
            // `[u8; 32usize]`, and `[u8 ; 32]` all hash the same:
            // strip whitespace, then strip any integer-type suffix
            // (`u8`, `u16`, …, `usize`, `i8`, …) that a literal may
            // carry. We only touch the trailing non-digit tail so an
            // expression-shaped length like `N + 1` stays intact.
            let raw = arr
                .len
                .to_token_stream()
                .to_string()
                .replace(char::is_whitespace, "");
            let canonical_len = strip_int_literal_suffix(&raw);
            format!("arr_{}_{}", elem, canonical_len)
        }
        syn::Type::Tuple(tup) if tup.elems.is_empty() => "unit".to_string(),
        // Fallback: deterministic whitespace-stripped token string.
        // Not ideal, but covers exotic types (references, slices,
        // bare function pointers) without producing a collision.
        other => other
            .to_token_stream()
            .to_string()
            .replace(char::is_whitespace, ""),
    }
}

fn byte_array_literal(bytes: &[u8; 8]) -> TokenStream {
    let items = bytes.iter();
    quote! { [#(#items),*] }
}

/// Strip a Rust integer literal's type suffix if the literal is a
/// pure digit run, e.g. `"32usize"` → `"32"`, `"255u8"` → `"255"`,
/// `"0x10u32"` → `"0x10"`. Expression-shaped strings like `"N + 1"`
/// or `"SIZE"` are returned unchanged.
fn strip_int_literal_suffix(raw: &str) -> String {
    // Only strip if the prefix is a plausible integer literal: starts
    // with a digit, and the trailing non-digit tail is one of the
    // known integer-type suffixes.
    if !raw.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return raw.to_string();
    }
    const SUFFIXES: &[&str] = &[
        "usize", "isize", "u128", "i128", "u64", "i64", "u32", "i32", "u16", "i16", "u8", "i8",
    ];
    for suffix in SUFFIXES {
        if let Some(stripped) = raw.strip_suffix(suffix) {
            // Guard: the character immediately before the suffix must
            // be a digit (or `_` which Rust allows inside literals),
            // otherwise the suffix is actually the type identifier for
            // something like `N_u32` (an identifier).
            let before_ok = stripped
                .chars()
                .last()
                .map_or(false, |c| c.is_ascii_digit() || c == '_');
            if before_ok {
                return stripped.to_string();
            }
        }
    }
    raw.to_string()
}

// ══════════════════════════════════════════════════════════════════════
//  Canonical wire fingerprint. regression tests
// ══════════════════════════════════════════════════════════════════════
//
// These tests lock in the audit's "no source-spelling drift" invariant:
// path imports, full-qualification, and phantom-only generic parameters
// MUST produce the same wire fingerprint, because none of them change
// the byte layout of the serialized account.

#[cfg(test)]
mod fingerprint_tests {
    use super::*;
    use syn::parse_quote;

    fn fp(ty: syn::Type) -> String {
        canonical_wire_stem(&ty)
    }

    #[test]
    fn path_spelling_drift_does_not_change_stem() {
        assert_eq!(fp(parse_quote!(WireU64)), fp(parse_quote!(crate::WireU64)));
        assert_eq!(
            fp(parse_quote!(WireU64)),
            fp(parse_quote!(hopper_native::wire::WireU64)),
        );
        assert_eq!(fp(parse_quote!(WireU64)), fp(parse_quote!(self::WireU64)));
    }

    #[test]
    fn phantom_generic_parameters_are_stripped() {
        // `TypedAddress<Authority>` and `TypedAddress<Token>` have
        // identical byte layout; they must hash the same under the
        // post-audit canonical algorithm.
        assert_eq!(
            fp(parse_quote!(TypedAddress<Authority>)),
            fp(parse_quote!(TypedAddress<Token>)),
        );
        assert_eq!(
            fp(parse_quote!(TypedAddress<Authority>)),
            fp(parse_quote!(TypedAddress)),
        );
    }

    #[test]
    fn arrays_normalize_whitespace_and_usize_suffix() {
        assert_eq!(fp(parse_quote!([u8; 32])), fp(parse_quote!([u8 ; 32])));
        assert_eq!(fp(parse_quote!([u8; 32])), fp(parse_quote!([u8; 32usize])));
    }

    #[test]
    fn different_wire_types_still_distinguishable() {
        // Two genuinely different wire types must produce different
        // stems, otherwise we've over-normalized and lost ABI safety.
        assert_ne!(fp(parse_quote!(WireU64)), fp(parse_quote!(WireU32)));
        assert_ne!(fp(parse_quote!([u8; 32])), fp(parse_quote!([u8; 64])));
        assert_ne!(fp(parse_quote!(u8)), fp(parse_quote!(u16)));
    }

    #[test]
    fn unit_and_tuple_roll_up_cleanly() {
        assert_eq!(fp(parse_quote!(())), "unit");
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
