//! `#[hopper_state]`. contract-aware zero-copy layout codegen.
//!
//! The canonical proc-macro path must participate in the same runtime,
//! schema, and receipt pipeline as hand-written Hopper layouts. This macro
//! therefore emits more than a `SegmentMap`: it generates field metadata,
//! layout fingerprints, typed load helpers, and schema export hooks.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use sha2::{Digest, Sha256};
use syn::{parse::Parser, parse2, Attribute, Fields, Field, ItemStruct, LitInt, LitStr, Result};

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

/// Per-field metadata extracted from Hopper-specific attributes.
///
/// These attributes (`#[role = "..."]` and `#[invariant = "..."]`) are
/// consumed by `#[hopper::state]` itself. they are stripped from the
/// re-emitted struct so the compiler never sees them. This is how the
/// layout ties individual fields to schema intents and to the named
/// invariants declared on an associated `#[hopper::error]` enum.
///
/// ## Innovation over Anchor / Quasar / Pinocchio
///
/// Anchor has no field-level intent. Quasar's field macros only touch
/// offsets. Pinocchio deliberately stays out of the schema business.
/// Hopper is the first of the four to give authored layouts a way to
/// say "this field is a balance" or "this field is guarded by
/// `balance_nonzero`" at declaration time. information that then
/// flows directly into the manifest, the receipt narrative, and the
/// Codama/Python client generators.
#[derive(Default, Clone)]
struct FieldMeta {
    /// Semantic role string (e.g. `"balance"`, `"authority"`). Empty
    /// string means no role was declared. the field falls back to
    /// `FieldIntent::Custom`.
    role: String,
    /// Invariant name this field is guarded by (e.g. `"balance_nonzero"`).
    /// Empty when the field has no declared invariant.
    invariant: String,
}

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let options = parse_state_options(attr)?;
    let dynamic_tail = options.dynamic_tail.clone();
    let mut input: ItemStruct = parse2(item)?;
    let name = input.ident.clone();
    let vis = input.vis.clone();

    if !has_repr_c(&input.attrs) {
        return Err(syn::Error::new_spanned(
            &input,
            "hopper_state requires #[repr(C)] so segment offsets and typed loads stay stable",
        ));
    }

    // Verify we have named fields before we start mutating. We can't
    // iterate `&input.fields` here because we'll need `&mut` access
    // below to strip the hopper-internal attributes from the struct
    // we re-emit. Do a bare shape-check first; actual iteration
    // happens against `input.fields` directly.
    if !matches!(input.fields, Fields::Named(_)) {
        return Err(syn::Error::new_spanned(
            &input,
            "hopper_state requires a struct with named fields",
        ));
    }

    let mut segment_entries = Vec::new();
    let mut module_items = Vec::new();
    let mut inherent_items = Vec::new();
    let mut field_name_literals = Vec::new();
    let mut field_type_literals = Vec::new();
    let mut field_types = Vec::new();
    let mut field_intent_tokens: Vec<TokenStream> = Vec::new();
    let mut field_role_literals: Vec<LitStr> = Vec::new();
    let mut field_invariant_literals: Vec<LitStr> = Vec::new();
    let mut running_offset = quote! { 0u32 };

    let struct_name_upper = to_screaming_snake(&name.to_string());

    // First pass: extract hopper-internal attributes from each field
    // and strip them from the re-emitted struct. The consumed attrs
    // (`#[role = "..."]`, `#[invariant = "..."]`) are bare identifiers
    // without a `::` path prefix, so leaving them in place would cause
    // rustc to reject the re-emitted struct with "unknown attribute".
    let field_metas: Vec<FieldMeta> = match &mut input.fields {
        Fields::Named(named) => {
            let mut out = Vec::with_capacity(named.named.len());
            for field in named.named.iter_mut() {
                let meta = parse_field_meta(field)?;
                strip_hopper_field_attrs(field);
                out.push(meta);
            }
            out
        }
        _ => unreachable!("checked above"),
    };

    // Borrow the (now-cleaned) named fields for the main codegen walk.
    let fields = match &input.fields {
        Fields::Named(f) => &f.named,
        _ => unreachable!("checked above"),
    };

    for (field, meta) in fields.iter().zip(field_metas.iter()) {
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
        field_intent_tokens.push(role_to_intent_tokens(&meta.role, field_name.span())?);
        field_role_literals.push(LitStr::new(&meta.role, field_name.span()));
        field_invariant_literals.push(LitStr::new(&meta.invariant, field_name.span()));

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
    let layout_id = layout_id_bytes(&name, version, fields);
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

    // Unique per-layout static that pins LAYOUT_ID bytes into
    // `.rodata`. `hopper verify` searches the compiled binary for
    // this exact 8-byte sequence to prove manifest/binary agreement.
    let layout_id_anchor_ident = format_ident!(
        "__HOPPER_LAYOUT_ID_ANCHOR_{}",
        struct_name_upper
    );

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

            /// Bytes to allocate for a fresh account of this layout.
            ///
            /// Equal to [`Self::LEN`] (header + body) and spelled
            /// `INIT_SPACE` so Anchor-style `#[account(init, space = T::INIT_SPACE)]`
            /// ports over unchanged. Zero-copy layouts are always
            /// fixed-size, so this is a const the compiler can fold
            /// into the System Program allocate CPI at the call site.
            pub const INIT_SPACE: usize = Self::LEN;

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

            // ── Field-level metadata registries ──────────────────────────
            //
            // These two tables are the structural twin of the `CODE_TABLE`
            // / `INVARIANT_TABLE` pair produced by `#[hopper::error]`.
            // Together they let off-chain tooling (SDK narrator, Codama
            // client generator, IDL exporter) answer three questions at
            // declaration time, without ever running the program:
            //
            //   1. "What does field `X` mean?". `FIELD_ROLES`
            //   2. "Which invariant guards field `X`?". `FIELD_INVARIANTS`
            //   3. "Which FieldIntent enum value does the runtime use?". //      the manifest's FieldDescriptor.intent column.
            //
            // No other Solana framework ships this. Anchor has no field
            // intent at all. Quasar tracks offsets but not semantics.
            // Pinocchio deliberately stays schema-free. Hopper's edge is
            // that the *same source declaration* flows into the manifest,
            // the Python client, the receipt narrative, and any future
            // lint pass that wants to say "this field was declared as a
            // balance but a non-financial invariant is guarding it."

            /// Declared semantic role per field, in struct declaration
            /// order. Empty string means no `#[role = "..."]` was given
            /// and the field falls back to `FieldIntent::Custom`.
            pub const FIELD_ROLES: &'static [(&'static str, &'static str)] = &[
                #( (#field_name_literals, #field_role_literals) ),*
            ];

            /// Declared invariant name per field, in struct declaration
            /// order. Empty string means no `#[invariant = "..."]` was
            /// given. Pair this with
            /// `<ErrorEnum>::INVARIANT_TABLE` to resolve the invariant
            /// name back to the error variant it raises. completing
            /// the field → invariant → error → receipt chain.
            pub const FIELD_INVARIANTS: &'static [(&'static str, &'static str)] = &[
                #( (#field_name_literals, #field_invariant_literals) ),*
            ];
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
        // Audit final-API Step 5 seal. `#[hopper::state]` stamps
        // the Hopper-authored marker so the `ZeroCopy` blanket
        // picks up the type. Bare `unsafe impl Pod` outside this
        // macro path is not automatically `ZeroCopy`.
        unsafe impl ::hopper::__runtime::__sealed::HopperZeroCopySealed for #name {}

        // Anchor the layout fingerprint in `.rodata` so it survives
        // dead-code elimination on SBF / LTO builds. `hopper verify`
        // scans the compiled `.so` for this exact byte sequence to
        // prove the binary matches the emitted manifest. Without
        // `#[used]` the SBF linker aggressively strips const bytes
        // that only appear inside inlined comparisons.
        #[used]
        #[doc(hidden)]
        #[no_mangle]
        pub static #layout_id_anchor_ident: [u8; 8] = #name::LAYOUT_ID;

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
                // Per-field declared intents, parsed from `#[role = "..."]`
                // field attributes at proc-macro expansion time. Fields
                // without a role attribute default to `Custom`. their
                // position in this table still keeps the slice
                // `FIELD_COUNT`-sized so downstream code never needs a
                // branch on presence.
                const INTENTS: [::hopper::hopper_schema::FieldIntent; FIELD_COUNT] = [
                    #(#field_intent_tokens),*
                ];
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
                            intent: INTENTS[index],
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

