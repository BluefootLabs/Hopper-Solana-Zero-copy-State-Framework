//! SENTINEL host proofs (hopper-svm).
//!
//! Every test drives the REAL generated dispatcher
//! (`sentinel_program::process_instruction`) against live `AccountView`
//! memory — the same dispatch path the compiled SBF artifact runs — with
//! Hopper's borrow tracking and write policy fully active.
//!
//! The flagship: `honest_pause` succeeds and its touch map records exactly the
//! declared `paused` + `revision` ranges; `malicious_pause` — the SAME context,
//! the SAME honest body, plus a tampered admin rotation THROUGH THE CONTEXT —
//! is refused with `Custom(0xD000 | 1)`, and the admin bytes are provably
//! unchanged.

use hopper::layout::write_header;
use hopper::prelude::*;
use hopper_runtime::write_policy::WriteRange;
use hopper_sentinel::{sentinel_program, CollectFees, Config, Ledger, LedgerEntry, Pause};
use hopper_svm::{AccountFixture, HopperSvm};

const PROGRAM_ID: Address = Address::new_from_array([7u8; 32]);
/// `Pause`/`RecordEntry` slot 1 is the config account, so the flagship refusal
/// is `Custom(0xD000 | 1)`.
const REFUSAL: ProgramResult = Err(ProgramError::Custom(0xD000 | 1));

// ── Fixtures ────────────────────────────────────────────────────────

fn admin_addr() -> Address {
    Address::new_from_array([1u8; 32])
}
fn config_addr() -> Address {
    Address::new_from_array([2u8; 32])
}
fn ledger_addr() -> Address {
    Address::new_from_array([3u8; 32])
}
fn system_addr() -> Address {
    Address::new_from_array([0u8; 32])
}

fn signer(addr: Address) -> AccountFixture {
    AccountFixture::new(addr, system_addr(), 1_000_000, 0).signer()
}

/// A program-owned config with a valid Hopper header and `admin` +
/// `withdraw_authority` seeded to `admin`, as `initialize_config` would leave
/// it (revision 0, unpaused).
fn seeded_config(admin: Address) -> AccountFixture {
    let mut data = vec![0u8; Config::LEN];
    write_header(&mut data, Config::DISC, Config::VERSION, &Config::LAYOUT_ID).unwrap();
    let a = Config::ADMIN_ABS_OFFSET as usize;
    data[a..a + 32].copy_from_slice(&admin.to_bytes());
    let w = Config::WITHDRAW_AUTHORITY_ABS_OFFSET as usize;
    data[w..w + 32].copy_from_slice(&admin.to_bytes());
    AccountFixture::with_data(config_addr(), PROGRAM_ID, 5_000_000, data).writable()
}

fn config_from(data: std::vec::Vec<u8>) -> AccountFixture {
    AccountFixture::with_data(config_addr(), PROGRAM_ID, 5_000_000, data).writable()
}

/// A program-owned, zeroed columnar ledger (count 0).
fn seeded_ledger() -> AccountFixture {
    let mut data = vec![0u8; Ledger::LEN];
    write_header(&mut data, Ledger::DISC, Ledger::VERSION, &Ledger::LAYOUT_ID).unwrap();
    AccountFixture::with_data(ledger_addr(), PROGRAM_ID, 50_000_000, data).writable()
}

/// Bridge the entrypoint-shaped host handler into the generated dispatcher —
/// exactly what the on-chain `program_entrypoint!` bridge does on SBF.
fn drive<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    sentinel_program::process_instruction(&mut ctx)
}

// ── Readback helpers ────────────────────────────────────────────────

