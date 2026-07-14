//! # SENTINEL — the byte-segment authority a compromised handler cannot escape
//!
//! The one thing SENTINEL proves, on real compiled bytecode:
//!
//! > A privileged handler can mutate ONLY the byte ranges it declared. A
//! > COMPROMISED handler that tries to write outside its declared segment is
//! > REFUSED BY THE FRAMEWORK at mutable-borrow acquisition — BEFORE any byte
//! > changes — with `Custom(0xD000 | account_index)`.
//!
//! This is the Drift-class rug shape: a `pause()` that has been tampered with
//! to ALSO rotate the admin key. Hopper stops it *structurally* — the strict
//! write policy compiled from the context's `mut(paused, revision)`
//! declaration is the SAME `&'static [WriteRange]` const the runtime enforces
//! at every Context-mediated write acquire (published == enforced). Anchor,
//! Quasar, and Pinocchio have no such gate.
//!
//! ## The flagship (instructions 1 and 2)
//!
//! Two pause handlers share ONE strict-writes context ([`Pause`]) that declares
//! `mut(paused, revision)` on the config account:
//!
//! - [`sentinel_program::honest_pause`] writes `paused = 1` and bumps
//!   `revision` through the bound context. It SUCCEEDS, and its touch map
//!   records exactly the `paused` + `revision` ranges.
//! - [`sentinel_program::malicious_pause`] runs the SAME honest body, then the
//!   tampered tail attempts to ALSO rotate `admin` THROUGH THE CONTEXT
//!   (`ctx.segment_mut` at the admin offset — a governed path, not an
//!   `AccountView::get_mut` bypass). The `admin` range is not in the declared
//!   set, so the acquisition is REFUSED with `Custom(0xD000 | 1)` before a
//!   single admin byte changes.
//!
//! There is no accessor for `admin` on the bound context at all — a
//! `mut(seg)` field only generates its declared per-segment accessors. The
//! only way to *reach* the admin bytes is the raw `Context`, and that path is
//! policy-gated. `AccountView::get_mut` / `try_borrow_mut` would bypass the
//! policy; SENTINEL never uses them (grep the source — the only writes are
//! `ctx.config_*_mut()` / `ctx.segment_mut(...)`).
//!
//! ## The rest of the showcase
//!
//! - `initialize_config` / `initialize_ledger` — the real `#[account(init)]`
//!   lifecycle (both accounts fit a single CreateAccount CPI).
//! - `unpause` — the third handler sharing [`Pause`] (sets `paused = 0`).
//! - `record_entry` — the COLUMNAR per-record pattern: the whole `entries`
//!   array is ONE declared `mut(seg)` range; the cell offset is computed at
//!   RUNTIME (`entries + i * size_of::<LedgerEntry>()`) and policy-checked
//!   against that column, so a write to a different field is refused while
//!   per-cell isolation comes from the segment registry.
//! - `begin_admin_transfer` / `accept_admin_transfer` — the two-step admin
//!   rotation. `accept` DECLARES `mut(admin, ...)`, so writing `admin` here is
//!   allowed — the exact contrast with the flagship, where `admin` is
//!   undeclared and therefore refused.
//! - `collect_fees` — a `mutation_complete` context (`strict_writes` +
//!   `lamports(fee_sink)`) with an over-declarable `treasury` account it
//!   touches in NEITHER dimension, so `InstructionDescriptor::effective_writable`
//!   demotes it writable -> read-only.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

// ── State ───────────────────────────────────────────────────────────
//
// Every field is an alignment-1 wire type (`Wire*` / `Address` / fixed
// arrays / `u8`), so the `#[hopper::state]` fence proves `align_of == 1`
// and `size_of == sum(field sizes)` — zero implicit padding.

