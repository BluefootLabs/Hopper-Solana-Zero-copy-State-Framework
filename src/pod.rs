//! `#[hopper::pod]`. derive the Hopper zero-copy marker contract.
//!
//! The Hopper Safety Audit asked for a standalone attribute that any
//! user-defined struct can opt into to pick up the full Pod +
//! FixedLayout + alignment-1 contract without also pulling in the full
//! `#[hopper::state]` machinery (header, layout_id, schema hooks). This
//! is that attribute.
//!
//! ## What it emits
//!
//! For an input struct:
//!
//! ```ignore
//! #[hopper::pod]
//! #[repr(C)]
//! pub struct SmallHeader {
//!     pub version: u8,
//!     pub flags: [u8; 3],
//!     pub counter: WireU64,
//! }
//! ```
//!
//! The macro emits:
//! - `unsafe impl ::hopper::__runtime::Pod for SmallHeader {}`. the
//!   canonical runtime Pod impl that unlocks every `segment_ref`,
//!   `segment_mut`, `raw_ref`, `raw_mut`, `read_data` API.
//! - `impl ::hopper::hopper_core::account::FixedLayout for SmallHeader
//!   { const SIZE: usize = size_of::<Self>(); }`. for any downstream
//!   code that needs `T::SIZE` without duplicating the integer literal.
//! - A trio of `const _: () = assert!(...)` guards:
//!     - `align_of::<T>() == 1`. catches `#[repr(C)]` with padded
//!       fields at compile time.
//!     - `size_of::<T>() == <sum of field sizes>`. catches implicit
//!       compiler-added padding between fields.
//!     - `size_of::<T>() > 0`. zero-sized overlays project to dangling
//!       pointers; audit-aligned we forbid them.
//!
//! Nothing else. If you want the Hopper 16-byte header, segment map,
//! schema export, or loaded-from-account_view helpers, reach for
//! `#[hopper::state]` instead.

use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse2, Attribute, Fields, ItemStruct, Result};

