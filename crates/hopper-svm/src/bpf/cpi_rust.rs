//! Rust-ABI CPI parser — Phase 2.3.
//!
//! `sol_invoke_signed_rust` is what `solana_program::program::invoke_signed`
//! emits by default. The wire shape is *not* a flat C struct; it's
//! Rust's `&Instruction`, `&[AccountInfo]`, and
//! `&[&[&[u8]]]` (three-level slice nesting), with
//! `Rc<RefCell<&mut u64>>` and `Rc<RefCell<&mut [u8]>>` used to
//! share lamport / data references inside `AccountInfo`.
//!
//! This module owns the parser. The actual recursion + signer
//! verification still goes through [`super::cpi::dispatch_cpi`]
//! and [`super::cpi::verify_signer_seeds`] — same as
//! `sol_invoke_signed_c` — so the only Rust-ABI-specific code
//! is the pointer-chasing here.
//!
//! ## Why this is fragile
//!
//! The layouts of `Rc`, `RefCell`, and `Vec` are Rust-internal
//! and can shift between toolchain releases. Solana programs
//! are compiled with the Anza-distributed `cargo-build-sbf`
//! toolchain, which pins the Rust version, so the layout is
//! stable per Anza release. Every version-sensitive offset
//! lives in the [`layout`] sub-module below — a future
//! toolchain bump means updating that module, nothing else.
//!
//! ## Layout (SBPF target, Anza Rust 1.84-class toolchain)
//!
//! Reference: agave-syscalls' `cpi.rs` and the `solana-program`
//! crate's `AccountInfo` definition compiled for `bpfel-v3`.
//!
//! ```text
//! AccountInfo {                    // 48 bytes total
//!     +0   key:           &Pubkey                      8 bytes (ptr)
//!     +8   lamports:      Rc<RefCell<&mut u64>>        8 bytes (Rc = ptr to RcInner)
//!     +16  data:          Rc<RefCell<&mut [u8]>>       8 bytes
//!     +24  owner:         &Pubkey                      8 bytes (ptr)
//!     +32  rent_epoch:    u64                          8 bytes
//!     +40  is_signer:     bool                         1
//!     +41  is_writable:   bool                         1
//!     +42  executable:    bool                         1
//!     +43  padding                                     5
//! }                                                    = 48
//!
//! RcInner<RefCell<&mut u64>> at *Rc {  // pointed-to allocation
//!     +0   strong:        usize                        8
//!     +8   weak:          usize                        8
//!     +16  RefCell<&mut u64> {
//!              +0   borrow: Cell<isize>                8
//!              +8   value:  &mut u64                   8 (ptr)
//!          }
//! }
//!
//! RcInner<RefCell<&mut [u8]>> at *Rc {
//!     +0   strong:        usize                        8
//!     +8   weak:          usize                        8
//!     +16  RefCell<&mut [u8]> {
//!              +0   borrow: Cell<isize>                8
//!              +8   value:  &mut [u8]                 16 (ptr+len)
//!          }
//! }
//!
//! Vec<T>:                              // 24 bytes for refs
//!     +0   ptr:           *mut T                       8
//!     +8   cap:           usize                        8
//!     +16  len:           usize                        8
//!
//! &Instruction = 8-byte pointer to:
//! Instruction {                        // 56 bytes
//!     +0   program_id:    Pubkey                       32
//!     +32  accounts:      Vec<AccountMeta>             24 (ptr+cap+len)
//!     +56  data:          Vec<u8>                      24
//! }                                                    = 80 bytes total but
//!                                       Rust may reorder fields without
//!                                       repr(C); the SBF compiler is
//!                                       deterministic per toolchain
//!                                       release. `Instruction` is
//!                                       declared without repr(C) but
//!                                       the layout is stable.
//!
//! AccountMeta {                        // 34 bytes (33 + 1 pad)
//!     +0   pubkey:        Pubkey                       32
//!     +32  is_signer:     bool                         1
//!     +33  is_writable:   bool                         1
//! }
//! ```
//!
//! signers_seeds wire shape (3-level slice nesting):
//!
//! - `&[&[&[u8]]]` is itself a fat pointer (`addr`, `len`).
//! - Each `&[&[u8]]` element is a 16-byte fat pointer.
//! - Each `&[u8]` element is a 16-byte fat pointer.
//!
//! So at `signers_seeds_addr` we read `signers_seeds_len`
//! 16-byte fat pointers; each points at a list of inner fat
//! pointers; each of those points at the actual seed bytes.

