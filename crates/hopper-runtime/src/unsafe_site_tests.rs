//! Sites that form a reference or a slice out of raw memory and that no
//! other test reached: the address casts, the owner reference, the typed
//! read of instruction data, the account list a token CPI assembles, and
//! the host side of the crypto wrappers. `audit/UNSAFE_MAP.md` lists these
//! sites as reached from here.

extern crate std;

use crate::context::Context;
use crate::crypto;
use crate::instruction::InstructionAccount;
use crate::lazy::LazyContext;
use crate::token::{invoke_token_signed, Trailing, MAX_TOKEN_MULTISIG_SIGNERS};
use crate::{AccountView, Address, ProgramError};
use hopper_native::{
    AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
};
use std::vec::Vec;

const OWNER: [u8; 32] = [0xA1; 32];

fn account(key: u8, is_signer: bool, is_writable: bool) -> (Vec<u64>, AccountView<'static>) {
    let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + 16).div_ceil(8)];
    let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
    // SAFETY: `backing` is word-aligned and holds one header plus 16 bytes;
    // one valid header is written before a view is made.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: u8::from(is_signer),
            is_writable: u8::from(is_writable),
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array([key; 32]),
            owner: NativeAddress::new_from_array(OWNER),
            lamports: 1,
            data_len: 16,
        });
    }
    // SAFETY: `raw` points at the header written above, and the caller
    // keeps `backing` alive next to the view.
    let backend = unsafe { NativeAccountView::new_unchecked(raw) };
    (backing, AccountView::from_backend(backend))
}

#[test]
fn an_address_is_the_same_bytes_on_both_sides_of_the_boundary() {
    let ours = Address::new([0x5C; 32]);
    let native: &NativeAddress = ours.as_upstream();
    assert_eq!(native.as_array(), &[0x5C; 32]);
    // The cast moves nothing: same place, same length.
    assert!(core::ptr::eq(
        native as *const NativeAddress as *const u8,
        &ours as *const Address as *const u8
    ));
    assert_eq!(
        core::mem::size_of::<NativeAddress>(),
        core::mem::size_of::<Address>()
    );
    assert_eq!(core::mem::align_of::<NativeAddress>(), 1);
    assert_eq!(core::mem::align_of::<Address>(), 1);

    let back = Address::from_upstream(native);
    assert_eq!(back, &ours);
    let same: &Address = ours.as_upstream();
    assert!(core::ptr::eq(same, &ours));
}

#[test]
fn the_owner_reference_reads_the_header_in_place() {
    let (_backing, view) = account(3, false, true);
    // SAFETY: the owner is not reassigned while the reference is used.
    let owner = unsafe { crate::native_boundary::account_owner(view.as_backend()) };
    assert_eq!(owner, &Address::new(OWNER));
    assert_eq!(
        crate::native_boundary::read_owner(view.as_backend()),
        *owner
    );
    assert_eq!(
        crate::native_boundary::account_address(view.as_backend()),
        &Address::new([3; 32])
    );
}

#[test]
fn instruction_data_is_read_by_value_at_any_offset() {
    let pid = Address::new([9; 32]);
    let data: [u8; 13] = [0xFF, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let ctx = Context::new(&pid, &[], &data);

    // Offset 1 is not aligned for a `u64`; the read copies the bytes.
    assert_eq!(
        ctx.read_data::<u64>(1).unwrap(),
        u64::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8])
    );
    assert_eq!(
        ctx.read_data::<u32>(9).unwrap(),
        u32::from_le_bytes([9, 10, 11, 12])
    );
    assert_eq!(ctx.read_data::<u8>(12).unwrap(), 12);
    assert_eq!(ctx.read_data::<[u8; 13]>(0).unwrap(), data);

    assert_eq!(
        ctx.read_data::<u32>(10),
        Err(ProgramError::InvalidInstructionData)
    );
    assert_eq!(
        ctx.read_data::<u8>(13),
        Err(ProgramError::InvalidInstructionData)
    );
    assert_eq!(
        ctx.read_data::<u64>(usize::MAX),
        Err(ProgramError::ArithmeticOverflow)
    );
    let empty = Context::new(&pid, &[], &[]);
    assert_eq!(
        empty.read_data::<u8>(0),
        Err(ProgramError::InvalidInstructionData)
    );
}

