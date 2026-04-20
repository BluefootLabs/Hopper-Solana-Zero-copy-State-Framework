//! # Hopper Registry Example
//!
//! Demonstrates Hopper's advanced features in a single program:
//!
//! - Segmented accounts with typed overlay access
//! - Validation pipeline with composable declarative checks
//! - State diff engine for field-level change tracking and audit logging
//! - Virtual state for multi-account logical views
//!
//! ## Account Layout
//!
//! ```text
//! Registry account (segmented):
//!   [Header: 16 bytes]
//!   [Segment Registry: 4 + 3x16 = 52 bytes]
//!   [Core Segment: 96 bytes]     -- authority, name, entry_count, version
//!   [Entries Segment: 512 bytes] -- up to 8 registry entries (64 bytes each)
//!   [Audit Segment: 256 bytes]   -- journal of recent operations
//! ```
//!
//! ## Instructions
//!
//! - `0` = InitRegistry: create and initialize segmented account
//! - `1` = AddEntry: add an entry with dedup check and audit trail
//! - `2` = ReadVirtual: demonstrate virtual state across multiple accounts

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;
use hopper::hopper_core::account;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

// --- Layouts ---

// Core segment: registry metadata
hopper_layout! {
    pub struct RegistryCore, disc = 10, version = 1 {
        authority:   TypedAddress<Authority> = 32,
        name:        [u8; 32]               = 32,
        entry_count: WireU32                = 4,
        max_entries: WireU32                = 4,
        version:     WireU16                = 2,
        flags:       WireU16                = 2,
    }
}

// A single registry entry
hopper_layout! {
    pub struct RegistryEntry, disc = 11, version = 1 {
        key:       [u8; 32]               = 32,
        value:     [u8; 16]               = 16,
        timestamp: WireU64                = 8,
        creator:   TypedAddress<Authority> = 32,
    }
}

// Audit log entry (for journal)
hopper_layout! {
    pub struct AuditEntry, disc = 12, version = 1 {
        actor:     TypedAddress<Authority> = 32,
        action:    WireU32                = 4,
        timestamp: WireU64                = 8,
        data_hash: [u8; 8]                = 8,
    }
}

// Lean audit record for journal storage (no header overhead).
// Journal entries don't need the 16-byte account header since they
// live inside an already-typed segment. 52 bytes per record gives
// 4 entries in a 256-byte circular journal.
#[derive(Clone, Copy)]
#[repr(C)]
struct AuditRecord {
    actor:     [u8; 32],
    action:    [u8; 4],
    timestamp: [u8; 8],
    data_hash: [u8; 8],
}

const _: () = assert!(core::mem::size_of::<AuditRecord>() == 52);
const _: () = assert!(core::mem::align_of::<AuditRecord>() == 1);

// Hopper's Pod supertrait requires bytemuck; both impls are safe
// because `#[repr(C)]` of byte-array fields is bytemuck-safe.
unsafe impl hopper::hopper_runtime::__hopper_native::bytemuck::Zeroable for AuditRecord {}
unsafe impl hopper::hopper_runtime::__hopper_native::bytemuck::Pod for AuditRecord {}
// SAFETY: #[repr(C)] of byte arrays, all bit patterns valid, align == 1.
unsafe impl Pod for AuditRecord {}
impl FixedLayout for AuditRecord {
    const SIZE: usize = 52;
}

const ACTION_ADD_ENTRY: [u8; 4] = 1u32.to_le_bytes();

// Segment IDs (const FNV-1a hashes)
const CORE_SEG: SegmentId = segment_id("core");
const ENTRIES_SEG: SegmentId = segment_id("entries");
const AUDIT_SEG: SegmentId = segment_id("audit");

// Segment sizes
const CORE_SIZE: u32 = 96;
const ENTRIES_SIZE: u32 = 512;
const AUDIT_SIZE: u32 = 256;

// Total account size = header(16) + registry_header(4) + 3 entries(48) + segment data
const REGISTRY_ACCOUNT_SIZE: usize = {
    HEADER_LEN
        + account::registry::REGISTRY_HEADER_SIZE
        + 3 * account::registry::SEGMENT_ENTRY_SIZE
        + CORE_SIZE as usize
        + ENTRIES_SIZE as usize
        + AUDIT_SIZE as usize
};