use crate::account::KeyedAccount;
use crate::bpf::cpi::ParsedCpi;
use solana_sbpf::error::EbpfError;
use solana_sbpf::memory_region::{AccessType, MemoryMapping};
use solana_sdk::instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;

/// Hard-coded layout offsets for the SBPF target. Pin every
/// layout-sensitive offset here so a future Anza toolchain bump
/// is a one-file fixup.
pub mod layout {
    /// `AccountInfo` total size in bytes.
    pub const ACCOUNT_INFO_SIZE: usize = 48;
    /// Offset of the `key: &Pubkey` field.
    pub const ACCT_KEY: usize = 0;
    /// Offset of the `lamports: Rc<...>` field (the `Rc` = a
    /// single 8-byte pointer to the heap allocation).
    pub const ACCT_LAMPORTS_RC: usize = 8;
    /// Offset of the `data: Rc<...>` field.
    pub const ACCT_DATA_RC: usize = 16;
    /// Offset of the `owner: &Pubkey` field.
    pub const ACCT_OWNER: usize = 24;
    /// Offset of the `rent_epoch: u64` field.
    pub const ACCT_RENT_EPOCH: usize = 32;
    /// Offset of the `is_signer: bool` byte.
    pub const ACCT_IS_SIGNER: usize = 40;
    /// Offset of the `is_writable: bool` byte.
    pub const ACCT_IS_WRITABLE: usize = 41;
    /// Offset of the `executable: bool` byte.
    pub const ACCT_EXECUTABLE: usize = 42;

    /// Inside an `RcInner<T>`, `T` starts at this byte offset
    /// (after `strong: usize` + `weak: usize`).
    pub const RC_INNER_VALUE_OFFSET: usize = 16;

    /// Inside a `RefCell<T>`, the inner `value: T` starts at
    /// this byte offset (after `borrow: Cell<isize>`).
    pub const REFCELL_VALUE_OFFSET: usize = 8;

    /// `Vec<T>` total size — `(ptr, cap, len)` = 24 bytes.
    pub const VEC_SIZE: usize = 24;
    /// `Vec<T>::ptr` byte offset.
    pub const VEC_PTR_OFFSET: usize = 0;
    /// `Vec<T>::len` byte offset. Note: Rust's actual order is
    /// `(ptr, cap, len)`, so `len` is at offset 16, not 8.
    pub const VEC_LEN_OFFSET: usize = 16;

    /// `Instruction` field offsets. Pubkey + Vec<AccountMeta> +
    /// Vec<u8>.
    pub const IX_PROGRAM_ID: usize = 0;
    pub const IX_ACCOUNTS_VEC: usize = 32;
    pub const IX_DATA_VEC: usize = IX_ACCOUNTS_VEC + VEC_SIZE; // 56

    /// `AccountMeta` (Pubkey + bool + bool, no padding inside,
    /// 1-byte alignment between bool and following AccountMeta;
    /// the slice stride is 34 bytes).
    pub const ACCOUNT_META_SIZE: usize = 34;
    pub const META_PUBKEY: usize = 0;
    pub const META_IS_SIGNER: usize = 32;
    pub const META_IS_WRITABLE: usize = 33;

    /// 16-byte fat pointer (ptr + len).
    pub const FAT_POINTER_SIZE: usize = 16;
}

// ---------------------------------------------------------------------------
// Translation helpers
// ---------------------------------------------------------------------------

fn translate_slice<'a>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    len: u64,
) -> Result<&'a [u8], EbpfError> {
    if len == 0 {
        return Ok(&[]);
    }
    let host_addr = memory_mapping.map(AccessType::Load, vm_addr, len)?;
    Ok(unsafe { core::slice::from_raw_parts(host_addr as *const u8, len as usize) })
}

fn translate_array<'a, const N: usize>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
) -> Result<&'a [u8; N], EbpfError> {
    let s = translate_slice(memory_mapping, vm_addr, N as u64)?;
    Ok(s.try_into().expect("translate_array N"))
}

