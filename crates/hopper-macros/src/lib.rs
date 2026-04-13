//! # Hopper Macros
//!
//! Declarative `macro_rules!` macros for Hopper. No proc macros required for
//! correctness — proc macros are available as optional DX accelerators for
//! schema generation, IDL export, and boilerplate reduction.
//!
//! ## Macros
//!
//! - `hopper_layout!`: zero-copy account layout with header, layout_id, tiered loading
//! - `hopper_check!`: composable account constraint checking
//! - `hopper_error!`: sequential error code generation
//! - `hopper_init!`: account creation + header initialization
//! - `hopper_close!`: safe account closure with sentinel
//! - `hopper_require!`: assert with specific error code
//! - `hopper_dispatch!`: instruction dispatch (re-exported from core)
//! - `hopper_segment!`: segmented account declaration
//! - `hopper_validate!`: inline validation pipeline from combinators
//! - `hopper_virtual!`: multi-account virtual state mapping
//! - `hopper_assert_compatible!`: compile-time layout version compatibility check
//! - `hopper_assert_fingerprint!`: compile-time fingerprint pinning
//! - `hopper_interface!`: cross-program read-only interface view

#![no_std]

/// Define a zero-copy account layout.
///
/// Generates a `#[repr(C)]` struct with:
/// - 16-byte Hopper header
/// - Alignment-1 fields
/// - Deterministic `LAYOUT_ID` via SHA-256
/// - Tiered loading: `load`, `load_mut`, `load_cross_program`, `load_compatible`, `load_unverified`
/// - Compile-time size and alignment assertions
///
/// # Example
///
/// ```ignore
/// hopper_layout! {
///     pub struct Vault, disc = 1, version = 1 {
///         authority: [u8; 32]  = 32,
///         mint:      [u8; 32]  = 32,
///         balance:   WireU64   = 8,
///         bump:      u8        = 1,
///     }
/// }
/// ```
#[macro_export]
macro_rules! hopper_layout {
    (
        $(#[$attr:meta])*
        pub struct $name:ident, disc = $disc:literal, version = $ver:literal
        {
            $( $field:ident : $fty:ty = $fsize:literal ),+ $(,)?
        }
    ) => {
        $(#[$attr])*
        #[derive(Clone, Copy)]
        #[repr(C)]
        pub struct $name {
            pub header: $crate::hopper_core::account::AccountHeader,
            $( pub $field: $fty, )+
        }

        // Compile-time assertions
        const _: () = {
            // Size check: header + sum of field sizes
            let expected = $crate::hopper_core::account::HEADER_LEN $( + $fsize )+;
            assert!(
                core::mem::size_of::<$name>() == expected,
                "Layout size mismatch: struct size != declared field sizes + header"
            );
            // Alignment-1 check
            assert!(
                core::mem::align_of::<$name>() == 1,
                "Layout alignment must be 1 for zero-copy safety"
            );
        };

        // SAFETY: #[repr(C)] over alignment-1 fields, all bit patterns valid
        // for the constituent Pod types (header, wire integers, byte arrays).
        unsafe impl $crate::hopper_core::account::Pod for $name {}

        impl $crate::hopper_core::account::FixedLayout for $name {
            const SIZE: usize = $crate::hopper_core::account::HEADER_LEN $( + $fsize )+;
        }

        impl $crate::hopper_core::field_map::FieldMap for $name {
            const FIELDS: &'static [$crate::hopper_core::field_map::FieldInfo] = {
                const FIELD_COUNT: usize = 0 $( + { let _ = stringify!($field); 1 } )+;
                const NAMES: [&str; FIELD_COUNT] = [ $( stringify!($field) ),+ ];
                const SIZES: [usize; FIELD_COUNT] = [ $( $fsize ),+ ];
                const FIELDS: [$crate::hopper_core::field_map::FieldInfo; FIELD_COUNT] = {
                    let mut result = [$crate::hopper_core::field_map::FieldInfo::new("", 0, 0); FIELD_COUNT];
                    let mut offset = $crate::hopper_core::account::HEADER_LEN;
                    let mut index = 0;
                    while index < FIELD_COUNT {
                        result[index] = $crate::hopper_core::field_map::FieldInfo::new(
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

        impl $crate::hopper_runtime::LayoutContract for $name {
            const DISC: u8 = $disc;
            const VERSION: u8 = $ver;
            const LAYOUT_ID: [u8; 8] = $name::LAYOUT_ID;
            const SIZE: usize = $name::LEN;
            const TYPE_OFFSET: usize = 0;
        }

        impl $crate::hopper_schema::SchemaExport for $name {
            fn layout_manifest() -> $crate::hopper_schema::LayoutManifest {
                const FIELD_COUNT: usize = 0 $( + { let _ = stringify!($field); 1 } )+;
                const SIZES: [u16; FIELD_COUNT] = [ $( $fsize ),+ ];
                const NAMES: [&str; FIELD_COUNT] = [ $( stringify!($field) ),+ ];
                const TYPES: [&str; FIELD_COUNT] = [ $( stringify!($fty) ),+ ];
                const FIELDS: [$crate::hopper_schema::FieldDescriptor; FIELD_COUNT] = {
                    let mut result = [$crate::hopper_schema::FieldDescriptor {
                        name: "", canonical_type: "", size: 0, offset: 0,
                        intent: $crate::hopper_schema::FieldIntent::Custom,
                    }; FIELD_COUNT];
                    let mut offset = $crate::hopper_core::account::HEADER_LEN as u16;
                    let mut index = 0;
                    while index < FIELD_COUNT {
                        result[index] = $crate::hopper_schema::FieldDescriptor {
                            name: NAMES[index],
                            canonical_type: TYPES[index],
                            size: SIZES[index],
                            offset,
                            intent: $crate::hopper_schema::FieldIntent::Custom,
                        };
                        offset += SIZES[index];
                        index += 1;
                    }
                    result
                };
                $crate::hopper_schema::LayoutManifest {
                    name: stringify!($name),
                    version: <$name>::VERSION,
                    disc: <$name>::DISC,
                    layout_id: <$name>::LAYOUT_ID,
                    total_size: <$name>::LEN,
                    field_count: FIELD_COUNT,
                    fields: &FIELDS,
                }
            }
        }

        impl $name {
            /// Total byte size of this layout.
            pub const LEN: usize = $crate::hopper_core::account::HEADER_LEN $( + $fsize )+;

            /// Discriminator tag.
            pub const DISC: u8 = $disc;

            /// Layout version.
            pub const VERSION: u8 = $ver;

            /// Deterministic layout fingerprint.
            ///
            /// SHA-256 of: `"hopper:v1:Name:version:field:type:size,..."`
            /// First 8 bytes for efficient comparison.
            pub const LAYOUT_ID: [u8; 8] = {
                // Build the canonical hash input at compile time
                const INPUT: &str = concat!(
                    "hopper:v1:",
                    stringify!($name), ":",
                    stringify!($ver), ":",
                    $( stringify!($field), ":", stringify!($fty), ":", stringify!($fsize), ",", )+
                );
                const HASH: [u8; 32] = $crate::hopper_core::__sha256_const(INPUT.as_bytes());
                [
                    HASH[0], HASH[1], HASH[2], HASH[3],
                    HASH[4], HASH[5], HASH[6], HASH[7],
                ]
            };

            /// Zero-copy overlay (immutable).
            #[deprecated(since = "0.2.0", note = "use load() for Hopper layouts or raw_ref() for explicit bypass")]
            #[inline(always)]
            pub fn overlay(data: &[u8]) -> Result<&Self, $crate::hopper_runtime::error::ProgramError> {
                $crate::hopper_core::account::pod_from_bytes::<Self>(data)
            }

            /// Zero-copy overlay (mutable).
            #[deprecated(since = "0.2.0", note = "use load_mut() or raw_mut() instead")]
            #[inline(always)]
            pub fn overlay_mut(data: &mut [u8]) -> Result<&mut Self, $crate::hopper_runtime::error::ProgramError> {
                $crate::hopper_core::account::pod_from_bytes_mut::<Self>(data)
            }

            /// Tier 1: Full validation load (own program accounts).
            ///
            /// Validates: owner + discriminator + version + layout_id + exact size.
            #[inline]
            pub fn load<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                program_id: &$crate::hopper_runtime::Address,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccount<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                $crate::hopper_core::check::check_owner(account, program_id)?;
                let data = account.try_borrow()?;
                $crate::hopper_core::account::check_header(
                    &*data,
                    Self::DISC,
                    Self::VERSION,
                    &Self::LAYOUT_ID,
                )?;
                $crate::hopper_core::check::check_size(&*data, Self::LEN)?;
                $crate::hopper_core::account::VerifiedAccount::from_ref(data)
            }

            /// Tier 1m: Full validation load (mutable).
            #[inline]
            pub fn load_mut<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                program_id: &$crate::hopper_runtime::Address,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccountMut<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                $crate::hopper_core::check::check_owner(account, program_id)?;
                $crate::hopper_core::check::check_writable(account)?;
                let data = account.try_borrow_mut()?;
                $crate::hopper_core::account::check_header(
                    &*data,
                    Self::DISC,
                    Self::VERSION,
                    &Self::LAYOUT_ID,
                )?;
                $crate::hopper_core::check::check_size(&*data, Self::LEN)?;
                $crate::hopper_core::account::VerifiedAccountMut::from_ref_mut(data)
            }

            /// Tier 2: Foreign account load (cross-program reads).
            ///
            /// Validates: owner + layout_id + exact size (no disc/version check).
            ///
            /// **Deprecated:** Renamed to `load_cross_program()` for clarity.
            #[deprecated(since = "0.2.0", note = "renamed to load_cross_program()")]
            #[inline]
            pub fn load_foreign<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                expected_owner: &$crate::hopper_runtime::Address,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccount<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                $crate::hopper_core::check::check_owner(account, expected_owner)?;
                let data = account.try_borrow()?;
                let layout_id = $crate::hopper_core::account::read_layout_id(&*data)?;
                if layout_id != Self::LAYOUT_ID {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                $crate::hopper_core::check::check_size(&*data, Self::LEN)?;
                $crate::hopper_core::account::VerifiedAccount::from_ref(data)
            }

            /// Cross-program account load (reads accounts owned by other programs).
            ///
            /// Validates: owner + layout_id + exact size (no disc/version check).
            /// This is the successor to `load_foreign()` with a clearer name.
            #[inline]
            pub fn load_cross_program<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                expected_owner: &$crate::hopper_runtime::Address,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccount<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                $crate::hopper_core::check::check_owner(account, expected_owner)?;
                let data = account.try_borrow()?;
                let layout_id = $crate::hopper_core::account::read_layout_id(&*data)?;
                if layout_id != Self::LAYOUT_ID {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                $crate::hopper_core::check::check_size(&*data, Self::LEN)?;
                $crate::hopper_core::account::VerifiedAccount::from_ref(data)
            }

            /// Tier 3: Version-compatible load for migration scenarios.
            ///
            /// Validates: owner + discriminator + minimum version + minimum size.
            /// Does **not** check layout_id, so it accepts any version of this
            /// account type whose version byte is ≥ `min_version` and whose
            /// data is at least as large as this layout.
            ///
            /// Use this during migration rollouts when a single instruction
            /// must accept both old and new versions of an account.
            ///
            /// # Arguments
            /// * `min_version` — lowest acceptable version byte (header byte 1).
            ///   Pass `1` to accept V1+, `2` to accept V2+ only, etc.
            #[inline]
            pub fn load_compatible<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                program_id: &$crate::hopper_runtime::Address,
                min_version: u8,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccount<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                $crate::hopper_core::check::check_owner(account, program_id)?;
                let data = account.try_borrow()?;
                if data.len() < $crate::hopper_core::account::HEADER_LEN {
                    return Err($crate::hopper_runtime::error::ProgramError::AccountDataTooSmall);
                }
                // Check discriminator (same account type, any version).
                if data[0] != Self::DISC {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                // Check minimum version.
                if data[1] < min_version {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                // Minimum size (account may be larger if migrated to a newer version).
                if data.len() < Self::LEN {
                    return Err($crate::hopper_runtime::error::ProgramError::AccountDataTooSmall);
                }
                $crate::hopper_core::account::VerifiedAccount::from_ref(data)
            }

            /// Tier 3m: Version-compatible load (mutable).
            ///
            /// Same as [`load_compatible`] but returns a mutable overlay.
            #[inline]
            pub fn load_compatible_mut<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                program_id: &$crate::hopper_runtime::Address,
                min_version: u8,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccountMut<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                $crate::hopper_core::check::check_owner(account, program_id)?;
                $crate::hopper_core::check::check_writable(account)?;
                let data = account.try_borrow_mut()?;
                if data.len() < $crate::hopper_core::account::HEADER_LEN {
                    return Err($crate::hopper_runtime::error::ProgramError::AccountDataTooSmall);
                }
                if data[0] != Self::DISC {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                if data[1] < min_version {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                if data.len() < Self::LEN {
                    return Err($crate::hopper_runtime::error::ProgramError::AccountDataTooSmall);
                }
                $crate::hopper_core::account::VerifiedAccountMut::from_ref_mut(data)
            }

            /// Tier 4: Unchecked load (caller assumes all risk).
            ///
            /// # Safety
            /// Caller must guarantee the data is valid for this layout.
            ///
            /// **Deprecated:** Use `load()` for safe access. For explicit
            /// unsafe access, use the raw byte pointer directly.
            #[deprecated(since = "0.2.0", note = "use load() for safe access")]
            #[inline(always)]
            pub unsafe fn load_unchecked(data: &[u8]) -> &Self {
                &*(data.as_ptr() as *const Self)
            }

            /// Write the header for a freshly initialized account.
            #[inline(always)]
            pub fn write_init_header(data: &mut [u8]) -> Result<(), $crate::hopper_runtime::error::ProgramError> {
                $crate::hopper_core::account::write_header(
                    data,
                    Self::DISC,
                    Self::VERSION,
                    &Self::LAYOUT_ID,
                )
            }

            // -- BUMP_OFFSET PDA Optimization ------
            //
            // Scans fields for a `bump` field. If found, generates
            // BUMP_OFFSET const and verify_pda_cached() convenience method.
            // Saves ~344 CU per PDA check vs find_program_address.

            /// Byte offset of the bump field (if present). Used by BUMP_OFFSET PDA optimization.
            /// If no bump field exists, this is set to usize::MAX as a sentinel.
            pub const BUMP_OFFSET: usize = {
                let mut offset = $crate::hopper_core::account::HEADER_LEN;
                let mut found = usize::MAX;
                $(
                    if $crate::hopper_core::__str_eq(stringify!($field), "bump") {
                        found = offset;
                    }
                    offset += $fsize;
                )+
                let _ = offset;
                found
            };

            /// Returns `true` if this layout has a bump field for PDA optimization.
            #[inline(always)]
            pub const fn has_bump_offset() -> bool {
                Self::BUMP_OFFSET != usize::MAX
            }

            // -- Tier 5: Unverified Overlay ------
            //
            // Best-effort loading for indexers and off-chain tooling.
            // Attempts header validation but returns the overlay even on
            // failure, with a bool indicating whether validation passed.

            /// Tier 5: Unverified overlay for indexers/tooling.
            ///
            /// Attempts to validate the header but returns the overlay
            /// regardless. The returned bool is `true` if validation passed.
            ///
            /// This is safe to call on any data -- it never panics.
            #[inline]
            pub fn load_unverified(data: &[u8]) -> Option<(&Self, bool)> {
                if data.len() < Self::LEN {
                    return None;
                }
                let validated = $crate::hopper_core::account::check_header(
                    data,
                    Self::DISC,
                    Self::VERSION,
                    &Self::LAYOUT_ID,
                )
                .is_ok();
                // SAFETY: Size checked above. T: Pod, alignment-1.
                let overlay = unsafe { &*(data.as_ptr() as *const Self) };
                Some((overlay, validated))
            }

            // -- Multi-Owner Foreign Load ------
            //
            // Load foreign account that could be owned by any of several
            // programs (e.g., Token vs Token-2022).

            /// Tier 2m: Foreign load with multiple possible owners.
            ///
            /// Returns `(VerifiedAccount, owner_index)` where `owner_index`
            /// indicates which owner matched.
            #[inline]
            pub fn load_foreign_multi<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                owners: &[&$crate::hopper_runtime::Address],
            ) -> Result<
                ($crate::hopper_core::account::VerifiedAccount<'a, Self>, usize),
                $crate::hopper_runtime::error::ProgramError,
            > {
                let owner_idx = $crate::hopper_core::check::check_owner_multi(account, owners)?;
                let data = account.try_borrow()?;
                let layout_id = $crate::hopper_core::account::read_layout_id(&*data)?;
                if layout_id != Self::LAYOUT_ID {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                $crate::hopper_core::check::check_size(&*data, Self::LEN)?;
                let verified = $crate::hopper_core::account::VerifiedAccount::from_ref(data)?;
                Ok((verified, owner_idx))
            }
        }

        // Implement HopperLayout for modifier-style wrappers.
        impl $crate::hopper_core::check::modifier::HopperLayout for $name {
            const DISC: u8 = $disc;
            const VERSION: u8 = $ver;
            const LAYOUT_ID: [u8; 8] = $name::LAYOUT_ID;
            const LEN_WITH_HEADER: usize = $name::LEN;
        }
    };
}

/// Composable account constraint checking.
///
/// ```ignore
/// hopper_check!(vault,
///     owner = program_id,
///     writable,
///     signer,
///     disc = Vault::DISC,
///     size >= Vault::LEN,
/// );
/// ```
#[macro_export]
macro_rules! hopper_check {
    ($account:expr, $( $constraint:tt )+) => {{
        $crate::_hopper_check_inner!($account, $( $constraint )+)
    }};
}

// Internal constraint dispatcher
#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_check_inner {
    // owner = $id
    ($account:expr, owner = $id:expr $(, $($rest:tt)+ )?) => {{
        $crate::hopper_core::check::check_owner($account, $id)?;
        $( $crate::_hopper_check_inner!($account, $($rest)+); )?
    }};
    // writable
    ($account:expr, writable $(, $($rest:tt)+ )?) => {{
        $crate::hopper_core::check::check_writable($account)?;
        $( $crate::_hopper_check_inner!($account, $($rest)+); )?
    }};
    // signer
    ($account:expr, signer $(, $($rest:tt)+ )?) => {{
        $crate::hopper_core::check::check_signer($account)?;
        $( $crate::_hopper_check_inner!($account, $($rest)+); )?
    }};
    // disc = $d
    ($account:expr, disc = $d:expr $(, $($rest:tt)+ )?) => {{
        let data = $account.try_borrow()?;
        $crate::hopper_core::check::check_discriminator(&*data, $d)?;
        $( $crate::_hopper_check_inner!($account, $($rest)+); )?
    }};
    // size >= $n
    ($account:expr, size >= $n:expr $(, $($rest:tt)+ )?) => {{
        let data = $account.try_borrow()?;
        $crate::hopper_core::check::check_size(&*data, $n)?;
        $( $crate::_hopper_check_inner!($account, $($rest)+); )?
    }};
    // Base case
    ($account:expr,) => {};
}

/// Generate sequential error codes.
///
/// ```ignore
/// hopper_error! {
///     base = 6000;
///     Undercollateralized,  // 6000
///     Expired,              // 6001
///     InvalidOracle,        // 6002
/// }
/// ```
#[macro_export]
macro_rules! hopper_error {
    (
        base = $base:literal;
        $( $name:ident ),+ $(,)?
    ) => {
        $crate::_hopper_error_inner!($base; $( $name ),+);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_error_inner {
    // Base case: single ident
    ($code:expr; $name:ident) => {
        pub struct $name;
        impl $name {
            pub const CODE: u32 = $code;
        }
        impl From<$name> for $crate::hopper_runtime::error::ProgramError {
            fn from(_: $name) -> $crate::hopper_runtime::error::ProgramError {
                $crate::hopper_runtime::error::ProgramError::Custom($code)
            }
        }
    };
    // Recursive case: first ident + rest
    ($code:expr; $name:ident, $($rest:ident),+) => {
        $crate::_hopper_error_inner!($code; $name);
        $crate::_hopper_error_inner!($code + 1; $($rest),+);
    };
}

/// Require a condition, returning a custom error if false.
///
/// ```ignore
/// hopper_require!(amount > 0, ZeroAmount)?;
/// ```
#[macro_export]
macro_rules! hopper_require {
    ($cond:expr, $err:expr) => {
        if !$cond {
            return Err($err.into());
        }
    };
}

/// Initialize an account: create via CPI, zero-init, write header.
///
/// ```ignore
/// hopper_init!(payer, account, system_program, program_id, Vault)?;
/// ```
#[macro_export]
macro_rules! hopper_init {
    ($payer:expr, $account:expr, $system:expr, $program_id:expr, $layout:ty) => {{
        // Calculate rent
        let lamports = $crate::hopper_core::check::rent_exempt_min(<$layout>::LEN);
        let space = <$layout>::LEN as u64;

        // CPI CreateAccount
        $crate::hopper_system::CreateAccount {
            from: $payer,
            to: $account,
            lamports,
            space,
            owner: $program_id,
        }
        .invoke()?;

        // Zero-init and write header
        let mut data = $account.try_borrow_mut()?;
        $crate::hopper_core::account::zero_init(&mut *data);
        <$layout>::write_init_header(&mut *data)
    }};
}

/// Safely close an account with sentinel protection.
///
/// ```ignore
/// hopper_close!(account, destination)?;
/// ```
#[macro_export]
macro_rules! hopper_close {
    ($account:expr, $destination:expr) => {
        $crate::hopper_core::account::safe_close_with_sentinel($account, $destination)
    };
}

/// Discriminator registry -- compile-time uniqueness enforcement.
///
/// Lists all account types for a program and asserts that no two share
/// a discriminator. This prevents silent bugs where `Vault::load()` could
/// accidentally succeed on a `Pool` account.
///
/// ```ignore
/// hopper_register_discs! {
///     Vault,
///     Pool,
///     Position,
/// }
/// ```
///
/// Fails at compile time if any two types share the same DISC value.
#[macro_export]
macro_rules! hopper_register_discs {
    ( $( $layout:ty ),+ $(,)? ) => {
        const _: () = {
            let discs: &[u8] = &[ $( <$layout>::DISC, )+ ];
            let names: &[&str] = &[ $( stringify!($layout), )+ ];
            let n = discs.len();
            let mut i = 0;
            while i < n {
                let mut j = i + 1;
                while j < n {
                    assert!(
                        discs[i] != discs[j],
                        // Can't format at const time, but this gives a clear enough message
                        "Duplicate discriminator detected in hopper_register_discs!"
                    );
                    j += 1;
                }
                i += 1;
            }
            let _ = names; // consumed for error messages in non-const contexts
        };
    };
}

/// PDA verification with BUMP_OFFSET optimization.
///
/// If the layout has a bump field, reads bump from account data and uses
/// `create_program_address` (~200 CU). Otherwise falls back to
/// `find_program_address` (~544 CU).
///
/// ```ignore
/// hopper_verify_pda!(vault_account, &[b"vault", authority.as_ref()], program_id, Vault)?;
/// ```
#[macro_export]
macro_rules! hopper_verify_pda {
    ($account:expr, $seeds:expr, $program_id:expr, $layout:ty) => {{
        if <$layout>::has_bump_offset() {
            $crate::hopper_core::check::verify_pda_cached(
                $account,
                $seeds,
                <$layout>::BUMP_OFFSET,
                $program_id,
            )
        } else {
            // Fallback: no bump field, use regular verify
            match $crate::hopper_core::check::find_and_verify_pda(
                $account,
                $seeds,
                $program_id,
            ) {
                Ok(_bump) => Ok(()),
                Err(e) => Err(e),
            }
        }
    }};
}

/// Invariant checking macro.
///
/// Defines a set of invariants for an instruction that run after mutation.
/// Each invariant is a closure over account data that returns `ProgramResult`.
///
/// ```ignore
/// hopper_invariant! {
///     "balance_conserved" => |vault: &Vault| {
///         let bal = vault.balance.get();
///         hopper_require!(bal <= MAX_SUPPLY, BalanceOverflow);
///         Ok(())
///     },
///     "authority_unchanged" => |vault: &Vault, old: &Vault| {
///         hopper_require!(vault.authority == old.authority, AuthorityChanged);
///         Ok(())
///     },
/// }
/// ```
///
/// Generates an inline invariant runner that returns the first failure.
#[macro_export]
macro_rules! hopper_invariant {
    ( $( $label:literal => $check:expr ),+ $(,)? ) => {{
        let mut _result: $crate::hopper_runtime::ProgramResult = Ok(());
        $(
            if _result.is_ok() {
                _result = $check;
            }
        )+
        _result
    }};
}

/// Generate a layout manifest for schema tooling.
///
/// Produces a `const LayoutManifest` for a layout type, with field
/// descriptors suitable for off-chain tooling, indexers, and migration
/// compatibility checks.
///
/// ```ignore
/// hopper_manifest! {
///     VAULT_MANIFEST = Vault {
///         authority: [u8; 32]  = 32,
///         mint:      [u8; 32]  = 32,
///         balance:   WireU64   = 8,
///         bump:      u8        = 1,
///     }
/// }
/// ```
///
/// Generates: `pub const VAULT_MANIFEST: hopper_schema::LayoutManifest`
#[macro_export]
macro_rules! hopper_manifest {
    (
        $const_name:ident = $name:ident {
            $( $field:ident : $fty:ty = $fsize:literal ),+ $(,)?
        }
    ) => {
        pub const $const_name: $crate::hopper_schema::LayoutManifest = {
            const FIELD_COUNT: usize = 0 $( + { let _ = stringify!($field); 1 } )+;
            const SIZES: [u16; FIELD_COUNT] = [ $( $fsize ),+ ];
            const NAMES: [&str; FIELD_COUNT] = [ $( stringify!($field) ),+ ];
            const TYPES: [&str; FIELD_COUNT] = [ $( stringify!($fty) ),+ ];
            const FIELDS: [$crate::hopper_schema::FieldDescriptor; FIELD_COUNT] = {
                let h = $crate::hopper_core::account::HEADER_LEN as u16;
                let mut result = [$crate::hopper_schema::FieldDescriptor {
                    name: "", canonical_type: "", size: 0, offset: 0,
                    intent: $crate::hopper_schema::FieldIntent::Custom,
                }; FIELD_COUNT];
                let mut offset = h;
                let mut i = 0;
                while i < FIELD_COUNT {
                    result[i] = $crate::hopper_schema::FieldDescriptor {
                        name: NAMES[i],
                        canonical_type: TYPES[i],
                        size: SIZES[i],
                        offset,
                        intent: $crate::hopper_schema::FieldIntent::Custom,
                    };
                    offset += SIZES[i];
                    i += 1;
                }
                result
            };
            $crate::hopper_schema::LayoutManifest {
                name: stringify!($name),
                version: <$name>::VERSION,
                disc: <$name>::DISC,
                layout_id: <$name>::LAYOUT_ID,
                total_size: <$name>::LEN,
                field_count: FIELD_COUNT,
                fields: &FIELDS,
            }
        };
    };
}

// -- Segmented Account Declaration --

/// Declare a segmented account with typed segments.
///
/// Generates:
/// - Segment ID constants (FNV-1a)
/// - A `register_segments` function that initializes the segment registry
/// - Per-segment accessor methods on a generated context struct
///
/// ```ignore
/// hopper_segment! {
///     pub struct Treasury, disc = 3 {
///         core:        TreasuryCore        = 128,
///         permissions: PermissionsTable     = 256,
///         history:     HistoryLog           = 512,
///     }
/// }
///
/// // Initialize:
/// Treasury::init_segments(data)?;
///
/// // Read:
/// let core: &TreasuryCore = Treasury::load_segment::<TreasuryCore>(data, Treasury::CORE_ID)?;
/// ```
#[macro_export]
macro_rules! hopper_segment {
    (
        $(#[$attr:meta])*
        pub struct $name:ident, disc = $disc:literal
        {
            $( $seg:ident : $sty:ty = $ssize:literal ),+ $(,)?
        }
    ) => {
        $(#[$attr])*
        pub struct $name;

        impl $name {
            pub const DISC: u8 = $disc;

            // Generate segment ID constants
            $crate::_hopper_segment_ids!($( $seg ),+);

            // Segment count
            pub const SEGMENT_COUNT: usize = $crate::_hopper_segment_count!($( $seg ),+);

            /// Total account size: header + registry header + entries + segment data.
            pub const TOTAL_SIZE: usize = {
                let registry_size = $crate::hopper_core::account::registry::REGISTRY_HEADER_SIZE
                    + (Self::SEGMENT_COUNT * $crate::hopper_core::account::registry::SEGMENT_ENTRY_SIZE);
                $crate::hopper_core::account::HEADER_LEN
                    + registry_size
                    $( + $ssize )+
            };

            /// Initialize the segment registry with all declared segments.
            #[inline]
            pub fn init_segments(data: &mut [u8]) -> Result<(), $crate::hopper_runtime::error::ProgramError> {
                let specs: &[($crate::hopper_core::account::registry::SegmentId, u32, u8)] = &[
                    $(
                        (
                            $crate::hopper_core::account::registry::segment_id(stringify!($seg)),
                            $ssize as u32,
                            1u8,
                        ),
                    )+
                ];
                $crate::hopper_core::account::SegmentRegistryMut::init(data, specs)
            }

            /// Load a typed overlay from a named segment (immutable).
            #[inline]
            pub fn load_segment<T: $crate::hopper_core::account::Pod + $crate::hopper_core::account::FixedLayout>(
                data: &[u8],
                seg_id: &$crate::hopper_core::account::registry::SegmentId,
            ) -> Result<&T, $crate::hopper_runtime::error::ProgramError> {
                let registry = $crate::hopper_core::account::SegmentRegistry::from_account(data)?;
                registry.segment_overlay::<T>(seg_id)
            }

            /// Load a typed overlay from a named segment (mutable).
            #[inline]
            pub fn load_segment_mut<T: $crate::hopper_core::account::Pod + $crate::hopper_core::account::FixedLayout>(
                data: &mut [u8],
                seg_id: &$crate::hopper_core::account::registry::SegmentId,
            ) -> Result<&mut T, $crate::hopper_runtime::error::ProgramError> {
                let mut registry = $crate::hopper_core::account::SegmentRegistryMut::from_account_mut(data)?;
                registry.segment_overlay_mut::<T>(seg_id)
            }
        }
    };
}

/// Generate uppercase segment ID constants from field names.
#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_segment_ids {
    ( $( $seg:ident ),+ ) => {
        $(
            // Use paste-style approach: just use the name directly as a const
            // The user references it as TypeName::SEGNAME_ID
            $crate::_hopper_segment_id_const!($seg);
        )+
    };
}

/// Generate a single segment ID constant.
///
/// Produces `pub const {NAME}_ID: SegmentId = segment_id("name");`
/// Due to macro_rules limitations we use the exact field name in
/// uppercase manually. This generates as-is with _ID suffix.
#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_segment_id_const {
    ($seg:ident) => {
        #[doc = concat!("Segment ID for `", stringify!($seg), "`.")]
        #[allow(non_upper_case_globals)]
        pub const $seg: $crate::hopper_core::account::registry::SegmentId =
            $crate::hopper_core::account::registry::segment_id(stringify!($seg));
    };
}

/// Count segments.
#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_segment_count {
    ( $( $seg:ident ),+ ) => {
        {
            let mut _n = 0usize;
            $( let _ = stringify!($seg); _n += 1; )+
            _n
        }
    };
}

// -- Validation Pipeline Builder --

/// Build a validation pipeline declaratively.
///
/// Each rule is a combinator that returns `impl Fn(&ValidationContext) -> ProgramResult`.
/// The macro creates a context, enforces unique writable accounts by default,
/// and then invokes each rule in order (fail-fast).
#[macro_export]
macro_rules! hopper_validate {
    (
        accounts = $accounts:expr,
        program_id = $program_id:expr,
        data = $data:expr,
        rules {
            $( $rule:expr ),+ $(,)?
        }
    ) => {{
        let _vctx = $crate::hopper_core::check::graph::ValidationContext::new(
            $program_id,
            $accounts,
            $data,
        );
        $crate::hopper_core::check::graph::require_unique_writable_accounts()(&_vctx)?;
        $( ($rule)(&_vctx)?; )+
        Ok::<(), $crate::hopper_runtime::error::ProgramError>(())
    }};
}

// -- Virtual State Builder --

/// Declare a multi-account virtual state mapping.
///
/// ```ignore
/// let market = hopper_virtual! {
///     slots = 3,
///     map {
///         0 => account_index: 1, owned, writable,
///         1 => account_index: 2, owned,
///         2 => account_index: 3,
///     }
/// };
///
/// market.validate(accounts, program_id)?;
/// let core: &MarketCore = market.overlay::<MarketCore>(accounts, 0)?;
/// ```
#[macro_export]
macro_rules! hopper_virtual {
    (
        slots = $n:literal,
        map {
            $( $slot:literal => account_index: $idx:literal
                $(, owned $( = $owner:expr )? )?
                $(, writable )?
            ),+ $(,)?
        }
    ) => {{
        let mut vs = $crate::hopper_core::virtual_state::VirtualState::<$n>::new();
        $(
            vs = $crate::_hopper_virtual_slot!(vs, $slot, $idx
                $(, owned $( = $owner )? )?
                $(, writable )?
            );
        )+
        vs
    }};
}

/// Apply a single virtual slot mapping.
#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_virtual_slot {
    // owned + writable
    ($vs:expr, $slot:literal, $idx:literal, owned, writable) => {
        $vs.map_mut($slot, $idx)
    };
    // owned only
    ($vs:expr, $slot:literal, $idx:literal, owned) => {
        $vs.map($slot, $idx)
    };
    // writable only (no owner check)
    ($vs:expr, $slot:literal, $idx:literal, writable) => {
        $vs.set_slot($slot, $crate::hopper_core::virtual_state::VirtualSlot {
            account_index: $idx,
            require_owned: false,
            require_writable: true,
        })
    };
    // bare (no constraints)
    ($vs:expr, $slot:literal, $idx:literal) => {
        $vs.map_foreign($slot, $idx)
    };
}

// -- Layout Compatibility Assertions --

/// Assert that two layout versions have compatible fingerprints.
///
/// Fails at compile time if the assertion doesn't hold.
/// Use this in tests and CI to catch accidental schema breaks.
///
/// ```ignore
/// // Assert V2 is a strict superset of V1 (append-only):
/// hopper_assert_compatible!(VaultV1, VaultV2, append);
///
/// // Assert two layouts have different fingerprints (version bump required):
/// hopper_assert_compatible!(VaultV1, VaultV2, differs);
/// ```
#[macro_export]
macro_rules! hopper_assert_compatible {
    // Assert V2 is append-compatible with V1: different fingerprint + larger size + same disc
    ($old:ty, $new:ty, append) => {
        const _: () = {
            assert!(
                <$new>::LEN > <$old>::LEN,
                "New layout must be larger than old for append compatibility"
            );
            assert!(
                <$new>::DISC == <$old>::DISC,
                "Discriminator must remain the same across versions"
            );
            assert!(
                <$new>::VERSION > <$old>::VERSION,
                "New version must be strictly greater"
            );
            // Layout IDs must differ (field set changed)
            let old_id = <$old>::LAYOUT_ID;
            let new_id = <$new>::LAYOUT_ID;
            let mut same = true;
            let mut i = 0;
            while i < 8 {
                if old_id[i] != new_id[i] {
                    same = false;
                }
                i += 1;
            }
            assert!(!same, "Layout IDs must differ between versions");
        };
    };
    // Assert two layouts have different fingerprints
    ($old:ty, $new:ty, differs) => {
        const _: () = {
            let old_id = <$old>::LAYOUT_ID;
            let new_id = <$new>::LAYOUT_ID;
            let mut same = true;
            let mut i = 0;
            while i < 8 {
                if old_id[i] != new_id[i] {
                    same = false;
                }
                i += 1;
            }
            assert!(!same, "Layout IDs must differ between versions");
        };
    };
}

/// Assert that a layout's fingerprint matches an expected value.
///
/// Use this to pin a layout's fingerprint in tests. If someone changes the
/// layout fields, this assertion catches the ABI break at compile time.
///
/// ```ignore
/// hopper_assert_fingerprint!(Vault, [0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81]);
/// ```
#[macro_export]
macro_rules! hopper_assert_fingerprint {
    ($layout:ty, $expected:expr) => {
        const _: () = {
            let actual = <$layout>::LAYOUT_ID;
            let expected: [u8; 8] = $expected;
            let mut i = 0;
            while i < 8 {
                assert!(
                    actual[i] == expected[i],
                    "Layout fingerprint doesn't match expected value -- ABI may have changed"
                );
                i += 1;
            }
        };
    };
}

// Re-export dispatch from core
pub use hopper_core;
pub use hopper_schema;
pub use hopper_runtime;
pub use hopper_system;

/// Compile-time assertion for safe manual `Pod` implementations.
///
/// Verifies that a type meets all Pod requirements:
/// - `align_of == 1` (required for zero-copy overlay at any offset)
/// - `size_of == SIZE` matches declared SIZE
///
/// Use this when implementing `Pod` manually (outside of `hopper_layout!`).
///
/// ```ignore
/// #[repr(C)]
/// #[derive(Clone, Copy)]
/// pub struct MyEntry {
///     pub key: [u8; 32],
///     pub value: WireU64,
/// }
///
/// const_assert_pod!(MyEntry, 40);
/// unsafe impl Pod for MyEntry {}
/// ```
#[macro_export]
macro_rules! const_assert_pod {
    ($ty:ty, $size:expr) => {
        const _: () = assert!(
            core::mem::align_of::<$ty>() == 1,
            concat!(
                "Pod type `", stringify!($ty), "` must have alignment 1 for zero-copy safety. ",
                "Ensure all fields use alignment-1 wire types ([u8; N], WireU64, etc.)."
            )
        );
        const _: () = assert!(
            core::mem::size_of::<$ty>() == $size,
            concat!(
                "Pod type `", stringify!($ty), "` size mismatch: ",
                "expected ", stringify!($size), " bytes"
            )
        );
    };
}

/// Declare a cross-program interface view.
///
/// Generates a read-only overlay struct for reading accounts owned by
/// another program **without any crate dependency**. The interface is
/// pinned by `LAYOUT_ID` -- the same deterministic SHA-256 fingerprint
/// used by `hopper_layout!`. If the originating program changes its
/// layout, the fingerprint will differ and `load_foreign()` will reject
/// the account at runtime.
///
/// The generated struct includes:
/// - `#[repr(C)]` zero-copy overlay with alignment-1 guarantee
/// - Deterministic `LAYOUT_ID` matching the originating layout
/// - `load_foreign(account, expected_owner)` for Tier-2 cross-program reads
/// - `load_foreign_multi(account, owners)` for multi-owner scenarios
/// - `load_with_profile(account, TrustProfile)` for configurable trust
/// - Compile-time size and alignment assertions
///
/// # Example
///
/// Program A defines a Vault:
/// ```ignore
/// hopper_layout! {
///     pub struct Vault, disc = 1, version = 1 {
///         authority: TypedAddress<Authority> = 32,
///         balance:   WireU64                = 8,
///         bump:      u8                     = 1,
///     }
/// }
/// ```
///
/// Program B reads it **without importing Program A**:
/// ```ignore
/// hopper_interface! {
///     /// Read-only view of Program A's Vault.
///     pub struct VaultView, disc = 1, version = 1 {
///         authority: TypedAddress<Authority> = 32,
///         balance:   WireU64                = 8,
///         bump:      u8                     = 1,
///     }
/// }
///
/// let verified = VaultView::load_foreign(vault_account, &PROGRAM_A_ID)?;
/// let balance = verified.get().balance.get();
/// ```
///
/// If the fields match Program A's Vault exactly, the LAYOUT_IDs will
/// be identical and `load_foreign` succeeds. Any structural divergence
/// produces a different hash and the load fails.
#[macro_export]
macro_rules! hopper_interface {
    (
        $(#[$attr:meta])*
        pub struct $name:ident, disc = $disc:literal, version = $ver:literal
        {
            $( $field:ident : $fty:ty = $fsize:literal ),+ $(,)?
        }
    ) => {
        $(#[$attr])*
        #[derive(Clone, Copy)]
        #[repr(C)]
        pub struct $name {
            pub header: $crate::hopper_core::account::AccountHeader,
            $( pub $field: $fty, )+
        }

        // Compile-time assertions
        const _: () = {
            let expected = $crate::hopper_core::account::HEADER_LEN $( + $fsize )+;
            assert!(
                core::mem::size_of::<$name>() == expected,
                "Interface size mismatch: struct size != declared field sizes + header"
            );
            assert!(
                core::mem::align_of::<$name>() == 1,
                "Interface alignment must be 1 for zero-copy safety"
            );
        };

        // SAFETY: #[repr(C)] over alignment-1 fields, all bit patterns valid.
        unsafe impl $crate::hopper_core::account::Pod for $name {}

        impl $crate::hopper_core::account::FixedLayout for $name {
            const SIZE: usize = $crate::hopper_core::account::HEADER_LEN $( + $fsize )+;
        }

        impl $crate::hopper_core::field_map::FieldMap for $name {
            const FIELDS: &'static [$crate::hopper_core::field_map::FieldInfo] = {
                const FIELD_COUNT: usize = 0 $( + { let _ = stringify!($field); 1 } )+;
                const NAMES: [&str; FIELD_COUNT] = [ $( stringify!($field) ),+ ];
                const SIZES: [usize; FIELD_COUNT] = [ $( $fsize ),+ ];
                const FIELDS: [$crate::hopper_core::field_map::FieldInfo; FIELD_COUNT] = {
                    let mut result = [$crate::hopper_core::field_map::FieldInfo::new("", 0, 0); FIELD_COUNT];
                    let mut offset = $crate::hopper_core::account::HEADER_LEN;
                    let mut index = 0;
                    while index < FIELD_COUNT {
                        result[index] = $crate::hopper_core::field_map::FieldInfo::new(
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

        impl $crate::hopper_runtime::LayoutContract for $name {
            const DISC: u8 = $disc;
            const VERSION: u8 = $ver;
            const LAYOUT_ID: [u8; 8] = $name::LAYOUT_ID;
            const SIZE: usize = $name::LEN;
            const TYPE_OFFSET: usize = 0;
        }

        impl $crate::hopper_schema::SchemaExport for $name {
            fn layout_manifest() -> $crate::hopper_schema::LayoutManifest {
                const FIELD_COUNT: usize = 0 $( + { let _ = stringify!($field); 1 } )+;
                const SIZES: [u16; FIELD_COUNT] = [ $( $fsize ),+ ];
                const NAMES: [&str; FIELD_COUNT] = [ $( stringify!($field) ),+ ];
                const TYPES: [&str; FIELD_COUNT] = [ $( stringify!($fty) ),+ ];
                const FIELDS: [$crate::hopper_schema::FieldDescriptor; FIELD_COUNT] = {
                    let mut result = [$crate::hopper_schema::FieldDescriptor {
                        name: "", canonical_type: "", size: 0, offset: 0,
                        intent: $crate::hopper_schema::FieldIntent::Custom,
                    }; FIELD_COUNT];
                    let mut offset = $crate::hopper_core::account::HEADER_LEN as u16;
                    let mut index = 0;
                    while index < FIELD_COUNT {
                        result[index] = $crate::hopper_schema::FieldDescriptor {
                            name: NAMES[index],
                            canonical_type: TYPES[index],
                            size: SIZES[index],
                            offset,
                            intent: $crate::hopper_schema::FieldIntent::Custom,
                        };
                        offset += SIZES[index];
                        index += 1;
                    }
                    result
                };
                $crate::hopper_schema::LayoutManifest {
                    name: stringify!($name),
                    version: <$name>::VERSION,
                    disc: <$name>::DISC,
                    layout_id: <$name>::LAYOUT_ID,
                    total_size: <$name>::LEN,
                    field_count: FIELD_COUNT,
                    fields: &FIELDS,
                }
            }
        }

        impl $name {
            /// Total byte size of this interface view.
            pub const LEN: usize = $crate::hopper_core::account::HEADER_LEN $( + $fsize )+;

            /// Expected discriminator of the originating layout.
            pub const DISC: u8 = $disc;

            /// Expected version of the originating layout.
            pub const VERSION: u8 = $ver;

            /// Deterministic layout fingerprint.
            ///
            /// Matches the originating layout's `LAYOUT_ID` if the field
            /// names, types, sizes, and ordering are identical.
            pub const LAYOUT_ID: [u8; 8] = {
                const INPUT: &str = concat!(
                    "hopper:v1:",
                    stringify!($name), ":",
                    stringify!($ver), ":",
                    $( stringify!($field), ":", stringify!($fty), ":", stringify!($fsize), ",", )+
                );
                const HASH: [u8; 32] = $crate::hopper_core::__sha256_const(INPUT.as_bytes());
                [
                    HASH[0], HASH[1], HASH[2], HASH[3],
                    HASH[4], HASH[5], HASH[6], HASH[7],
                ]
            };

            /// Read-only overlay (immutable).
            #[inline(always)]
            pub fn overlay(data: &[u8]) -> Result<&Self, $crate::hopper_runtime::error::ProgramError> {
                $crate::hopper_core::account::pod_from_bytes::<Self>(data)
            }

            /// Tier 2: Cross-program foreign load (read-only).
            ///
            /// Validates: owner + layout_id + exact size.
            /// No discriminator or version check -- the layout_id is the ABI proof.
            ///
            /// **Deprecated:** Renamed to `load_cross_program()` for clarity.
            #[deprecated(since = "0.2.0", note = "renamed to load_cross_program()")]
            #[inline]
            pub fn load_foreign<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                expected_owner: &$crate::hopper_runtime::Address,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccount<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                Self::load_cross_program(account, expected_owner)
            }

            /// Tier 2: Cross-program load (read-only).
            ///
            /// Validates: owner + layout_id + exact size.
            /// The layout_id is the ABI proof — no discriminator or version check needed.
            #[inline]
            pub fn load_cross_program<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                expected_owner: &$crate::hopper_runtime::Address,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccount<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                $crate::hopper_core::check::check_owner(account, expected_owner)?;
                let data = account.try_borrow()?;
                let layout_id = $crate::hopper_core::account::read_layout_id(&*data)?;
                if layout_id != Self::LAYOUT_ID {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                $crate::hopper_core::check::check_size(&*data, Self::LEN)?;
                $crate::hopper_core::account::VerifiedAccount::from_ref(data)
            }

            /// Tier 2m: Foreign load with multiple possible owners.
            ///
            /// Returns `(VerifiedAccount, owner_index)` where `owner_index`
            /// indicates which expected owner matched.
            #[inline]
            pub fn load_foreign_multi<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                owners: &[&$crate::hopper_runtime::Address],
            ) -> Result<
                ($crate::hopper_core::account::VerifiedAccount<'a, Self>, usize),
                $crate::hopper_runtime::error::ProgramError,
            > {
                let owner_idx = $crate::hopper_core::check::check_owner_multi(account, owners)?;
                let data = account.try_borrow()?;
                let layout_id = $crate::hopper_core::account::read_layout_id(&*data)?;
                if layout_id != Self::LAYOUT_ID {
                    return Err($crate::hopper_runtime::error::ProgramError::InvalidAccountData);
                }
                $crate::hopper_core::check::check_size(&*data, Self::LEN)?;
                let verified = $crate::hopper_core::account::VerifiedAccount::from_ref(data)?;
                Ok((verified, owner_idx))
            }

            /// Load with a TrustProfile for configurable cross-program validation.
            ///
            /// Supports Strict, Compatible, and Observational trust levels.
            #[inline]
            pub fn load_with_profile<'a>(
                account: &'a $crate::hopper_runtime::AccountView,
                profile: &$crate::hopper_core::check::trust::TrustProfile<'a>,
            ) -> Result<
                $crate::hopper_core::account::VerifiedAccount<'a, Self>,
                $crate::hopper_runtime::error::ProgramError,
            > {
                let data = profile.load(account)?;
                $crate::hopper_core::account::VerifiedAccount::from_ref(data)
            }

            /// Tier 5: Unverified overlay for indexers/tooling.
            #[inline]
            pub fn load_unverified(data: &[u8]) -> Option<(&Self, bool)> {
                if data.len() < Self::LEN {
                    return None;
                }
                let validated = $crate::hopper_core::account::check_header(
                    data,
                    Self::DISC,
                    Self::VERSION,
                    &Self::LAYOUT_ID,
                )
                .is_ok();
                // SAFETY: Size checked above. T: Pod, alignment-1.
                let overlay = unsafe { &*(data.as_ptr() as *const Self) };
                Some((overlay, validated))
            }
        }
    };
}

// ---------------------------------------------------------------------------
// hopper_accounts! -- typed context generation
// ---------------------------------------------------------------------------

/// Generate a typed instruction context struct with validated account parsing.
///
/// Produces:
/// - The account struct itself
/// - A `Bumps` struct for PDA bump storage
/// - A `HopperAccounts` impl with `try_from_accounts`
/// - A static `ContextDescriptor` for schema/explain
///
/// # Account kinds
///
/// Each field specifies a `kind` wrapped in parentheses, with optional modifiers:
///
/// | Kind                 | Description                     | Writable | Signer |
/// |----------------------|---------------------------------|----------|--------|
/// | `(signer)`           | Verified signer (SignerAccount) | no       | yes    |
/// | `(mut signer)`       | Mutable + signer                | yes      | yes    |
/// | `(account<T>)`       | Layout-bound HopperAccount      | no       | no     |
/// | `(mut account<T>)`   | Mutable layout-bound account    | yes      | no     |
/// | `(program)`          | Verified executable (ProgramRef)| no       | no     |
/// | `(unchecked)`        | No-validation passthrough       | no       | no     |
/// | `(mut unchecked)`    | Mutable unchecked passthrough   | yes      | no     |
///
/// # Example
///
/// ```ignore
/// hopper_accounts! {
///     pub struct Deposit {
///         authority: (mut signer),
///         vault: (mut account<VaultState>),
///         system_program: (program),
///     }
/// }
/// ```
///
/// Then use with `hopper_entry`:
///
/// ```ignore
/// hopper_entry::<DepositIx, _>(program_id, accounts, data, |ctx, args| {
///     let vault = ctx.accounts.vault.write()?;
///     // ...
///     Ok(())
/// })
/// ```
#[macro_export]
macro_rules! hopper_accounts {
    // Main entry: parse struct with field list
    (
        $(#[$attr:meta])*
        pub struct $name:ident {
            $( $field:ident : ( $($kind:tt)+ ) ),+ $(,)?
        }
    ) => {
        // Wrap each kind in parens so it becomes a single tt group,
        // which eliminates the greedy-tt ambiguity in the inner macro.
        $crate::_hopper_accounts_struct!($name; $( $field: ($($kind)+) ; )+);
    };
}

/// Internal: parse each field's kind and generate the struct + impls.
///
/// Each `$kind` is a single parenthesised token tree, e.g. `(mut signer)`.
#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_accounts_struct {
    ($name:ident; $( $field:ident : $kind:tt ; )+) => {

        // --- Context struct ---
        pub struct $name<'a> {
            $(
                pub $field: $crate::_hopper_field_type!($kind),
            )+
        }

        // --- HopperAccounts impl ---
        impl<'a> $crate::hopper_core::accounts::HopperAccounts<'a> for $name<'a> {
            type Bumps = ();

            const ACCOUNT_COUNT: usize = {
                // Count fields at compile time using the array-length trick.
                #[allow(unused)]
                const N: usize = [$( { let _ = stringify!($field); 0u8 }, )+].len();
                N
            };

            fn try_from_accounts(
                program_id: &'a $crate::hopper_runtime::Address,
                accounts: &'a [$crate::hopper_runtime::AccountView],
                _instruction_data: &'a [u8],
            ) -> Result<(Self, Self::Bumps), $crate::hopper_runtime::error::ProgramError> {
                let mut _idx: usize = 0;
                $(
                    if _idx >= accounts.len() {
                        return Err($crate::hopper_runtime::error::ProgramError::NotEnoughAccountKeys);
                    }
                    let $field = $crate::_hopper_field_parse!(
                        &accounts[_idx], program_id, $kind
                    )?;
                    _idx += 1;
                )+
                Ok((Self { $( $field, )+ }, ()))
            }

            fn context_schema() -> Option<
                &'static $crate::hopper_core::accounts::explain::ContextSchema
            > {
                static FIELDS: &[$crate::hopper_core::accounts::explain::AccountFieldSchema] = &[
                    $(
                        $crate::hopper_core::accounts::explain::AccountFieldSchema {
                            name: stringify!($field),
                            kind: $crate::_hopper_field_kind_name!($kind),
                            mutable: $crate::_hopper_field_is_mut!($kind),
                            signer: $crate::_hopper_field_is_signer!($kind),
                            layout: $crate::_hopper_field_layout_name!($kind),
                            policy: None,
                            seeds: &[],
                            optional: false,
                        },
                    )+
                ];
                static SCHEMA: $crate::hopper_core::accounts::explain::ContextSchema =
                    $crate::hopper_core::accounts::explain::ContextSchema {
                        name: stringify!($name),
                        fields: FIELDS,
                        policy_names: &[],
                        receipts_expected: false,
                        mutation_classes: &[],
                    };
                Some(&SCHEMA)
            }
        }
    };
}

// --- Field type resolution ---

#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_field_type {
    ((mut signer)) => { $crate::hopper_core::accounts::SignerAccount<'a> };
    ((signer)) => { $crate::hopper_core::accounts::SignerAccount<'a> };
    ((mut account < $layout:ty >)) => { $crate::hopper_core::accounts::HopperAccount<'a, $layout> };
    ((account < $layout:ty >)) => { $crate::hopper_core::accounts::HopperAccount<'a, $layout> };
    ((program)) => { $crate::hopper_core::accounts::ProgramRef<'a> };
    ((unchecked)) => { $crate::hopper_core::accounts::UncheckedAccount<'a> };
    ((mut unchecked)) => { $crate::hopper_core::accounts::UncheckedAccount<'a> };
}

// --- Field parsing at runtime ---

#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_field_parse {
    ($account:expr, $program_id:expr, (mut signer)) => {{
        $crate::hopper_core::check::check_writable($account)?;
        $crate::hopper_core::accounts::SignerAccount::from_account($account)
    }};
    ($account:expr, $program_id:expr, (signer)) => {
        $crate::hopper_core::accounts::SignerAccount::from_account($account)
    };
    ($account:expr, $program_id:expr, (mut account < $layout:ty >)) => {
        $crate::hopper_core::accounts::HopperAccount::<$layout>::from_account_mut(
            $account, $program_id,
        )
    };
    ($account:expr, $program_id:expr, (account < $layout:ty >)) => {
        $crate::hopper_core::accounts::HopperAccount::<$layout>::from_account(
            $account, $program_id,
        )
    };
    ($account:expr, $program_id:expr, (program)) => {
        $crate::hopper_core::accounts::ProgramRef::from_account($account)
    };
    ($account:expr, $program_id:expr, (unchecked)) => {
        Ok::<_, $crate::hopper_runtime::error::ProgramError>(
            $crate::hopper_core::accounts::UncheckedAccount::new($account)
        )
    };
    ($account:expr, $program_id:expr, (mut unchecked)) => {{
        $crate::hopper_core::check::check_writable($account)?;
        Ok::<_, $crate::hopper_runtime::error::ProgramError>(
            $crate::hopper_core::accounts::UncheckedAccount::new($account)
        )
    }};
}

// --- Static metadata helpers ---

#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_field_kind_name {
    ((mut signer)) => { "Signer" };
    ((signer)) => { "Signer" };
    ((mut account < $layout:ty >)) => { "HopperAccount" };
    ((account < $layout:ty >)) => { "HopperAccount" };
    ((program)) => { "ProgramRef" };
    ((unchecked)) => { "Unchecked" };
    ((mut unchecked)) => { "Unchecked" };
}

#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_field_is_mut {
    ((mut signer)) => { true };
    ((signer)) => { false };
    ((mut account < $layout:ty >)) => { true };
    ((account < $layout:ty >)) => { false };
    ((program)) => { false };
    ((unchecked)) => { false };
    ((mut unchecked)) => { true };
}

#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_field_is_signer {
    ((mut signer)) => { true };
    ((signer)) => { true };
    ((mut account < $layout:ty >)) => { false };
    ((account < $layout:ty >)) => { false };
    ((program)) => { false };
    ((unchecked)) => { false };
    ((mut unchecked)) => { false };
}

#[doc(hidden)]
#[macro_export]
macro_rules! _hopper_field_layout_name {
    ((mut signer)) => { None };
    ((signer)) => { None };
    ((mut account < $layout:ty >)) => { Some(stringify!($layout)) };
    ((account < $layout:ty >)) => { Some(stringify!($layout)) };
    ((program)) => { None };
    ((unchecked)) => { None };
    ((mut unchecked)) => { None };
}