pub fn expand(_attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let input: ItemStruct = parse2(item)?;
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    if !has_repr_c_or_transparent(&input.attrs) {
        return Err(syn::Error::new_spanned(
            &input,
            "#[hopper::pod] requires #[repr(C)] or #[repr(transparent)] so the \
             zero-copy overlay has a stable layout",
        ));
    }

    // Sum every field's size so the compile-time assertion fires if the
    // struct has hidden padding. e.g. a `#[repr(C)]` with a `u32`
    // followed by a `u64` on a target where the runtime would insert
    // align-4 padding.
    let field_types: Vec<_> = match &input.fields {
        Fields::Named(f) => f.named.iter().map(|f| f.ty.clone()).collect(),
        Fields::Unnamed(f) => f.unnamed.iter().map(|f| f.ty.clone()).collect(),
        Fields::Unit => Vec::new(),
    };

    let sum_sizes = if field_types.is_empty() {
        quote! { 0usize }
    } else {
        let pieces = field_types.iter().map(|ty| {
            quote! { ::core::mem::size_of::<#ty>() }
        });
        quote! { #(#pieces)+* }
    };

    let struct_name_str = name.to_string();
    let size_msg = format!(
        "#[hopper::pod] struct `{}` has implicit padding; sum of field \
         sizes must equal size_of::<Self>(). Add explicit padding fields \
         or reorder for alignment-1 layout.",
        struct_name_str,
    );
    let align_msg = format!(
        "#[hopper::pod] struct `{}` must be alignment-1 (use Hopper wire \
         types such as WireU64, WireI32, or TypedAddress<T> instead of \
         raw u64/i32/Pubkey).",
        struct_name_str,
    );
    let nonzero_msg = format!(
        "#[hopper::pod] struct `{}` has zero size; zero-sized overlays \
         project to dangling pointers and are rejected.",
        struct_name_str,
    );

    // Forward the original item unchanged plus the derived impls.
    //
    // Two layers of safety proof fire at compile time:
    //
    // 1. A per-field `__FieldPodProof<T: bytemuck::Pod + bytemuck::Zeroable>`
    //    marker forces every field type to already satisfy bytemuck's
    //    all-bits-valid / no-pointers / no-padding contract. A `bool`,
    //    `char`, reference, or non-`bytemuck::Pod` struct field fails
    //    *this* bound, not some later use-site bound. so the compile
    //    error points at the field, not at a distant `segment_ref::<T>()`.
    //
    // 2. Rubber-stamp `unsafe impl bytemuck::{Pod, Zeroable} for #name`
    //    lifts those per-field proofs to the whole struct. They're
    //    `unsafe` because bytemuck's own marker contract is `unsafe`;
    //    the field-level proofs above are the evidence that satisfies
    //    the safety obligation.
    let expanded = quote! {
        #input

        // Field-level proof: every field must itself implement both
        // `bytemuck::Pod` and `bytemuck::Zeroable`. This closes the
        // Hopper Safety Audit Must-Fix #5 / #4 gap. rubber-stamp
        // `unsafe impl` alone cannot catch `bool` / `char` /
        // reference / padded nested fields. The `__FieldPodProof`
        // marker instantiation forces a trait-bound check per field.
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

        // Rubber-stamp bytemuck impls so `#[hopper::pod]` types
        // participate in bytemuck-gated APIs without a separate derive.
        // Safety: upheld by the per-field proof above, the `#[repr(C)]`
        // / `#[repr(transparent)]` check, and the alignment + padding
        // asserts below.
        unsafe impl #impl_generics ::hopper::__runtime::__hopper_native::bytemuck::Zeroable
            for #name #ty_generics #where_clause {}
        unsafe impl #impl_generics ::hopper::__runtime::__hopper_native::bytemuck::Pod
            for #name #ty_generics #where_clause {}

        // Hopper runtime Pod marker + FixedLayout.
        unsafe impl #impl_generics ::hopper::__runtime::Pod for #name #ty_generics #where_clause {}

        // Audit final-API Step 5 seal. `#[hopper::pod]` types stamp
        // themselves with the Hopper-authored marker so the
        // `ZeroCopy` blanket picks them up. Bare `unsafe impl Pod`
        // outside the macro path does not get this seal, so it also
        // does not automatically satisfy `ZeroCopy`.
        unsafe impl #impl_generics ::hopper::__runtime::__sealed::HopperZeroCopySealed
            for #name #ty_generics #where_clause {}

        impl #impl_generics ::hopper::hopper_core::account::FixedLayout
            for #name #ty_generics #where_clause
        {
            const SIZE: usize = ::core::mem::size_of::<Self>();
        }

        // Anchor-parity `INIT_SPACE` const. Pod structs are fixed
        // layout by construction, so the value is just `size_of`.
        // Exposed inherently (not through a trait) so call sites can
        // write `MyPod::INIT_SPACE` without importing a trait.
        impl #impl_generics #name #ty_generics #where_clause {
            /// Bytes a System Program allocation needs to hold this
            /// layout. Matches Anchor's `#[derive(InitSpace)]` contract
            /// for fixed-size structs.
            pub const INIT_SPACE: usize = ::core::mem::size_of::<Self>();
        }

        const _: () = {
            assert!(
                ::core::mem::align_of::<#name #ty_generics>() == 1,
                #align_msg,
            );
            assert!(
                ::core::mem::size_of::<#name #ty_generics>() == (#sum_sizes),
                #size_msg,
            );
            assert!(
                ::core::mem::size_of::<#name #ty_generics>() > 0,
                #nonzero_msg,
            );
        };
    };

    Ok(expanded)
}

fn has_repr_c_or_transparent(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("repr") {
            return false;
        }
        let tokens = attr.meta.to_token_stream().to_string();
        tokens.contains("C") || tokens.contains("transparent")
    })
}