/// The governance account. `admin` is the privileged key a rug would try to
/// rotate; `paused` + `revision` are the ONLY fields a pause handler declares.
///
/// Body layout (offsets are body-relative; add `HEADER_LEN` = 16 for the
/// buffer-absolute `*_ABS_OFFSET` the segment accessors use):
///
/// | field              | offset | size |
/// |--------------------|--------|------|
/// | admin              |    0   |  32  |
/// | pending_admin      |   32   |  32  |
/// | withdraw_authority |   64   |  32  |
/// | fee_bps            |   96   |   2  |
/// | paused             |   98   |   1  |
/// | revision           |   99   |   8  |
/// | reserved           |  107   |  21  |
///
/// Body = 128 bytes, `LEN` = 16 + 128 = 144.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Config {
    /// The privileged authority. Rotating this is the whole prize of the
    /// Drift-class rug — and the byte range a tampered `pause()` is refused.
    pub admin: Address,
    /// Two-step transfer target (written only by `begin_admin_transfer`).
    pub pending_admin: Address,
    /// Authority allowed to withdraw (never mutated by the pause path).
    pub withdraw_authority: Address,
    /// Protocol fee in basis points.
    pub fee_bps: WireU16,
    /// 0 = live, 1 = paused. One of the two fields a pause may write.
    pub paused: u8,
    /// Monotonic state revision, bumped on every governed mutation.
    pub revision: WireU64,
    /// Forward-compat reserve.
    pub reserved: [u8; 21],
}

/// A single columnar ledger record. `#[hopper::pod]` gives it the zero-copy
/// Pod contract (alignment-1, no padding) so `ctx.segment_mut::<LedgerEntry>`
/// can project one cell at a runtime offset.
#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LedgerEntry {
    pub actor: Address,
    pub amount: WireU64,
    pub slot: WireU64,
}

/// How many records the ledger holds. Sized so the whole account fits one
/// `#[account(init)]` CreateAccount CPI (capped near
/// `MAX_PERMITTED_DATA_INCREASE` = 10,240 bytes):
///
/// ```text
/// LedgerEntry   = 32 (actor) + 8 (amount) + 8 (slot)         =    48 bytes
/// Ledger body   = 4 (count) + 200 * 48 (entries)             = 9,604 bytes
/// Ledger::LEN   = 16 (Hopper header) + 9,604                 = 9,620 bytes
///                                                    9,620 <= 10,240  ✓
/// ```
///
/// So a single `init` allocation suffices — no `#[account(zero)]`
/// client-preallocation dance is needed.
pub const LEDGER_CAPACITY: usize = 200;

/// The columnar ledger: one `count` header field followed by the `entries`
/// column. `record_entry` declares `mut(entries, count)` — the whole `entries`
/// array is ONE range; individual cells are addressed at runtime.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 2, version = 1)]
pub struct Ledger {
    /// Number of populated entries (also the next write index).
    pub count: WireU32,
    /// The record column.
    pub entries: [LedgerEntry; LEDGER_CAPACITY],
}

// ── Errors ──────────────────────────────────────────────────────────

// `LedgerFull` — the columnar ledger is full (`count == LEDGER_CAPACITY`).
// `ZeroAmount` — a zero-valued amount was supplied to `record_entry`.
hopper::hopper_error! {
    base = 7000;
    LedgerFull,
    ZeroAmount,
}

// ── Flagship account indices (fixed by the context's field order) ───

/// `Pause` slot 0: the admin signer.
const PAUSE_ADMIN_IDX: usize = 0;
/// `Pause` slot 1: the config account. The refusal error is
/// `Custom(0xD000 | 1)` = `Custom(0xD001)`.
const PAUSE_CONFIG_IDX: usize = 1;

/// `RecordEntry` slot 0/1/2.
const RECORD_ACTOR_IDX: usize = 0;
const RECORD_CONFIG_IDX: usize = 1;
const RECORD_LEDGER_IDX: usize = 2;

// ── Contexts ────────────────────────────────────────────────────────

