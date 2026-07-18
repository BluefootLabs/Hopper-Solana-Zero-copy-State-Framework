//! # Cicada — transport-neutral protected execution intents
//!
//! Cicada is Hopper's first production-shaped flagship program slice. It is
//! not coupled to Jito, BAM, a particular RPC provider, or one swap
//! aggregator. Users publish execution constraints in a shared,
//! column-oriented shard; solvers execute permissionless intents atomically
//! or reserve an explicitly allowlisted intent, invoke a route through CPI,
//! and settle only when the **observed token deltas** satisfy the user's
//! envelope.
//!
//! The design uses Hopper's differentiators for a real security boundary:
//!
//! - immutable user columns are never declared writable to claim or execution
//!   instructions;
//! - executor state is updated through exact runtime-selected cells inside
//!   statically declared columns;
//! - each source vault has an owner-bound PDA authority, limiting arbitrary
//!   route CPI signer power to one user's committed vault rather than a global
//!   protocol PDA;
//! - route account order, duplicates, and privileges can be committed exactly;
//! - source and destination token-account policy bytes must remain unchanged
//!   across the route, while only the amount field may move;
//! - actual input/output deltas, not a router's return value, determine success;
//! - any unused source balance is atomically returned before settlement.
//!
//! V1 supports two route policies:
//!
//! - [`ROUTE_MODE_EXACT`]: the user commits the complete route envelope;
//! - [`ROUTE_MODE_PROGRAM`]: the user trusts one route program while Cicada
//!   still limits signer authority, protects its own state, freezes token
//!   account policy bytes, and enforces the economic result.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use core::mem::size_of;

use hopper::cpi::{DynCpi, InstructionAccount, InstructionView};
use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

pub const CONFIG_SEED: &[u8] = b"cicada-config";
pub const VAULT_AUTHORITY_SEED: &[u8] = b"cicada-vault";
pub const SOURCE_LEASE_SEED: &[u8] = b"cicada-source";

/// A shard remains directly initializable under Solana's 10,240-byte
/// per-instruction growth ceiling, including Hopper's account header.
pub const INTENTS_PER_SHARD: usize = 20;
pub const MAX_ROUTE_ACCOUNTS: usize = 32;
const ROUTE_HASH_CHUNKS: usize = MAX_ROUTE_ACCOUNTS.div_ceil(8);
pub const MAX_ROUTE_DATA: usize = 512;

pub const ROUTE_META_WRITABLE: u8 = 1 << 0;
pub const ROUTE_META_SIGNER: u8 = 1 << 1;
pub const ROUTE_META_KNOWN_FLAGS: u8 = ROUTE_META_WRITABLE | ROUTE_META_SIGNER;

pub const ROUTE_MODE_EXACT: u8 = 0;
pub const ROUTE_MODE_PROGRAM: u8 = 1;

pub const STATUS_EMPTY: u8 = 0;
pub const STATUS_OPEN: u8 = 1;
pub const STATUS_CLAIMED: u8 = 2;
pub const STATUS_SETTLED: u8 = 3;
pub const STATUS_CANCELLED: u8 = 4;

const ZERO_ADDRESS: Address = Address::new_from_array([0u8; 32]);
const ZERO_HASH: [u8; 32] = [0u8; 32];

// ── State ───────────────────────────────────────────────────────────

/// Global Cicada controls.
///
/// This account is deliberately not the authority of user source vaults.
/// Every source vault receives a separate PDA derived from its owner and
/// address, containing route-CPI signer power to one user's isolated vault.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 80, version = 1)]
pub struct CicadaConfig {
    pub admin: Address,
    pub emergency_authority: Address,
    pub default_claim_ttl: WireU64,
    pub paused: u8,
    pub bump: u8,
    pub revision: WireU64,
    pub reserved: [u8; 14],
}

/// Global uniqueness marker for one funded source vault.
///
/// The PDA is derived from the source token account itself, so two shards
/// cannot concurrently register intents against the same custody account.
/// It is closed back to the intent owner only after the record is final.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 82, version = 1)]
pub struct SourceLease {
    pub source_token: Address,
    pub shard: Address,
    pub owner: Address,
    pub slot: WireU16,
    pub sequence: WireU64,
    pub bump: u8,
    pub reserved: [u8; 5],
}

/// Shared column-oriented intent state.
///
/// Columns are separated by authority domain. An execution handler can write
/// claim and settlement cells without receiving mutable access to owners,
/// vaults, route policy, limits, or expiry.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 81, version = 1)]
pub struct IntentShard {
    pub config: Address,
    pub shard_id: WireU32,
    pub occupied: WireU32,
    pub occupied_count: WireU16,
    pub next_sequence: WireU64,
    pub reserved_header: [u8; 14],

    // Immutable user constraints.
    pub owners: [Address; INTENTS_PER_SHARD],
    pub source_tokens: [Address; INTENTS_PER_SHARD],
    pub vault_authorities: [Address; INTENTS_PER_SHARD],
    pub refund_tokens: [Address; INTENTS_PER_SHARD],
    pub destination_tokens: [Address; INTENTS_PER_SHARD],
    pub input_mints: [Address; INTENTS_PER_SHARD],
    pub output_mints: [Address; INTENTS_PER_SHARD],
    pub max_inputs: [WireU64; INTENTS_PER_SHARD],
    pub min_outputs: [WireU64; INTENTS_PER_SHARD],
    pub expiries: [WireU64; INTENTS_PER_SHARD],
    pub allowed_executors: [Address; INTENTS_PER_SHARD],
    pub route_programs: [Address; INTENTS_PER_SHARD],
    pub route_commitments: [[u8; 32]; INTENTS_PER_SHARD],
    pub route_modes: [u8; INTENTS_PER_SHARD],
    pub sequences: [WireU64; INTENTS_PER_SHARD],

    // Executor/lifecycle state.
    pub statuses: [u8; INTENTS_PER_SHARD],
    pub claimants: [Address; INTENTS_PER_SHARD],
    pub claim_expiries: [WireU64; INTENTS_PER_SHARD],
    pub settled_inputs: [WireU64; INTENTS_PER_SHARD],
    pub settled_outputs: [WireU64; INTENTS_PER_SHARD],
    pub settlement_hashes: [[u8; 32]; INTENTS_PER_SHARD],
    pub revisions: [WireU64; INTENTS_PER_SHARD],
}

const _: () = assert!(IntentShard::LEN <= 10_240);
// The `occupied` slot bitmap is a `u32`, so `slot_bit` shifts `1u32 << slot`.
// Keep the slot count within the bitmap width: a larger `INTENTS_PER_SHARD`
// would silently shift out of range (UB in debug, a wrapping no-op in
// release) and corrupt occupancy tracking. Compile-time, not a runtime check.
const _: () = assert!(INTENTS_PER_SHARD <= 32);

/// Stack snapshot copied out before route CPI. No account-data borrow is held
/// across the external invocation.
#[derive(Clone, Copy)]
pub struct IntentSnapshot {
    pub owner: Address,
    pub source_token: Address,
    pub vault_authority: Address,
    pub refund_token: Address,
    pub destination_token: Address,
    pub input_mint: Address,
    pub output_mint: Address,
    pub max_input: u64,
    pub min_output: u64,
    pub expiry: u64,
    pub allowed_executor: Address,
    pub route_program: Address,
    pub route_commitment: [u8; 32],
    pub route_mode: u8,
    pub sequence: u64,
    pub status: u8,
    pub claimant: Address,
    pub claim_expiry: u64,
    pub revision: u64,
}

// ── Errors ──────────────────────────────────────────────────────────

hopper::hopper_error! {
    base = 7200;
    ProtocolPaused,
    InvalidClaimTtl,
    ShardFull,
    SlotOutOfRange,
    SlotNotOccupied,
    SourceAlreadyInUse,
    ZeroInputLimit,
    ZeroOutputLimit,
    IntentAlreadyExpired,
    InvalidRouteMode,
    EmptyRouteProgram,
    EmptyRouteCommitment,
    UnexpectedRouteCommitment,
    TokenAccountMismatch,
    AliasedSettlementAccounts,
    TokenMintMismatch,
    TokenProgramMismatch,
    TokenAuthorityMismatch,
    UnsafeTokenExtension,
    InsufficientVaultFunds,
    InvalidIntentStatus,
    UnauthorizedIntentOwner,
    UnauthorizedExecutor,
    PermissionlessClaimForbidden,
    ClaimStillActive,
    ClaimExpired,
    InvalidClaimLease,
    RouteProgramMismatch,
    RouteAccountCountMismatch,
    InvalidRouteMetaFlags,
    NonZeroUnusedRouteFlags,
    RouteMetaPrivilegeEscalation,
    ProtectedAccountDelegation,
    OtherIntentVaultDelegation,
    RouteCommitmentMismatch,
    SourceTokenPolicyChanged,
    DestinationTokenPolicyChanged,
    InputMintPolicyChanged,
    OutputMintPolicyChanged,
    InputBalanceIncreased,
    OutputBalanceDecreased,
    MaximumInputExceeded,
    MinimumOutputNotMet,
    EmptySettlement,
    RefundAccountMismatch,
    RefundNotEmpty,
    SourceNotEmpty,
    IntentNotFinal,
}