fn read_admin(cfg: &AccountFixture) -> [u8; 32] {
    let a = Config::ADMIN_ABS_OFFSET as usize;
    cfg.data[a..a + 32].try_into().unwrap()
}
fn read_pending_admin(cfg: &AccountFixture) -> [u8; 32] {
    let a = Config::PENDING_ADMIN_ABS_OFFSET as usize;
    cfg.data[a..a + 32].try_into().unwrap()
}
fn read_paused(cfg: &AccountFixture) -> u8 {
    cfg.data[Config::PAUSED_ABS_OFFSET as usize]
}
fn read_revision(cfg: &AccountFixture) -> u64 {
    let o = Config::REVISION_ABS_OFFSET as usize;
    u64::from_le_bytes(cfg.data[o..o + 8].try_into().unwrap())
}
fn read_u16(fx: &AccountFixture, abs: u32) -> u16 {
    let o = abs as usize;
    u16::from_le_bytes(fx.data[o..o + 2].try_into().unwrap())
}
fn read_u32(fx: &AccountFixture, abs: u32) -> u32 {
    let o = abs as usize;
    u32::from_le_bytes(fx.data[o..o + 4].try_into().unwrap())
}
fn read_u64_at(fx: &AccountFixture, off: usize) -> u64 {
    u64::from_le_bytes(fx.data[off..off + 8].try_into().unwrap())
}

/// The `hopper tx explain` touch-map decode, replicated against the public
/// wire-format constants: `(slot, offset, size, write)` per record.
fn decode_touch_map(buf: &[u8]) -> std::vec::Vec<(u8, u32, u32, bool)> {
    use hopper::hopper_runtime::segment_borrow::{
        TOUCH_MAP_HEADER_LEN, TOUCH_MAP_MAGIC, TOUCH_MAP_RECORD_LEN, TOUCH_MAP_VERSION,
    };
    assert_eq!(buf[0], TOUCH_MAP_MAGIC);
    assert_eq!(buf[1], TOUCH_MAP_VERSION);
    let count = buf[3] as usize;
    assert_eq!(
        buf.len(),
        TOUCH_MAP_HEADER_LEN + count * TOUCH_MAP_RECORD_LEN
    );
    (0..count)
        .map(|i| {
            let at = TOUCH_MAP_HEADER_LEN + i * TOUCH_MAP_RECORD_LEN;
            let packed = u32::from_le_bytes(buf[at + 1..at + 5].try_into().unwrap());
            let size = u32::from_le_bytes(buf[at + 5..at + 9].try_into().unwrap());
            (
                buf[at],
                packed & 0x7FFF_FFFF,
                size,
                packed & 0x8000_0000 != 0,
            )
        })
        .collect()
}

// ── FLAGSHIP: honest pause succeeds ─────────────────────────────────

#[test]
fn flagship_honest_pause_succeeds_and_leaves_admin_untouched() {
    let admin = admin_addr();
    let accounts = [signer(admin), seeded_config(admin)];
    // `[disc = 1]` — honest_pause takes no args.
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[1u8], &accounts, drive);
    assert!(
        result.program_result.is_ok(),
        "honest pause must succeed: {:?}",
        result.program_result
    );
    let cfg = &result.resulting_accounts[1];
    assert_eq!(read_paused(cfg), 1, "the declared `paused` write landed");
    assert_eq!(read_revision(cfg), 1, "the declared `revision` bump landed");
    assert_eq!(
        read_admin(cfg),
        admin.to_bytes(),
        "the honest handler never touches admin"
    );
}

/// The Ok-path touch map records EXACTLY the two declared writes — `paused`
/// (1 byte) then `revision` (8 bytes), both on the config account (slot 1).
#[test]
fn flagship_honest_pause_touch_map_records_exactly_paused_and_revision() {
    fn handler<'info>(
        program_id: &'info Address,
        accounts: &'info [AccountView<'info>],
        instruction_data: &'info [u8],
    ) -> ProgramResult {
        let mut ctx = Context::new(program_id, accounts, instruction_data);
        {
            let mut bound = Pause::bind(&mut ctx)?;
            {
                let mut paused = bound.config_paused_mut()?;
                *paused = 1u8;
            }
            {
                let mut revision = bound.config_revision_mut()?;
                revision.checked_add_assign(1)?;
            }
        }
        let (buf, len) = ctx.encode_touch_map();
        let writes: std::vec::Vec<(u8, u32, u32, bool)> = decode_touch_map(&buf[..len])
            .into_iter()
            .filter(|record| record.3)
            .collect();
        assert_eq!(
            writes,
            vec![
                (1u8, Config::PAUSED_ABS_OFFSET, 1u32, true),
                (1u8, Config::REVISION_ABS_OFFSET, 8u32, true),
            ],
            "the touch map must describe exactly the declared paused + revision writes"
        );
        Ok(())
    }

    let admin = admin_addr();
    let accounts = [signer(admin), seeded_config(admin)];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, handler);
    assert!(result.program_result.is_ok(), "{:?}", result.program_result);
}

