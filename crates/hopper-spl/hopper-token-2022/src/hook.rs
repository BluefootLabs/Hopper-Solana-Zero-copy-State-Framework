//! Token-2022 transfer-hook `ExtraAccountMetaList` parsing and resolution.
//!
//! When a Token-2022 mint carries the `TransferHook` extension, every
//! transfer CPIs into the hook program's `Execute` instruction, and the
//! caller must append the *extra accounts* the hook needs. Those extra
//! accounts are described by an `ExtraAccountMetaList` stored in a PDA
//! (`["extra-account-metas", mint]`) owned by the hook program.
//!
//! No other zero-copy Solana framework ships a `no_std`, zero-alloc
//! resolver for that list. This module gives Hopper one: it unpacks the
//! TLV-encoded list and resolves each entry into a concrete
//! `(address, is_signer, is_writable)` so a program can build a correct
//! transfer-with-hook CPI without pulling in `spl-tlv-account-resolution`.
//!
//! # Wire format
//!
//! The extra-account-metas account data is:
//!
//! ```text
//! [0..8]    ExtraAccountMetaList TLV type discriminator (8 bytes)
//! [8..12]   TLV value length (u32 LE)
//! [12..16]  entry count (u32 LE, the PodSlice length)
//! [16..]    `count` × 35-byte ExtraAccountMeta entries
//! ```
//!
//! Each 35-byte `ExtraAccountMeta` is `discriminator: u8`,
//! `address_config: [u8; 32]`, `is_signer: u8`, `is_writable: u8`.
//!
//! # Coverage
//!
//! Resolution supports the seed kinds that derive purely from the
//! instruction data and already-known account keys — `Literal`,
//! `InstructionData`, and `AccountKey` — plus literal-pubkey (disc `0`),
//! this-program PDA (disc `1`), and external-program PDA (disc `>= 128`)
//! entries. `AccountData` seeds (disc-`4`) and the
//! pubkey-from-account-data entry (disc `2`) require reading other
//! accounts' bytes and return [`HookError::UnsupportedSeed`] so the
//! caller can resolve them explicitly rather than silently producing a
//! wrong account list.

use hopper_runtime::pda::find_program_address;
use hopper_runtime::Address;

/// PDA seed prefix for the extra-account-metas account.
pub const EXTRA_ACCOUNT_METAS_SEED: &[u8] = b"extra-account-metas";

/// Size in bytes of one packed `ExtraAccountMeta` entry.
pub const EXTRA_ACCOUNT_META_SIZE: usize = 35;

/// Byte offset where the entry count (PodSlice length) begins.
const COUNT_OFFSET: usize = 12;

/// Byte offset where the first entry begins.
const ENTRIES_OFFSET: usize = 16;

/// Top-bit threshold separating local discriminators (`0..=2`) from
/// external-program-index discriminators (`>= 128`).
const U8_TOP_BIT: u8 = 128;

/// Maximum seeds in a single PDA derivation (Solana's limit is 16).
pub const MAX_PDA_SEEDS: usize = 16;

/// Errors returned while parsing or resolving an extra-account-meta list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookError {
    /// The account data was shorter than the declared structure.
    Truncated,
    /// The declared entry count does not fit the buffer.
    BadCount,
    /// An `address_config` seed referenced an out-of-range index.
    IndexOutOfRange,
    /// A seed kind that needs another account's data (disc-`2` /
    /// `AccountData`); resolve it explicitly.
    UnsupportedSeed,
    /// More seeds than [`MAX_PDA_SEEDS`].
    TooManySeeds,
    /// More resolved accounts than the caller's output buffer holds.
    OutputFull,
}

/// Derive the extra-account-metas PDA for a mint under a hook program.
#[inline]
pub fn extra_account_metas_pda(mint: &Address, hook_program: &Address) -> (Address, u8) {
    find_program_address(&[EXTRA_ACCOUNT_METAS_SEED, mint.as_array()], hook_program)
}