// ── Contexts ────────────────────────────────────────────────────────

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        payer = payer,
        space = CicadaConfig::INIT_SPACE,
        seeds = [CONFIG_SEED],
        bump,
    )]
    pub config: InitAccount<'info, CicadaConfig>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeShard<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        has_one = admin,
        seeds = [CONFIG_SEED],
        bump = config.load::<CicadaConfig>()?.bump,
    )]
    pub config: Account<'info, CicadaConfig>,

    #[account(init, payer = admin, space = IntentShard::INIT_SPACE)]
    pub shard: InitAccount<'info, IntentShard>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
pub struct CreateIntent<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.load::<CicadaConfig>()?.bump)]
    pub config: Account<'info, CicadaConfig>,

    #[account(
        mut(
            occupied,
            occupied_count,
            next_sequence,
            owners,
            source_tokens,
            vault_authorities,
            refund_tokens,
            destination_tokens,
            input_mints,
            output_mints,
            max_inputs,
            min_outputs,
            expiries,
            allowed_executors,
            route_programs,
            route_commitments,
            route_modes,
            sequences,
            statuses,
            claimants,
            claim_expiries,
            settled_inputs,
            settled_outputs,
            settlement_hashes,
            revisions
        ),
        has_one = config,
    )]
    pub shard: Account<'info, IntentShard>,

    pub source_token: UncheckedAccount<'info>,

    #[account(
        seeds = [
            VAULT_AUTHORITY_SEED,
            owner.address().as_array(),
            source_token.address().as_array()
        ],
        bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub refund_token: UncheckedAccount<'info>,
    pub destination_token: UncheckedAccount<'info>,
    pub input_mint: UncheckedAccount<'info>,
    pub output_mint: UncheckedAccount<'info>,
    pub route_program: UncheckedAccount<'info>,

    #[account(
        init,
        payer = owner,
        space = SourceLease::INIT_SPACE,
        seeds = [SOURCE_LEASE_SEED, source_token.address().as_array()],
        bump,
    )]
    pub source_lease: InitAccount<'info, SourceLease>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
#[instruction(slot: u16)]
pub struct ClaimIntent<'info> {
    pub executor: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.load::<CicadaConfig>()?.bump)]
    pub config: Account<'info, CicadaConfig>,

    #[account(
        cells(slot; statuses, claimants, claim_expiries, revisions),
        has_one = config,
    )]
    pub shard: Account<'info, IntentShard>,
}

#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
#[instruction(slot: u16)]
pub struct ReleaseClaim<'info> {
    #[account(seeds = [CONFIG_SEED], bump = config.load::<CicadaConfig>()?.bump)]
    pub config: Account<'info, CicadaConfig>,

    #[account(
        cells(slot; statuses, claimants, claim_expiries, revisions),
        has_one = config,
    )]
    pub shard: Account<'info, IntentShard>,
}

#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
#[instruction(slot: u16)]
pub struct CancelIntent<'info> {
    pub owner: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.load::<CicadaConfig>()?.bump)]
    pub config: Account<'info, CicadaConfig>,

    #[account(
        cells(slot; statuses, claimants, claim_expiries, revisions),
        has_one = config,
    )]
    pub shard: Account<'info, IntentShard>,

    #[account(mut)]
    pub source_token: UncheckedAccount<'info>,

    #[account(
        seeds = [
            VAULT_AUTHORITY_SEED,
            owner.address().as_array(),
            source_token.address().as_array()
        ],
        bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub refund_token: UncheckedAccount<'info>,
    pub input_mint: UncheckedAccount<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
#[instruction(slot: u16)]
pub struct ExecuteIntent<'info> {
    pub executor: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.load::<CicadaConfig>()?.bump)]
    pub config: Account<'info, CicadaConfig>,

    #[account(
        cells(slot;
            statuses,
            claimants,
            claim_expiries,
            settled_inputs,
            settled_outputs,
            settlement_hashes,
            revisions
        ),
        has_one = config,
    )]
    pub shard: Account<'info, IntentShard>,

    /// The committed owner need not sign execution, but including the account
    /// lets the context validate the owner-bound vault-authority PDA before
    /// arbitrary route CPI is attempted.
    pub intent_owner: UncheckedAccount<'info>,

    #[account(mut)]
    pub source_token: UncheckedAccount<'info>,

    #[account(
        seeds = [
            VAULT_AUTHORITY_SEED,
            intent_owner.address().as_array(),
            source_token.address().as_array()
        ],
        bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub refund_token: UncheckedAccount<'info>,
    #[account(mut)]
    pub destination_token: UncheckedAccount<'info>,
    pub input_mint: UncheckedAccount<'info>,
    pub output_mint: UncheckedAccount<'info>,
    pub token_program: UncheckedAccount<'info>,
    pub route_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
#[instruction(slot: u16)]
pub struct ReclaimIntent<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.load::<CicadaConfig>()?.bump)]
    pub config: Account<'info, CicadaConfig>,

    #[account(
        mut(
            occupied,
            occupied_count
        ),
        cells(slot;
            owners,
            source_tokens,
            vault_authorities,
            refund_tokens,
            destination_tokens,
            input_mints,
            output_mints,
            max_inputs,
            min_outputs,
            expiries,
            allowed_executors,
            route_programs,
            route_commitments,
            route_modes,
            sequences,
            statuses,
            claimants,
            claim_expiries,
            settled_inputs,
            settled_outputs,
            settlement_hashes,
            revisions
        ),
        has_one = config,
    )]
    pub shard: Account<'info, IntentShard>,

    #[account(mut)]
    pub source_token: UncheckedAccount<'info>,

    #[account(
        seeds = [
            VAULT_AUTHORITY_SEED,
            owner.address().as_array(),
            source_token.address().as_array()
        ],
        bump,
    )]
    pub vault_authority: UncheckedAccount<'info>,

    pub token_program: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [SOURCE_LEASE_SEED, source_token.address().as_array()],
        bump = source_lease.load::<SourceLease>()?.bump,
        has_one = source_token,
        has_one = shard,
        has_one = owner,
        close = owner,
    )]
    pub source_lease: Account<'info, SourceLease>,
}

#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
pub struct SetPause<'info> {
    pub emergency_authority: Signer<'info>,

    #[account(
        mut(paused, revision),
        has_one = emergency_authority,
        seeds = [CONFIG_SEED],
        bump = config.load::<CicadaConfig>()?.bump,
    )]
    pub config: Account<'info, CicadaConfig>,
}

// ── Program ─────────────────────────────────────────────────────────

#[hopper::program(sealed, max_accounts = 48)]
pub mod cicada_program {
    use super::*;

    #[instruction(0)]
    pub fn initialize_config(
        ctx: Ctx<InitializeConfig>,
        emergency_authority: Address,
        default_claim_ttl: u64,
    ) -> ProgramResult {
        hopper::hopper_require!(default_claim_ttl > 0, InvalidClaimTtl);
        ctx.init_config()?;

        let mut config = ctx.accounts.config.get_mut_after_init()?;
        config.admin = *ctx.accounts.payer.key();
        config.emergency_authority = emergency_authority;
        config.default_claim_ttl = WireU64::new(default_claim_ttl);
        config.paused = 0;
        config.bump = ctx.bumps.config;
        config.revision = WireU64::new(0);
        Ok(())
    }

    #[instruction(1)]
    pub fn initialize_shard(ctx: Ctx<InitializeShard>, shard_id: u32) -> ProgramResult {
        ctx.init_shard()?;
        let mut shard = ctx.accounts.shard.get_mut_after_init()?;
        shard.config = *ctx.accounts.config.key();
        shard.shard_id = WireU32::new(shard_id);
        shard.occupied = WireU32::new(0);
        shard.occupied_count = WireU16::new(0);
        shard.next_sequence = WireU64::new(1);
        Ok(())
    }

    #[instruction(2)]
    pub fn create_intent(
        mut ctx: Ctx<CreateIntent>,
        max_input: u64,
        min_output: u64,
        expiry_slot: u64,
        allowed_executor: Address,
        route_mode: u8,
        route_commitment: [u8; 32],
    ) -> ProgramResult {
        hopper::hopper_require!(max_input > 0, ZeroInputLimit);
        hopper::hopper_require!(min_output > 0, ZeroOutputLimit);
        hopper::hopper_require!(route_mode <= ROUTE_MODE_PROGRAM, InvalidRouteMode);
        if route_mode == ROUTE_MODE_EXACT {
            hopper::hopper_require!(route_commitment != ZERO_HASH, EmptyRouteCommitment);
        } else {
            // Program-trust mode intentionally does not carry a dead, unaudited
            // commitment field. Requiring zero keeps the intent canonical and
            // prevents clients from disagreeing about whether it matters.
            hopper::hopper_require!(route_commitment == ZERO_HASH, UnexpectedRouteCommitment);
        }

        let now = Clock::get()?.slot;
        hopper::hopper_require!(expiry_slot > now, IntentAlreadyExpired);
        ensure_live(ctx.accounts.config.as_account())?;
        ctx.accounts.route_program.as_account().check_executable()?;
        hopper::hopper_require!(
            !address::address_is_zero(ctx.accounts.route_program.key()),
            EmptyRouteProgram
        );
        verify_create_token_accounts(&ctx.accounts, max_input)?;

        let (
            slot,
            sequence,
            occupied,
            occupied_count,
            owner,
            source_token,
            vault_authority,
            refund_token,
            destination_token,
            input_mint,
            output_mint,
            route_program,
        ) = {
            let shard = ctx.accounts.shard.get()?;
            let slot = find_free_slot(&shard).ok_or_else(|| ProgramError::from(ShardFull))?;
            ensure_source_unique(&shard, ctx.accounts.source_token.key())?;
            (
                slot,
                shard.next_sequence.get(),
                shard.occupied.get(),
                shard.occupied_count.get(),
                *ctx.accounts.owner.key(),
                *ctx.accounts.source_token.key(),
                *ctx.accounts.vault_authority.key(),
                *ctx.accounts.refund_token.key(),
                *ctx.accounts.destination_token.key(),
                *ctx.accounts.input_mint.key(),
                *ctx.accounts.output_mint.key(),
                *ctx.accounts.route_program.key(),
            )
        };

        ctx.init_source_lease()?;
        {
            let mut lease = ctx.accounts.source_lease.get_mut_after_init()?;
            lease.source_token = source_token;
            lease.shard = *ctx.accounts.shard.key();
            lease.owner = owner;
            lease.slot = WireU16::new(slot as u16);
            lease.sequence = WireU64::new(sequence);
            lease.bump = ctx.bumps.source_lease;
        }

        let mut raw = ctx.raw();
        write_create_cells(
            &mut raw,
            slot,
            sequence,
            occupied,
            occupied_count,
            owner,
            source_token,
            vault_authority,
            refund_token,
            destination_token,
            input_mint,
            output_mint,
            max_input,
            min_output,
            expiry_slot,
            allowed_executor,
            route_program,
            route_mode,
            route_commitment,
        )
    }