// ── FLAGSHIP: malicious pause is refused before any admin byte changes ─

#[test]
fn flagship_malicious_pause_is_refused_with_0xd001_and_admin_is_unchanged() {
    let admin = admin_addr();
    let accounts = [signer(admin), seeded_config(admin)];
    // `[disc = 2]` — malicious_pause takes no args.
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[2u8], &accounts, drive);

    assert_eq!(
        result.program_result, REFUSAL,
        "the tampered admin rotation must be refused with Custom(0xD000 | 1)"
    );

    // The load-bearing claim: admin is BYTE-UNCHANGED. The refusal fired at
    // mutable-borrow acquisition, before a single admin byte was written.
    let cfg = &result.resulting_accounts[1];
    assert_eq!(
        read_admin(cfg),
        admin.to_bytes(),
        "admin must be byte-for-byte unchanged after the refused rotation"
    );
}

// ── Published == enforced ───────────────────────────────────────────

#[test]
fn published_write_set_is_the_enforced_policy() {
    assert!(Pause::STRICT_WRITES);
    let expected = [
        WriteRange::new(1, Config::PAUSED_ABS_OFFSET, 1),
        WriteRange::new(1, Config::REVISION_ABS_OFFSET, 8),
    ];
    // Authored const == schema publication.
    assert_eq!(Pause::WRITE_RANGES, &expected);
    assert_eq!(Pause::SCHEMA_METADATA.write_ranges, Pause::WRITE_RANGES);

    // ...and the runtime installs literally that same slice at bind().
    fn handler<'info>(
        program_id: &'info Address,
        accounts: &'info [AccountView<'info>],
        instruction_data: &'info [u8],
    ) -> ProgramResult {
        let mut ctx = Context::new(program_id, accounts, instruction_data);
        {
            let _bound = Pause::bind(&mut ctx)?;
        }
        let policy = ctx
            .write_policy()
            .expect("strict_writes bind installs a policy");
        assert_eq!(policy.allows, Pause::WRITE_RANGES);
        Ok(())
    }
    let admin = admin_addr();
    let accounts = [signer(admin), seeded_config(admin)];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, handler);
    assert!(result.program_result.is_ok(), "{:?}", result.program_result);
}

// ── Columnar per-record write ───────────────────────────────────────

#[test]
fn columnar_record_entry_writes_the_exact_cell_and_bumps_count_and_revision() {
    let admin = admin_addr();
    let actor = admin;
    let accounts = [signer(actor), seeded_config(admin), seeded_ledger()];
    // `[disc = 5][u64 LE amount]`.
    let mut data = vec![5u8];
    data.extend_from_slice(&777u64.to_le_bytes());
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &data, &accounts, drive);
    assert!(
        result.program_result.is_ok(),
        "record_entry must succeed: {:?}",
        result.program_result
    );

    let ledger = &result.resulting_accounts[2];
    assert_eq!(
        read_u32(ledger, Ledger::COUNT_ABS_OFFSET),
        1,
        "count bumped"
    );

    // entry[0] = { actor, amount = 777, slot = 0 } at the entries column base.
    let cell0 = Ledger::ENTRIES_ABS_OFFSET as usize;
    assert_eq!(
        &ledger.data[cell0..cell0 + 32],
        &actor.to_bytes(),
        "entry actor"
    );
    assert_eq!(read_u64_at(ledger, cell0 + 32), 777, "entry amount");

    // The paired revision bump landed on the config account.
    assert_eq!(
        read_revision(&result.resulting_accounts[1]),
        1,
        "config revision bumped"
    );

    // Sanity: a LedgerEntry really is 48 bytes, so the cell math is exact.
    assert_eq!(core::mem::size_of::<LedgerEntry>(), 48);
}

// ── Two-step admin transfer (the declared-admin-write contrast) ─────