/// One resolved extra account, ready to add to a CPI instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedHookAccount {
    /// Resolved account address.
    pub address: Address,
    /// Whether the hook requires this account as a signer.
    pub is_signer: bool,
    /// Whether the hook requires this account writable.
    pub is_writable: bool,
}

/// A single packed `ExtraAccountMeta` entry, borrowed from the list data.
#[derive(Clone, Copy, Debug)]
pub struct ExtraAccountMeta<'a> {
    /// Entry discriminator (`0` literal, `1` PDA, `>= 128` external PDA).
    pub discriminator: u8,
    /// 32-byte address or packed seed configuration.
    pub address_config: &'a [u8; 32],
    /// Signer flag.
    pub is_signer: bool,
    /// Writable flag.
    pub is_writable: bool,
}

/// Zero-copy view over an `ExtraAccountMetaList`.
#[derive(Clone, Copy, Debug)]
pub struct ExtraAccountMetaList<'a> {
    entries: &'a [u8],
    count: usize,
}

impl<'a> ExtraAccountMetaList<'a> {
    /// Unpack the list from raw extra-account-metas account data.
    ///
    /// The 8-byte TLV type discriminator is not validated against a
    /// hard-coded value (it varies by interface version); the structural
    /// length/count framing is validated instead.
    #[inline]
    pub fn unpack(data: &'a [u8]) -> Result<Self, HookError> {
        if data.len() < ENTRIES_OFFSET {
            return Err(HookError::Truncated);
        }
        let count = u32::from_le_bytes([
            data[COUNT_OFFSET],
            data[COUNT_OFFSET + 1],
            data[COUNT_OFFSET + 2],
            data[COUNT_OFFSET + 3],
        ]) as usize;
        let needed = count
            .checked_mul(EXTRA_ACCOUNT_META_SIZE)
            .and_then(|n| n.checked_add(ENTRIES_OFFSET))
            .ok_or(HookError::BadCount)?;
        if data.len() < needed {
            return Err(HookError::BadCount);
        }
        Ok(Self {
            entries: &data[ENTRIES_OFFSET..needed],
            count,
        })
    }