    #[instruction(3, ctx_args = 1)]
    pub fn claim_intent(
        mut ctx: Ctx<ClaimIntent>,
        slot: u16,
        requested_lease_slots: u64,
    ) -> ProgramResult {
        hopper::hopper_require!(requested_lease_slots > 0, InvalidClaimLease);
        ensure_live(ctx.accounts.config.as_account())?;
        let now = Clock::get()?.slot;

        let (intent, executor, lease) = {
            let shard = ctx.accounts.shard.get()?;
            let intent = snapshot(&shard, slot as usize)?;
            let executor = *ctx.accounts.executor.key();
            validate_claim_access(&intent, &executor, now)?;

            let default_ttl = ctx.accounts.config.get()?.default_claim_ttl.get();
            let requested = core::cmp::min(requested_lease_slots, default_ttl);
            let lease = core::cmp::min(now.saturating_add(requested), intent.expiry);
            (intent, executor, lease)
        };

        let mut raw = ctx.raw();
        write_cell(
            &mut raw,
            ClaimIntent::SHARD_INDEX,
            IntentShard::STATUSES_ABS_OFFSET,
            slot,
            STATUS_CLAIMED,
        )?;
        write_cell(
            &mut raw,
            ClaimIntent::SHARD_INDEX,
            IntentShard::CLAIMANTS_ABS_OFFSET,
            slot,
            executor,
        )?;
        write_cell(
            &mut raw,
            ClaimIntent::SHARD_INDEX,
            IntentShard::CLAIM_EXPIRIES_ABS_OFFSET,
            slot,
            WireU64::new(lease),
        )?;
        write_cell(
            &mut raw,
            ClaimIntent::SHARD_INDEX,
            IntentShard::REVISIONS_ABS_OFFSET,
            slot,
            WireU64::new(intent.revision.saturating_add(1)),
        )
    }

    #[instruction(4, ctx_args = 1)]
    pub fn release_claim(mut ctx: Ctx<ReleaseClaim>, slot: u16) -> ProgramResult {
        let now = Clock::get()?.slot;
        let intent = {
            let shard = ctx.accounts.shard.get()?;
            let intent = snapshot(&shard, slot as usize)?;
            hopper::hopper_require!(intent.status == STATUS_CLAIMED, InvalidIntentStatus);
            hopper::hopper_require!(now > intent.claim_expiry, ClaimStillActive);
            intent
        };

        let mut raw = ctx.raw();
        write_cell(
            &mut raw,
            ReleaseClaim::SHARD_INDEX,
            IntentShard::STATUSES_ABS_OFFSET,
            slot,
            STATUS_OPEN,
        )?;
        write_cell(
            &mut raw,
            ReleaseClaim::SHARD_INDEX,
            IntentShard::CLAIMANTS_ABS_OFFSET,
            slot,
            ZERO_ADDRESS,
        )?;
        write_cell(
            &mut raw,
            ReleaseClaim::SHARD_INDEX,
            IntentShard::CLAIM_EXPIRIES_ABS_OFFSET,
            slot,
            WireU64::new(0),
        )?;
        write_cell(
            &mut raw,
            ReleaseClaim::SHARD_INDEX,
            IntentShard::REVISIONS_ABS_OFFSET,
            slot,
            WireU64::new(intent.revision.saturating_add(1)),
        )
    }

    #[instruction(5, ctx_args = 1)]
    pub fn cancel_intent(mut ctx: Ctx<CancelIntent>, slot: u16) -> ProgramResult {
        let now = Clock::get()?.slot;
        let intent = {
            let shard = ctx.accounts.shard.get()?;
            snapshot(&shard, slot as usize)?
        };
        hopper::hopper_require!(
            intent.owner == *ctx.accounts.owner.key(),
            UnauthorizedIntentOwner
        );
        hopper::hopper_require!(
            intent.status == STATUS_OPEN
                || (intent.status == STATUS_CLAIMED && now > intent.claim_expiry),
            InvalidIntentStatus
        );

        let input_decimals = verify_refund_accounts(&ctx.accounts, &intent)?;
        let amount = token_amount(ctx.accounts.source_token.as_account())?;
        if amount > 0 {
            let owner_key = intent.owner;
            let source_key = *ctx.accounts.source_token.key();
            let bump_bytes = [ctx.bumps.vault_authority];
            let seeds = hopper::seeds!(
                VAULT_AUTHORITY_SEED,
                owner_key.as_array(),
                source_key.as_array(),
                &bump_bytes
            );
            let signers = [hopper::cpi::Signer::from(&seeds)];
            interface_transfer_checked_signed_with_program(
                ctx.accounts.source_token.as_account(),
                ctx.accounts.input_mint.as_account(),
                ctx.accounts.refund_token.as_account(),
                ctx.accounts.vault_authority.as_account(),
                ctx.accounts.token_program.as_account(),
                amount,
                input_decimals,
                &signers,
            )?;
        }
        hopper::hopper_require!(
            token_amount(ctx.accounts.source_token.as_account())? == 0,
            RefundNotEmpty
        );

        let mut raw = ctx.raw();
        write_cell(
            &mut raw,
            CancelIntent::SHARD_INDEX,
            IntentShard::STATUSES_ABS_OFFSET,
            slot,
            STATUS_CANCELLED,
        )?;
        write_cell(
            &mut raw,
            CancelIntent::SHARD_INDEX,
            IntentShard::CLAIMANTS_ABS_OFFSET,
            slot,
            ZERO_ADDRESS,
        )?;
        write_cell(
            &mut raw,
            CancelIntent::SHARD_INDEX,
            IntentShard::CLAIM_EXPIRIES_ABS_OFFSET,
            slot,
            WireU64::new(0),
        )?;
        write_cell(
            &mut raw,
            CancelIntent::SHARD_INDEX,
            IntentShard::REVISIONS_ABS_OFFSET,
            slot,
            WireU64::new(intent.revision.saturating_add(1)),
        )
    }

