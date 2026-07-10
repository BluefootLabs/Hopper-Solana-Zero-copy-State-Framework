//! # Hopper Smoke Test
//!
//! A single macro-first program that exercises a broad slice of the
//! framework end-to-end, intended to be built to SBF and run on devnet:
//!
//! - `#[account]` versioned zero-copy layout with a self-describing header
//! - `#[derive(Accounts)]` constraints: `init`, `payer`, `space`, `mut`,
//!   `signer`, `has_one`, `close`
//! - `#[program]` dispatch with typed handler args
//! - System-program CPI (`Transfer`) for deposits
//! - Direct lamport debit for program-owned withdrawals
//! - Clock sysvar read via the native syscall path
//! - A typed, zero-alloc event emitted through `sol_log_data`
//! - Checked arithmetic everywhere (no silent overflow)
//! - `safe_close` semantics via the `close` constraint
//! - Self-describing transactions (innovation I7): the `Withdraw`
//!   context opts in via `emit_touch_map`, so every successful
//!   withdraw emits its touch map as one `sol_log_data` record that
//!   `hopper tx explain` decodes into field-level state effects
//!
//! Instructions:
//! - `0` = Initialize (creates the vault account, stamps the clock)
//! - `1` = Deposit     (signer transfers SOL in, count++)
//! - `2` = Withdraw    (authority pulls SOL out, checked, self-describing)
//! - `3` = Close       (authority closes the vault, lamports refunded)
//! - `4` = EmitReceipt (authority bumps the touch counter and emits a
//!   state receipt as an authenticated self-CPI — the `event_cpi`
//!   demo: indexers read it from inner-instruction metadata, which RPC
//!   nodes never truncate, unlike the log-based event on Deposit)
//! - `5` = BumpWholeVault (authority bumps the counter through the
//!   WRAPPER accessor `vault.get_mut()` — the whole-account borrow the
//!   touch map used to be blind to; its emitted map carries one
//!   full-account write record from the instruction-ambient touch log)

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

// --- State ----------------------------------------------------------

/// The vault account. Versioned so a future layout change is a typed,
/// fingerprinted migration rather than a silent reinterpretation.
#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 7, version = 1)]
pub struct Vault {
    /// Owner allowed to withdraw and close.
    pub authority: Address,
    /// Running lamport balance tracked in state (mirrors the account's
    /// own lamports; kept in state to exercise checked arithmetic).
    pub balance: WireU64,
    /// Number of deposits seen, ever.
    pub deposit_count: WireU32,
    /// Unix timestamp the vault was created (from the Clock sysvar).
    pub created_at: WireI64,
    /// Last slot a deposit landed.
    pub last_deposit_slot: WireU64,
}

// --- Event ----------------------------------------------------------

/// Emitted on every deposit. Align-1 wire types only, so it is a valid
/// zero-copy `Pod` payload for `emit_event`.
#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct DepositEvent {
    pub amount: WireU64,
    pub new_balance: WireU64,
    pub deposit_count: WireU32,
    pub slot: WireU64,
}

/// Emitted by `EmitReceipt` as an authenticated self-CPI
/// (`#[accounts(event_cpi)]` + `ctx.emit_event_cpi`). Unlike
/// [`DepositEvent`] above — which rides `sol_log_data` and can be
/// truncated out of RPC logs — this lands in the transaction's
/// inner-instruction metadata. Wire: `[0xE0, 0x1E, 0x02, payload]`,
/// 3 bytes of overhead vs Anchor `emit_cpi!`'s 16.
#[hopper::event(tag = 2)]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct DepositReceipt {
    /// Vault state balance at emission time.
    pub balance: WireU64,
    /// Deposit/touch counter after the bump.
    pub deposit_count: WireU32,
}

// --- Errors ---------------------------------------------------------

hopper::hopper_error! {
    base = 6000;
    Unauthorized,
    InsufficientBalance,
    ZeroAmount,
}