/// Creates the [`Config`] account.
#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init, payer = payer, space = Config::INIT_SPACE)]
    pub config: InitAccount<'info, Config>,

    pub system_program: Program<'info, System>,
}

/// Creates the columnar [`Ledger`] account (all-zero: `count = 0`).
#[derive(Accounts)]
pub struct InitializeLedger<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init, payer = payer, space = Ledger::INIT_SPACE)]
    pub ledger: InitAccount<'info, Ledger>,

    pub system_program: Program<'info, System>,
}

/// THE FLAGSHIP CONTEXT. Shared by `honest_pause`, `malicious_pause`, and
/// `unpause`. `strict_writes` compiles the `mut(paused, revision)`
/// declaration into a static [`WriteRange`](hopper::hopper_runtime::write_policy::WriteRange)
/// policy; `emit_touch_map` makes a successful pause self-describing.
///
/// The declared write set is exactly two ranges on the config account
/// (slot 1): `paused` (1 byte) and `revision` (8 bytes). `admin` is NOT
/// declared, so no accessor exists for it and any Context-mediated write to
/// its bytes is refused with `Custom(0xD000 | 1)`.
#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
pub struct Pause<'info> {
    pub admin: Signer<'info>,

    #[account(mut(paused, revision), has_one = admin)]
    pub config: Account<'info, Config>,
}

/// The COLUMNAR context. `config.revision` is bumped (a field range on slot 1);
/// the whole `ledger.entries` column plus `ledger.count` are declared writable
/// (slot 2). The per-cell offset is computed at runtime and policy-checked
/// against the `entries` column.
#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
pub struct RecordEntry<'info> {
    pub actor: Signer<'info>,

    #[account(mut(revision))]
    pub config: Account<'info, Config>,

    #[account(mut(entries, count))]
    pub ledger: Account<'info, Ledger>,
}

/// Two-step admin rotation, step 1: the current admin nominates a successor.
/// Declares `mut(pending_admin, revision)` — `admin` itself stays immutable.
#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
pub struct BeginAdminTransfer<'info> {
    pub admin: Signer<'info>,

    #[account(mut(pending_admin, revision), has_one = admin)]
    pub config: Account<'info, Config>,
}

/// Two-step admin rotation, step 2: the nominated admin accepts. This context
/// DECLARES `mut(admin, ...)`, so writing `admin` here is allowed — the exact
/// contrast with the flagship, where `admin` is undeclared and refused.
#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
pub struct AcceptAdminTransfer<'info> {
    pub pending_admin: Signer<'info>,

    #[account(mut(admin, pending_admin, revision), has_one = pending_admin)]
    pub config: Account<'info, Config>,
}

/// A mutation-complete context: both the data dimension (`strict_writes`, via
/// `mut(revision)` on `config`) and the lamport dimension (`lamports(fee_sink)`)
/// are declared. `treasury` is declared in NEITHER dimension, so a client that
/// over-marks it writable can be provably demoted.
///
/// Flattened slots: `[0] config · [1] fee_sink · [2] treasury · [3] admin`.
#[derive(Accounts)]
#[accounts(strict_writes, lamports(fee_sink))]
pub struct CollectFees<'info> {
    #[account(mut(revision))]
    pub config: Account<'info, Config>,

    /// Lamport credit target — a lamport permission, no data range.
    pub fee_sink: UncheckedAccount<'info>,

    /// Touched in NEITHER dimension: the demotion target.
    pub treasury: UncheckedAccount<'info>,

    pub admin: Signer<'info>,
}

// ── Program ─────────────────────────────────────────────────────────

#[hopper::program]
pub mod sentinel_program {
    use super::*;

    /// Create the config account and stamp initial governance state.
    #[instruction(0)]
    pub fn initialize_config(ctx: Ctx<InitializeConfig>, fee_bps: u16) -> ProgramResult {
        ctx.init_config()?;
        ctx.accounts.set_initial_state(fee_bps)
    }