    #[instruction(6, ctx_args = 1)]
    #[remaining_accounts(max = MAX_ROUTE_ACCOUNTS)]
    pub fn execute_intent(
        mut ctx: Ctx<ExecuteIntent>,
        slot: u16,
        route_data: HopperVec<u8, MAX_ROUTE_DATA>,
        route_meta_flags: [u8; MAX_ROUTE_ACCOUNTS],
    ) -> ProgramResult {
        ensure_live(ctx.accounts.config.as_account())?;
        ctx.accounts.route_program.as_account().check_executable()?;
        let now = Clock::get()?.slot;

        let (intent, executor, input_decimals) = {
            let shard = ctx.accounts.shard.get()?;
            let intent = snapshot(&shard, slot as usize)?;
            let executor = *ctx.accounts.executor.key();

            validate_execution_access(&intent, &executor, now)?;
            let input_decimals = verify_execute_accounts(&ctx.accounts, &intent)?;
            (intent, executor, input_decimals)
        };

        let pre_source = token_amount(ctx.accounts.source_token.as_account())?;
        let pre_destination = token_amount(ctx.accounts.destination_token.as_account())?;
        let pre_source_policy = token_policy_hash(ctx.accounts.source_token.as_account())?;
        let pre_destination_policy =
            token_policy_hash(ctx.accounts.destination_token.as_account())?;
        let pre_input_mint_policy = mint_policy_hash(ctx.accounts.input_mint.as_account())?;
        let pre_output_mint_policy = mint_policy_hash(ctx.accounts.output_mint.as_account())?;

        let remaining = ctx
            .remaining_accounts_passthrough()
            .account_views::<MAX_ROUTE_ACCOUNTS>()?;
        validate_unused_route_flags(remaining.len(), &route_meta_flags)?;
        validate_route_accounts(
            ctx.program_id(),
            ctx.accounts.config.key(),
            ctx.accounts.shard.key(),
            ctx.accounts.source_token.key(),
            ctx.accounts.vault_authority.key(),
            ctx.accounts.refund_token.key(),
            &remaining,
            &route_meta_flags,
        )?;

        let route_program = ctx.accounts.route_program.key();
        hopper::hopper_require!(*route_program == intent.route_program, RouteProgramMismatch);
        let route_hash = compute_route_commitment(
            route_program,
            route_data.as_slice(),
            &remaining,
            &route_meta_flags,
        )?;
        if intent.route_mode == ROUTE_MODE_EXACT {
            hopper::hopper_require!(
                route_hash == intent.route_commitment,
                RouteCommitmentMismatch
            );
        }

        let owner_key = intent.owner;
        let source_key = *ctx.accounts.source_token.key();
        let bump_bytes = [ctx.bumps.vault_authority];
        let seeds = hopper::seeds!(
            VAULT_AUTHORITY_SEED,
            owner_key.as_array(),
            source_key.as_array(),
            &bump_bytes
        );
        let signers = [hopper::cpi::Signer::from(&seeds)];

        invoke_route(
            route_program,
            route_data.as_slice(),
            &remaining,
            &route_meta_flags,
            &signers,
        )?;

        let route_post_source = token_amount(ctx.accounts.source_token.as_account())?;
        let post_destination = token_amount(ctx.accounts.destination_token.as_account())?;
        hopper::hopper_require!(
            token_policy_hash(ctx.accounts.source_token.as_account())? == pre_source_policy,
            SourceTokenPolicyChanged
        );
        hopper::hopper_require!(
            token_policy_hash(ctx.accounts.destination_token.as_account())?
                == pre_destination_policy,
            DestinationTokenPolicyChanged
        );
        hopper::hopper_require!(
            mint_policy_hash(ctx.accounts.input_mint.as_account())? == pre_input_mint_policy,
            InputMintPolicyChanged
        );
        hopper::hopper_require!(
            mint_policy_hash(ctx.accounts.output_mint.as_account())? == pre_output_mint_policy,
            OutputMintPolicyChanged
        );

        let spent = pre_source
            .checked_sub(route_post_source)
            .ok_or_else(|| ProgramError::from(InputBalanceIncreased))?;
        let received = post_destination
            .checked_sub(pre_destination)
            .ok_or_else(|| ProgramError::from(OutputBalanceDecreased))?;

        hopper::hopper_require!(spent > 0 && received > 0, EmptySettlement);
        hopper::hopper_require!(spent <= intent.max_input, MaximumInputExceeded);
        hopper::hopper_require!(received >= intent.min_output, MinimumOutputNotMet);

        // Refund every unused source token before state becomes final.
        if route_post_source > 0 {
            interface_transfer_checked_signed_with_program(
                ctx.accounts.source_token.as_account(),
                ctx.accounts.input_mint.as_account(),
                ctx.accounts.refund_token.as_account(),
                ctx.accounts.vault_authority.as_account(),
                ctx.accounts.token_program.as_account(),
                route_post_source,
                input_decimals,
                &signers,
            )?;
        }
        hopper::hopper_require!(
            token_amount(ctx.accounts.source_token.as_account())? == 0,
            RefundNotEmpty
        );

        // Drop the borrowed remaining-account set before acquiring shard
        // mutation leases through the generated typed context.
        drop(remaining);

        let settlement_hash = compute_settlement_hash(
            &intent,
            ctx.accounts.shard.key(),
            &executor,
            &route_hash,
            spent,
            received,
            now,
        )?;

        let mut raw = ctx.raw();
        write_cell(
            &mut raw,
            ExecuteIntent::SHARD_INDEX,
            IntentShard::STATUSES_ABS_OFFSET,
            slot,
            STATUS_SETTLED,
        )?;
        write_cell(
            &mut raw,
            ExecuteIntent::SHARD_INDEX,
            IntentShard::CLAIMANTS_ABS_OFFSET,
            slot,
            executor,
        )?;
        write_cell(
            &mut raw,
            ExecuteIntent::SHARD_INDEX,
            IntentShard::SETTLED_INPUTS_ABS_OFFSET,
            slot,
            WireU64::new(spent),
        )?;
        write_cell(
            &mut raw,
            ExecuteIntent::SHARD_INDEX,
            IntentShard::SETTLED_OUTPUTS_ABS_OFFSET,
            slot,
            WireU64::new(received),
        )?;
        write_cell(
            &mut raw,
            ExecuteIntent::SHARD_INDEX,
            IntentShard::SETTLEMENT_HASHES_ABS_OFFSET,
            slot,
            settlement_hash,
        )?;
        write_cell(
            &mut raw,
            ExecuteIntent::SHARD_INDEX,
            IntentShard::CLAIM_EXPIRIES_ABS_OFFSET,
            slot,
            WireU64::new(0),
        )?;
        write_cell(
            &mut raw,
            ExecuteIntent::SHARD_INDEX,
            IntentShard::REVISIONS_ABS_OFFSET,
            slot,
            WireU64::new(intent.revision.saturating_add(1)),
        )
    }

    #[instruction(7, ctx_args = 1)]
    pub fn reclaim_intent(mut ctx: Ctx<ReclaimIntent>, slot: u16) -> ProgramResult {
        let (intent, occupied, occupied_count) = {
            let shard = ctx.accounts.shard.get()?;
            let intent = snapshot(&shard, slot as usize)?;
            (intent, shard.occupied.get(), shard.occupied_count.get())
        };
        hopper::hopper_require!(
            intent.status == STATUS_SETTLED || intent.status == STATUS_CANCELLED,
            IntentNotFinal
        );
        hopper::hopper_require!(
            *ctx.accounts.owner.key() == intent.owner,
            UnauthorizedIntentOwner
        );
        hopper::hopper_require!(
            *ctx.accounts.source_token.key() == intent.source_token,
            TokenAccountMismatch
        );
        hopper::hopper_require!(
            *ctx.accounts.vault_authority.key() == intent.vault_authority,
            TokenAuthorityMismatch
        );
        verify_token_account(
            ctx.accounts.source_token.as_account(),
            &intent.input_mint,
            &intent.vault_authority,
        )?;
        hopper::hopper_require!(
            token_amount(ctx.accounts.source_token.as_account())? == 0,
            SourceNotEmpty
        );

        // Return custody of the now-empty source token account before the
        // record and its global uniqueness lease disappear. This prevents an
        // otherwise successful Cicada lifecycle from leaving the user's token
        // account permanently controlled by an unreachable PDA.
        let owner_key = intent.owner;
        let source_key = intent.source_token;
        let bump_bytes = [ctx.bumps.vault_authority];
        let seeds = hopper::seeds!(
            VAULT_AUTHORITY_SEED,
            owner_key.as_array(),
            source_key.as_array(),
            &bump_bytes
        );
        let signers = [hopper::cpi::Signer::from(&seeds)];
        interface_set_account_owner_signed(
            ctx.accounts.source_token.as_account(),
            ctx.accounts.vault_authority.as_account(),
            ctx.accounts.token_program.as_account(),
            &intent.owner,
            &signers,
        )?;

        let mut raw = ctx.raw();
        clear_intent_cells(&mut raw, slot, occupied, occupied_count)?;
        drop(raw);

        // Release the source-vault uniqueness marker only after custody is
        // restored and the record is final. `close = owner` generates this
        // sentinel-protected method; lifecycle closes remain explicit so a
        // handler cannot accidentally drain an account on a partial path.
        ctx.close_source_lease()
    }

    #[instruction(8)]
    pub fn pause(mut ctx: Ctx<SetPause>) -> ProgramResult {
        {
            let mut paused = ctx.config_paused_mut()?;
            *paused = 1;
        }
        {
            let mut revision = ctx.config_revision_mut()?;
            revision.checked_add_assign(1)?;
        }
        Ok(())
    }

    #[instruction(9)]
    pub fn unpause(mut ctx: Ctx<SetPause>) -> ProgramResult {
        {
            let mut paused = ctx.config_paused_mut()?;
            *paused = 0;
        }
        {
            let mut revision = ctx.config_revision_mut()?;
            revision.checked_add_assign(1)?;
        }
        Ok(())
    }
}

// ── Account indices ─────────────────────────────────────────────────
//
// Runtime-indexed column cells need the flattened shard slot. Keep these
// adjacent to their context layouts and cover them with manifest tests below.

impl<'info> CreateIntent<'info> {
    const SHARD_INDEX: usize = 2;
}

impl<'info> ClaimIntent<'info> {
    const SHARD_INDEX: usize = 2;
}

impl<'info> ReleaseClaim<'info> {
    const SHARD_INDEX: usize = 1;
}

impl<'info> CancelIntent<'info> {
    const SHARD_INDEX: usize = 2;
}

impl<'info> ExecuteIntent<'info> {
    const SHARD_INDEX: usize = 2;
}

impl<'info> ReclaimIntent<'info> {
    const SHARD_INDEX: usize = 2;
}

// ── State helpers ───────────────────────────────────────────────────

#[inline]
fn ensure_live(config: &AccountView<'_>) -> ProgramResult {
    let config = config.load::<CicadaConfig>()?;
    hopper::hopper_require!(config.paused == 0, ProtocolPaused);
    Ok(())
}

#[inline]
fn slot_bit(slot: usize) -> u32 {
    1u32 << slot
}

#[inline]
fn is_occupied(shard: &IntentShard, slot: usize) -> bool {
    slot < INTENTS_PER_SHARD && shard.occupied.get() & slot_bit(slot) != 0
}

