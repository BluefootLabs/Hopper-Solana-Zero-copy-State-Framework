//! # Cicada, transport-neutral protected execution intents
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
use hopper::hopper_solana::constants::BPF_LOADER_UPGRADEABLE_ID;
use hopper::hopper_solana::token2022_ext::{
    ACCOUNT_TYPE_MINT as TOKEN_2022_ACCOUNT_TYPE_MINT,
    ACCOUNT_TYPE_OFFSET as TOKEN_2022_ACCOUNT_TYPE_OFFSET,
    ACCOUNT_TYPE_TOKEN as TOKEN_2022_ACCOUNT_TYPE_TOKEN, EXT_CPI_GUARD, EXT_DEFAULT_ACCOUNT_STATE,
    EXT_GROUP_MEMBER_POINTER, EXT_GROUP_POINTER, EXT_IMMUTABLE_OWNER, EXT_MEMO_TRANSFER,
    EXT_METADATA_POINTER, EXT_MINT_CLOSE_AUTHORITY, EXT_TOKEN_METADATA,
    MAX_KNOWN_EXTENSION_TYPE as TOKEN_2022_LATEST_EXTENSION_TYPE,
    MINT_BASE_SIZE as TOKEN_MINT_BASE_SIZE, TLV_OFFSET as TOKEN_2022_TLV_OFFSET,
    TOKEN_ACCOUNT_BASE_SIZE, TOKEN_MULTISIG_SIZE,
};
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

/// One ordered account record in an exact-route commitment.
///
/// `writable` and `signer` are encoded with the same bit assignments accepted
/// by [`cicada_program::execute_intent`]. Constructing records from booleans
/// keeps host clients from committing unknown flag bits that the program will
/// reject before CPI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteCommitmentAccount {
    address: [u8; 32],
    writable: bool,
    signer: bool,
}

impl RouteCommitmentAccount {
    /// Create one route account record. Positional order and duplicates are
    /// preserved by [`compute_route_commitment_records`].
    pub const fn new(address: [u8; 32], writable: bool, signer: bool) -> Self {
        Self {
            address,
            writable,
            signer,
        }
    }

    pub const fn address(&self) -> &[u8; 32] {
        &self.address
    }

    pub const fn is_writable(&self) -> bool {
        self.writable
    }

    pub const fn is_signer(&self) -> bool {
        self.signer
    }

    const fn flags(&self) -> u8 {
        ((self.writable as u8) * ROUTE_META_WRITABLE) | ((self.signer as u8) * ROUTE_META_SIGNER)
    }
}

pub const STATUS_EMPTY: u8 = 0;
pub const STATUS_OPEN: u8 = 1;
pub const STATUS_CLAIMED: u8 = 2;
pub const STATUS_SETTLED: u8 = 3;
pub const STATUS_CANCELLED: u8 = 4;

const ZERO_ADDRESS: Address = Address::new_from_array([0u8; 32]);
const ZERO_HASH: [u8; 32] = [0u8; 32];
const BPF_LOADER_V4_ID: Address = Address::new_from_array(
    hopper::hopper_runtime::__decode_base58_32("LoaderV411111111111111111111111111111111111"),
);

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
    #[bump]
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
    // Append-only error ABI. New production gates stay after every published
    // V1 code so existing clients keep decoding the original numeric values.
    InvalidTokenState,
    SourceDelegatePresent,
    SourceCloseAuthorityPresent,
    SourceLeaseMismatch,
    SettlementCloseAuthorityPresent,
    InvalidProgramAccount,
    UnsupportedProgramLoader,
    InvalidProgramData,
    UnauthorizedInitializer,
    ConflictingDuplicateRouteMeta,
    EmptyEmergencyAuthority,
    SourceLamportsDecreased,
    DestinationLamportsShortfall,
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

    /// The currently executing Cicada program account. Initialization binds
    /// the payer to this deployment's real loader authority.
    pub program: UncheckedAccount<'info>,

    /// Loader-v3 ProgramData account. For loader-v4 deployments this is the
    /// program account repeated in the second position because v4 stores its
    /// authority in the executable account itself.
    pub program_data: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeShard<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        has_one = admin,
        seeds = [CONFIG_SEED],
        bump = stored,
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

    #[account(seeds = [CONFIG_SEED], bump = stored)]
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

    pub refund_token: UncheckedAccount<'info>,
    pub destination_token: UncheckedAccount<'info>,
    pub input_mint: UncheckedAccount<'info>,
    pub output_mint: UncheckedAccount<'info>,
    pub token_program: UncheckedAccount<'info>,
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

    #[account(seeds = [CONFIG_SEED], bump = stored)]
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
    #[account(seeds = [CONFIG_SEED], bump = stored)]
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

    #[account(seeds = [CONFIG_SEED], bump = stored)]
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

    #[account(seeds = [CONFIG_SEED], bump = stored)]
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

    #[account(seeds = [CONFIG_SEED], bump = stored)]
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
        bump = stored,
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
        validate_config_initialization_args(&emergency_authority, default_claim_ttl)?;
        verify_initialization_authority(
            ctx.program_id(),
            ctx.accounts.payer.key(),
            ctx.accounts.program.as_account(),
            ctx.accounts.program_data.as_account(),
        )?;
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
            // Program-trust mode intentionally does not carry a dead, unreviewed
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

        // Custody entry is one atomic state transition. The source starts
        // under the transaction-signing owner, and Cicada moves its token
        // authority to the per-vault PDA in this instruction. If any later
        // lease or shard write fails, Solana rolls this CPI back with the rest
        // of the instruction, so a failed create cannot orphan a pre-adopted
        // token account behind an otherwise unreachable PDA.
        let owner_key = *ctx.accounts.owner.key();
        let source_key = *ctx.accounts.source_token.key();
        let vault_key = *ctx.accounts.vault_authority.key();
        interface_set_account_owner_signed(
            ctx.accounts.source_token.as_account(),
            ctx.accounts.owner.as_account(),
            ctx.accounts.token_program.as_account(),
            &vault_key,
            &[],
        )?;
        verify_token_account(
            ctx.accounts.source_token.as_account(),
            ctx.accounts.input_mint.key(),
            &vault_key,
            TokenAccountRole::CustodySource,
        )?;

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
                owner_key,
                source_key,
                vault_key,
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
        let next_revision = next_revision(intent.revision)?;

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
            WireU64::new(next_revision),
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
        let next_revision = next_revision(intent.revision)?;

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
            WireU64::new(next_revision),
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
        let next_revision = next_revision(intent.revision)?;

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
            WireU64::new(next_revision),
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
        let next_revision = next_revision(intent.revision)?;

        let pre_source = token_amount(ctx.accounts.source_token.as_account())?;
        let pre_destination = token_amount(ctx.accounts.destination_token.as_account())?;
        let pre_source_lamports = ctx.accounts.source_token.as_account().lamports();
        let pre_destination_lamports = ctx.accounts.destination_token.as_account().lamports();
        let source_is_native = token_is_native(ctx.accounts.source_token.as_account())?;
        let destination_is_native = token_is_native(ctx.accounts.destination_token.as_account())?;
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
            ctx.accounts.input_mint.key(),
            ctx.accounts.output_mint.key(),
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
        validate_route_lamport_floors(
            pre_source_lamports,
            ctx.accounts.source_token.as_account().lamports(),
            pre_destination_lamports,
            ctx.accounts.destination_token.as_account().lamports(),
            spent,
            received,
            source_is_native,
            destination_is_native,
        )?;

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
            WireU64::new(next_revision),
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
        {
            let lease = ctx.accounts.source_lease.get()?;
            hopper::hopper_require!(
                lease.slot.get() == slot && lease.sequence.get() == intent.sequence,
                SourceLeaseMismatch
            );
        }
        verify_token_program_account(
            ctx.accounts.source_token.as_account(),
            ctx.accounts.token_program.as_account(),
        )?;
        verify_token_account(
            ctx.accounts.source_token.as_account(),
            &intent.input_mint,
            &intent.vault_authority,
            TokenAccountRole::CustodySource,
        )?;

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

        // Settlement and cancellation prove the source empty before marking
        // the record final, but token accounts remain publicly creditable. A
        // later unsolicited deposit must not pin the shard slot forever:
        // SetAuthority is balance-independent, so restore custody with any
        // post-final dust still in the user's original source account.
        interface_set_account_owner_signed(
            ctx.accounts.source_token.as_account(),
            ctx.accounts.vault_authority.as_account(),
            ctx.accounts.token_program.as_account(),
            &intent.owner,
            &signers,
        )?;
        verify_token_account(
            ctx.accounts.source_token.as_account(),
            &intent.input_mint,
            &intent.owner,
            TokenAccountRole::CustodySource,
        )?;

        let mut raw = ctx.raw();
        clear_intent_cells(&mut raw, slot, occupied, occupied_count)?;

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

