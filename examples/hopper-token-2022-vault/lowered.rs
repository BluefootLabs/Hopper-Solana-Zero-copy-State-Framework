// Hopper lowered Rust preview.
// Generated from ProgramManifest metadata to make the runtime path explicit.
// This is the code shape Hopper wants you to reason about: indexes, offsets, borrows, and accessors.

use hopper::prelude::*;
use hopper::__runtime::{Ref, RefMut};

pub mod hopper_token_2022_vault_generated {
    pub const PROGRAM_NAME: &str = "hopper_token_2022_vault";
    pub const PROGRAM_VERSION: &str = "0.1.0";
    pub const PROGRAM_DESCRIPTION: &str = "A Hopper-authored Token-2022 vault with local manifest-backed CLI preview.";
    pub const HEADER_LEN: usize = 16;

    pub mod layouts {
        pub mod reward_vault {
            pub const NAME: &str = "RewardVault";
            pub const DISC: u8 = 41;
            pub const VERSION: u8 = 1;
            pub const TOTAL_SIZE: usize = 129;
            pub const LAYOUT_ID: [u8; 8] = [84, 50, 86, 65, 85, 76, 84, 1];
            pub const TYPE_OFFSET: usize = HEADER_LEN;
            
            // authority: [u8;32] @ bytes 16..48
            // pointer path: account.try_borrow()? -> base_ptr.add(16) as *const [u8;32]
            pub const AUTHORITY_OFFSET: usize = 16;
            pub const AUTHORITY_SIZE: usize = 32;
            
            // mint: [u8;32] @ bytes 48..80
            // pointer path: account.try_borrow()? -> base_ptr.add(48) as *const [u8;32]
            pub const MINT_OFFSET: usize = 48;
            pub const MINT_SIZE: usize = 32;
            
            // vault_ata: [u8;32] @ bytes 80..112
            // pointer path: account.try_borrow()? -> base_ptr.add(80) as *const [u8;32]
            pub const VAULT_ATA_OFFSET: usize = 80;
            pub const VAULT_ATA_SIZE: usize = 32;
            
            // minted_total: WireU64 @ bytes 112..120
            // pointer path: account.try_borrow()? -> base_ptr.add(112) as *const WireU64
            pub const MINTED_TOTAL_OFFSET: usize = 112;
            pub const MINTED_TOTAL_SIZE: usize = 8;
            
            // swept_total: WireU64 @ bytes 120..128
            // pointer path: account.try_borrow()? -> base_ptr.add(120) as *const WireU64
            pub const SWEPT_TOTAL_OFFSET: usize = 120;
            pub const SWEPT_TOTAL_SIZE: usize = 8;
            
            // bump: u8 @ bytes 128..129
            // pointer path: account.try_borrow()? -> base_ptr.add(128) as *const u8
            pub const BUMP_OFFSET: usize = 128;
            pub const BUMP_SIZE: usize = 1;
            
        }

    }

    pub mod instructions {
        pub mod init_vault {
            pub const NAME: &str = "init_vault";
            pub const TAG: u8 = 0;
            pub const READS: &[&str] = &["authority", "system_program"];
            pub const WRITES: &[&str] = &["payer", "vault_state"];
            pub const SIGNERS: &[&str] = &["payer", "authority"];
            pub const POLICY_PACK: &str = "TOKEN_2022_VAULT_WRITE";
            pub const RECEIPT_EXPECTED: bool = true;
            
            // Generated from InstructionDescriptor account order.
            pub struct InitVaultAccounts;
            impl InitVaultAccounts {
                pub const ACCOUNT_LEN: usize = 4;
                
                pub const PAYER_INDEX: usize = 0;
                pub const VAULT_STATE_INDEX: usize = 1;
                pub const AUTHORITY_INDEX: usize = 2;
                pub const SYSTEM_PROGRAM_INDEX: usize = 3;
                
                // payer: AccountView [mut] [signer]
                pub fn payer_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::PAYER_INDEX)
                }
                