// --- Contexts -------------------------------------------------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init, payer = payer, space = Vault::INIT_SPACE)]
    pub vault: InitAccount<'info, Vault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,

    /// Required because `deposit` CPIs into the System program.
    pub system_program: Program<'info, System>,
}

/// The self-describing context (innovation I7). Two opt-ins compose:
///
/// - `strict_writes`: the `mut(balance)` declaration below is compiled
///   into a static write policy, so any Context-mediated write outside
///   `vault.balance` fails at acquisition time.
/// - `emit_touch_map`: on the handler's **Ok** path (and only then) the
///   generated dispatcher emits the instruction's cumulative touch map
///   as a single `sol_log_data` record — magic `0x7A`, version `0x01`,
///   then `(slot, offset, size, R/W)` entries. `hopper tx explain`
///   decodes it from the transaction's log stream, so a successful
///   withdraw advertises exactly which bytes of which account it wrote.
///   A failed withdraw emits nothing: the dispatcher short-circuits on
///   `Err` before the emit, so a rolled-back instruction never
///   advertises effects it did not keep.
///
/// The emission only compiles in because this crate enables the
/// `touch-map` feature; without it the opt-in is a no-op.
#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut(balance), has_one = authority)]
    pub vault: Account<'info, Vault>,
}

#[derive(Accounts)]
pub struct CloseVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority, close = authority)]
    pub vault: Account<'info, Vault>,
}

/// Self-describing through the WRAPPER path: this context opts into
/// `emit_touch_map` but its handler writes via `vault.get_mut()` — a
/// whole-account borrow with no segment lease and no `Context` at the
/// borrow site. The instruction-AMBIENT touch log (same reserved-heap
/// scheme as the lamport gate store) is what makes that borrow visible:
/// the emitted map carries one full-account `W` record. Contrast with
/// `Withdraw` above, whose `mut(balance)` segment lease produces a
/// field-precise 8-byte record.
#[derive(Accounts)]
#[accounts(emit_touch_map)]
pub struct BumpWholeVault<'info> {
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
}

/// The `event_cpi` context: the option auto-appends two TRAILING
/// account slots the struct does not declare — the event-authority PDA
/// (seeds `[b"__hopper_event_authority"]`, verified at bind on-chain
/// via the sha256 compare loop) and this program's own account (pinned
/// to `ctx.program_id()`; a self-CPI target must be a transaction
/// account) — so the instruction's account shape is
/// `[authority, vault, event_authority, program]`, exactly the two
/// extras Anchor's `#[event_cpi]` appends. The bound context gains
/// `emit_event_cpi(&event)`.
#[derive(Accounts)]
#[accounts(event_cpi)]
pub struct EmitReceipt<'info> {
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
}

// --- Program --------------------------------------------------------

#[program]
mod smoke_program {
    use super::*;

    #[instruction(0)]
    pub fn initialize(ctx: Ctx<Initialize>) -> ProgramResult {
        ctx.init_vault()?;
        ctx.accounts.initialize()
    }