    /// Number of extra accounts in the list.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the list is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Borrow the `i`-th entry.
    #[inline]
    pub fn get(&self, i: usize) -> Option<ExtraAccountMeta<'a>> {
        if i >= self.count {
            return None;
        }
        let base = i * EXTRA_ACCOUNT_META_SIZE;
        let bytes = &self.entries[base..base + EXTRA_ACCOUNT_META_SIZE];
        // SAFETY: slice is exactly 35 bytes; bytes 1..33 are the config.
        let address_config: &[u8; 32] = bytes[1..33].try_into().ok()?;
        Some(ExtraAccountMeta {
            discriminator: bytes[0],
            address_config,
            is_signer: bytes[33] != 0,
            is_writable: bytes[34] != 0,
        })
    }

    /// Resolve every entry into concrete `(address, signer, writable)`
    /// metas, appending them to `out`.
    ///
    /// `instruction_data` is the hook `Execute` instruction data (used by
    /// `InstructionData` seeds). `known` is the list of accounts already
    /// fixed by the transfer-hook interface (source, mint, destination,
    /// authority, …) followed by any metas resolved so far — entries can
    /// reference earlier-resolved accounts by index, exactly as the SPL
    /// resolver allows.
    ///
    /// Returns the number of accounts appended.
    pub fn resolve_into<const MAX: usize>(
        &self,
        out: &mut HookAccountBuf<MAX>,
        instruction_data: &[u8],
        hook_program: &Address,
        known: &[Address],
    ) -> Result<usize, HookError> {
        let start = out.len();
        for i in 0..self.count {
            let entry = self.get(i).ok_or(HookError::Truncated)?;
            // Earlier-resolved entries are addressable by index too.
            let resolved =
                self.resolve_entry(&entry, instruction_data, hook_program, known, out)?;
            out.push(resolved)?;
        }
        Ok(out.len() - start)
    }

    fn resolve_entry<const MAX: usize>(
        &self,
        entry: &ExtraAccountMeta<'a>,
        instruction_data: &[u8],
        hook_program: &Address,
        known: &[Address],
        resolved_so_far: &HookAccountBuf<MAX>,
    ) -> Result<ResolvedHookAccount, HookError> {
        let disc = entry.discriminator;
        if disc == 0 {
            // Literal pubkey stored directly in address_config.
            return Ok(ResolvedHookAccount {
                address: Address::new_from_array(*entry.address_config),
                is_signer: entry.is_signer,
                is_writable: entry.is_writable,
            });
        }
        if disc == 2 {
            // Pubkey loaded from an account's data; needs the account bytes.
            return Err(HookError::UnsupportedSeed);
        }

        // PDA cases: build the seed slices from the packed config.
        let mut seed_refs: [&[u8]; MAX_PDA_SEEDS] = [&[]; MAX_PDA_SEEDS];
        let n = self.parse_seeds(
            entry.address_config,
            instruction_data,
            known,
            resolved_so_far,
            &mut seed_refs,
        )?;

        let program = if disc == 1 {
            hook_program
        } else {
            // disc >= 128: external PDA off the account at (disc - 128).
            let idx = (disc - U8_TOP_BIT) as usize;
            address_at(idx, known, resolved_so_far)?
        };

        let (address, _bump) = find_program_address(&seed_refs[..n], program);
        Ok(ResolvedHookAccount {
            address,
            is_signer: entry.is_signer,
            is_writable: entry.is_writable,
        })
    }

    /// Parse the packed seed config into borrowed seed slices.
    fn parse_seeds<'s, const MAX: usize>(
        &'s self,
        config: &'s [u8; 32],
        instruction_data: &'s [u8],
        known: &'s [Address],
        resolved_so_far: &'s HookAccountBuf<MAX>,
        out: &mut [&'s [u8]; MAX_PDA_SEEDS],
    ) -> Result<usize, HookError> {
        let mut n = 0usize;
        let mut pos = 0usize;
        while pos < config.len() {
            let kind = config[pos];
            if kind == 0 {
                break; // Uninitialized: end of seed list.
            }
            if n >= MAX_PDA_SEEDS {
                return Err(HookError::TooManySeeds);
            }
            match kind {
                1 => {
                    // Literal: [1, len, bytes..]
                    let len = *config.get(pos + 1).ok_or(HookError::Truncated)? as usize;
                    let start = pos + 2;
                    let end = start.checked_add(len).ok_or(HookError::Truncated)?;
                    out[n] = config.get(start..end).ok_or(HookError::Truncated)?;
                    pos = end;
                }
                2 => {
                    // InstructionData: [2, index, len]
                    let index = *config.get(pos + 1).ok_or(HookError::Truncated)? as usize;
                    let len = *config.get(pos + 2).ok_or(HookError::Truncated)? as usize;
                    let end = index.checked_add(len).ok_or(HookError::IndexOutOfRange)?;
                    out[n] = instruction_data
                        .get(index..end)
                        .ok_or(HookError::IndexOutOfRange)?;
                    pos += 3;
                }
                3 => {
                    // AccountKey: [3, index]
                    let index = *config.get(pos + 1).ok_or(HookError::Truncated)? as usize;
                    out[n] = address_at(index, known, resolved_so_far)?.as_array();
                    pos += 2;
                }
                4 => {
                    // AccountData seeds need another account's bytes.
                    return Err(HookError::UnsupportedSeed);
                }
                _ => return Err(HookError::UnsupportedSeed),
            }
            n += 1;
        }
        Ok(n)
    }
}

/// Look up a resolved address by index across the fixed `known` accounts
/// and the metas resolved so far (which follow them).
fn address_at<'r, const MAX: usize>(
    index: usize,
    known: &'r [Address],
    resolved_so_far: &'r HookAccountBuf<MAX>,
) -> Result<&'r Address, HookError> {
    if index < known.len() {
        Ok(&known[index])
    } else {
        resolved_so_far
            .as_slice()
            .get(index - known.len())
            .map(|m| &m.address)
            .ok_or(HookError::IndexOutOfRange)
    }
}