    /// FLAGSHIP (honest). A privileged pause that mutates ONLY the byte ranges
    /// it declared: `paused` and `revision`. The Ok-path touch map records
    /// exactly those two writes.
    #[instruction(1)]
    pub fn honest_pause(mut ctx: Ctx<Pause>) -> ProgramResult {
        {
            let mut paused = ctx.config_paused_mut()?;
            *paused = 1u8;
        }
        {
            let mut revision = ctx.config_revision_mut()?;
            revision.checked_add_assign(1)?;
        }
        Ok(())
    }

    /// FLAGSHIP (malicious). The SAME strict-writes context and the SAME honest
    /// body — then a tampered tail that ALSO tries to rotate the admin key
    /// THROUGH THE CONTEXT.
    ///
    /// This is a RAW `&mut Context` handler so it can reach the raw
    /// `segment_mut` after binding: `Pause::bind` installs the strict-writes
    /// `WritePolicy` and it PERSISTS on `ctx` after the bound guard drops. The
    /// admin write is a GOVERNED path (`ctx.segment_mut`, not
    /// `AccountView::get_mut`), so it is gated at
    /// `Context::check_write_policy` and REFUSED with `Custom(0xD000 | 1)`
    /// before any admin byte changes.
    #[instruction(2)]
    pub fn malicious_pause(ctx: &mut Context) -> ProgramResult {
        // 1) The honest pause runs first, through the bound context.
        {
            let mut bound = Pause::bind(ctx)?;
            {
                let mut paused = bound.config_paused_mut()?;
                *paused = 1u8;
            }
            {
                let mut revision = bound.config_revision_mut()?;
                revision.checked_add_assign(1)?;
            }
            // `bound` drops here; the WritePolicy stays installed on `ctx`.
        }

        // 2) TAMPERED: rotate the admin key to the attacker. The `admin` range
        //    is undeclared, so this acquisition is refused at the gate — the
        //    `?` returns `Custom(0xD000 | 1)` and the assignment below never
        //    runs. No admin byte is ever written.
        let attacker = Address::new_from_array([0xAAu8; 32]);
        let mut admin = ctx.segment_mut::<Address>(PAUSE_CONFIG_IDX, Config::ADMIN_ABS_OFFSET)?;
        *admin = attacker;
        Ok(())
    }

    /// The third handler sharing [`Pause`]: clears `paused`, bumps `revision`.
    #[instruction(3)]
    pub fn unpause(mut ctx: Ctx<Pause>) -> ProgramResult {
        {
            let mut paused = ctx.config_paused_mut()?;
            *paused = 0u8;
        }
        {
            let mut revision = ctx.config_revision_mut()?;
            revision.checked_add_assign(1)?;
        }
        Ok(())
    }

    /// Create the columnar ledger account (zero-initialized).
    #[instruction(4)]
    pub fn initialize_ledger(ctx: Ctx<InitializeLedger>) -> ProgramResult {
        ctx.init_ledger()?;
        Ok(())
    }

    /// COLUMNAR per-record write. The `entries` column boundary is enforced by
    /// the static policy; per-cell isolation comes from the segment registry;
    /// the touch map records the EXACT cell touched.
    ///
    /// A RAW handler, because writing cell `i` needs a RUNTIME offset via
    /// `ctx.segment_mut::<LedgerEntry>(ledger_idx, entries + i*size)` — a
    /// `mut(seg)` field exposes no whole-account runtime-offset accessor, so
    /// the write goes through the raw `Context` (policy persists after bind).
    #[instruction(5)]
    pub fn record_entry(ctx: &mut Context, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);

        // Bind validates config + ledger and installs the strict-writes policy.
        let next_index: u32 = {
            let bound = RecordEntry::bind(ctx)?;
            let ledger = bound.ledger_load()?;
            ledger.count.get()
        };
        hopper::hopper_require!((next_index as usize) < LEDGER_CAPACITY, LedgerFull);

        let actor = *ctx.account(RECORD_ACTOR_IDX)?.address();