    #[instruction(1)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        ctx.accounts.deposit(amount)
    }

    #[instruction(2)]
    pub fn withdraw(ctx: Ctx<Withdraw>, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);

        // Balance update through the segment lease declared by
        // `mut(balance)`: the write registers in the instruction's
        // segment registry, which is exactly what the Ok-path touch-map
        // emission snapshots — the on-chain record will carry one
        // `W vault [balance..balance+8)` entry for this write.
        {
            let mut balance = ctx.vault_balance_mut()?;
            if balance.get() < amount {
                return Err(InsufficientBalance.into());
            }
            balance.checked_sub_assign(amount)?;
        }

        ctx.accounts.settle_withdraw_lamports(amount)
    }

    #[instruction(3)]
    pub fn close(ctx: Ctx<CloseVault>) -> ProgramResult {
        // The `close = authority` constraint zeroes the data, stamps the
        // revival sentinel, and refunds lamports to the authority. Nothing
        // else to do in the body.
        Ok(())
    }

    /// The `event_cpi` one-liner demo: bump the vault's touch counter,
    /// then emit the post-state as an authenticated self-CPI receipt.
    /// The dispatcher's generated `[0xE0, 0x1E]` sink (armed because
    /// `EmitReceipt` opted in) authenticates the inner instruction:
    /// event-authority signer + on-chain PDA address pin, so a foreign
    /// program cannot plant forged receipts in this program's
    /// inner-instruction list.
    #[instruction(4)]
    pub fn emit_receipt(ctx: Ctx<EmitReceipt>) -> ProgramResult {
        let (balance, count) = {
            let mut vault = ctx.accounts.vault.get_mut()?;
            vault.deposit_count.checked_add_assign(1)?;
            (vault.balance.get(), vault.deposit_count.get())
        };
        ctx.emit_event_cpi(&DepositReceipt {
            balance: WireU64::new(balance),
            deposit_count: WireU32::new(count),
        })
    }

    /// The wrapper-borrow demo: the SAME state change as a segment
    /// path would make, but through `get_mut()` — and the Ok-path
    /// touch map still describes it (one whole-account write record).
    #[instruction(5)]
    pub fn bump_whole_vault(ctx: Ctx<BumpWholeVault>) -> ProgramResult {
        let mut vault = ctx.accounts.vault.get_mut()?;
        vault.deposit_count.checked_add_assign(1)?;
        Ok(())
    }
}

impl<'info> Initialize<'info> {
    pub fn initialize(&self) -> ProgramResult {
        // Read the on-chain clock so the vault records its birth time.
        // NOTE: this sample uses the low-level `substrate` layer on purpose to
        // demonstrate it; application code should prefer the ergonomic
        // `hopper::sysvar::Clock::get()` (or `hopper::sysvar::get_clock()`).
        let now = hopper::substrate::sysvar::get_clock()?;

        let mut vault = self.vault.get_mut_after_init()?;
        vault.authority = *self.payer.key();
        vault.balance = WireU64::new(0);
        vault.deposit_count = WireU32::new(0);
        vault.created_at = WireI64::new(now.unix_timestamp);
        vault.last_deposit_slot = WireU64::new(now.slot);
        Ok(())
    }
}

impl<'info> Deposit<'info> {
    pub fn deposit(&self, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);

        // Move the SOL in via the System program.
        let authority = self.authority.as_account();
        let vault_account = self.vault.as_account();
        hopper::system::Transfer {
            from: authority,
            to: vault_account,
            lamports: amount,
        }
        .invoke()?;

        let now = hopper::substrate::sysvar::get_clock()?;

        let (new_balance, count, slot) = {
            let mut vault = self.vault.get_mut()?;
            vault.balance.checked_add_assign(amount)?;
            vault.deposit_count.checked_add_assign(1)?;
            vault.last_deposit_slot = WireU64::new(now.slot);
            (vault.balance.get(), vault.deposit_count.get(), now.slot)
        };

        // Emit a typed, zero-alloc event for off-chain indexers.
        let event = DepositEvent {
            amount: WireU64::new(amount),
            new_balance: WireU64::new(new_balance),
            deposit_count: WireU32::new(count),
            slot: WireU64::new(slot),
        };
        hopper::systems::emit_event_tagged(1, &event)?;
        Ok(())
    }
}

impl<'info> Withdraw<'info> {
    /// Move the withdrawn lamports out. The balance-field update happens
    /// in the handler through the `mut(balance)` segment lease (so it is
    /// captured by the touch map); this settles the actual funds.
    pub fn settle_withdraw_lamports(&self, amount: u64) -> ProgramResult {
        // The vault is program-owned, so debit lamports directly after
        // Hopper has validated authority (`has_one`) and layout.
        let authority = self.authority.as_account();
        let vault_account = self.vault.as_account();
        vault_account.set_lamports(
            vault_account
                .lamports()
                .checked_sub(amount)
                .ok_or(ProgramError::InsufficientFunds)?,
        )?;
        authority.set_lamports(
            authority
                .lamports()
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        )?;
        Ok(())
    }
}