/// A loader input frame of `owners.len()` accounts with no data.
fn frame(owners: &[[u8; 32]]) -> Vec<u64> {
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(&(owners.len() as u64).to_le_bytes());
    for (i, owner) in owners.iter().enumerate() {
        let header = RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 0,
            is_writable: 1,
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array([i as u8 + 1; 32]),
            owner: NativeAddress::new_from_array(*owner),
            lamports: 5,
            data_len: 0,
        };
        // SAFETY: a plain-data header viewed as its own bytes.
        bytes.extend_from_slice(unsafe {
            core::slice::from_raw_parts(
                &header as *const RuntimeAccount as *const u8,
                core::mem::size_of::<RuntimeAccount>(),
            )
        });
        // The realloc reserve, already a multiple of 8, then the rent epoch.
        bytes.extend_from_slice(&std::vec![0u8; 10 * 1024]);
        bytes.extend_from_slice(&u64::MAX.to_le_bytes());
    }
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&[42; 32]);
    // Word-aligned storage: the loader's buffer is, and the headers hold
    // `u64` fields.
    let mut words = std::vec![0u64; bytes.len().div_ceil(8)];
    // SAFETY: `words` holds at least `bytes.len()` bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr() as *mut u8, bytes.len());
    }
    words
}

#[test]
fn the_lazy_parser_checks_the_owner_it_was_given() {
    let mut input = frame(&[OWNER, [9; 32], OWNER]);
    // SAFETY: `input` is a well-formed loader frame and outlives the
    // context.
    let mut native = unsafe { hopper_native::lazy::lazy_deserialize(input.as_mut_ptr().cast()) };
    let mut ctx = LazyContext::from_native(&mut native);
    let program = Address::new(OWNER);

    let first = ctx.next_owned_by(&program).unwrap();
    assert_eq!(first.address(), &Address::new([1; 32]));
    // The second account belongs to someone else.
    assert!(ctx.next_owned_by(&program).is_err());
}

fn refs<'a>(views: &'a [AccountView<'static>]) -> Vec<&'a AccountView<'static>> {
    views.iter().collect()
}

#[test]
fn a_token_cpi_lists_fixed_accounts_first_and_trailing_runs_in_order() {
    let (_a, source) = account(1, false, true);
    let (_b, authority) = account(2, false, false);
    let mut keep = Vec::new();
    let mut signers = Vec::new();
    for key in 10..13 {
        let (backing, view) = account(key, true, false);
        keep.push(backing);
        signers.push(view);
    }
    let program = Address::new([6; 32]);
    let signer_refs = refs(&signers);

    // Accepted by every check that runs before the call leaves the
    // program: addresses line up with views, privileges are covered.
    let fixed = [
        InstructionAccount::writable(source.address()),
        InstructionAccount::readonly(authority.address()),
    ];
    let sent = invoke_token_signed(
        &program,
        &[3],
        fixed,
        [&source, &authority],
        &[Trailing::signers(&signer_refs)],
        &[],
    );
    assert_ne!(sent, Err(ProgramError::InvalidAccountData));
    assert_ne!(sent, Err(ProgramError::MissingRequiredSignature));
    assert_ne!(sent, Err(ProgramError::Immutable));

    // A view in the wrong slot is caught: the meta names `source`, the
    // view is `authority`.
    let crossed = invoke_token_signed(
        &program,
        &[3],
        fixed,
        [&authority, &source],
        &[Trailing::signers(&signer_refs)],
        &[],
    );
    assert_eq!(crossed, Err(ProgramError::InvalidAccountData));

    // A trailing signer that did not sign is caught at its own position,
    // so the trailing run was laid out after the fixed accounts.
    let (_c, absent) = account(13, false, false);
    let with_absent = [&signers[0], &absent, &signers[2]];
    let unsigned = invoke_token_signed(
        &program,
        &[3],
        fixed,
        [&source, &authority],
        &[Trailing::signers(&with_absent)],
        &[],
    );
    assert_eq!(unsigned, Err(ProgramError::MissingRequiredSignature));

    // A fixed account asked to be writable that is not.
    let read_only = [
        InstructionAccount::writable(authority.address()),
        InstructionAccount::readonly(source.address()),
    ];
    let immutable = invoke_token_signed(&program, &[3], read_only, [&authority, &source], &[], &[]);
    assert_eq!(immutable, Err(ProgramError::Immutable));
}