fn find_free_slot(shard: &IntentShard) -> Option<usize> {
    let occupied = shard.occupied.get();
    let mut slot = 0usize;
    while slot < INTENTS_PER_SHARD {
        if occupied & slot_bit(slot) == 0 {
            return Some(slot);
        }
        slot += 1;
    }
    None
}

fn ensure_source_unique(shard: &IntentShard, source: &Address) -> ProgramResult {
    let mut slot = 0usize;
    while slot < INTENTS_PER_SHARD {
        if is_occupied(shard, slot) && shard.source_tokens[slot] == *source {
            return Err(SourceAlreadyInUse.into());
        }
        slot += 1;
    }
    Ok(())
}

fn snapshot(shard: &IntentShard, slot: usize) -> Result<IntentSnapshot> {
    hopper::hopper_require!(slot < INTENTS_PER_SHARD, SlotOutOfRange);
    hopper::hopper_require!(is_occupied(shard, slot), SlotNotOccupied);
    Ok(IntentSnapshot {
        owner: shard.owners[slot],
        source_token: shard.source_tokens[slot],
        vault_authority: shard.vault_authorities[slot],
        refund_token: shard.refund_tokens[slot],
        destination_token: shard.destination_tokens[slot],
        input_mint: shard.input_mints[slot],
        output_mint: shard.output_mints[slot],
        max_input: shard.max_inputs[slot].get(),
        min_output: shard.min_outputs[slot].get(),
        expiry: shard.expiries[slot].get(),
        allowed_executor: shard.allowed_executors[slot],
        route_program: shard.route_programs[slot],
        route_commitment: shard.route_commitments[slot],
        route_mode: shard.route_modes[slot],
        sequence: shard.sequences[slot].get(),
        status: shard.statuses[slot],
        claimant: shard.claimants[slot],
        claim_expiry: shard.claim_expiries[slot].get(),
        revision: shard.revisions[slot].get(),
    })
}

fn validate_claim_access(intent: &IntentSnapshot, executor: &Address, now: u64) -> ProgramResult {
    hopper::hopper_require!(intent.status == STATUS_OPEN, InvalidIntentStatus);
    hopper::hopper_require!(now < intent.expiry, IntentAlreadyExpired);
    // A pre-execution lease is a reservation, not a public lock. Open
    // permissionless intents execute atomically from STATUS_OPEN so a solver
    // cannot grief them by repeatedly taking short leases.
    hopper::hopper_require!(
        !address::address_is_zero(&intent.allowed_executor),
        PermissionlessClaimForbidden
    );
    hopper::hopper_require!(intent.allowed_executor == *executor, UnauthorizedExecutor);
    Ok(())
}

fn validate_execution_access(
    intent: &IntentSnapshot,
    executor: &Address,
    now: u64,
) -> ProgramResult {
    if address::address_is_zero(&intent.allowed_executor) {
        // Permissionless intents are one-shot and atomic: no separate lease
        // can be used to censor or delay another solver.
        hopper::hopper_require!(intent.status == STATUS_OPEN, InvalidIntentStatus);
    } else {
        hopper::hopper_require!(intent.allowed_executor == *executor, UnauthorizedExecutor);
        hopper::hopper_require!(
            intent.status == STATUS_OPEN || intent.status == STATUS_CLAIMED,
            InvalidIntentStatus
        );
        if intent.status == STATUS_CLAIMED {
            hopper::hopper_require!(intent.claimant == *executor, UnauthorizedExecutor);
            hopper::hopper_require!(now <= intent.claim_expiry, ClaimExpired);
        }
    }
    hopper::hopper_require!(now < intent.expiry, IntentAlreadyExpired);
    Ok(())
}

#[inline]
fn cell_offset<T>(column_abs_offset: u32, slot: u16) -> u32 {
    column_abs_offset + slot as u32 * size_of::<T>() as u32
}

#[inline]
fn write_cell<T: hopper::layout::Pod + Copy>(
    ctx: &mut ScopedContext<'_, '_>,
    account_index: usize,
    column_abs_offset: u32,
    slot: u16,
    value: T,
) -> ProgramResult {
    hopper::hopper_require!((slot as usize) < INTENTS_PER_SHARD, SlotOutOfRange);
    let mut cell =
        ctx.segment_mut::<T>(account_index, cell_offset::<T>(column_abs_offset, slot))?;
    *cell = value;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_create_cells(
    ctx: &mut ScopedContext<'_, '_>,
    slot: usize,
    sequence: u64,
    occupied: u32,
    occupied_count: u16,
    owner: Address,
    source_token: Address,
    vault_authority: Address,
    refund_token: Address,
    destination_token: Address,
    input_mint: Address,
    output_mint: Address,
    max_input: u64,
    min_output: u64,
    expiry_slot: u64,
    allowed_executor: Address,
    route_program: Address,
    route_mode: u8,
    route_commitment: [u8; 32],
) -> ProgramResult {
    let slot = slot as u16;
    let shard = CreateIntent::SHARD_INDEX;

    write_cell(ctx, shard, IntentShard::OWNERS_ABS_OFFSET, slot, owner)?;
    write_cell(
        ctx,
        shard,
        IntentShard::SOURCE_TOKENS_ABS_OFFSET,
        slot,
        source_token,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::VAULT_AUTHORITIES_ABS_OFFSET,
        slot,
        vault_authority,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::REFUND_TOKENS_ABS_OFFSET,
        slot,
        refund_token,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::DESTINATION_TOKENS_ABS_OFFSET,
        slot,
        destination_token,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::INPUT_MINTS_ABS_OFFSET,
        slot,
        input_mint,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::OUTPUT_MINTS_ABS_OFFSET,
        slot,
        output_mint,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::MAX_INPUTS_ABS_OFFSET,
        slot,
        WireU64::new(max_input),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::MIN_OUTPUTS_ABS_OFFSET,
        slot,
        WireU64::new(min_output),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::EXPIRIES_ABS_OFFSET,
        slot,
        WireU64::new(expiry_slot),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::ALLOWED_EXECUTORS_ABS_OFFSET,
        slot,
        allowed_executor,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::ROUTE_PROGRAMS_ABS_OFFSET,
        slot,
        route_program,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::ROUTE_COMMITMENTS_ABS_OFFSET,
        slot,
        route_commitment,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::ROUTE_MODES_ABS_OFFSET,
        slot,
        route_mode,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::SEQUENCES_ABS_OFFSET,
        slot,
        WireU64::new(sequence),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::STATUSES_ABS_OFFSET,
        slot,
        STATUS_OPEN,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::CLAIMANTS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::CLAIM_EXPIRIES_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::SETTLED_INPUTS_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::SETTLED_OUTPUTS_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::SETTLEMENT_HASHES_ABS_OFFSET,
        slot,
        ZERO_HASH,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::REVISIONS_ABS_OFFSET,
        slot,
        WireU64::new(1),
    )?;

    {
        let mut value = ctx.segment_mut::<WireU32>(shard, IntentShard::OCCUPIED_ABS_OFFSET)?;
        *value = WireU32::new(occupied | slot_bit(slot as usize));
    }
    {
        let mut value =
            ctx.segment_mut::<WireU16>(shard, IntentShard::OCCUPIED_COUNT_ABS_OFFSET)?;
        *value = WireU16::new(
            occupied_count
                .checked_add(1)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );
    }
    {
        let mut value = ctx.segment_mut::<WireU64>(shard, IntentShard::NEXT_SEQUENCE_ABS_OFFSET)?;
        *value = WireU64::new(
            sequence
                .checked_add(1)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );
    }
    Ok(())
}

fn clear_intent_cells(
    ctx: &mut ScopedContext<'_, '_>,
    slot: u16,
    occupied: u32,
    occupied_count: u16,
) -> ProgramResult {
    hopper::hopper_require!((slot as usize) < INTENTS_PER_SHARD, SlotOutOfRange);
    let shard = ReclaimIntent::SHARD_INDEX;

    {
        let mut value = ctx.segment_mut::<WireU32>(shard, IntentShard::OCCUPIED_ABS_OFFSET)?;
        *value = WireU32::new(occupied & !slot_bit(slot as usize));
    }
    {
        let mut value =
            ctx.segment_mut::<WireU16>(shard, IntentShard::OCCUPIED_COUNT_ABS_OFFSET)?;
        *value = WireU16::new(
            occupied_count
                .checked_sub(1)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );
    }

    write_cell(
        ctx,
        shard,
        IntentShard::OWNERS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::SOURCE_TOKENS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::VAULT_AUTHORITIES_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::REFUND_TOKENS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::DESTINATION_TOKENS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::INPUT_MINTS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::OUTPUT_MINTS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::MAX_INPUTS_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::MIN_OUTPUTS_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::EXPIRIES_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::ALLOWED_EXECUTORS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::ROUTE_PROGRAMS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::ROUTE_COMMITMENTS_ABS_OFFSET,
        slot,
        ZERO_HASH,
    )?;
    write_cell(ctx, shard, IntentShard::ROUTE_MODES_ABS_OFFSET, slot, 0u8)?;
    write_cell(
        ctx,
        shard,
        IntentShard::SEQUENCES_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::STATUSES_ABS_OFFSET,
        slot,
        STATUS_EMPTY,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::CLAIMANTS_ABS_OFFSET,
        slot,
        ZERO_ADDRESS,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::CLAIM_EXPIRIES_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::SETTLED_INPUTS_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::SETTLED_OUTPUTS_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::SETTLEMENT_HASHES_ABS_OFFSET,
        slot,
        ZERO_HASH,
    )?;
    write_cell(
        ctx,
        shard,
        IntentShard::REVISIONS_ABS_OFFSET,
        slot,
        WireU64::new(0),
    )
}

// ── Token and route verification ────────────────────────────────────

/// Return an SPL Token or Token-2022 account's owner authority.
///
/// Both token programs share SetAuthority's wire layout. Resolving the target
/// program from the token account owner keeps reclaim transport-neutral and
/// avoids trapping Token-2022 accounts behind the Cicada vault PDA.
fn interface_set_account_owner_signed<'a>(
    account: &'a AccountView<'a>,
    current_authority: &'a AccountView<'a>,
    token_program: &'a AccountView<'a>,
    new_authority: &'a Address,
    signers: &[hopper::cpi::Signer<'_, '_>],
) -> ProgramResult {
    let kind = TokenProgramKind::for_account(account)?;
    hopper::hopper_require!(
        token_program.address() == kind.program_id(),
        TokenProgramMismatch
    );
    token_program.check_executable()?;
    let mut data = [0u8; 35];
    data[0] = 6; // SetAuthority
    data[1] = 2; // AccountOwner
    data[2] = 1; // COption::Some
    data[3..].copy_from_slice(new_authority.as_array());

    let accounts = [
        InstructionAccount::writable(account.address()),
        InstructionAccount::readonly_signer(current_authority.address()),
    ];
    let views = [account, current_authority, token_program];
    let instruction = InstructionView {
        program_id: kind.program_id(),
        data: &data,
        accounts: &accounts,
    };
    hopper::cpi::invoke_signed(&instruction, &views, signers)
}