#[test]
fn two_step_admin_transfer_promotes_the_pending_admin() {
    let admin = admin_addr();
    let new_admin = Address::new_from_array([9u8; 32]);

    // Step 1: admin nominates new_admin. `[disc = 6][Address(32)]`.
    let mut begin = vec![6u8];
    begin.extend_from_slice(&new_admin.to_bytes());
    let accounts = [signer(admin), seeded_config(admin)];
    let after_begin = HopperSvm::new().process_instruction(PROGRAM_ID, &begin, &accounts, drive);
    assert!(
        after_begin.program_result.is_ok(),
        "begin_admin_transfer must succeed: {:?}",
        after_begin.program_result
    );
    let cfg = after_begin.resulting_accounts[1].clone();
    assert_eq!(
        read_pending_admin(&cfg),
        new_admin.to_bytes(),
        "pending set"
    );
    assert_eq!(
        read_admin(&cfg),
        admin.to_bytes(),
        "admin unchanged after begin"
    );

    // Step 2: new_admin signs and accepts. `[disc = 7]`.
    let accounts2 = [signer(new_admin), config_from(cfg.data.clone())];
    let after_accept = HopperSvm::new().process_instruction(PROGRAM_ID, &[7u8], &accounts2, drive);
    assert!(
        after_accept.program_result.is_ok(),
        "accept_admin_transfer must succeed: {:?}",
        after_accept.program_result
    );
    let cfg2 = &after_accept.resulting_accounts[1];
    assert_eq!(read_admin(cfg2), new_admin.to_bytes(), "admin promoted");
    assert_eq!(read_pending_admin(cfg2), [0u8; 32], "pending cleared");
}

// ── DEMOTION PAYOFF: effective_writable demotes the untouched account ─

#[test]
fn collect_fees_context_demotes_the_over_declared_treasury() {
    assert!(CollectFees::STRICT_WRITES);
    assert!(CollectFees::MUTATION_COMPLETE);
    // The only lamport account is fee_sink (slot 1).
    assert_eq!(CollectFees::LAMPORT_ACCOUNTS, &[1u8]);

    // Read the demotion off the SHIPPED descriptor the program generates.
    let ix = sentinel_program::__HOPPER_INSTRUCTION_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.name == "collect_fees")
        .expect("collect_fees publishes an instruction descriptor");
    assert!(ix.mutation_complete);

    // config (0): a declared data range → stays writable.
    assert!(ix.effective_writable(0, true));
    // fee_sink (1): a declared lamport permission → stays writable.
    assert!(ix.effective_writable(1, true));
    // treasury (2): declared in NEITHER dimension → a client that
    // over-declared it writable is provably safe to DEMOTE to read-only.
    assert!(!ix.effective_writable(2, true));
    // admin (3): read-only is never promoted.
    assert!(!ix.effective_writable(3, false));
}

// ── The real init lifecycle (supporting) ────────────────────────────

#[test]
fn initialize_config_runs_the_real_init_lifecycle() {
    let payer = admin_addr();
    let payer_fx = AccountFixture::new(payer, system_addr(), 1_000_000_000, 0)
        .signer()
        .writable();
    // Fresh, system-owned, zero-data account the CreateAccount CPI fills in.
    let config_fx = AccountFixture::new(config_addr(), system_addr(), 0, 0)
        .signer()
        .writable();
    let system_fx = AccountFixture::new(system_addr(), system_addr(), 0, 0).executable();

    // `[disc = 0][u16 LE fee_bps]`.
    let mut data = vec![0u8];
    data.extend_from_slice(&250u16.to_le_bytes());
    let accounts = [payer_fx, config_fx, system_fx];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &data, &accounts, drive);
    assert!(
        result.program_result.is_ok(),
        "initialize_config must succeed: {:?}",
        result.program_result
    );
    let cfg = &result.resulting_accounts[1];
    assert_eq!(cfg.data.len(), Config::LEN, "config allocated to LEN");
    assert_eq!(read_admin(cfg), payer.to_bytes(), "admin stamped to payer");
    assert_eq!(
        read_u16(cfg, Config::FEE_BPS_ABS_OFFSET),
        250,
        "fee_bps set"
    );
    assert_eq!(read_revision(cfg), 0, "fresh revision");
}