#[test]
fn a_token_cpi_refuses_more_accounts_than_its_buffers_hold() {
    let (_a, source) = account(1, false, true);
    let mut keep = Vec::new();
    let mut many = Vec::new();
    for key in 0..crate::cpi::MAX_STATIC_CPI_ACCOUNTS {
        let (backing, view) = account(100 + key as u8, true, false);
        keep.push(backing);
        many.push(view);
    }
    let program = Address::new([6; 32]);
    let fixed = [InstructionAccount::writable(source.address())];
    let many_refs = refs(&many);

    // Twelve signers are one more than a token multisig has.
    let twelve = &many_refs[..MAX_TOKEN_MULTISIG_SIGNERS + 1];
    assert_eq!(
        invoke_token_signed(
            &program,
            &[3],
            fixed,
            [&source],
            &[Trailing::signers(twelve)],
            &[],
        ),
        Err(ProgramError::InvalidArgument)
    );

    // One fixed account and 64 trailing ones are 65, one past the buffer.
    let run = Trailing {
        views: &many_refs,
        writable: false,
        signer: false,
    };
    assert_eq!(
        invoke_token_signed(&program, &[3], fixed, [&source], &[run], &[]),
        Err(ProgramError::InvalidArgument)
    );

    // 63 trailing accounts fill it exactly and pass the capacity check.
    let full = Trailing {
        views: &many_refs[..crate::cpi::MAX_STATIC_CPI_ACCOUNTS - 1],
        writable: false,
        signer: false,
    };
    assert_ne!(
        invoke_token_signed(&program, &[3], fixed, [&source], &[full], &[]),
        Err(ProgramError::InvalidArgument)
    );
}

#[test]
fn the_edwards_check_runs_on_the_host() {
    // The Ed25519 base point, and the identity.
    let mut base = [0x66u8; 32];
    base[0] = 0x58;
    let mut identity = [0u8; 32];
    identity[0] = 1;
    assert_eq!(crypto::curve25519_edwards_validate_point(&base), Ok(true));
    assert_eq!(
        crypto::curve25519_edwards_validate_point(&identity),
        Ok(true)
    );

    // y = 2 has no x on the curve.
    let mut off = [0u8; 32];
    off[0] = 2;
    assert_eq!(crypto::curve25519_edwards_validate_point(&off), Ok(false));

    // Agrees with `solana-pubkey` on a spread of inputs.
    let mut bytes = [0u8; 32];
    let mut on = 0;
    for round in 0u32..512 {
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = (round.wrapping_mul(2_654_435_761).rotate_left(i as u32) >> 3) as u8 ^ i as u8;
        }
        let expected = solana_pubkey::Pubkey::new_from_array(bytes).is_on_curve();
        assert_eq!(
            crypto::curve_validate_point(crypto::CURVE25519_EDWARDS, &bytes),
            Ok(expected),
            "{bytes:?}"
        );
        on += expected as u32;
    }
    assert!(on > 100 && on < 400, "{on} of 512 on the curve");

    // No host implementation: the answer is `false`, never a guess.
    assert_eq!(
        crypto::curve_validate_point(crypto::CURVE25519_RISTRETTO, &base),
        Ok(false)
    );
}

#[test]
fn syscall_only_operations_fail_on_the_host_instead_of_inventing_a_result() {
    assert_eq!(
        crypto::secp256k1_recover(&[1; 32], 0, &[2; 64]),
        Err(ProgramError::InvalidArgument)
    );
    assert!(crypto::recover_ethereum_address(&[1; 32], 0, &[2; 64]).is_err());
    #[cfg(feature = "crypto-curve")]
    {
        let point = [0x58; 32];
        assert!(crypto::curve_group_add(crypto::CURVE25519_EDWARDS, &point, &point).is_err());
        assert!(crypto::curve_group_sub(crypto::CURVE25519_EDWARDS, &point, &point).is_err());
        assert!(crypto::curve_group_mul(crypto::CURVE25519_EDWARDS, &[1; 32], &point).is_err());
    }
    #[cfg(feature = "crypto-bn254")]
    {
        assert!(crypto::alt_bn128_g1_compress_be(&[0; 64]).is_err());
    }
}

#[test]
fn the_sibling_instruction_syscall_reports_failure_on_the_host() {
    let mut meta = [0u8; 16];
    let mut program_id = [0u8; 32];
    // SAFETY: the host body reads and writes none of the pointers; the
    // buffers are there so the call has the shape a program gives it.
    let status = unsafe {
        crate::syscalls::sol_get_processed_sibling_instruction(
            0,
            meta.as_mut_ptr(),
            program_id.as_mut_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    assert_ne!(status, 0, "no transaction, so no sibling instruction");
    assert_eq!(meta, [0; 16]);
    assert_eq!(program_id, [0; 32]);
}