fn token_amount(account: &AccountView<'_>) -> Result<u64> {
    let kind = TokenProgramKind::for_account(account)?;
    let data = account.try_borrow()?;
    let token = InterfaceTokenAccount::from_data(&data, kind)?;
    token.assert_initialized()?;
    token.amount()
}

/// Verify a mint and return its decimals. Token-2022 mints are accepted only
/// when their extensions preserve Cicada's amount-only settlement model.
fn verified_mint_decimals(account: &AccountView<'_>) -> Result<u8> {
    let kind = TokenProgramKind::for_account(account)?;
    let data = account.try_borrow()?;
    let mint = InterfaceMint::from_data(&data, kind)?;
    mint.assert_initialized()?;
    if matches!(kind, TokenProgramKind::Token2022) {
        hopper::token_2022::check_safe_token_2022_mint(&data)
            .map_err(|_| ProgramError::from(UnsafeTokenExtension))?;
    }
    mint.decimals()
}

fn verify_token_program_pair(
    token_account: &AccountView<'_>,
    mint_account: &AccountView<'_>,
) -> ProgramResult {
    hopper::hopper_require!(
        TokenProgramKind::for_account(token_account)?
            == TokenProgramKind::for_account(mint_account)?,
        TokenProgramMismatch
    );
    Ok(())
}

fn verify_token_program_account(
    token_account: &AccountView<'_>,
    token_program: &AccountView<'_>,
) -> ProgramResult {
    let kind = TokenProgramKind::for_account(token_account)?;
    hopper::hopper_require!(
        token_program.address() == kind.program_id(),
        TokenProgramMismatch
    );
    token_program.check_executable()?;
    Ok(())
}

fn verify_token_account(
    account: &AccountView<'_>,
    mint: &Address,
    authority: &Address,
) -> ProgramResult {
    let kind = TokenProgramKind::for_account(account)?;
    let data = account.try_borrow()?;
    let token = InterfaceTokenAccount::from_data(&data, kind)?;
    token.assert_initialized()?;
    if token.mint()? != mint {
        return Err(TokenMintMismatch.into());
    }
    if token.owner()? != authority {
        return Err(TokenAuthorityMismatch.into());
    }
    Ok(())
}

fn verify_create_token_accounts(accounts: &CreateIntent<'_>, max_input: u64) -> ProgramResult {
    let owner = *accounts.owner.key();
    let vault_authority = *accounts.vault_authority.key();
    let source_token = *accounts.source_token.key();
    let refund_token = *accounts.refund_token.key();
    let destination_token = *accounts.destination_token.key();
    let input_mint = *accounts.input_mint.key();
    let output_mint = *accounts.output_mint.key();

    hopper::hopper_require!(
        source_token != refund_token
            && source_token != destination_token
            && refund_token != destination_token,
        AliasedSettlementAccounts
    );

    let _ = verified_mint_decimals(accounts.input_mint.as_account())?;
    let _ = verified_mint_decimals(accounts.output_mint.as_account())?;
    verify_token_program_pair(
        accounts.source_token.as_account(),
        accounts.input_mint.as_account(),
    )?;
    verify_token_program_pair(
        accounts.refund_token.as_account(),
        accounts.input_mint.as_account(),
    )?;
    verify_token_program_pair(
        accounts.destination_token.as_account(),
        accounts.output_mint.as_account(),
    )?;
    verify_token_account(
        accounts.source_token.as_account(),
        &input_mint,
        &vault_authority,
    )?;
    verify_token_account(accounts.refund_token.as_account(), &input_mint, &owner)?;
    verify_token_account(
        accounts.destination_token.as_account(),
        &output_mint,
        &owner,
    )?;
    hopper::hopper_require!(
        token_amount(accounts.source_token.as_account())? >= max_input,
        InsufficientVaultFunds
    );
    Ok(())
}

fn verify_refund_accounts(accounts: &CancelIntent<'_>, intent: &IntentSnapshot) -> Result<u8> {
    hopper::hopper_require!(
        *accounts.source_token.key() == intent.source_token,
        TokenAccountMismatch
    );
    hopper::hopper_require!(
        *accounts.vault_authority.key() == intent.vault_authority,
        TokenAuthorityMismatch
    );
    hopper::hopper_require!(
        *accounts.refund_token.key() == intent.refund_token,
        RefundAccountMismatch
    );
    hopper::hopper_require!(
        *accounts.input_mint.key() == intent.input_mint,
        TokenMintMismatch
    );

    let decimals = verified_mint_decimals(accounts.input_mint.as_account())?;
    verify_token_program_pair(
        accounts.source_token.as_account(),
        accounts.input_mint.as_account(),
    )?;
    verify_token_program_pair(
        accounts.refund_token.as_account(),
        accounts.input_mint.as_account(),
    )?;
    verify_token_program_account(
        accounts.source_token.as_account(),
        accounts.token_program.as_account(),
    )?;
    verify_token_account(
        accounts.source_token.as_account(),
        &intent.input_mint,
        &intent.vault_authority,
    )?;
    verify_token_account(
        accounts.refund_token.as_account(),
        &intent.input_mint,
        &intent.owner,
    )?;
    Ok(decimals)
}

fn verify_execute_accounts(accounts: &ExecuteIntent<'_>, intent: &IntentSnapshot) -> Result<u8> {
    hopper::hopper_require!(
        *accounts.intent_owner.key() == intent.owner,
        UnauthorizedIntentOwner
    );
    hopper::hopper_require!(
        *accounts.source_token.key() == intent.source_token,
        TokenAccountMismatch
    );
    hopper::hopper_require!(
        *accounts.vault_authority.key() == intent.vault_authority,
        TokenAuthorityMismatch
    );
    hopper::hopper_require!(
        *accounts.refund_token.key() == intent.refund_token,
        RefundAccountMismatch
    );
    hopper::hopper_require!(
        *accounts.destination_token.key() == intent.destination_token,
        TokenAccountMismatch
    );
    hopper::hopper_require!(
        *accounts.input_mint.key() == intent.input_mint,
        TokenMintMismatch
    );
    hopper::hopper_require!(
        *accounts.output_mint.key() == intent.output_mint,
        TokenMintMismatch
    );
    hopper::hopper_require!(
        *accounts.route_program.key() == intent.route_program,
        RouteProgramMismatch
    );

    let input_decimals = verified_mint_decimals(accounts.input_mint.as_account())?;
    let _ = verified_mint_decimals(accounts.output_mint.as_account())?;
    verify_token_program_pair(
        accounts.source_token.as_account(),
        accounts.input_mint.as_account(),
    )?;
    verify_token_program_pair(
        accounts.refund_token.as_account(),
        accounts.input_mint.as_account(),
    )?;
    verify_token_program_pair(
        accounts.destination_token.as_account(),
        accounts.output_mint.as_account(),
    )?;
    verify_token_program_account(
        accounts.source_token.as_account(),
        accounts.token_program.as_account(),
    )?;
    verify_token_account(
        accounts.source_token.as_account(),
        &intent.input_mint,
        &intent.vault_authority,
    )?;
    verify_token_account(
        accounts.refund_token.as_account(),
        &intent.input_mint,
        &intent.owner,
    )?;
    verify_token_account(
        accounts.destination_token.as_account(),
        &intent.output_mint,
        &intent.owner,
    )?;
    Ok(input_decimals)
}

/// Commit to every token-account policy byte except the amount at bytes 64..72.
/// With unsafe Token-2022 extensions rejected, a normal route may change only
/// the balance while owner, mint, delegate, state, close authority, and TLV
/// policy remain byte-identical.
fn token_policy_hash(account: &AccountView<'_>) -> Result<[u8; 32]> {
    let data = account.try_borrow()?;
    if data.len() < 165 {
        return Err(ProgramError::InvalidAccountData);
    }
    crypto::sha256(&[b"cicada-token-policy-v1", &data[..64], &data[72..]])
}