/// Consume hopper-internal attributes (`#[role = "..."]`,
/// `#[invariant = "..."]`) from a single field and return the
/// extracted metadata. Attributes that fail to parse raise a
/// compile error pointing at the offending span.
///
/// Accepted forms:
///
/// ```ignore
/// #[role = "balance"]
/// #[role(balance)]          // path form, also accepted
/// #[invariant = "balance_nonzero"]
/// ```
fn parse_field_meta(field: &Field) -> Result<FieldMeta> {
    let mut meta = FieldMeta::default();
    for attr in &field.attrs {
        if attr.path().is_ident("role") {
            // Accept either `#[role = "balance"]` or `#[role(balance)]`.
            // The former uses `NameValue` meta; the latter uses `List`.
            // Both are equivalent at the attribute level. we just want
            // the string value.
            if let Ok(nv) = attr.meta.require_name_value() {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
                {
                    meta.role = s.value();
                    continue;
                }
                return Err(syn::Error::new_spanned(
                    &nv.value,
                    "#[role = \"...\"] expects a string literal (e.g. \"balance\", \"authority\")",
                ));
            }
            if let Ok(list) = attr.meta.require_list() {
                // Parse `#[role(balance)]` by pulling the single path ident.
                let tokens: TokenStream = list.tokens.clone();
                let parsed: syn::Ident = parse2(tokens).map_err(|_| {
                    syn::Error::new_spanned(
                        &list.tokens,
                        "#[role(...)] expects a single identifier (e.g. balance, authority)",
                    )
                })?;
                meta.role = parsed.to_string();
                continue;
            }
            return Err(syn::Error::new_spanned(
                attr,
                "unsupported #[role] form; use #[role = \"balance\"] or #[role(balance)]",
            ));
        }
        if attr.path().is_ident("invariant") {
            let nv = attr.meta.require_name_value().map_err(|_| {
                syn::Error::new_spanned(
                    attr,
                    "#[invariant] on a field expects name-value form: #[invariant = \"balance_nonzero\"]",
                )
            })?;
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            {
                meta.invariant = s.value();
                continue;
            }
            return Err(syn::Error::new_spanned(
                &nv.value,
                "#[invariant = \"...\"] expects a string literal (the invariant name)",
            ));
        }
    }
    Ok(meta)
}