// Deployment authority helpers

const LOADER_V3_PROGRAM_STATE_LEN: usize = 36;
const LOADER_V3_PROGRAM_DATA_METADATA_LEN: usize = 45;
const LOADER_V3_PROGRAM_TAG: u32 = 2;
const LOADER_V3_PROGRAM_DATA_TAG: u32 = 3;
const LOADER_V4_STATE_LEN: usize = 48;
const LOADER_V4_AUTHORITY_OFFSET: usize = 8;
const LOADER_V4_STATUS_OFFSET: usize = 40;
const LOADER_V4_STATUS_DEPLOYED: u64 = 1;

fn validate_config_initialization_args(
    emergency_authority: &Address,
    default_claim_ttl: u64,
) -> ProgramResult {
    hopper::hopper_require!(default_claim_ttl > 0, InvalidClaimTtl);
    hopper::hopper_require!(
        !address::address_is_zero(emergency_authority),
        EmptyEmergencyAuthority
    );
    Ok(())
}

#[inline]
fn bytes_match_address(bytes: &[u8], address: &Address) -> bool {
    bytes.len() == 32 && bytes == address.as_array()
}

fn verify_loader_v3_initialization_authority(
    program_state: &[u8],
    program_data_address: &Address,
    program_data_state: &[u8],
    initializer: &Address,
) -> ProgramResult {
    hopper::hopper_require!(
        program_state.len() == LOADER_V3_PROGRAM_STATE_LEN,
        InvalidProgramData
    );
    let program_tag = u32::from_le_bytes([
        program_state[0],
        program_state[1],
        program_state[2],
        program_state[3],
    ]);
    hopper::hopper_require!(program_tag == LOADER_V3_PROGRAM_TAG, InvalidProgramData);
    hopper::hopper_require!(
        bytes_match_address(&program_state[4..36], program_data_address),
        InvalidProgramData
    );

    hopper::hopper_require!(
        program_data_state.len() >= LOADER_V3_PROGRAM_DATA_METADATA_LEN,
        InvalidProgramData
    );
    let program_data_tag = u32::from_le_bytes([
        program_data_state[0],
        program_data_state[1],
        program_data_state[2],
        program_data_state[3],
    ]);
    hopper::hopper_require!(
        program_data_tag == LOADER_V3_PROGRAM_DATA_TAG,
        InvalidProgramData
    );
    match program_data_state[12] {
        1 => {
            hopper::hopper_require!(
                bytes_match_address(&program_data_state[13..45], initializer),
                UnauthorizedInitializer
            );
            Ok(())
        }
        0 => Err(UnauthorizedInitializer.into()),
        _ => Err(InvalidProgramData.into()),
    }
}

fn verify_loader_v4_initialization_authority(
    program_state: &[u8],
    initializer: &Address,
) -> ProgramResult {
    hopper::hopper_require!(
        program_state.len() >= LOADER_V4_STATE_LEN,
        InvalidProgramData
    );
    let status = u64::from_le_bytes([
        program_state[LOADER_V4_STATUS_OFFSET],
        program_state[LOADER_V4_STATUS_OFFSET + 1],
        program_state[LOADER_V4_STATUS_OFFSET + 2],
        program_state[LOADER_V4_STATUS_OFFSET + 3],
        program_state[LOADER_V4_STATUS_OFFSET + 4],
        program_state[LOADER_V4_STATUS_OFFSET + 5],
        program_state[LOADER_V4_STATUS_OFFSET + 6],
        program_state[LOADER_V4_STATUS_OFFSET + 7],
    ]);
    hopper::hopper_require!(status == LOADER_V4_STATUS_DEPLOYED, UnauthorizedInitializer);
    hopper::hopper_require!(
        bytes_match_address(
            &program_state[LOADER_V4_AUTHORITY_OFFSET..LOADER_V4_STATUS_OFFSET],
            initializer,
        ),
        UnauthorizedInitializer
    );
    Ok(())
}

