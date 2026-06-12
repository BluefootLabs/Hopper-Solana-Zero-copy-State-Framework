//! Well-known Solana program and sysvar addresses.
//!
//! Compile-time decoded via Hopper Runtime's native base58 decoder.

use hopper_runtime::Address;

/// System program.
pub const SYSTEM_PROGRAM_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "11111111111111111111111111111111",
));

/// SPL Token program.
pub const TOKEN_PROGRAM_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
));

/// Token-2022 program.
pub const TOKEN_2022_PROGRAM_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
));

/// Associated Token Account program.
pub const ATA_PROGRAM_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
));

/// Sysvar Instructions.
pub const SYSVAR_INSTRUCTIONS_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "Sysvar1nstructions1111111111111111111111111",
));

/// Sysvar Clock.
pub const SYSVAR_CLOCK_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "SysvarC1ock11111111111111111111111111111111",
));

/// Sysvar Rent.
pub const SYSVAR_RENT_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "SysvarRent111111111111111111111111111111111",
));

/// Compute Budget program.
pub const COMPUTE_BUDGET_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "ComputeBudget111111111111111111111111111111",
));

/// BPF Loader Upgradeable.
pub const BPF_LOADER_UPGRADEABLE_ID: Address = Address::new_from_array(
    hopper_runtime::__decode_base58_32("BPFLoaderUpgradeab1e11111111111111111111111"),
);

/// Metaplex Token Metadata program.
pub const METADATA_PROGRAM_ID: Address = Address::new_from_array(hopper_runtime::__decode_base58_32(
    "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
));