/// Commit to mint policy while permitting ordinary mint/burn supply changes.
///
/// SPL Mint stores `supply` at bytes 36..44. Cicada excludes only that field;
/// mint authority, decimals, initialization, freeze authority, and all
/// Token-2022 extension bytes must remain identical across the route CPI.
fn mint_policy_hash(account: &AccountView<'_>) -> Result<[u8; 32]> {
    let data = account.try_borrow()?;
    if data.len() < 82 {
        return Err(ProgramError::InvalidAccountData);
    }
    crypto::sha256(&[b"cicada-mint-policy-v1", &data[..36], &data[44..]])
}

fn validate_unused_route_flags(
    account_count: usize,
    flags: &[u8; MAX_ROUTE_ACCOUNTS],
) -> ProgramResult {
    hopper::hopper_require!(
        account_count <= MAX_ROUTE_ACCOUNTS,
        RouteAccountCountMismatch
    );
    let mut index = account_count;
    while index < MAX_ROUTE_ACCOUNTS {
        hopper::hopper_require!(flags[index] == 0, NonZeroUnusedRouteFlags);
        index += 1;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_route_accounts<const N: usize>(
    cicada_program: &Address,
    config: &Address,
    shard: &Address,
    source_token: &Address,
    vault_authority: &Address,
    refund_token: &Address,
    accounts: &hopper::hopper_runtime::RemainingAccountViews<'_, N>,
    flags: &[u8; MAX_ROUTE_ACCOUNTS],
) -> ProgramResult {
    let mut index = 0usize;
    while index < accounts.len() {
        let account = accounts
            .get(index)
            .ok_or_else(|| ProgramError::from(RouteAccountCountMismatch))?;
        let meta = flags[index];
        hopper::hopper_require!(meta & !ROUTE_META_KNOWN_FLAGS == 0, InvalidRouteMetaFlags);

        let writable = meta & ROUTE_META_WRITABLE != 0;
        let signer = meta & ROUTE_META_SIGNER != 0;
        if writable {
            hopper::hopper_require!(account.is_writable(), RouteMetaPrivilegeEscalation);
        }
        if signer && !account.is_signer() {
            hopper::hopper_require!(
                account.address() == vault_authority,
                RouteMetaPrivilegeEscalation
            );
        }
        // The PDA is a signing capability only. A route never needs to mutate
        // or allocate the authority account itself, and allowing that would
        // let a callee persist arbitrary state at the per-vault signer address.
        if writable && account.address() == vault_authority {
            return Err(ProtectedAccountDelegation.into());
        }

        // Cicada's own state is never delegated to an arbitrary route CPI.
        if account.address() == shard || account.address() == config {
            return Err(ProtectedAccountDelegation.into());
        }
        if writable && account.owned_by(cicada_program) {
            return Err(ProtectedAccountDelegation.into());
        }

        // Refund is controlled by Cicada after route settlement. A route may
        // inspect it, but cannot write to it.
        if writable && account.address() == refund_token {
            return Err(ProtectedAccountDelegation.into());
        }

        // The per-vault PDA may own only the committed source vault inside
        // this route. This prevents a solver from smuggling another Cicada
        // vault under the same signer capability.
        if writable && account.address() != source_token {
            if let Ok(kind) = TokenProgramKind::for_account(account) {
                if let Ok(data) = account.try_borrow() {
                    if let Ok(token) = InterfaceTokenAccount::from_data(&data, kind) {
                        if token.owner()? == vault_authority {
                            return Err(OtherIntentVaultDelegation.into());
                        }
                    }
                }
            }
        }
        index += 1;
    }
    Ok(())
}

/// Submit the user-selected route in a separate SBF stack frame.
///
/// `DynCpi::invoke_signed` assembles ordered metas and a deduplicated account-
/// info projection. Keeping those bounded scratch arrays out of the main
/// execution handler leaves room for the intent snapshot, route bytes, and
/// four policy commitments without reducing the useful 32-account ceiling.
#[inline(never)]
fn invoke_route<'a, const N: usize>(
    route_program: &'a Address,
    route_data: &[u8],
    accounts: &hopper::hopper_runtime::RemainingAccountViews<'a, N>,
    flags: &[u8; MAX_ROUTE_ACCOUNTS],
    signers: &[hopper::cpi::Signer<'_, '_>],
) -> ProgramResult {
    let mut cpi: DynCpi<MAX_ROUTE_ACCOUNTS, MAX_ROUTE_DATA> = DynCpi::new(route_program);
    let mut index = 0usize;
    while index < accounts.len() {
        let account = accounts
            .get(index)
            .ok_or_else(|| ProgramError::from(RouteAccountCountMismatch))?;
        let meta = flags[index];
        cpi.push_account(
            account,
            meta & ROUTE_META_WRITABLE != 0,
            meta & ROUTE_META_SIGNER != 0,
        )?;
        index += 1;
    }
    cpi.push_data(route_data)?;
    cpi.invoke_signed(signers)
}

/// Hash an exact route envelope without allocating one large buffer.
///
/// Route data is hashed once. Account metas are hashed in ordered chunks of
/// eight `(address, flags)` records. Duplicate accounts and positional order
/// are preserved, while the final domain-separated digest commits to the
/// target program, data, account count, and every chunk.
pub fn compute_route_commitment<const N: usize>(
    route_program: &Address,
    route_data: &[u8],
    accounts: &hopper::hopper_runtime::RemainingAccountViews<'_, N>,
    flags: &[u8; MAX_ROUTE_ACCOUNTS],
) -> Result<[u8; 32]> {
    hopper::hopper_require!(
        accounts.len() <= MAX_ROUTE_ACCOUNTS,
        RouteAccountCountMismatch
    );
    let data_hash = crypto::sha256_single(route_data)?;
    let mut chunk_hashes = [[0u8; 32]; ROUTE_HASH_CHUNKS];
    let mut chunk_count = 0usize;
    let mut cursor = 0usize;

    while cursor < accounts.len() {
        let take = core::cmp::min(8, accounts.len() - cursor);
        let mut chunk = [0u8; 8 * 33];
        let mut index = 0usize;
        while index < take {
            let account = accounts
                .get(cursor + index)
                .ok_or_else(|| ProgramError::from(RouteAccountCountMismatch))?;
            let base = index * 33;
            chunk[base..base + 32].copy_from_slice(account.address().as_array());
            chunk[base + 32] = flags[cursor + index];
            index += 1;
        }
        chunk_hashes[chunk_count] = crypto::sha256_single(&chunk[..take * 33])?;
        chunk_count += 1;
        cursor += take;
    }

    let mut final_bytes = [0u8; 16 + 32 + 32 + 1 + ROUTE_HASH_CHUNKS * 32];
    final_bytes[..16].copy_from_slice(b"cicada-route-v1!");
    final_bytes[16..48].copy_from_slice(route_program.as_array());
    final_bytes[48..80].copy_from_slice(&data_hash);
    final_bytes[80] = accounts.len() as u8;
    let mut index = 0usize;
    while index < chunk_count {
        let base = 81 + index * 32;
        final_bytes[base..base + 32].copy_from_slice(&chunk_hashes[index]);
        index += 1;
    }
    crypto::sha256_single(&final_bytes[..81 + chunk_count * 32])
}

fn compute_settlement_hash(
    intent: &IntentSnapshot,
    shard: &Address,
    executor: &Address,
    route_hash: &[u8; 32],
    spent: u64,
    received: u64,
    slot: u64,
) -> Result<[u8; 32]> {
    let sequence = intent.sequence.to_le_bytes();
    let spent = spent.to_le_bytes();
    let received = received.to_le_bytes();
    let slot = slot.to_le_bytes();
    crypto::sha256(&[
        b"cicada-settlement-v1",
        shard.as_array(),
        intent.owner.as_array(),
        executor.as_array(),
        intent.source_token.as_array(),
        intent.destination_token.as_array(),
        &sequence,
        route_hash,
        &spent,
        &received,
        &slot,
    ])
}

hopper::program_manifest! {
    program = cicada_program,
    layouts = [CicadaConfig, SourceLease, IntentShard],
}

#[cfg(test)]
mod tests {
    use super::*;
    use grillo_manifest::MutationManifest;
    use grillo_verifier::{
        verify, verify_invocation, AccountDelta, InconclusiveReason, TouchMap, TouchRecord, Verdict,
    };
    use hopper::hopper_runtime::write_policy::{WritePolicy, WriteRange};

    fn test_intent() -> IntentSnapshot {
        IntentSnapshot {
            owner: Address::new([1u8; 32]),
            source_token: Address::new([2u8; 32]),
            vault_authority: Address::new([3u8; 32]),
            refund_token: Address::new([4u8; 32]),
            destination_token: Address::new([5u8; 32]),
            input_mint: Address::new([6u8; 32]),
            output_mint: Address::new([7u8; 32]),
            max_input: 100,
            min_output: 90,
            expiry: 1_000,
            allowed_executor: ZERO_ADDRESS,
            route_program: Address::new([8u8; 32]),
            route_commitment: ZERO_HASH,
            route_mode: ROUTE_MODE_PROGRAM,
            sequence: 1,
            status: STATUS_OPEN,
            claimant: ZERO_ADDRESS,
            claim_expiry: 0,
            revision: 1,
        }
    }

    fn range_covers(ranges: &[WriteRange], account: usize, offset: u32, len: u32) -> bool {
        ranges.iter().any(|range| {
            range.account_index == account as u8
                && range.offset <= offset
                && (range.size == u32::MAX
                    || range.offset.saturating_add(range.size) >= offset.saturating_add(len))
        })
    }

    #[test]
    fn shard_fits_single_instruction_initialization_limit() {
        assert!(IntentShard::LEN <= 10_240);
        assert_eq!(INTENTS_PER_SHARD, 20);
    }

    #[test]
    fn claim_manifest_can_write_executor_columns_but_not_owner_or_limits() {
        let ranges = ClaimIntent::WRITE_RANGES;
        assert!(range_covers(
            ranges,
            ClaimIntent::SHARD_INDEX,
            IntentShard::STATUSES_ABS_OFFSET,
            INTENTS_PER_SHARD as u32,
        ));
        assert!(range_covers(
            ranges,
            ClaimIntent::SHARD_INDEX,
            IntentShard::CLAIMANTS_ABS_OFFSET,
            (INTENTS_PER_SHARD * size_of::<Address>()) as u32,
        ));
        assert!(!range_covers(
            ranges,
            ClaimIntent::SHARD_INDEX,
            IntentShard::OWNERS_ABS_OFFSET,
            size_of::<Address>() as u32,
        ));
        assert!(!range_covers(
            ranges,
            ClaimIntent::SHARD_INDEX,
            IntentShard::MAX_INPUTS_ABS_OFFSET,
            size_of::<WireU64>() as u32,
        ));
    }

    #[test]
    fn execute_manifest_cannot_rewrite_route_or_economic_constraints() {
        let ranges = ExecuteIntent::WRITE_RANGES;
        assert!(range_covers(
            ranges,
            ExecuteIntent::SHARD_INDEX,
            IntentShard::SETTLED_OUTPUTS_ABS_OFFSET,
            (INTENTS_PER_SHARD * size_of::<WireU64>()) as u32,
        ));
        for offset in [
            IntentShard::OWNERS_ABS_OFFSET,
            IntentShard::VAULT_AUTHORITIES_ABS_OFFSET,
            IntentShard::MAX_INPUTS_ABS_OFFSET,
            IntentShard::MIN_OUTPUTS_ABS_OFFSET,
            IntentShard::ROUTE_PROGRAMS_ABS_OFFSET,
            IntentShard::ROUTE_COMMITMENTS_ABS_OFFSET,
        ] {
            assert!(!range_covers(ranges, ExecuteIntent::SHARD_INDEX, offset, 1));
        }
    }

    #[test]
    fn claim_policy_is_narrowed_to_the_invocation_slot() {
        let rules = ClaimIntent::PARAMETRIC_WRITE_RANGES;
        assert_eq!(rules.len(), 4);
        let status = rules
            .iter()
            .find(|rule| rule.segment_name == "statuses")
            .expect("statuses exact-cell rule");
        assert_eq!(status.argument_name, "slot");
        assert_eq!(status.count, INTENTS_PER_SHARD as u32);
        assert_eq!(status.stride, size_of::<u8>() as u32);

        let policy = WritePolicy::with_parametric(ClaimIntent::WRITE_RANGES, rules);
        let selected = IntentShard::STATUSES_ABS_OFFSET + 7;
        assert!(policy
            .check_write_with_args(ClaimIntent::SHARD_INDEX as u8, selected, 1, &[7])
            .is_ok());
        assert!(policy
            .check_write_with_args(ClaimIntent::SHARD_INDEX as u8, selected + 1, 1, &[7])
            .is_err());
    }

    #[test]
    fn grillo_resolves_real_cicada_manifest_to_the_selected_cell() {
        let json = hopper::hopper_schema::codama::ManifestJson(&PROGRAM_MANIFEST).to_string();
        let manifest = MutationManifest::from_json(&json).expect("real Cicada manifest parses");
        let claim = manifest
            .instruction("claim_intent")
            .expect("claim mutation contract");
        assert_eq!(claim.parametric.len(), 4);

        // The unresolved static column envelope may never be treated as an
        // invocation contract: doing so would authorize every neighboring
        // slot in the column.
        assert!(matches!(
            verify(
                claim,
                &[],
                &TouchMap {
                    overflowed: false,
                    skipped: false,
                    records: vec![],
                }
            ),
            Verdict::Inconclusive(InconclusiveReason::ParametricArgumentsRequired)
        ));

        let slot = 7u16;
        let mut payload = slot.to_le_bytes().to_vec();
        payload.extend_from_slice(&5u64.to_le_bytes());
        let status_rule = claim
            .parametric
            .iter()
            .find(|rule| rule.segment_name == "statuses")
            .expect("statuses rule");
        let selected = status_rule.base_offset + slot as u32 * status_rule.stride;
        let neighbor = selected + status_rule.stride;

        let data_len = IntentShard::INIT_SPACE;
        let pre = vec![0u8; data_len];
        let mut selected_post = pre.clone();
        let mut selected_records = std::vec::Vec::new();
        for (index, rule) in claim.parametric.iter().enumerate() {
            let offset = rule.base_offset + slot as u32 * rule.stride;
            // Each handler write acquires the complete selected cell. A
            // representative changed byte in every one of claim_intent's
            // four columns makes the synthetic delta exercise the complete
            // parametric effect, not only the one-byte status column.
            selected_post[offset as usize] = (index as u8) + 1;
            selected_records.push(TouchRecord {
                slot: ClaimIntent::SHARD_INDEX as u8,
                offset,
                size: rule.cell_size,
                write: true,
            });
        }
        selected_post[selected as usize] = STATUS_CLAIMED;
        let selected_map = TouchMap {
            overflowed: false,
            skipped: false,
            records: selected_records,
        };
        assert!(verify_invocation(
            claim,
            &payload,
            &[AccountDelta::new(
                ClaimIntent::SHARD_INDEX as u8,
                &pre,
                &selected_post,
            )],
            &selected_map,
        )
        .expect("real invocation resolves")
        .is_pass());

        let mut neighbor_post = pre.clone();
        neighbor_post[neighbor as usize] = STATUS_CLAIMED;
        let neighbor_map = TouchMap {
            overflowed: false,
            skipped: false,
            records: vec![TouchRecord {
                slot: ClaimIntent::SHARD_INDEX as u8,
                offset: neighbor,
                size: 1,
                write: true,
            }],
        };
        assert!(matches!(
            verify_invocation(
                claim,
                &payload,
                &[AccountDelta::new(
                    ClaimIntent::SHARD_INDEX as u8,
                    &pre,
                    &neighbor_post,
                )],
                &neighbor_map,
            )
            .expect("hostile invocation still resolves"),
            Verdict::Violation(_)
        ));
    }

    #[test]
    fn execute_manifest_resolves_bounded_and_const_generic_wire_shapes() {
        let execute = cicada_program::__HOPPER_INSTRUCTION_DESCRIPTORS
            .iter()
            .find(|ix| ix.name == "execute_intent")
            .expect("execute descriptor");
        assert_eq!(
            execute.remaining_accounts.unwrap().max as usize,
            MAX_ROUTE_ACCOUNTS
        );
        assert_eq!(execute.parametric_write_ranges.len(), 7);

        let route_data = execute
            .args
            .iter()
            .find(|arg| arg.name == "route_data")
            .expect("route_data arg");
        assert_eq!(route_data.size as usize, 2 + MAX_ROUTE_DATA);
        assert_eq!(
            route_data.encoding,
            hopper::hopper_schema::ArgEncoding::BoundedVec {
                max_len: MAX_ROUTE_DATA as u16,
                element_size: 1,
            }
        );

        let flags = execute
            .args
            .iter()
            .find(|arg| arg.name == "route_meta_flags")
            .expect("route flags arg");
        assert_eq!(flags.size as usize, MAX_ROUTE_ACCOUNTS);
        assert_eq!(flags.encoding, hopper::hopper_schema::ArgEncoding::Fixed);
    }

    #[test]
    fn permissionless_intent_cannot_be_preclaimed_but_executes_atomically() {
        let intent = test_intent();
        let executor = Address::new([9u8; 32]);
        assert_eq!(
            validate_claim_access(&intent, &executor, 10),
            Err(ProgramError::from(PermissionlessClaimForbidden))
        );
        assert_eq!(validate_execution_access(&intent, &executor, 10), Ok(()));
    }

    #[test]
    fn allowlisted_executor_can_reserve_or_execute_directly() {
        let executor = Address::new([9u8; 32]);
        let mut intent = test_intent();
        intent.allowed_executor = executor;
        assert_eq!(validate_claim_access(&intent, &executor, 10), Ok(()));
        assert_eq!(validate_execution_access(&intent, &executor, 10), Ok(()));

        intent.status = STATUS_CLAIMED;
        intent.claimant = executor;
        intent.claim_expiry = 20;
        assert_eq!(validate_execution_access(&intent, &executor, 20), Ok(()));
        assert_eq!(
            validate_execution_access(&intent, &executor, 21),
            Err(ProgramError::from(ClaimExpired))
        );
    }

    #[test]
    fn route_flag_tail_must_be_canonical() {
        let mut flags = [0u8; MAX_ROUTE_ACCOUNTS];
        flags[3] = ROUTE_META_WRITABLE;
        assert_eq!(
            validate_unused_route_flags(3, &flags),
            Err(ProgramError::from(NonZeroUnusedRouteFlags))
        );
        flags[3] = 0;
        assert_eq!(validate_unused_route_flags(3, &flags), Ok(()));
    }
}