fn verify_initialization_authority(
    executing_program_id: &Address,
    initializer: &Address,
    program: &AccountView<'_>,
    program_data: &AccountView<'_>,
) -> ProgramResult {
    hopper::hopper_require!(
        program.address() == executing_program_id,
        InvalidProgramAccount
    );
    program.check_executable()?;

    if program.owned_by(&BPF_LOADER_UPGRADEABLE_ID) {
        hopper::hopper_require!(
            program_data.owned_by(&BPF_LOADER_UPGRADEABLE_ID) && !program_data.executable(),
            InvalidProgramData
        );
        let program_state = program.try_borrow()?;
        let program_data_state = program_data.try_borrow()?;
        verify_loader_v3_initialization_authority(
            &program_state,
            program_data.address(),
            &program_data_state,
            initializer,
        )
    } else if program.owned_by(&BPF_LOADER_V4_ID) {
        // Loader-v4 keeps authority and code in one account. Requiring the
        // repeated key makes the fixed ABI unambiguous across both loaders.
        hopper::hopper_require!(
            program_data.address() == program.address(),
            InvalidProgramData
        );
        let program_state = program.try_borrow()?;
        verify_loader_v4_initialization_authority(&program_state, initializer)
    } else {
        Err(UnsupportedProgramLoader.into())
    }
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

#[inline]
fn next_revision(revision: u64) -> Result<u64> {
    revision
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)
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

const TOKEN_STATE_INITIALIZED: u8 = 1;
const TOKEN_DELEGATE_OPTION_OFFSET: usize = 72;
const TOKEN_IS_NATIVE_OPTION_OFFSET: usize = 109;
const TOKEN_DELEGATED_AMOUNT_OFFSET: usize = 121;
const TOKEN_CLOSE_AUTHORITY_OPTION_OFFSET: usize = 129;

// Cicada shares the framework's reviewed Token-2022 wire constants, then
// applies a narrower role-specific policy with exact value-length checks.
// Unknown future types stay rejected until both layers are updated together.

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenAccountRole {
    CustodySource,
    SettlementDestination,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Token2022Shape {
    Mint,
    Token,
}

#[inline]
fn coption_is_some(data: &[u8], offset: usize) -> Result<bool> {
    let end = offset
        .checked_add(4)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let tag = data
        .get(offset..end)
        .ok_or(ProgramError::InvalidAccountData)?;
    match u32::from_le_bytes([tag[0], tag[1], tag[2], tag[3]]) {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProgramError::InvalidAccountData),
    }
}

fn verify_source_authority_surface(data: &[u8]) -> ProgramResult {
    hopper::hopper_require!(
        !coption_is_some(data, TOKEN_DELEGATE_OPTION_OFFSET)?,
        SourceDelegatePresent
    );
    let delegated_end = TOKEN_DELEGATED_AMOUNT_OFFSET
        .checked_add(8)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let delegated = data
        .get(TOKEN_DELEGATED_AMOUNT_OFFSET..delegated_end)
        .ok_or(ProgramError::InvalidAccountData)?;
    let delegated_amount = u64::from_le_bytes([
        delegated[0],
        delegated[1],
        delegated[2],
        delegated[3],
        delegated[4],
        delegated[5],
        delegated[6],
        delegated[7],
    ]);
    hopper::hopper_require!(delegated_amount == 0, SourceDelegatePresent);
    hopper::hopper_require!(
        !coption_is_some(data, TOKEN_CLOSE_AUTHORITY_OPTION_OFFSET)?,
        SourceCloseAuthorityPresent
    );
    Ok(())
}

fn verify_settlement_authority_surface(data: &[u8]) -> ProgramResult {
    // A distinct close authority could delete an empty refund or destination
    // account after creation and prevent both execution and cancellation. The
    // token owner can still manage its own accounts, but no additional actor
    // is admitted into Cicada's liveness boundary.
    hopper::hopper_require!(
        !coption_is_some(data, TOKEN_CLOSE_AUTHORITY_OPTION_OFFSET)?,
        SettlementCloseAuthorityPresent
    );
    Ok(())
}

#[inline]
fn token_2022_extension_shape(extension_type: u16) -> Option<Token2022Shape> {
    match extension_type {
        // Mint extensions in the current canonical Token-2022 interface.
        1 | 3 | 4 | 6 | 9 | 10 | 12 | 14 | 16 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25 | 26 | 28 => {
            Some(Token2022Shape::Mint)
        }
        // Token-account extensions in the current canonical interface.
        2 | 5 | 7 | 8 | 11 | 13 | 15 | 17 | 27 => Some(Token2022Shape::Token),
        _ => None,
    }
}

fn verify_token_2022_extension(
    shape: Token2022Shape,
    role: TokenAccountRole,
    extension_type: u16,
    value: &[u8],
) -> ProgramResult {
    hopper::hopper_require!(
        extension_type <= TOKEN_2022_LATEST_EXTENSION_TYPE
            && token_2022_extension_shape(extension_type) == Some(shape),
        UnsafeTokenExtension
    );

    match shape {
        Token2022Shape::Mint => match extension_type {
            // These extensions do not change raw transfer amounts or grant a
            // transfer/burn capability. Their bytes are frozen across route
            // CPI by `mint_policy_hash`.
            EXT_MINT_CLOSE_AUTHORITY => {
                hopper::hopper_require!(value.len() == 32, UnsafeTokenExtension);
                Ok(())
            }
            EXT_DEFAULT_ACCOUNT_STATE => {
                hopper::hopper_require!(value.len() == 1, UnsafeTokenExtension);
                Ok(())
            }
            EXT_METADATA_POINTER | EXT_GROUP_POINTER | EXT_GROUP_MEMBER_POINTER => {
                hopper::hopper_require!(value.len() == 64, UnsafeTokenExtension);
                Ok(())
            }
            // TokenMetadata is variable length. The TLV walker still proves
            // its envelope is bounded and non-overlapping; the canonical token
            // program owns and validates the serialized value itself.
            EXT_TOKEN_METADATA => Ok(()),
            // Transfer fees, delegates, hooks, confidential balances,
            // non-transferability, UI-denomination changes, pause controls,
            // permissioned burns, embedded group state, and every future type
            // are unsupported by Cicada's raw-amount V1 settlement contract.
            _ => Err(UnsafeTokenExtension.into()),
        },
        Token2022Shape::Token => match extension_type {
            // Immutable owner is safe for recipient accounts, but a source
            // carrying it can never satisfy Cicada's owner-restoration
            // postcondition.
            EXT_IMMUTABLE_OWNER if role == TokenAccountRole::SettlementDestination => {
                hopper::hopper_require!(value.is_empty(), UnsafeTokenExtension);
                Ok(())
            }
            // Disabled policy bits are inert. Enabled MemoTransfer can block
            // Cicada's refund CPI, while enabled CpiGuard blocks owner changes
            // through CPI, so neither is admitted.
            EXT_MEMO_TRANSFER | EXT_CPI_GUARD => {
                hopper::hopper_require!(value.len() == 1, UnsafeTokenExtension);
                hopper::hopper_require!(value[0] == 0, UnsafeTokenExtension);
                Ok(())
            }
            _ => Err(UnsafeTokenExtension.into()),
        },
    }
}

/// Validate the complete current Token-2022 TLV envelope without allocation.
///
/// Unlike a sequence of `has_extension` probes, this walk cannot turn a
/// malformed or future extension into "not present". It validates the account
/// shape, mint padding, every type/length boundary, duplicate types, and the
/// explicit Cicada allowlist. Unknown extensions fail closed until reviewed.
fn verify_token_2022_tlv(
    data: &[u8],
    shape: Token2022Shape,
    role: TokenAccountRole,
) -> ProgramResult {
    // Canonical StateWithExtensions rejects this exact length because it is
    // indistinguishable from the Token/Token-2022 Multisig body. Cicada must
    // not accept a multisig whose attacker-chosen signer bytes happen to
    // satisfy a token-account or mint overlay.
    hopper::hopper_require!(data.len() != TOKEN_MULTISIG_SIZE, UnsafeTokenExtension);
    let base_len = match shape {
        Token2022Shape::Mint => TOKEN_MINT_BASE_SIZE,
        Token2022Shape::Token => TOKEN_ACCOUNT_BASE_SIZE,
    };
    if data.len() == base_len {
        return Ok(());
    }
    hopper::hopper_require!(data.len() >= TOKEN_2022_TLV_OFFSET, UnsafeTokenExtension);
    if shape == Token2022Shape::Mint {
        hopper::hopper_require!(
            data[TOKEN_MINT_BASE_SIZE..TOKEN_2022_ACCOUNT_TYPE_OFFSET]
                .iter()
                .all(|byte| *byte == 0),
            UnsafeTokenExtension
        );
    }
    let expected_type = match shape {
        Token2022Shape::Mint => TOKEN_2022_ACCOUNT_TYPE_MINT,
        Token2022Shape::Token => TOKEN_2022_ACCOUNT_TYPE_TOKEN,
    };
    hopper::hopper_require!(
        data[TOKEN_2022_ACCOUNT_TYPE_OFFSET] == expected_type,
        UnsafeTokenExtension
    );

    let mut seen = 0u32;
    let mut cursor = TOKEN_2022_TLV_OFFSET;
    while cursor < data.len() {
        let remaining = data.len() - cursor;
        if remaining < 2 {
            hopper::hopper_require!(
                data[cursor..].iter().all(|byte| *byte == 0),
                UnsafeTokenExtension
            );
            break;
        }
        let extension_type = u16::from_le_bytes([data[cursor], data[cursor + 1]]);
        if extension_type == 0 {
            hopper::hopper_require!(
                data[cursor..].iter().all(|byte| *byte == 0),
                UnsafeTokenExtension
            );
            break;
        }
        hopper::hopper_require!(remaining >= 4, UnsafeTokenExtension);
        let value_len = u16::from_le_bytes([data[cursor + 2], data[cursor + 3]]) as usize;
        let value_start = cursor
            .checked_add(4)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let value_end = value_start
            .checked_add(value_len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        hopper::hopper_require!(value_end <= data.len(), UnsafeTokenExtension);
        let bit = 1u32
            .checked_shl(extension_type as u32)
            .ok_or_else(|| ProgramError::from(UnsafeTokenExtension))?;
        hopper::hopper_require!(seen & bit == 0, UnsafeTokenExtension);
        seen |= bit;
        verify_token_2022_extension(shape, role, extension_type, &data[value_start..value_end])?;
        cursor = value_end;
    }
    Ok(())
}

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

fn token_is_native(account: &AccountView<'_>) -> Result<bool> {
    let kind = TokenProgramKind::for_account(account)?;
    let data = account.try_borrow()?;
    let _ = InterfaceTokenAccount::from_data(&data, kind)?;
    coption_is_some(&data, TOKEN_IS_NATIVE_OPTION_OFFSET)
}

/// Verify a mint and return its decimals. Token-2022 mints are accepted only
/// when their extensions preserve Cicada's amount-only settlement model.
fn verified_mint_decimals(account: &AccountView<'_>) -> Result<u8> {
    let kind = TokenProgramKind::for_account(account)?;
    let data = account.try_borrow()?;
    let mint = InterfaceMint::from_data(&data, kind)?;
    mint.assert_initialized()?;
    if matches!(kind, TokenProgramKind::Token2022) {
        verify_token_2022_tlv(
            &data,
            Token2022Shape::Mint,
            TokenAccountRole::SettlementDestination,
        )?;
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
    role: TokenAccountRole,
) -> ProgramResult {
    let kind = TokenProgramKind::for_account(account)?;
    let data = account.try_borrow()?;
    let token = InterfaceTokenAccount::from_data(&data, kind)?;
    hopper::hopper_require!(token.state()? == TOKEN_STATE_INITIALIZED, InvalidTokenState);
    let _ = coption_is_some(&data, TOKEN_IS_NATIVE_OPTION_OFFSET)?;
    if matches!(kind, TokenProgramKind::Token2022) {
        verify_token_2022_tlv(&data, Token2022Shape::Token, role)?;
    }
    if token.mint()? != mint {
        return Err(TokenMintMismatch.into());
    }
    if token.owner()? != authority {
        return Err(TokenAuthorityMismatch.into());
    }
    if role == TokenAccountRole::CustodySource {
        verify_source_authority_surface(&data)?;
    } else {
        verify_settlement_authority_surface(&data)?;
    }
    Ok(())
}

fn verify_create_token_accounts(accounts: &CreateIntent<'_>, max_input: u64) -> ProgramResult {
    let owner = *accounts.owner.key();
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
    verify_token_program_account(
        accounts.source_token.as_account(),
        accounts.token_program.as_account(),
    )?;
    verify_token_account(
        accounts.source_token.as_account(),
        &input_mint,
        &owner,
        TokenAccountRole::CustodySource,
    )?;
    verify_token_account(
        accounts.refund_token.as_account(),
        &input_mint,
        &owner,
        TokenAccountRole::SettlementDestination,
    )?;
    verify_token_account(
        accounts.destination_token.as_account(),
        &output_mint,
        &owner,
        TokenAccountRole::SettlementDestination,
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
        TokenAccountRole::CustodySource,
    )?;
    verify_token_account(
        accounts.refund_token.as_account(),
        &intent.input_mint,
        &intent.owner,
        TokenAccountRole::SettlementDestination,
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
        TokenAccountRole::CustodySource,
    )?;
    verify_token_account(
        accounts.refund_token.as_account(),
        &intent.input_mint,
        &intent.owner,
        TokenAccountRole::SettlementDestination,
    )?;
    verify_token_account(
        accounts.destination_token.as_account(),
        &intent.output_mint,
        &intent.owner,
        TokenAccountRole::SettlementDestination,
    )?;
    Ok(input_decimals)
}

/// Commit to every token-account policy byte except the amount at bytes 64..72.
/// With unsafe Token-2022 extensions rejected, a normal route may change only
/// the balance while owner, mint, delegate, state, close authority, and TLV
/// policy remain byte-identical.
fn token_policy_hash(account: &AccountView<'_>) -> Result<[u8; 32]> {
    let data = account.try_borrow()?;
    if data.len() < TOKEN_ACCOUNT_BASE_SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    crypto::sha256(&[b"cicada-token-policy-v1", &data[..64], &data[72..]])
}

/// Commit to the complete mint body, including supply.
///
/// A route that can mint its promised output is not evidence of a real swap.
/// This end-state commitment catches persistent mint changes. The separate
/// pre-CPI writable-mint gate also prevents supply-neutral MintTo-plus-Burn
/// sequences that could restore every byte before this hash is checked.
fn mint_policy_hash(account: &AccountView<'_>) -> Result<[u8; 32]> {
    let data = account.try_borrow()?;
    mint_policy_hash_bytes(&data)
}

fn mint_policy_hash_bytes(data: &[u8]) -> Result<[u8; 32]> {
    let (before_supply, supply, after_supply) = mint_policy_parts(data)?;
    crypto::sha256(&[
        b"cicada-mint-policy-v2",
        before_supply,
        supply,
        after_supply,
    ])
}

fn mint_policy_parts(data: &[u8]) -> Result<(&[u8], &[u8], &[u8])> {
    if data.len() < TOKEN_MINT_BASE_SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok((&data[..36], &data[36..44], &data[44..]))
}

#[allow(clippy::too_many_arguments)]
fn validate_route_lamport_floors(
    pre_source_lamports: u64,
    post_source_lamports: u64,
    pre_destination_lamports: u64,
    post_destination_lamports: u64,
    spent: u64,
    received: u64,
    source_is_native: bool,
    destination_is_native: bool,
) -> ProgramResult {
    // A non-native token transfer never debits account lamports. Wrapped SOL
    // debits exactly the token amount transferred. These lower bounds preserve
    // any excess lamports even if a route can sign for the token-account
    // address, closes it through the canonical token program, and recreates a
    // byte-identical account before returning.
    let source_floor = if source_is_native {
        pre_source_lamports
            .checked_sub(spent)
            .ok_or(ProgramError::ArithmeticOverflow)?
    } else {
        pre_source_lamports
    };
    hopper::hopper_require!(
        post_source_lamports >= source_floor,
        SourceLamportsDecreased
    );

    let destination_floor = if destination_is_native {
        pre_destination_lamports
            .checked_add(received)
            .ok_or(ProgramError::ArithmeticOverflow)?
    } else {
        pre_destination_lamports
    };
    hopper::hopper_require!(
        post_destination_lamports >= destination_floor,
        DestinationLamportsShortfall
    );
    Ok(())
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

#[inline]
fn validate_duplicate_route_meta(
    address: &Address,
    flags: u8,
    prior_address: &Address,
    prior_flags: u8,
) -> ProgramResult {
    hopper::hopper_require!(
        address != prior_address || (flags == prior_flags && flags & ROUTE_META_WRITABLE == 0),
        ConflictingDuplicateRouteMeta
    );
    Ok(())
}

#[inline]
fn validate_route_mint_delegation(
    address: &Address,
    writable: bool,
    input_mint: &Address,
    output_mint: &Address,
) -> ProgramResult {
    hopper::hopper_require!(
        !writable || (address != input_mint && address != output_mint),
        ProtectedAccountDelegation
    );
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
    input_mint: &Address,
    output_mint: &Address,
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

        // Solana collapses duplicate Pubkeys for CPI and unions their
        // privileges. Hopper's safe deduplicated CPI tier deliberately
        // rejects repeated writable metas because one unique AccountView
        // cannot safely represent several mutable positions. Reject those
        // routes here, before commitment acceptance or CPI. Ordered duplicate
        // read-only positions remain valid only with identical flags.
        let mut prior_index = 0usize;
        while prior_index < index {
            let prior = accounts
                .get(prior_index)
                .ok_or_else(|| ProgramError::from(RouteAccountCountMismatch))?;
            validate_duplicate_route_meta(
                account.address(),
                meta,
                prior.address(),
                flags[prior_index],
            )?;
            prior_index += 1;
        }

        let writable = meta & ROUTE_META_WRITABLE != 0;
        let signer = meta & ROUTE_META_SIGNER != 0;
        // End-state mint hashing detects persistent policy/supply changes, but
        // a mint authority could otherwise perform a supply-neutral MintTo
        // plus Burn inside one route CPI. A route may inspect either committed
        // mint, but Cicada never delegates writable access to it.
        validate_route_mint_delegation(account.address(), writable, input_mint, output_mint)?;
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
    compute_route_commitment_from(
        route_program.as_array(),
        route_data,
        accounts.len(),
        |index| {
            let account = accounts
                .get(index)
                .ok_or_else(|| ProgramError::from(RouteAccountCountMismatch))?;
            let flags = flags[index];
            Ok(RouteCommitmentAccount::new(
                *account.address().as_array(),
                flags & ROUTE_META_WRITABLE != 0,
                flags & ROUTE_META_SIGNER != 0,
            ))
        },
    )
}

/// Compute Cicada's exact-route commitment from host-friendly account records.
///
/// This is the same allocation-free implementation used by the on-chain
/// adapter. It rejects envelopes that cannot be submitted to Cicada: more than
/// [`MAX_ROUTE_ACCOUNTS`] ordered records, more than [`MAX_ROUTE_DATA`] bytes
/// of instruction data, or duplicate addresses with writable or conflicting
/// privileges. Read-only duplicates with identical signer flags remain
/// distinct records and order is commitment-significant. This structural
/// check does not replace execution's account-owner, mint, and custody checks.
pub fn compute_route_commitment_records(
    route_program: &[u8; 32],
    route_data: &[u8],
    accounts: &[RouteCommitmentAccount],
) -> Result<[u8; 32]> {
    hopper::hopper_require!(
        accounts.len() <= MAX_ROUTE_ACCOUNTS,
        RouteAccountCountMismatch
    );
    for (index, account) in accounts.iter().enumerate() {
        for prior in &accounts[..index] {
            validate_duplicate_route_meta(
                &Address::new_from_array(*account.address()),
                account.flags(),
                &Address::new_from_array(*prior.address()),
                prior.flags(),
            )?;
        }
    }
    compute_route_commitment_from(route_program, route_data, accounts.len(), |index| {
        Ok(accounts[index])
    })
}

fn compute_route_commitment_from<F>(
    route_program: &[u8; 32],
    route_data: &[u8],
    account_count: usize,
    mut account_at: F,
) -> Result<[u8; 32]>
where
    F: FnMut(usize) -> Result<RouteCommitmentAccount>,
{
    hopper::hopper_require!(
        account_count <= MAX_ROUTE_ACCOUNTS,
        RouteAccountCountMismatch
    );
    if route_data.len() > MAX_ROUTE_DATA {
        return Err(ProgramError::InvalidInstructionData);
    }
    let data_hash = route_sha256(route_data)?;
    let mut chunk_hashes = [[0u8; 32]; ROUTE_HASH_CHUNKS];
    let mut chunk_count = 0usize;
    let mut cursor = 0usize;

    while cursor < account_count {
        let take = core::cmp::min(8, account_count - cursor);
        let mut chunk = [0u8; 8 * 33];
        let mut index = 0usize;
        while index < take {
            let account = account_at(cursor + index)?;
            let base = index * 33;
            chunk[base..base + 32].copy_from_slice(account.address());
            chunk[base + 32] = account.flags();
            index += 1;
        }
        chunk_hashes[chunk_count] = route_sha256(&chunk[..take * 33])?;
        chunk_count += 1;
        cursor += take;
    }

    let mut final_bytes = [0u8; 16 + 32 + 32 + 1 + ROUTE_HASH_CHUNKS * 32];
    final_bytes[..16].copy_from_slice(b"cicada-route-v1!");
    final_bytes[16..48].copy_from_slice(route_program);
    final_bytes[48..80].copy_from_slice(&data_hash);
    final_bytes[80] = account_count as u8;
    let mut index = 0usize;
    while index < chunk_count {
        let base = 81 + index * 32;
        final_bytes[base..base + 32].copy_from_slice(&chunk_hashes[index]);
        index += 1;
    }
    route_sha256(&final_bytes[..81 + chunk_count * 32])
}

#[inline]
fn route_sha256(input: &[u8]) -> Result<[u8; 32]> {
    #[cfg(target_os = "solana")]
    {
        crypto::sha256_single(input)
    }
    #[cfg(not(target_os = "solana"))]
    {
        // Runtime syscall shims intentionally return zeroes off chain. Use
        // Hopper's allocation-free software implementation so host-generated
        // commitments are byte-identical to the SBF syscall result.
        Ok(hopper::hopper_runtime::sha256::sha256(input))
    }
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

/// Host-only application probes for the manifest-derived C3 adapter.
///
/// This module deliberately has no on-chain build path. It lets the separate,
/// non-publish adapter exercise Cicada's actual private authorization and
/// route-safety helpers for every structural manifest case without exposing a
/// callable program instruction or pretending that host semantics are an SBF
/// transaction execution.
#[doc(hidden)]
#[cfg(not(target_os = "solana"))]
pub mod fuzz_semantics {
    use super::*;

    /// Execute the core Cicada business gates against both valid and hostile
    /// seeded reference states. Every `Err` identifies a probe that stopped
    /// distinguishing the accepted path from the rejected path.
    pub fn exercise_business_guards(seed: [u8; 16]) -> Result<(), &'static str> {
        let now = 1_000u64 + u64::from(seed[0]);
        let executor = nonzero_address(seed[1], 0x31);
        let other_executor = nonzero_address(seed[2], 0xA7);
        let mut intent = reference_intent(now, executor, seed);

        must_accept(
            validate_claim_access(&intent, &executor, now),
            "valid allowlisted claim was rejected",
        )?;
        must_reject(
            validate_claim_access(&intent, &other_executor, now),
            "claim accepted the wrong executor",
        )?;

        intent.allowed_executor = ZERO_ADDRESS;
        must_reject(
            validate_claim_access(&intent, &executor, now),
            "permissionless intent accepted a pre-claim",
        )?;
        must_accept(
            validate_execution_access(&intent, &executor, now),
            "permissionless open execution was rejected",
        )?;

        intent.allowed_executor = executor;
        intent.status = STATUS_CLAIMED;
        intent.claimant = executor;
        intent.claim_expiry = now + 2;
        must_accept(
            validate_execution_access(&intent, &executor, now),
            "valid claimed execution was rejected",
        )?;
        intent.claimant = other_executor;
        must_reject(
            validate_execution_access(&intent, &executor, now),
            "claimed execution accepted a different claimant",
        )?;
        intent.claimant = executor;
        intent.claim_expiry = now.saturating_sub(1);
        must_reject(
            validate_execution_access(&intent, &executor, now),
            "expired claim lease was accepted",
        )?;
        intent.claim_expiry = now + 2;
        intent.expiry = now;
        must_reject(
            validate_execution_access(&intent, &executor, now),
            "expired intent was executable",
        )?;

        exercise_route_lamport_guards(seed)?;
        exercise_route_meta_guards(seed)?;
        Ok(())
    }

    fn exercise_route_lamport_guards(seed: [u8; 16]) -> Result<(), &'static str> {
        let source = 10_000u64 + u64::from(seed[3]);
        let destination = 20_000u64 + u64::from(seed[4]);
        let spent = 10u64 + u64::from(seed[5] % 32);
        let received = 7u64 + u64::from(seed[6] % 32);

        must_accept(
            validate_route_lamport_floors(
                source,
                source,
                destination,
                destination,
                spent,
                received,
                false,
                false,
            ),
            "valid non-native lamport floors were rejected",
        )?;
        must_reject(
            validate_route_lamport_floors(
                source,
                source - 1,
                destination,
                destination,
                spent,
                received,
                false,
                false,
            ),
            "non-native source lamport drain was accepted",
        )?;
        must_reject(
            validate_route_lamport_floors(
                source,
                source,
                destination,
                destination - 1,
                spent,
                received,
                false,
                false,
            ),
            "non-native destination lamport drain was accepted",
        )?;
        must_accept(
            validate_route_lamport_floors(
                source,
                source - spent,
                destination,
                destination + received,
                spent,
                received,
                true,
                true,
            ),
            "valid native-token lamport movement was rejected",
        )?;
        must_reject(
            validate_route_lamport_floors(
                source,
                source - spent - 1,
                destination,
                destination + received,
                spent,
                received,
                true,
                true,
            ),
            "native source moved below token backing",
        )?;
        must_reject(
            validate_route_lamport_floors(
                source,
                source - spent,
                destination,
                destination + received - 1,
                spent,
                received,
                true,
                true,
            ),
            "native destination missed received token backing",
        )?;
        Ok(())
    }

    fn exercise_route_meta_guards(seed: [u8; 16]) -> Result<(), &'static str> {
        let account = nonzero_address(seed[7], 0x19);
        let other = nonzero_address(seed[8], 0xD3);
        let input_mint = nonzero_address(seed[9], 0x41);
        let output_mint = nonzero_address(seed[10], 0x52);

        let mut flags = [0u8; MAX_ROUTE_ACCOUNTS];
        flags[0] = ROUTE_META_WRITABLE;
        must_accept(
            validate_unused_route_flags(1, &flags),
            "canonical used route flag was rejected",
        )?;
        flags[1] = ROUTE_META_SIGNER;
        must_reject(
            validate_unused_route_flags(1, &flags),
            "nonzero unused route flag was accepted",
        )?;
        must_reject(
            validate_unused_route_flags(MAX_ROUTE_ACCOUNTS + 1, &flags),
            "oversized route-account set was accepted",
        )?;

        must_accept(
            validate_duplicate_route_meta(&account, 0, &account, 0),
            "identical read-only duplicate route metadata was rejected",
        )?;
        must_reject(
            validate_duplicate_route_meta(
                &account,
                ROUTE_META_WRITABLE,
                &account,
                ROUTE_META_WRITABLE,
            ),
            "duplicate writable route metadata was accepted",
        )?;
        must_reject(
            validate_duplicate_route_meta(&account, ROUTE_META_WRITABLE, &account, 0),
            "conflicting duplicate route metadata was accepted",
        )?;
        must_accept(
            validate_duplicate_route_meta(&account, ROUTE_META_WRITABLE, &other, 0),
            "distinct route accounts were treated as aliases",
        )?;

        must_accept(
            validate_route_mint_delegation(&input_mint, false, &input_mint, &output_mint),
            "readonly committed mint was rejected",
        )?;
        must_reject(
            validate_route_mint_delegation(&input_mint, true, &input_mint, &output_mint),
            "writable input mint delegation was accepted",
        )?;
        must_reject(
            validate_route_mint_delegation(&output_mint, true, &input_mint, &output_mint),
            "writable output mint delegation was accepted",
        )?;
        Ok(())
    }

    fn reference_intent(now: u64, executor: Address, seed: [u8; 16]) -> IntentSnapshot {
        IntentSnapshot {
            owner: nonzero_address(seed[11], 1),
            source_token: nonzero_address(seed[12], 2),
            vault_authority: nonzero_address(seed[13], 3),
            refund_token: nonzero_address(seed[14], 4),
            destination_token: nonzero_address(seed[15], 5),
            input_mint: nonzero_address(seed[0], 6),
            output_mint: nonzero_address(seed[1], 7),
            max_input: 100,
            min_output: 90,
            expiry: now + 10,
            allowed_executor: executor,
            route_program: nonzero_address(seed[2], 8),
            route_commitment: ZERO_HASH,
            route_mode: ROUTE_MODE_PROGRAM,
            sequence: 1 + u64::from(seed[3]),
            status: STATUS_OPEN,
            claimant: ZERO_ADDRESS,
            claim_expiry: 0,
            revision: 1,
        }
    }

    fn nonzero_address(variable: u8, domain: u8) -> Address {
        let mut bytes = [domain; 32];
        bytes[0] = variable.wrapping_add(1);
        Address::new(bytes)
    }

    fn must_accept(result: ProgramResult, message: &'static str) -> Result<(), &'static str> {
        if result.is_ok() {
            Ok(())
        } else {
            Err(message)
        }
    }

    fn must_reject(result: ProgramResult, message: &'static str) -> Result<(), &'static str> {
        if result.is_err() {
            Ok(())
        } else {
            Err(message)
        }
    }
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

    #[test]
    fn revision_increment_fails_instead_of_reusing_u64_max() {
        assert_eq!(next_revision(41), Ok(42));
        assert_eq!(
            next_revision(u64::MAX),
            Err(ProgramError::ArithmeticOverflow)
        );
    }

    fn decode_hash(value: &str) -> [u8; 32] {
        assert_eq!(value.len(), 64);
        let mut hash = [0u8; 32];
        for (index, byte) in hash.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
        }
        hash
    }

    fn route_commitment_accounts(count: usize) -> std::vec::Vec<RouteCommitmentAccount> {
        (0..count)
            .map(|index| {
                RouteCommitmentAccount::new([(index + 1) as u8; 32], index % 2 == 0, index % 3 == 0)
            })
            .collect()
    }

    #[test]
    fn route_commitment_host_helper_matches_boundary_golden_vectors() {
        let route_program = [0xa5; 32];
        let route_data = [0xde, 0xad, 0xbe, 0xef];
        let expected = [
            (
                0,
                "093da7ebbe1bdb5732f15079e860b1fb91fcb8a7ee85cc22cef54b5a7a0858d1",
            ),
            (
                1,
                "34e047f9040faab3f87b4c394d3c238bd5ca19f2de715415fd2e57ecea9ae899",
            ),
            (
                8,
                "a34d9315f1234b29e5b4946f8c61586df8ec91e3e70fa7cfae949203e73169b6",
            ),
            (
                9,
                "2b9067b8be54dfa80fd70a50d67571505de5bca324be1380f4bff852695accce",
            ),
        ];

        for (count, expected_hash) in expected {
            let accounts = route_commitment_accounts(count);
            assert_eq!(
                compute_route_commitment_records(&route_program, &route_data, &accounts).unwrap(),
                decode_hash(expected_hash),
                "golden vector for {count} records changed"
            );
        }
    }

    #[test]
    fn route_commitment_preserves_duplicates_and_order() {
        let route_program = [0xa5; 32];
        let route_data = [0xde, 0xad, 0xbe, 0xef];
        let a = RouteCommitmentAccount::new([0x11; 32], false, false);
        let b = RouteCommitmentAccount::new([0x22; 32], true, false);

        let ab = compute_route_commitment_records(&route_program, &route_data, &[a, b]).unwrap();
        let ba = compute_route_commitment_records(&route_program, &route_data, &[b, a]).unwrap();
        let aa = compute_route_commitment_records(&route_program, &route_data, &[a, a]).unwrap();

        assert_eq!(
            ab,
            decode_hash("f1ce9c9505ebe372d3e6180259024f7c99e1ca6b161daed32b736ebe989d510b")
        );
        assert_eq!(
            ba,
            decode_hash("1da171417add120ef80f77c1fbd929eca6d649b4fc57a5e7860e0d2ccbb994c0")
        );
        assert_eq!(
            aa,
            decode_hash("567aac32b6a658b33d25fdfe5be449c0751c5818003713fef09f4b5385b13ea9")
        );
        assert_ne!(ab, ba, "ordered route records must not commute");
        assert_ne!(ab, aa, "duplicate positions must remain committed");
    }

    #[test]
    fn route_commitment_rejects_unexecutable_host_envelopes() {
        let route_program = [0xa5; 32];
        let too_many_accounts = route_commitment_accounts(48);
        assert_eq!(
            compute_route_commitment_records(&route_program, &[], &too_many_accounts),
            Err(ProgramError::from(RouteAccountCountMismatch))
        );
        let oversized_route_data = [0u8; MAX_ROUTE_DATA + 1];
        assert_eq!(
            compute_route_commitment_records(&route_program, &oversized_route_data, &[]),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn route_commitment_host_alias_rules_match_execution() {
        // Check the complete flag cross-product, including read-only signer
        // duplicates. Put the second occurrence beyond the hash chunk boundary.
        for left in 0..4 {
            for right in 0..4 {
                let a = RouteCommitmentAccount::new([0x11; 32], left & 1 != 0, left & 2 != 0);
                let b = RouteCommitmentAccount::new([0x11; 32], right & 1 != 0, right & 2 != 0);
                let mut accounts = route_commitment_accounts(8);
                accounts[0] = a;
                accounts.push(b);
                let result = compute_route_commitment_records(&[0xa5; 32], &[], &accounts);
                if left == right && left & ROUTE_META_WRITABLE == 0 {
                    assert!(result.is_ok());
                } else {
                    assert_eq!(
                        result,
                        Err(ProgramError::from(ConflictingDuplicateRouteMeta))
                    );
                }
            }
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
        const { assert!(IntentShard::LEN <= 10_240) };
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

    fn extended_token_2022(shape: Token2022Shape, entries: &[(u16, &[u8])]) -> std::vec::Vec<u8> {
        let mut data = vec![0u8; TOKEN_2022_TLV_OFFSET];
        data[TOKEN_2022_ACCOUNT_TYPE_OFFSET] = match shape {
            Token2022Shape::Mint => TOKEN_2022_ACCOUNT_TYPE_MINT,
            Token2022Shape::Token => TOKEN_2022_ACCOUNT_TYPE_TOKEN,
        };
        for (extension_type, value) in entries {
            data.extend_from_slice(&extension_type.to_le_bytes());
            data.extend_from_slice(&(value.len() as u16).to_le_bytes());
            data.extend_from_slice(value);
        }
        data
    }

    #[test]
    fn config_initialization_rejects_unrecoverable_authority_inputs() {
        let authority = Address::new([40u8; 32]);
        assert_eq!(validate_config_initialization_args(&authority, 1), Ok(()));
        assert_eq!(
            validate_config_initialization_args(&authority, 0),
            Err(ProgramError::from(InvalidClaimTtl))
        );
        assert_eq!(
            validate_config_initialization_args(&ZERO_ADDRESS, 1),
            Err(ProgramError::from(EmptyEmergencyAuthority))
        );
    }

    #[test]
    fn token_shapes_reject_oversized_spl_and_multisig_collisions() {
        assert_eq!(
            InterfaceMint::from_data(&[0u8; TOKEN_MINT_BASE_SIZE], TokenProgramKind::Spl)
                .map(|_| ()),
            Ok(()),
        );
        assert_eq!(
            InterfaceMint::from_data(&[0u8; TOKEN_MINT_BASE_SIZE + 1], TokenProgramKind::Spl)
                .map(|_| ()),
            Err(ProgramError::InvalidAccountData)
        );
        assert_eq!(
            InterfaceTokenAccount::from_data(
                &[0u8; TOKEN_ACCOUNT_BASE_SIZE],
                TokenProgramKind::Spl,
            )
            .map(|_| ()),
            Ok(())
        );
        assert_eq!(
            InterfaceTokenAccount::from_data(
                &[0u8; TOKEN_ACCOUNT_BASE_SIZE + 1],
                TokenProgramKind::Spl,
            )
            .map(|_| ()),
            Err(ProgramError::InvalidAccountData)
        );

        let multisig = [0u8; TOKEN_MULTISIG_SIZE];
        assert_eq!(
            InterfaceMint::from_data(&multisig, TokenProgramKind::Token2022).map(|_| ()),
            Err(ProgramError::InvalidAccountData)
        );
        assert_eq!(
            InterfaceTokenAccount::from_data(&multisig, TokenProgramKind::Token2022).map(|_| ()),
            Err(ProgramError::InvalidAccountData)
        );
        assert_eq!(
            verify_token_2022_tlv(
                &multisig,
                Token2022Shape::Token,
                TokenAccountRole::SettlementDestination,
            ),
            Err(ProgramError::from(UnsafeTokenExtension))
        );
    }

    #[test]
    fn loader_v3_initialization_is_bound_to_upgrade_authority() {
        let program_data_address = Address::new([41u8; 32]);
        let authority = Address::new([42u8; 32]);
        let impostor = Address::new([43u8; 32]);
        let mut program = [0u8; LOADER_V3_PROGRAM_STATE_LEN];
        program[..4].copy_from_slice(&LOADER_V3_PROGRAM_TAG.to_le_bytes());
        program[4..].copy_from_slice(program_data_address.as_array());
        let mut program_data = [0u8; LOADER_V3_PROGRAM_DATA_METADATA_LEN];
        program_data[..4].copy_from_slice(&LOADER_V3_PROGRAM_DATA_TAG.to_le_bytes());
        program_data[12] = 1;
        program_data[13..45].copy_from_slice(authority.as_array());

        assert_eq!(
            verify_loader_v3_initialization_authority(
                &program,
                &program_data_address,
                &program_data,
                &authority,
            ),
            Ok(())
        );
        assert_eq!(
            verify_loader_v3_initialization_authority(
                &program,
                &program_data_address,
                &program_data,
                &impostor,
            ),
            Err(ProgramError::from(UnauthorizedInitializer))
        );

        program_data[12] = 0;
        assert_eq!(
            verify_loader_v3_initialization_authority(
                &program,
                &program_data_address,
                &program_data,
                &authority,
            ),
            Err(ProgramError::from(UnauthorizedInitializer))
        );
    }

    #[test]
    fn loader_v4_initialization_requires_live_deployed_authority() {
        let authority = Address::new([51u8; 32]);
        let impostor = Address::new([52u8; 32]);
        let mut state = [0u8; LOADER_V4_STATE_LEN];
        state[LOADER_V4_AUTHORITY_OFFSET..LOADER_V4_STATUS_OFFSET]
            .copy_from_slice(authority.as_array());
        state[LOADER_V4_STATUS_OFFSET..LOADER_V4_STATE_LEN]
            .copy_from_slice(&LOADER_V4_STATUS_DEPLOYED.to_le_bytes());

        assert_eq!(
            verify_loader_v4_initialization_authority(&state, &authority),
            Ok(())
        );
        assert_eq!(
            verify_loader_v4_initialization_authority(&state, &impostor),
            Err(ProgramError::from(UnauthorizedInitializer))
        );

        state[LOADER_V4_STATUS_OFFSET..LOADER_V4_STATE_LEN].copy_from_slice(&2u64.to_le_bytes());
        assert_eq!(
            verify_loader_v4_initialization_authority(&state, &authority),
            Err(ProgramError::from(UnauthorizedInitializer))
        );
    }

    #[test]
    fn cicada_token_2022_policy_is_complete_and_fails_closed() {
        assert_eq!(
            verify_token_2022_tlv(
                &[0u8; 82],
                Token2022Shape::Mint,
                TokenAccountRole::SettlementDestination,
            ),
            Ok(())
        );
        assert_eq!(
            verify_token_2022_tlv(
                &[0u8; 165],
                Token2022Shape::Token,
                TokenAccountRole::CustodySource,
            ),
            Ok(())
        );

        let metadata_pointer =
            extended_token_2022(Token2022Shape::Mint, &[(EXT_METADATA_POINTER, &[0u8; 64])]);
        assert_eq!(
            verify_token_2022_tlv(
                &metadata_pointer,
                Token2022Shape::Mint,
                TokenAccountRole::SettlementDestination,
            ),
            Ok(())
        );

        // Transfer fees are a known amount-changing extension, and type 29 is
        // deliberately beyond the current canonical ExtensionType surface.
        for mint in [
            extended_token_2022(Token2022Shape::Mint, &[(1, &[0u8; 108])]),
            extended_token_2022(Token2022Shape::Mint, &[(29, &[])]),
        ] {
            assert_eq!(
                verify_token_2022_tlv(
                    &mint,
                    Token2022Shape::Mint,
                    TokenAccountRole::SettlementDestination,
                ),
                Err(ProgramError::from(UnsafeTokenExtension))
            );
        }

        let mut truncated = extended_token_2022(Token2022Shape::Mint, &[]);
        truncated.extend_from_slice(&EXT_METADATA_POINTER.to_le_bytes());
        truncated.extend_from_slice(&64u16.to_le_bytes());
        truncated.extend_from_slice(&[0u8; 8]);
        assert_eq!(
            verify_token_2022_tlv(
                &truncated,
                Token2022Shape::Mint,
                TokenAccountRole::SettlementDestination,
            ),
            Err(ProgramError::from(UnsafeTokenExtension))
        );

        let duplicate = extended_token_2022(
            Token2022Shape::Mint,
            &[
                (EXT_METADATA_POINTER, &[0u8; 64]),
                (EXT_METADATA_POINTER, &[0u8; 64]),
            ],
        );
        assert_eq!(
            verify_token_2022_tlv(
                &duplicate,
                Token2022Shape::Mint,
                TokenAccountRole::SettlementDestination,
            ),
            Err(ProgramError::from(UnsafeTokenExtension))
        );
    }

    #[test]
    fn token_2022_custody_rejects_unrestorable_or_blocking_account_extensions() {
        let immutable = extended_token_2022(Token2022Shape::Token, &[(EXT_IMMUTABLE_OWNER, &[])]);
        assert_eq!(
            verify_token_2022_tlv(
                &immutable,
                Token2022Shape::Token,
                TokenAccountRole::CustodySource,
            ),
            Err(ProgramError::from(UnsafeTokenExtension))
        );
        assert_eq!(
            verify_token_2022_tlv(
                &immutable,
                Token2022Shape::Token,
                TokenAccountRole::SettlementDestination,
            ),
            Ok(())
        );

        for extension_type in [EXT_MEMO_TRANSFER, EXT_CPI_GUARD] {
            let enabled = extended_token_2022(Token2022Shape::Token, &[(extension_type, &[1])]);
            assert_eq!(
                verify_token_2022_tlv(
                    &enabled,
                    Token2022Shape::Token,
                    TokenAccountRole::CustodySource,
                ),
                Err(ProgramError::from(UnsafeTokenExtension))
            );
            let disabled = extended_token_2022(Token2022Shape::Token, &[(extension_type, &[0])]);
            assert_eq!(
                verify_token_2022_tlv(
                    &disabled,
                    Token2022Shape::Token,
                    TokenAccountRole::CustodySource,
                ),
                Ok(())
            );
        }
    }

    #[test]
    fn source_authority_surface_rejects_delegate_and_close_authority() {
        let mut source = [0u8; TOKEN_ACCOUNT_BASE_SIZE];
        assert_eq!(verify_source_authority_surface(&source), Ok(()));

        source[TOKEN_DELEGATE_OPTION_OFFSET..TOKEN_DELEGATE_OPTION_OFFSET + 4]
            .copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            verify_source_authority_surface(&source),
            Err(ProgramError::from(SourceDelegatePresent))
        );
        source[TOKEN_DELEGATE_OPTION_OFFSET..TOKEN_DELEGATE_OPTION_OFFSET + 4].fill(0);
        source[TOKEN_DELEGATED_AMOUNT_OFFSET] = 1;
        assert_eq!(
            verify_source_authority_surface(&source),
            Err(ProgramError::from(SourceDelegatePresent))
        );
        source[TOKEN_DELEGATED_AMOUNT_OFFSET] = 0;
        source[TOKEN_CLOSE_AUTHORITY_OPTION_OFFSET..TOKEN_CLOSE_AUTHORITY_OPTION_OFFSET + 4]
            .copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            verify_source_authority_surface(&source),
            Err(ProgramError::from(SourceCloseAuthorityPresent))
        );
    }

    #[test]
    fn settlement_accounts_reject_a_distinct_close_authority() {
        let mut settlement = [0u8; TOKEN_ACCOUNT_BASE_SIZE];
        assert_eq!(verify_settlement_authority_surface(&settlement), Ok(()));

        settlement[TOKEN_CLOSE_AUTHORITY_OPTION_OFFSET..TOKEN_CLOSE_AUTHORITY_OPTION_OFFSET + 4]
            .copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            verify_settlement_authority_surface(&settlement),
            Err(ProgramError::from(SettlementCloseAuthorityPresent))
        );
    }

    #[test]
    fn mint_policy_preimage_commits_supply_bytes() {
        let mint = [0u8; TOKEN_MINT_BASE_SIZE];
        let mut inflated = mint;
        inflated[36..44].copy_from_slice(&1u64.to_le_bytes());
        let (before, supply, after) = mint_policy_parts(&mint).unwrap();
        let (inflated_before, inflated_supply, inflated_after) =
            mint_policy_parts(&inflated).unwrap();
        assert_eq!(before, inflated_before);
        assert_ne!(supply, inflated_supply);
        assert_eq!(after, inflated_after);
        assert_eq!(before.len() + supply.len() + after.len(), mint.len());
    }

    #[test]
    fn route_lamport_floors_preserve_excess_sol_and_native_backing() {
        assert_eq!(
            validate_route_lamport_floors(100, 100, 200, 200, 20, 30, false, false),
            Ok(())
        );
        assert_eq!(
            validate_route_lamport_floors(100, 99, 200, 200, 20, 30, false, false),
            Err(ProgramError::from(SourceLamportsDecreased))
        );
        assert_eq!(
            validate_route_lamport_floors(100, 80, 200, 230, 20, 30, true, true),
            Ok(())
        );
        assert_eq!(
            validate_route_lamport_floors(100, 79, 200, 230, 20, 30, true, true),
            Err(ProgramError::from(SourceLamportsDecreased))
        );
        assert_eq!(
            validate_route_lamport_floors(100, 80, 200, 229, 20, 30, true, true),
            Err(ProgramError::from(DestinationLamportsShortfall))
        );
        assert_eq!(
            validate_route_lamport_floors(100, 100, u64::MAX, u64::MAX, 20, 1, false, true,),
            Err(ProgramError::ArithmeticOverflow)
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

    #[test]
    fn duplicate_route_pubkeys_must_be_readonly_with_identical_flags() {
        let duplicate = Address::new([61u8; 32]);
        let other = Address::new([62u8; 32]);
        assert_eq!(
            validate_duplicate_route_meta(
                &duplicate,
                ROUTE_META_WRITABLE,
                &duplicate,
                ROUTE_META_WRITABLE,
            ),
            Err(ProgramError::from(ConflictingDuplicateRouteMeta))
        );
        assert_eq!(
            validate_duplicate_route_meta(&duplicate, 0, &duplicate, 0),
            Ok(())
        );
        assert_eq!(
            validate_duplicate_route_meta(&duplicate, ROUTE_META_WRITABLE, &other, 0),
            Ok(())
        );
        assert_eq!(
            validate_duplicate_route_meta(&duplicate, ROUTE_META_WRITABLE, &duplicate, 0),
            Err(ProgramError::from(ConflictingDuplicateRouteMeta))
        );
    }

    #[test]
    fn committed_mints_may_only_be_duplicated_readonly() {
        let input_mint = Address::new([63u8; 32]);
        let output_mint = Address::new([64u8; 32]);
        let other = Address::new([65u8; 32]);

        for mint in [&input_mint, &output_mint] {
            assert_eq!(
                validate_route_mint_delegation(mint, true, &input_mint, &output_mint),
                Err(ProgramError::from(ProtectedAccountDelegation))
            );
            assert_eq!(
                validate_route_mint_delegation(mint, false, &input_mint, &output_mint),
                Ok(())
            );
        }
        assert_eq!(
            validate_route_mint_delegation(&other, true, &input_mint, &output_mint),
            Ok(())
        );
    }
}