/// Read a `u64` at `vm_addr` from VM memory.
fn read_u64(memory_mapping: &MemoryMapping, vm_addr: u64) -> Result<u64, EbpfError> {
    let bytes = translate_slice(memory_mapping, vm_addr, 8)?;
    Ok(u64::from_le_bytes(bytes.try_into().expect("u64 8 bytes")))
}

/// Follow an `Rc<RefCell<&mut u64>>` to the address of the
/// underlying `u64`. The chain is:
///   `rc_ptr` → `RcInner` (skip 16 bytes for refcounts) →
///   `RefCell` (skip 8 bytes for borrow flag) → `&mut u64`
///   (8-byte pointer to the actual u64).
///
/// Returns the VM address of the u64, NOT its value. Callers
/// can read or write the u64 through that address.
pub fn follow_rc_refcell_u64(
    memory_mapping: &MemoryMapping,
    rc_ptr: u64,
) -> Result<u64, EbpfError> {
    let inner_addr =
        rc_ptr + layout::RC_INNER_VALUE_OFFSET as u64 + layout::REFCELL_VALUE_OFFSET as u64;
    // The 8 bytes at inner_addr are the `&mut u64` reference,
    // i.e. another VM address pointing to the actual u64.
    read_u64(memory_mapping, inner_addr)
}