/// Remove hopper-internal attributes from a field before the struct is
/// re-emitted. Leaving bare `#[role = "..."]` / `#[invariant = "..."]`
/// in place would cause rustc to reject the struct because those
/// attribute names are not registered with the compiler.
fn strip_hopper_field_attrs(field: &mut Field) {
    field
        .attrs
        .retain(|a| !a.path().is_ident("role") && !a.path().is_ident("invariant"));
}

/// Map a role string to a `FieldIntent` variant path token stream.
///
/// The role string is matched case-insensitively. An empty string
/// resolves to `FieldIntent::Custom` so fields without a declared
/// role still emit a well-formed descriptor. An unknown role raises
/// a compile error listing the full accepted vocabulary. this is
/// the cheapest place to catch typos (`"authorty"`, `"ballance"`)
/// that would otherwise silently degrade to `Custom`.
fn role_to_intent_tokens(role: &str, span: proc_macro2::Span) -> Result<TokenStream> {
    let normalized = role.to_ascii_lowercase();
    let variant = match normalized.as_str() {
        "" => "Custom",
        "balance" => "Balance",
        "authority" => "Authority",
        "timestamp" => "Timestamp",
        "counter" => "Counter",
        "index" => "Index",
        "basis_points" | "basispoints" | "bps" => "BasisPoints",
        "flag" | "bool" => "Flag",
        "address" | "pubkey" => "Address",
        "hash" | "fingerprint" => "Hash",
        "pda_seed" | "pdaseed" | "seed" => "PDASeed",
        "version" => "Version",
        "bump" => "Bump",
        "nonce" => "Nonce",
        "supply" | "total_supply" => "Supply",
        "limit" | "cap" | "ceiling" => "Limit",
        "threshold" => "Threshold",
        "owner" => "Owner",
        "delegate" => "Delegate",
        "status" | "state" | "lifecycle" => "Status",
        "custom" => "Custom",
        other => {
            return Err(syn::Error::new(
                span,
                format!(
                    "unknown #[role = \"{}\"]. accepted: balance, authority, timestamp, \
                     counter, index, basis_points, flag, address, hash, pda_seed, version, \
                     bump, nonce, supply, limit, threshold, owner, delegate, status, custom",
                    other
                ),
            ));
        }
    };
    let ident = syn::Ident::new(variant, span);
    Ok(quote! { ::hopper::hopper_schema::FieldIntent::#ident })
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

// ══════════════════════════════════════════════════════════════════════
//  Field-attribute parsing. regression tests (Task 18)
// ══════════════════════════════════════════════════════════════════════
//
// These tests lock in the per-field declarative surface introduced to
// close the "Anchor has no field intent" gap. Three properties matter:
//
//   1. Both `#[role = "..."]` and `#[role(...)]` work and agree.
//   2. Role strings map to the correct `FieldIntent` variant path, and
//      unknown roles produce a readable compile error rather than
//      silently degrading to `Custom`.
//   3. Stripping the hopper-internal attrs leaves the struct re-emit
//      valid. no `#[role]` leaks through to rustc.

#[cfg(test)]
mod field_attr_tests {
    use super::*;
    use syn::parse_quote;

    fn parse_first_field(f: syn::Field) -> FieldMeta {
        parse_field_meta(&f).expect("valid field meta")
    }

    #[test]
    fn name_value_role_parses() {
        let f: syn::Field = parse_quote!(
            #[role = "balance"]
            pub amount: WireU64
        );
        let m = parse_first_field(f);
        assert_eq!(m.role, "balance");
        assert_eq!(m.invariant, "");
    }

    #[test]
    fn path_form_role_parses() {
        let f: syn::Field = parse_quote!(
            #[role(authority)]
            pub owner: TypedAddress<Authority>
        );
        let m = parse_first_field(f);
        assert_eq!(m.role, "authority");
    }

    #[test]
    fn invariant_attr_parses() {
        let f: syn::Field = parse_quote!(
            #[invariant = "balance_nonzero"]
            pub amount: WireU64
        );
        let m = parse_first_field(f);
        assert_eq!(m.invariant, "balance_nonzero");
        assert_eq!(m.role, "");
    }

    #[test]
    fn both_attrs_parse_together() {
        let f: syn::Field = parse_quote!(
            #[role = "balance"]
            #[invariant = "balance_nonzero"]
            pub amount: WireU64
        );
        let m = parse_first_field(f);
        assert_eq!(m.role, "balance");
        assert_eq!(m.invariant, "balance_nonzero");
    }

    #[test]
    fn unknown_role_is_rejected() {
        // Typo in role name must produce a compile error, not a silent
        // downgrade to `Custom`. That's the whole point of making this
        // a closed vocabulary instead of a free-form string.
        let span = proc_macro2::Span::call_site();
        let err = role_to_intent_tokens("authorty", span).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown #[role = \"authorty\"]"),
            "expected helpful error, got: {msg}",
        );
        assert!(msg.contains("authority"), "error message should suggest valid vocabulary");
    }

    #[test]
    fn empty_role_maps_to_custom() {
        // Fields without `#[role = ...]` must still emit a well-formed
        // FieldIntent (Custom) rather than a compile error. The parallel
        // arrays inside `SchemaExport::layout_manifest` rely on every
        // field slot being populated.
        let span = proc_macro2::Span::call_site();
        let tokens = role_to_intent_tokens("", span).expect("empty role is valid");
        let rendered = tokens.to_string().replace(' ', "");
        assert!(
            rendered.ends_with("FieldIntent::Custom"),
            "empty role should render FieldIntent::Custom, got: {rendered}",
        );
    }

    #[test]
    fn role_mapping_covers_full_vocabulary() {
        // Every FieldIntent variant (except the `Custom` fallback) must
        // be reachable through at least one accepted role spelling.
        // If this list drifts from `FieldIntent`, the test fails loudly
        // rather than letting partial coverage rot silently.
        let expected = [
            ("balance", "Balance"),
            ("authority", "Authority"),
            ("timestamp", "Timestamp"),
            ("counter", "Counter"),
            ("index", "Index"),
            ("basis_points", "BasisPoints"),
            ("flag", "Flag"),
            ("address", "Address"),
            ("hash", "Hash"),
            ("pda_seed", "PDASeed"),
            ("version", "Version"),
            ("bump", "Bump"),
            ("nonce", "Nonce"),
            ("supply", "Supply"),
            ("limit", "Limit"),
            ("threshold", "Threshold"),
            ("owner", "Owner"),
            ("delegate", "Delegate"),
            ("status", "Status"),
        ];
        let span = proc_macro2::Span::call_site();
        for (role, variant) in expected {
            let tokens = role_to_intent_tokens(role, span).unwrap_or_else(|e| {
                panic!("role `{role}` should map to FieldIntent::{variant}: {e}")
            });
            let rendered = tokens.to_string().replace(' ', "");
            assert!(
                rendered.ends_with(&format!("FieldIntent::{variant}")),
                "role `{role}` should map to FieldIntent::{variant}, got: {rendered}",
            );
        }
    }

    #[test]
    fn case_variants_and_aliases_resolve() {
        let span = proc_macro2::Span::call_site();
        // Case insensitivity: users should not be punished for
        // `#[role = "Balance"]` vs `#[role = "balance"]`.
        let a = role_to_intent_tokens("Balance", span).unwrap().to_string();
        let b = role_to_intent_tokens("balance", span).unwrap().to_string();
        assert_eq!(a, b);
        // Aliases: `bps` ≡ `basis_points`, `seed` ≡ `pda_seed`, etc.
        let c = role_to_intent_tokens("bps", span).unwrap().to_string();
        let d = role_to_intent_tokens("basis_points", span).unwrap().to_string();
        assert_eq!(c, d);
        let e = role_to_intent_tokens("seed", span).unwrap().to_string();
        let f = role_to_intent_tokens("pda_seed", span).unwrap().to_string();
        assert_eq!(e, f);
    }

    #[test]
    fn strip_removes_hopper_attrs_but_preserves_others() {
        let mut f: syn::Field = parse_quote!(
            #[role = "balance"]
            #[invariant = "balance_nonzero"]
            #[serde(skip)]
            pub amount: WireU64
        );
        strip_hopper_field_attrs(&mut f);
        // `#[serde(skip)]` must survive. anything else the user
        // stacked alongside our attrs is not ours to consume.
        assert_eq!(f.attrs.len(), 1);
        assert!(f.attrs[0].path().is_ident("serde"));
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