        // Runtime cell offset, policy-checked against the whole `entries`
        // column. A write to any other field/column is refused.
        let cell_offset =
            Ledger::ENTRIES_ABS_OFFSET + next_index * core::mem::size_of::<LedgerEntry>() as u32;
        {
            let mut cell = ctx.segment_mut::<LedgerEntry>(RECORD_LEDGER_IDX, cell_offset)?;
            cell.actor = actor;
            cell.amount = WireU64::new(amount);
            cell.slot = WireU64::new(next_index as u64);
        }
        {
            let mut count =
                ctx.segment_mut::<WireU32>(RECORD_LEDGER_IDX, Ledger::COUNT_ABS_OFFSET)?;
            *count = WireU32::new(next_index + 1);
        }
        {
            let mut revision =
                ctx.segment_mut::<WireU64>(RECORD_CONFIG_IDX, Config::REVISION_ABS_OFFSET)?;
            revision.checked_add_assign(1)?;
        }

        // Ok-path self-describing emit (raw handler: called by hand, mirroring
        // what the typed dispatcher does for `Ctx<Spec>` handlers).
        ctx.finish_with_touch_map();
        Ok(())
    }

    /// Two-step admin rotation, step 1. Writes ONLY `pending_admin` + `revision`.
    #[instruction(6)]
    pub fn begin_admin_transfer(
        mut ctx: Ctx<BeginAdminTransfer>,
        new_admin: Address,
    ) -> ProgramResult {
        {
            let mut pending = ctx.config_pending_admin_mut()?;
            *pending = new_admin;
        }
        {
            let mut revision = ctx.config_revision_mut()?;
            revision.checked_add_assign(1)?;
        }
        Ok(())
    }

    /// Two-step admin rotation, step 2. The nominated admin (a signer) accepts:
    /// promote `pending_admin` into `admin`, clear `pending_admin`, bump
    /// `revision`. `admin` IS declared writable here, so the promotion lands —
    /// the deliberate contrast with the flagship refusal.
    #[instruction(7)]
    pub fn accept_admin_transfer(mut ctx: Ctx<AcceptAdminTransfer>) -> ProgramResult {
        let new_admin = *ctx.account(0)?.address();
        {
            let mut admin = ctx.config_admin_mut()?;
            *admin = new_admin;
        }
        {
            let mut pending = ctx.config_pending_admin_mut()?;
            *pending = Address::new_from_array([0u8; 32]);
        }
        {
            let mut revision = ctx.config_revision_mut()?;
            revision.checked_add_assign(1)?;
        }
        Ok(())
    }

    /// A mutation-complete instruction that declares `lamports(fee_sink)` but
    /// leaves `treasury` untouched in both dimensions. The handler bumps only
    /// the declared `revision` data range. See the demotion test: a client
    /// that over-declares `treasury` writable is provably safe to demote.
    #[instruction(8)]
    pub fn collect_fees(mut ctx: Ctx<CollectFees>, _amount: u64) -> ProgramResult {
        let mut revision = ctx.config_revision_mut()?;
        revision.checked_add_assign(1)?;
        Ok(())
    }
}

// ── Init state helpers (kept off the context so borrows stay simple) ─

impl<'info> InitializeConfig<'info> {
    fn set_initial_state(&self, fee_bps: u16) -> ProgramResult {
        let admin = *self.payer.key();
        let mut config = self.config.get_mut_after_init()?;
        config.admin = admin;
        config.pending_admin = Address::new_from_array([0u8; 32]);
        config.withdraw_authority = admin;
        config.fee_bps = WireU16::new(fee_bps);
        config.paused = 0;
        config.revision = WireU64::new(0);
        Ok(())
    }
}

// ── Program manifest (exported schema, built from the SAME consts the
//    runtime enforces) ─────────────────────────────────────────────

hopper::program_manifest! {
    program = sentinel_program,
    layouts = [Config, Ledger],
}