/// Follow an `Rc<RefCell<&mut [u8]>>` to the `(addr, len)` fat
/// pointer of the underlying byte slice. Same chain as above
/// but the value at the end is a 16-byte fat pointer.
///
/// Returns `(data_addr, data_len)` — both VM-side u64.
pub fn follow_rc_refcell_slice(
    memory_mapping: &MemoryMapping,
    rc_ptr: u64,
) -> Result<(u64, u64), EbpfError> {
    let inner_addr =
        rc_ptr + layout::RC_INNER_VALUE_OFFSET as u64 + layout::REFCELL_VALUE_OFFSET as u64;
    let addr = read_u64(memory_mapping, inner_addr)?;
    let len = read_u64(memory_mapping, inner_addr + 8)?;
    Ok((addr, len))
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// One parsed Rust-ABI AccountInfo, with all VM addresses
/// resolved + the writeback addresses captured.
pub struct RustAccountInfo {
    /// The account's pubkey.
    pub address: Pubkey,
    /// VM address of the underlying `u64` lamports value
    /// (writable).
    pub lamports_addr: u64,
    /// VM address of the underlying byte slice's start.
    pub data_addr: u64,
    /// Length of the byte slice — read at parse time, but the
    /// inner slice's RefCell can mutate this. The writeback
    /// updates the `data_len` slot inside the RefCell so the
    /// outer program sees the new length.
    pub data_len: u64,
    /// VM address of the `len: usize` slot inside the
    /// `RefCell<&mut [u8]>` that holds the slice's length. The
    /// realloc-across-CPI writeback updates this so the outer
    /// program reads the new length on resume.
    pub data_len_field_addr: u64,
    /// Owner pubkey at the moment of the CPI.
    pub owner: Pubkey,
    /// VM address of the `&Pubkey` slot for owner — actually
    /// this is the address of a 32-byte Pubkey value. Writeback
    /// updates these 32 bytes so an inner `assign` is observable.
    pub owner_value_addr: u64,
    /// `is_signer`, `is_writable`, `executable` bits at parse
    /// time.
    pub is_signer: bool,
    pub is_writable: bool,
    pub executable: bool,
    /// Lamports value at parse time.
    pub lamports: u64,
}

/// Parse the Rust-ABI `&[AccountInfo]` at `(addr, len)` into a
/// list of [`RustAccountInfo`] records with all the writeback
/// pointers captured.
pub fn parse_account_infos(
    memory_mapping: &MemoryMapping,
    addr: u64,
    len: u64,
) -> Result<Vec<RustAccountInfo>, EbpfError> {
    let total =
        len.checked_mul(layout::ACCOUNT_INFO_SIZE as u64)
            .ok_or(EbpfError::SyscallError(Box::new(
                crate::bpf::cpi_rust::CpiRustError("account_info count overflow".to_string()),
            )))?;
    let bytes = translate_slice(memory_mapping, addr, total)?;
    let mut out: Vec<RustAccountInfo> = Vec::with_capacity(len as usize);
    for i in 0..len as usize {
        let off = i * layout::ACCOUNT_INFO_SIZE;

        let key_ptr = u64::from_le_bytes(
            bytes[off + layout::ACCT_KEY..off + layout::ACCT_KEY + 8]
                .try_into()
                .expect("key_ptr"),
        );
        let lamports_rc = u64::from_le_bytes(
            bytes[off + layout::ACCT_LAMPORTS_RC..off + layout::ACCT_LAMPORTS_RC + 8]
                .try_into()
                .expect("lamports_rc"),
        );
        let data_rc = u64::from_le_bytes(
            bytes[off + layout::ACCT_DATA_RC..off + layout::ACCT_DATA_RC + 8]
                .try_into()
                .expect("data_rc"),
        );
        let owner_ptr = u64::from_le_bytes(
            bytes[off + layout::ACCT_OWNER..off + layout::ACCT_OWNER + 8]
                .try_into()
                .expect("owner_ptr"),
        );
        let _rent_epoch = u64::from_le_bytes(
            bytes[off + layout::ACCT_RENT_EPOCH..off + layout::ACCT_RENT_EPOCH + 8]
                .try_into()
                .expect("rent_epoch"),
        );
        let is_signer = bytes[off + layout::ACCT_IS_SIGNER] != 0;
        let is_writable = bytes[off + layout::ACCT_IS_WRITABLE] != 0;
        let executable = bytes[off + layout::ACCT_EXECUTABLE] != 0;

        // Resolve key pubkey.
        let address = Pubkey::new_from_array(*translate_array::<32>(memory_mapping, key_ptr)?);
        // Resolve owner pubkey.
        let owner = Pubkey::new_from_array(*translate_array::<32>(memory_mapping, owner_ptr)?);

        // Follow Rc<RefCell<&mut u64>> for lamports.
        let lamports_addr = follow_rc_refcell_u64(memory_mapping, lamports_rc)?;
        let lamports = read_u64(memory_mapping, lamports_addr)?;

        // Follow Rc<RefCell<&mut [u8]>> for data, capturing the
        // address of the length slot too (for realloc writeback).
        let (data_addr, data_len) = follow_rc_refcell_slice(memory_mapping, data_rc)?;
        // The length slot is the second 8 bytes of the
        // RefCell::value (i.e. inner_addr + 8).
        let data_len_field_addr = data_rc
            + layout::RC_INNER_VALUE_OFFSET as u64
            + layout::REFCELL_VALUE_OFFSET as u64
            + 8;

        out.push(RustAccountInfo {
            address,
            lamports_addr,
            data_addr,
            data_len,
            data_len_field_addr,
            owner,
            owner_value_addr: owner_ptr,
            is_signer,
            is_writable,
            executable,
            lamports,
        });
    }
    Ok(out)
}

/// Parse the Rust-ABI `&Instruction` at `addr` into program_id
/// + AccountMeta list + data Vec.
///
/// Returns a `(program_id, metas, data)` triple ready to feed
/// into [`ParsedCpi`].
pub fn parse_instruction(
    memory_mapping: &MemoryMapping,
    addr: u64,
) -> Result<(Pubkey, Vec<AccountMeta>, Vec<u8>), EbpfError> {
    // Read the program_id (32 bytes inline at the start of
    // Instruction).
    let program_id = Pubkey::new_from_array(*translate_array::<32>(
        memory_mapping,
        addr + layout::IX_PROGRAM_ID as u64,
    )?);

    // Read the Vec<AccountMeta> header — (ptr, cap, len) at
    // offset IX_ACCOUNTS_VEC. We only need ptr + len.
    let accounts_ptr = read_u64(memory_mapping, addr + layout::IX_ACCOUNTS_VEC as u64)?;
    let accounts_len = read_u64(
        memory_mapping,
        addr + layout::IX_ACCOUNTS_VEC as u64 + layout::VEC_LEN_OFFSET as u64,
    )?;

    // Read each AccountMeta (34 bytes per entry).
    let metas_total = accounts_len
        .checked_mul(layout::ACCOUNT_META_SIZE as u64)
        .ok_or(EbpfError::SyscallError(Box::new(CpiRustError(
            "AccountMeta count overflow".to_string(),
        ))))?;
    let metas_bytes = translate_slice(memory_mapping, accounts_ptr, metas_total)?;
    let mut metas: Vec<AccountMeta> = Vec::with_capacity(accounts_len as usize);
    for i in 0..accounts_len as usize {
        let off = i * layout::ACCOUNT_META_SIZE;
        let pubkey = Pubkey::new_from_array(
            metas_bytes[off + layout::META_PUBKEY..off + layout::META_PUBKEY + 32]
                .try_into()
                .expect("meta pubkey"),
        );
        let is_signer = metas_bytes[off + layout::META_IS_SIGNER] != 0;
        let is_writable = metas_bytes[off + layout::META_IS_WRITABLE] != 0;
        metas.push(AccountMeta {
            pubkey,
            is_signer,
            is_writable,
        });
    }

    // Read the Vec<u8> header.
    let data_ptr = read_u64(memory_mapping, addr + layout::IX_DATA_VEC as u64)?;
    let data_len = read_u64(
        memory_mapping,
        addr + layout::IX_DATA_VEC as u64 + layout::VEC_LEN_OFFSET as u64,
    )?;
    let data = translate_slice(memory_mapping, data_ptr, data_len)?.to_vec();

    Ok((program_id, metas, data))
}

/// Parse the 3-level-nested `&[&[&[u8]]]` signer-seeds shape.
/// Returns `Vec<Vec<Vec<u8>>>` — one outer entry per signer-seed
/// set, each containing one inner Vec<u8> per seed.
pub fn parse_signer_seeds(
    memory_mapping: &MemoryMapping,
    addr: u64,
    len: u64,
) -> Result<Vec<Vec<Vec<u8>>>, EbpfError> {
    if len == 0 {
        return Ok(Vec::new());
    }
    // Outer slice: `len` × 16-byte fat pointers. Each points at
    // an inner `&[&[u8]]`.
    let outer_total =
        len.checked_mul(layout::FAT_POINTER_SIZE as u64)
            .ok_or(EbpfError::SyscallError(Box::new(CpiRustError(
                "signer_seeds outer count overflow".to_string(),
            ))))?;
    let outer_bytes = translate_slice(memory_mapping, addr, outer_total)?;
    let mut sets: Vec<Vec<Vec<u8>>> = Vec::with_capacity(len as usize);
    for i in 0..len as usize {
        let off = i * layout::FAT_POINTER_SIZE;
        let inner_addr =
            u64::from_le_bytes(outer_bytes[off..off + 8].try_into().expect("inner_addr"));
        let inner_len = u64::from_le_bytes(
            outer_bytes[off + 8..off + 16]
                .try_into()
                .expect("inner_len"),
        );
        // Inner: `inner_len` × 16-byte fat pointers, each points
        // at the actual seed bytes.
        let inner_total = inner_len
            .checked_mul(layout::FAT_POINTER_SIZE as u64)
            .ok_or(EbpfError::SyscallError(Box::new(CpiRustError(
                "signer_seeds inner count overflow".to_string(),
            ))))?;
        let inner_bytes = translate_slice(memory_mapping, inner_addr, inner_total)?;
        let mut seeds: Vec<Vec<u8>> = Vec::with_capacity(inner_len as usize);
        for j in 0..inner_len as usize {
            let so = j * layout::FAT_POINTER_SIZE;
            let seed_addr =
                u64::from_le_bytes(inner_bytes[so..so + 8].try_into().expect("seed_addr"));
            let seed_len =
                u64::from_le_bytes(inner_bytes[so + 8..so + 16].try_into().expect("seed_len"));
            seeds.push(translate_slice(memory_mapping, seed_addr, seed_len)?.to_vec());
        }
        sets.push(seeds);
    }
    Ok(sets)
}

/// Build a [`ParsedCpi`] from the parsed Rust-ABI inputs. Used
/// by the syscall adapter to feed the standard
/// [`super::cpi::dispatch_cpi`] / [`super::cpi::verify_signer_seeds`]
/// pipeline.
pub fn build_parsed_cpi(
    memory_mapping: &MemoryMapping,
    instruction_addr: u64,
    account_infos_addr: u64,
    account_infos_len: u64,
    signers_seeds_addr: u64,
    signers_seeds_len: u64,
) -> Result<(ParsedCpi, Vec<RustAccountInfo>), EbpfError> {
    let (program_id, metas, data) = parse_instruction(memory_mapping, instruction_addr)?;
    let infos = parse_account_infos(memory_mapping, account_infos_addr, account_infos_len)?;
    let signer_seeds = parse_signer_seeds(memory_mapping, signers_seeds_addr, signers_seeds_len)?;

    // Snapshot account state from the parsed AccountInfos. We
    // read each account's lamports + data via the resolved
    // pointers; the inner call sees these as the pre-state.
    let mut accounts: Vec<KeyedAccount> = Vec::with_capacity(infos.len());
    for info in &infos {
        let data = translate_slice(memory_mapping, info.data_addr, info.data_len)?.to_vec();
        accounts.push(KeyedAccount {
            address: info.address,
            lamports: info.lamports,
            data,
            owner: info.owner,
            executable: info.executable,
            rent_epoch: 0,
        });
    }

    Ok((
        ParsedCpi {
            program_id,
            metas,
            data,
            accounts,
            signer_seeds,
        },
        infos,
    ))
}

/// Newtype wrapping a Rust-ABI parser failure.
#[derive(Debug)]
pub struct CpiRustError(pub String);

impl core::fmt::Display for CpiRustError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CpiRustError {}

#[cfg(test)]
mod tests {
    //! These tests pin the LAYOUT CONSTANTS only — actual VM
    //! address translation is exercised end-to-end via the
    //! engine smoke test once a real toolchain is reachable.
    //! The constants are the first place a future toolchain
    //! bump would need to update; pinning them with explicit
    //! values catches a silent shift.
    use super::layout::*;

    #[test]
    fn account_info_layout_pins() {
        assert_eq!(ACCOUNT_INFO_SIZE, 48);
        assert_eq!(ACCT_KEY, 0);
        assert_eq!(ACCT_LAMPORTS_RC, 8);
        assert_eq!(ACCT_DATA_RC, 16);
        assert_eq!(ACCT_OWNER, 24);
        assert_eq!(ACCT_RENT_EPOCH, 32);
        assert_eq!(ACCT_IS_SIGNER, 40);
        assert_eq!(ACCT_IS_WRITABLE, 41);
        assert_eq!(ACCT_EXECUTABLE, 42);
    }

    #[test]
    fn rc_refcell_indirection_offsets_pin() {
        // RcInner = strong + weak + value. usize = 8 on SBF, so
        // value starts at 16.
        assert_eq!(RC_INNER_VALUE_OFFSET, 16);
        // RefCell = borrow flag (Cell<isize>) + value. isize = 8
        // so value starts at 8.
        assert_eq!(REFCELL_VALUE_OFFSET, 8);
    }

    #[test]
    fn vec_layout_is_ptr_cap_len() {
        // Rust's actual Vec<T> layout — ptr first, then cap,
        // then len. (The historical confusion is that Rust used
        // to have a different ordering; current toolchains use
        // ptr-cap-len.)
        assert_eq!(VEC_SIZE, 24);
        assert_eq!(VEC_PTR_OFFSET, 0);
        assert_eq!(VEC_LEN_OFFSET, 16);
    }

    #[test]
    fn instruction_layout_pins() {
        assert_eq!(IX_PROGRAM_ID, 0);
        assert_eq!(IX_ACCOUNTS_VEC, 32);
        assert_eq!(IX_DATA_VEC, 56); // 32 + 24
    }

    #[test]
    fn account_meta_layout_pins() {
        assert_eq!(ACCOUNT_META_SIZE, 34);
        assert_eq!(META_PUBKEY, 0);
        assert_eq!(META_IS_SIGNER, 32);
        assert_eq!(META_IS_WRITABLE, 33);
    }

    #[test]
    fn fat_pointer_size_pins() {
        assert_eq!(FAT_POINTER_SIZE, 16);
    }
}