// --- Errors ---

hopper_error! {
    base = 7000;
    Unauthorized,
    RegistryFull,
    DuplicateKey,
    InvalidSegment,
    EntryNotFound
}

// --- Entrypoint ---

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init_registry,
        1 => process_add_entry,
        2 => process_read_virtual,
    }
}

// --- Init Registry ---

fn process_init_registry(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let payer = &accounts[0];
    let registry_account = &accounts[1];
    let _system_program = &accounts[2];

    // Validation pipeline
    // Compose all precondition checks declaratively:
    hopper_validate! {
        accounts = accounts,
        program_id = program_id,
        data = data,
        rules {
            require_signer_at(0),
            require_writable_at(1),
            require_data_min(0)
        }
    }?;

    // Create account via CPI
    let lamports = rent_exempt_min(REGISTRY_ACCOUNT_SIZE);
    let space = REGISTRY_ACCOUNT_SIZE as u64;

    hopper::hopper_system::CreateAccount {
        from: payer,
        to: registry_account,
        lamports,
        space,
        owner: program_id,
    }
    .invoke()?;

    let mut account_data = registry_account.try_borrow_mut()?;

    // Zero-init + write header
    zero_init(&mut account_data);
    write_header(&mut account_data, 10, 1, &RegistryCore::LAYOUT_ID)?;

    // Initialize segments
    let specs: &[(SegmentId, u32, u8)] = &[
        (CORE_SEG, CORE_SIZE, 1),
        (ENTRIES_SEG, ENTRIES_SIZE, 1),
        (AUDIT_SEG, AUDIT_SIZE, 1),
    ];
    SegmentRegistryMut::init(&mut account_data, specs)?;

    // Write core segment
    let mut reg_mut = SegmentRegistryMut::from_account_mut(&mut account_data)?;
    let core_data_mut = reg_mut.segment_data_mut(&CORE_SEG)?;

    // Core data is already zeroed from account zero_init
    if core_data_mut.len() >= RegistryCore::LEN {
        // Write a mini header for the segment overlay
        write_header(core_data_mut, RegistryCore::DISC, RegistryCore::VERSION, &RegistryCore::LAYOUT_ID)?;
        let core = RegistryCore::overlay_mut(core_data_mut)?;
        core.authority = TypedAddress::from_account(payer);
        // Copy name from instruction data if provided
        if data.len() >= 32 {
            core.name.copy_from_slice(&data[..32]);
        }
        core.entry_count = WireU32::new(0);
        core.max_entries = WireU32::new(8); // 512 / 64 (entry LEN) = 8 entries max
        core.version = WireU16::new(1);
        core.flags = WireU16::new(0);
    }

    // Initialize audit segment as a circular journal.
    // Circular mode wraps around when full, keeping the most recent entries.
    let audit_data = reg_mut.segment_data_mut(&AUDIT_SEG)?;
    let mut journal = Journal::<AuditRecord>::from_bytes_mut(audit_data)?;
    journal.init(true);

    emit_slices(&[b"registry_init"]);

    Ok(())
}

// --- Add Entry ---