/// Fixed-capacity, zero-alloc accumulator for resolved hook accounts.
#[derive(Clone, Copy, Debug)]
pub struct HookAccountBuf<const MAX: usize> {
    items: [ResolvedHookAccount; MAX],
    len: usize,
}

impl<const MAX: usize> Default for HookAccountBuf<MAX> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const MAX: usize> HookAccountBuf<MAX> {
    /// An empty buffer.
    #[inline]
    pub const fn new() -> Self {
        Self {
            items: [ResolvedHookAccount {
                address: Address::new_from_array([0u8; 32]),
                is_signer: false,
                is_writable: false,
            }; MAX],
            len: 0,
        }
    }

    /// Number of resolved accounts collected.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no accounts have been collected.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Append one resolved account.
    #[inline]
    pub fn push(&mut self, item: ResolvedHookAccount) -> Result<(), HookError> {
        if self.len >= MAX {
            return Err(HookError::OutputFull);
        }
        self.items[self.len] = item;
        self.len += 1;
        Ok(())
    }

    /// The collected accounts as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[ResolvedHookAccount] {
        &self.items[..self.len]
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec as std_vec;

    /// Build a minimal extra-account-metas buffer with `entries` 35-byte
    /// records already laid out.
    fn build(entries: &[[u8; EXTRA_ACCOUNT_META_SIZE]]) -> std_vec::Vec<u8> {
        let mut buf = std_vec::Vec::new();
        buf.extend_from_slice(&[0u8; 8]); // type discriminator
        let value_len = 4 + entries.len() * EXTRA_ACCOUNT_META_SIZE;
        buf.extend_from_slice(&(value_len as u32).to_le_bytes());
        buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for e in entries {
            buf.extend_from_slice(e);
        }
        buf
    }

    #[test]
    fn unpacks_count_and_literal_entry() {
        let mut entry = [0u8; EXTRA_ACCOUNT_META_SIZE];
        entry[0] = 0; // literal
        entry[1..33].copy_from_slice(&[7u8; 32]);
        entry[33] = 0;
        entry[34] = 1; // writable
        let data = build(&[entry]);

        let list = ExtraAccountMetaList::unpack(&data).unwrap();
        assert_eq!(list.len(), 1);

        let mut out = HookAccountBuf::<8>::new();
        let program = Address::new_from_array([9u8; 32]);
        let n = list.resolve_into(&mut out, &[], &program, &[]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            out.as_slice()[0].address,
            Address::new_from_array([7u8; 32])
        );
        assert!(out.as_slice()[0].is_writable);
        assert!(!out.as_slice()[0].is_signer);
    }

    #[test]
    fn rejects_truncated_buffer() {
        assert_eq!(
            ExtraAccountMetaList::unpack(&[0u8; 4]).unwrap_err(),
            HookError::Truncated
        );
    }

    #[test]
    fn rejects_count_overrunning_buffer() {
        let mut data = std::vec![0u8; ENTRIES_OFFSET];
        data[COUNT_OFFSET..COUNT_OFFSET + 4].copy_from_slice(&100u32.to_le_bytes());
        assert_eq!(
            ExtraAccountMetaList::unpack(&data).unwrap_err(),
            HookError::BadCount
        );
    }

    #[test]
    fn account_data_seed_is_unsupported() {
        let mut entry = [0u8; EXTRA_ACCOUNT_META_SIZE];
        entry[0] = 1; // PDA off this program
        entry[1] = 4; // AccountData seed kind
        entry[2] = 0;
        entry[3] = 0;
        entry[4] = 8;
        let data = build(&[entry]);
        let list = ExtraAccountMetaList::unpack(&data).unwrap();
        let mut out = HookAccountBuf::<4>::new();
        let program = Address::new_from_array([1u8; 32]);
        assert_eq!(
            list.resolve_into(&mut out, &[], &program, &[]).unwrap_err(),
            HookError::UnsupportedSeed
        );
    }
}