                // vault_state: AccountView [mut]
                // layout = RewardVault
                pub fn vault_state_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_STATE_INDEX)
                }
                pub fn vault_state_load(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_ref(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_ref::<RewardVault>() }
                }
                // Whole-account mutable path. Use Context::segment_mut(...) when you only need a narrower region.
                pub fn vault_state_load_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load_mut::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_mut::<RewardVault>() }
                }
                
                // authority: AccountView [signer]
                pub fn authority_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::AUTHORITY_INDEX)
                }
                
                // system_program: AccountView
                pub fn system_program_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::SYSTEM_PROGRAM_INDEX)
                }
                
            }
        }

        pub mod prepare_vault_ata {
            pub const NAME: &str = "prepare_vault_ata";
            pub const TAG: u8 = 1;
            pub const READS: &[&str] = &["authority", "mint", "system_program", "token_program_2022"];
            pub const WRITES: &[&str] = &["payer", "vault_state", "vault_ata"];
            pub const SIGNERS: &[&str] = &["payer", "authority"];
            pub const POLICY_PACK: &str = "TOKEN_2022_VAULT_WRITE";
            pub const RECEIPT_EXPECTED: bool = true;
            
            // Generated from InstructionDescriptor account order.
            pub struct PrepareVaultAtaAccounts;
            impl PrepareVaultAtaAccounts {
                pub const ACCOUNT_LEN: usize = 7;
                
                pub const PAYER_INDEX: usize = 0;
                pub const AUTHORITY_INDEX: usize = 1;
                pub const VAULT_STATE_INDEX: usize = 2;
                pub const VAULT_ATA_INDEX: usize = 3;
                pub const MINT_INDEX: usize = 4;
                pub const SYSTEM_PROGRAM_INDEX: usize = 5;
                pub const TOKEN_PROGRAM_2022_INDEX: usize = 6;
                
                // payer: AccountView [mut] [signer]
                pub fn payer_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::PAYER_INDEX)
                }
                
                // authority: AccountView [signer]
                pub fn authority_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::AUTHORITY_INDEX)
                }
                
                // vault_state: AccountView [mut]
                // layout = RewardVault
                pub fn vault_state_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_STATE_INDEX)
                }
                pub fn vault_state_load(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_ref(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_ref::<RewardVault>() }
                }
                // Whole-account mutable path. Use Context::segment_mut(...) when you only need a narrower region.
                pub fn vault_state_load_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load_mut::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_mut::<RewardVault>() }
                }
                
                // vault_ata: AccountView [mut]
                pub fn vault_ata_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_ATA_INDEX)
                }
                
                // mint: AccountView
                pub fn mint_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::MINT_INDEX)
                }
                
                // system_program: AccountView
                pub fn system_program_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::SYSTEM_PROGRAM_INDEX)
                }
                
                // token_program_2022: AccountView
                pub fn token_program_2022_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::TOKEN_PROGRAM_2022_INDEX)
                }
                
            }
        }

        pub mod mint_rewards {
            pub const NAME: &str = "mint_rewards";
            pub const TAG: u8 = 2;
            pub const READS: &[&str] = &["authority", "token_program_2022"];
            pub const WRITES: &[&str] = &["vault_state", "vault_ata", "mint"];
            pub const SIGNERS: &[&str] = &["authority"];
            pub const POLICY_PACK: &str = "TOKEN_2022_MINT";
            pub const RECEIPT_EXPECTED: bool = true;
            
            // Instruction arguments:
            //   amount: WireU64 (8 bytes)
            
            // Generated from InstructionDescriptor account order.
            pub struct MintRewardsAccounts;
            impl MintRewardsAccounts {
                pub const ACCOUNT_LEN: usize = 5;
                
                pub const AUTHORITY_INDEX: usize = 0;
                pub const VAULT_STATE_INDEX: usize = 1;
                pub const VAULT_ATA_INDEX: usize = 2;
                pub const MINT_INDEX: usize = 3;
                pub const TOKEN_PROGRAM_2022_INDEX: usize = 4;
                
                // authority: AccountView [signer]
                pub fn authority_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::AUTHORITY_INDEX)
                }
                
                // vault_state: AccountView [mut]
                // layout = RewardVault
                pub fn vault_state_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_STATE_INDEX)
                }
                pub fn vault_state_load(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_ref(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_ref::<RewardVault>() }
                }
                // Whole-account mutable path. Use Context::segment_mut(...) when you only need a narrower region.
                pub fn vault_state_load_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load_mut::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_mut::<RewardVault>() }
                }
                
                // vault_ata: AccountView [mut]
                pub fn vault_ata_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_ATA_INDEX)
                }
                
                // mint: AccountView [mut]
                pub fn mint_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::MINT_INDEX)
                }
                
                // token_program_2022: AccountView
                pub fn token_program_2022_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::TOKEN_PROGRAM_2022_INDEX)
                }
                
            }
        }

        pub mod sweep_rewards {
            pub const NAME: &str = "sweep_rewards";
            pub const TAG: u8 = 3;
            pub const READS: &[&str] = &["authority", "token_program_2022"];
            pub const WRITES: &[&str] = &["vault_state", "vault_ata", "destination_ata"];
            pub const SIGNERS: &[&str] = &["authority"];
            pub const POLICY_PACK: &str = "TOKEN_2022_SWEEP";
            pub const RECEIPT_EXPECTED: bool = true;
            
            // Instruction arguments:
            //   amount: WireU64 (8 bytes)
            
            // Generated from InstructionDescriptor account order.
            pub struct SweepRewardsAccounts;
            impl SweepRewardsAccounts {
                pub const ACCOUNT_LEN: usize = 5;
                
                pub const AUTHORITY_INDEX: usize = 0;
                pub const VAULT_STATE_INDEX: usize = 1;
                pub const VAULT_ATA_INDEX: usize = 2;
                pub const DESTINATION_ATA_INDEX: usize = 3;
                pub const TOKEN_PROGRAM_2022_INDEX: usize = 4;
                
                // authority: AccountView [signer]
                pub fn authority_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::AUTHORITY_INDEX)
                }
                
                // vault_state: AccountView [mut]
                // layout = RewardVault
                pub fn vault_state_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_STATE_INDEX)
                }
                pub fn vault_state_load(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_ref(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_ref::<RewardVault>() }
                }
                // Whole-account mutable path. Use Context::segment_mut(...) when you only need a narrower region.
                pub fn vault_state_load_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load_mut::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_mut::<RewardVault>() }
                }
                
                // vault_ata: AccountView [mut]
                pub fn vault_ata_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_ATA_INDEX)
                }
                
                // destination_ata: AccountView [mut]
                pub fn destination_ata_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::DESTINATION_ATA_INDEX)
                }
                
                // token_program_2022: AccountView
                pub fn token_program_2022_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::TOKEN_PROGRAM_2022_INDEX)
                }
                
            }
        }

    }

    pub mod contexts {
        pub mod init_vault {
            pub const NAME: &str = "InitVault";
            pub const POLICIES: &[&str] = &["TOKEN_2022_VAULT_WRITE"];
            pub const MUTATION_CLASSES: &[&str] = &["StateTransition"];
            pub const RECEIPTS_EXPECTED: bool = true;
            
            // Generated from ContextDescriptor account order.
            pub struct InitVaultContext;
            impl InitVaultContext {
                pub const ACCOUNT_LEN: usize = 4;
                
                pub const PAYER_INDEX: usize = 0;
                pub const VAULT_STATE_INDEX: usize = 1;
                pub const AUTHORITY_INDEX: usize = 2;
                pub const SYSTEM_PROGRAM_INDEX: usize = 3;
                
                // payer: Signer [mut] [signer]
                pub fn payer_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::PAYER_INDEX)
                }
                
                // vault_state: HopperAccount [mut]
                // layout = RewardVault
                // policy = TOKEN_2022_VAULT_WRITE
                pub fn vault_state_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_STATE_INDEX)
                }
                pub fn vault_state_load(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_ref(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_ref::<RewardVault>() }
                }
                // Whole-account mutable path. Use Context::segment_mut(...) when you only need a narrower region.
                pub fn vault_state_load_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load_mut::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_mut::<RewardVault>() }
                }
                
                // authority: Signer [signer]
                pub fn authority_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::AUTHORITY_INDEX)
                }
                
                // system_program: ProgramRef
                pub fn system_program_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::SYSTEM_PROGRAM_INDEX)
                }
                
            }
        }

        pub mod prepare_vault_ata {
            pub const NAME: &str = "PrepareVaultAta";
            pub const POLICIES: &[&str] = &["TOKEN_2022_VAULT_WRITE"];
            pub const MUTATION_CLASSES: &[&str] = &["StateTransition", "InPlace"];
            pub const RECEIPTS_EXPECTED: bool = true;
            
            // Generated from ContextDescriptor account order.
            pub struct PrepareVaultAtaContext;
            impl PrepareVaultAtaContext {
                pub const ACCOUNT_LEN: usize = 7;
                
                pub const PAYER_INDEX: usize = 0;
                pub const AUTHORITY_INDEX: usize = 1;
                pub const VAULT_STATE_INDEX: usize = 2;
                pub const VAULT_ATA_INDEX: usize = 3;
                pub const MINT_INDEX: usize = 4;
                pub const SYSTEM_PROGRAM_INDEX: usize = 5;
                pub const TOKEN_PROGRAM_2022_INDEX: usize = 6;
                
                // payer: Signer [mut] [signer]
                pub fn payer_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::PAYER_INDEX)
                }
                
                // authority: Signer [signer]
                pub fn authority_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::AUTHORITY_INDEX)
                }
                
                // vault_state: HopperAccount [mut]
                // layout = RewardVault
                // policy = TOKEN_2022_VAULT_WRITE
                pub fn vault_state_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_STATE_INDEX)
                }
                pub fn vault_state_load(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_ref(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_ref::<RewardVault>() }
                }
                // Whole-account mutable path. Use Context::segment_mut(...) when you only need a narrower region.
                pub fn vault_state_load_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load_mut::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_mut::<RewardVault>() }
                }
                
                // vault_ata: TokenAccount [mut]
                pub fn vault_ata_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_ATA_INDEX)
                }
                
                // mint: MintAccount
                pub fn mint_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::MINT_INDEX)
                }
                
                // system_program: ProgramRef
                pub fn system_program_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::SYSTEM_PROGRAM_INDEX)
                }
                
                // token_program_2022: ProgramRef
                pub fn token_program_2022_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::TOKEN_PROGRAM_2022_INDEX)
                }
                
            }
        }

        pub mod mint_rewards {
            pub const NAME: &str = "MintRewards";
            pub const POLICIES: &[&str] = &["TOKEN_2022_MINT"];
            pub const MUTATION_CLASSES: &[&str] = &["Financial", "InPlace"];
            pub const RECEIPTS_EXPECTED: bool = true;
            
            // Generated from ContextDescriptor account order.
            pub struct MintRewardsContext;
            impl MintRewardsContext {
                pub const ACCOUNT_LEN: usize = 5;
                
                pub const AUTHORITY_INDEX: usize = 0;
                pub const VAULT_STATE_INDEX: usize = 1;
                pub const VAULT_ATA_INDEX: usize = 2;
                pub const MINT_INDEX: usize = 3;
                pub const TOKEN_PROGRAM_2022_INDEX: usize = 4;
                
                // authority: Signer [signer]
                pub fn authority_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::AUTHORITY_INDEX)
                }
                
                // vault_state: HopperAccount [mut]
                // layout = RewardVault
                // policy = TOKEN_2022_MINT
                pub fn vault_state_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_STATE_INDEX)
                }
                pub fn vault_state_load(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_ref(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_ref::<RewardVault>() }
                }
                // Whole-account mutable path. Use Context::segment_mut(...) when you only need a narrower region.
                pub fn vault_state_load_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load_mut::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_mut::<RewardVault>() }
                }
                
                // vault_ata: TokenAccount [mut]
                pub fn vault_ata_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_ATA_INDEX)
                }
                
                // mint: MintAccount [mut]
                pub fn mint_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::MINT_INDEX)
                }
                
                // token_program_2022: ProgramRef
                pub fn token_program_2022_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::TOKEN_PROGRAM_2022_INDEX)
                }
                
            }
        }

        pub mod sweep_rewards {
            pub const NAME: &str = "SweepRewards";
            pub const POLICIES: &[&str] = &["TOKEN_2022_SWEEP"];
            pub const MUTATION_CLASSES: &[&str] = &["Financial", "InPlace"];
            pub const RECEIPTS_EXPECTED: bool = true;
            
            // Generated from ContextDescriptor account order.
            pub struct SweepRewardsContext;
            impl SweepRewardsContext {
                pub const ACCOUNT_LEN: usize = 5;
                
                pub const AUTHORITY_INDEX: usize = 0;
                pub const VAULT_STATE_INDEX: usize = 1;
                pub const VAULT_ATA_INDEX: usize = 2;
                pub const DESTINATION_ATA_INDEX: usize = 3;
                pub const TOKEN_PROGRAM_2022_INDEX: usize = 4;
                
                // authority: Signer [signer]
                pub fn authority_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::AUTHORITY_INDEX)
                }
                
                // vault_state: HopperAccount [mut]
                // layout = RewardVault
                // policy = TOKEN_2022_SWEEP
                pub fn vault_state_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_STATE_INDEX)
                }
                pub fn vault_state_load(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_ref(ctx: &Context<'_>) -> Result<Ref<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_ref::<RewardVault>() }
                }
                // Whole-account mutable path. Use Context::segment_mut(...) when you only need a narrower region.
                pub fn vault_state_load_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    Self::vault_state_account(ctx)?.load_mut::<RewardVault>()
                }
                pub unsafe fn vault_state_raw_mut(ctx: &Context<'_>) -> Result<RefMut<'_, RewardVault>, ProgramError> {
                    unsafe { Self::vault_state_account(ctx)?.raw_mut::<RewardVault>() }
                }
                
                // vault_ata: TokenAccount [mut]
                pub fn vault_ata_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::VAULT_ATA_INDEX)
                }
                
                // destination_ata: TokenAccount [mut]
                pub fn destination_ata_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account_mut(Self::DESTINATION_ATA_INDEX)
                }
                
                // token_program_2022: ProgramRef
                pub fn token_program_2022_account(ctx: &Context<'_>) -> Result<&AccountView, ProgramError> {
                    ctx.account(Self::TOKEN_PROGRAM_2022_INDEX)
                }
                
            }
        }

    }

}