fn process_add_entry(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let authority = &accounts[0];
    let registry_account = &accounts[1];

    // Validation pipeline: composable checks
    hopper_validate! {
        accounts = accounts,
        program_id = program_id,
        data = data,
        rules {
            require_signer_at(0),
            require_writable_at(1),
            require_owned_at(1),
            require_data_min(48)
        }
    }?;

    // Need at least 48 bytes: key(32) + value(16)
    if data.len() < 48 {
        return Err(ProgramError::InvalidInstructionData);
    }

    let mut account_data = registry_account.try_borrow_mut()?;

    // Capture snapshot before mutation for audit diff
    let snapshot = StateSnapshot::<864>::capture(&account_data);

    let mut reg = SegmentRegistryMut::from_account_mut(&mut account_data)?;
    let core_data = reg.segment_data_mut(&CORE_SEG)?;

    if core_data.len() < RegistryCore::LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }

    let core = RegistryCore::overlay_mut(core_data)?;

    // Check authority
    if !core.authority.eq_account(authority) {
        return Err(Unauthorized.into());
    }

    let count = core.entry_count.get();
    let max = core.max_entries.get();
    if count >= max {
        return Err(RegistryFull.into());
    }

    // Duplicate key scan: check all existing entries for matching key
    let new_key = &data[..32];
    let entries_data_ro = reg.segment_data_mut(&ENTRIES_SEG)?;
    {
        let mut i = 0u32;
        while (i as usize) < count as usize {
            let off = i as usize * RegistryEntry::LEN;
            let end = off + RegistryEntry::LEN;
            if end <= entries_data_ro.len() {
                if let Ok(existing) = RegistryEntry::overlay(&entries_data_ro[off..end]) {
                    if existing.key == *new_key {
                        return Err(DuplicateKey.into());
                    }
                }
            }
            i += 1;
        }
    }

    // Write the new entry
    let entry_offset = count as usize * RegistryEntry::LEN;
    let entry_end = entry_offset + RegistryEntry::LEN;

    if entry_end > entries_data_ro.len() {
        return Err(RegistryFull.into());
    }

    let entry_slice = &mut entries_data_ro[entry_offset..entry_end];
    write_header(entry_slice, RegistryEntry::DISC, RegistryEntry::VERSION, &RegistryEntry::LAYOUT_ID)?;
    let entry = RegistryEntry::overlay_mut(entry_slice)?;
    entry.key.copy_from_slice(new_key);
    entry.value.copy_from_slice(&data[32..48]);
    entry.timestamp = WireU64::new(0); // In production: use Clock sysvar
    entry.creator = TypedAddress::from_account(authority);

    // Update count in core (need to re-borrow since entries_data consumed the mut ref)
    let core_data2 = reg.segment_data_mut(&CORE_SEG)?;
    let core2 = RegistryCore::overlay_mut(core_data2)?;
    core2.entry_count = WireU32::new(count + 1);

    // Compute diff in a scoped block so the immutable borrow is released
    // before we take a fresh mutable borrow for the journal write.
    let changed_bytes = {
        let diff = snapshot.diff(&account_data);
        if diff.has_changes() {
            diff.changed_byte_count() as u64
        } else {
            0u64
        }
    };

    // Write audit record to the journal segment
    {
        let mut reg2 = SegmentRegistryMut::from_account_mut(&mut account_data)?;
        let audit_data = reg2.segment_data_mut(&AUDIT_SEG)?;
        let mut journal = Journal::<AuditRecord>::from_bytes_mut(audit_data)?;

        let mut data_hash = [0u8; 8];
        data_hash.copy_from_slice(&changed_bytes.to_le_bytes());

        let record = AuditRecord {
            actor: {
                let mut a = [0u8; 32];
                a.copy_from_slice(authority.address().as_ref());
                a
            },
            action: ACTION_ADD_ENTRY,
            timestamp: [0u8; 8], // In production: Clock sysvar
            data_hash,
        };
        journal.append(record)?;
    }

    emit_slices(&[
        b"entry_added",
        new_key,
    ]);

    Ok(())
}

// --- Read Virtual ---
//
// Read across multiple accounts as a single logical entity.

fn process_read_virtual(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    // Virtual state: map 3 accounts into a logical view
    let vstate = VirtualState::<3>::new()
        .map(0, 0)   // Slot 0 -> Account 0 (owned, read-only)
        .map(1, 1)   // Slot 1 -> Account 1 (owned, read-only)
        .map_foreign(2, 2);  // Slot 2 -> Account 2 (foreign read)

    // Validate all slot constraints
    vstate.validate(accounts, program_id)?;

    // Read from virtual slots -- each slot overlays a different account
    // This demonstrates unified access across sharded state
    let _registry_a_data = vstate.data(accounts, 0)?;
    let _registry_b_data = vstate.data(accounts, 1)?;

    // In a real scenario, you'd aggregate data across these virtual views
    // For example, counting total entries across multiple registry shards

    emit_slices(&[b"virtual_read"]);

    Ok(())
}
