//! Zero-allocation event emission via `sol_log_data`.
//!
//! Events are emitted as raw Pod bytes through the `sol_log_data` syscall.
//! This is ~100 CU and zero-allocation. For unforgeable events, use
//! `emit_event_cpi` which self-invokes the current program (~1000 CU).
//!
//! ## Dual emission model
//!
//! | Function | CU Cost | Spoofable? | Use case |
//! |---|---|---|---|
//! | `emit_event` | ~100 | Yes (any program can log) | Fast indexer events |
//! | `emit_event_cpi` | ~1000 | No (verified via self-CPI) | Trustworthy audit trail |

use hopper_runtime::error::ProgramError;
use crate::account::{Pod, FixedLayout};

/// Emit a Pod event via `sol_log_data`.
///
/// The event is logged as raw bytes. Clients decode using the schema manifest
/// or known layout. Costs ~100 CU, zero allocation.
#[inline(always)]
pub fn emit_event<T: Pod + FixedLayout>(value: &T) -> Result<(), ProgramError> {
    // SAFETY: T: Pod guarantees all bit patterns valid and no padding invariants.
    // The resulting slice covers exactly T::SIZE bytes from a valid reference.
    let bytes = unsafe {
        core::slice::from_raw_parts(value as *const T as *const u8, T::SIZE)
    };
    emit_slices(&[bytes]);
    Ok(())
}

/// Emit event with a discriminator prefix for easy client-side filtering.
///
/// Layout: `[event_disc: u8][event_data: T::SIZE bytes]`
#[inline]
pub fn emit_event_tagged<T: Pod + FixedLayout>(disc: u8, value: &T) -> Result<(), ProgramError> {
    // SAFETY: T: Pod guarantees all bit patterns valid. Slice covers T::SIZE bytes.
    let value_bytes = unsafe {
        core::slice::from_raw_parts(value as *const T as *const u8, T::SIZE)
    };
    let disc_bytes = [disc];
    emit_slices(&[&disc_bytes[..], value_bytes]);
    Ok(())
}

/// Emit one or more byte slices as a single `sol_log_data` entry.
#[inline(always)]
pub fn emit_slices(segments: &[&[u8]]) {
    #[cfg(target_os = "solana")]
    {
        // SAFETY: segments is a valid slice of (ptr, len) pairs as expected
        // by the sol_log_data syscall. BPF ABI guarantees layout compatibility.
        unsafe {
            hopper_runtime::syscalls::sol_log_data(
                segments.as_ptr() as *const u8,
                segments.len() as u64,
            );
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = segments;
    }
}

/// Emit an unforgeable event via self-CPI (~1000 CU).
///
/// This self-invokes the current program with special event data, so
/// the event appears in the transaction as a CPI from the program itself.
/// Indexers can verify the event origin matches the program ID, making
/// spoofing impossible.
///
/// Event CPI data layout: `[0xFF, 0xFE][event_disc: u8][event_data: T::SIZE bytes]`
///
/// The `0xFF 0xFE` prefix is an invalid instruction discriminator reserved
/// for event CPI, the program's dispatch will never match it.
#[cfg(feature = "cpi")]
#[inline]
pub fn emit_event_cpi<T: Pod + FixedLayout>(
    disc: u8,
    value: &T,
    program_id: &hopper_runtime::Address,
    accounts: &[&hopper_runtime::AccountView],
) -> Result<(), ProgramError> {
    // Build event data: [0xFF, 0xFE, disc, ...value_bytes]
    const EVENT_CPI_PREFIX: [u8; 2] = [0xFF, 0xFE];
    let value_bytes = unsafe {
        core::slice::from_raw_parts(value as *const T as *const u8, T::SIZE)
    };

    // Emit via sol_log_data first (cheap, for indexers).
    let disc_byte = [disc];
    emit_slices(&[&disc_byte[..], value_bytes]);

    // Self-CPI for unforgeable proof.
    #[cfg(target_os = "solana")]
    {
        use hopper_runtime::instruction::{InstructionAccount, InstructionView};

        // Build instruction data on stack: prefix + disc + value bytes.
        // Max event data = 1024 - 3 = 1021 bytes.
        let data_len = 3 + T::SIZE;
        if data_len > 1024 {
            return Err(ProgramError::InvalidArgument);
        }
        let mut data_buf = [0u8; 1024];
        data_buf[0] = EVENT_CPI_PREFIX[0];
        data_buf[1] = EVENT_CPI_PREFIX[1];
        data_buf[2] = disc;
        // SAFETY: value_bytes length = T::SIZE, verified via Pod + FixedLayout.
        unsafe {
            core::ptr::copy_nonoverlapping(
                value_bytes.as_ptr(),
                data_buf.as_mut_ptr().add(3),
                T::SIZE,
            );
        }

        let empty_accounts: [InstructionAccount; 0] = [];
        let instruction = InstructionView {
            program_id,
            data: &data_buf[..data_len],
            accounts: &empty_accounts,
        };

        // Build CPI accounts from the provided account views.
        let mut cpi_accounts_buf: [core::mem::MaybeUninit<hopper_runtime::instruction::CpiAccount>; 32] =
            unsafe { core::mem::MaybeUninit::uninit().assume_init() };
        let count = accounts.len().min(32);
        let mut i = 0;
        while i < count {
            cpi_accounts_buf[i] = core::mem::MaybeUninit::new(
                hopper_runtime::instruction::CpiAccount::from(accounts[i])
            );
            i += 1;
        }
        let cpi_accounts = unsafe {
            core::slice::from_raw_parts(
                cpi_accounts_buf.as_ptr() as *const hopper_runtime::instruction::CpiAccount,
                count,
            )
        };

        unsafe {
            hopper_runtime::cpi::invoke_unchecked(&instruction, cpi_accounts)?;
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (program_id, accounts);
    }

    Ok(())
}
