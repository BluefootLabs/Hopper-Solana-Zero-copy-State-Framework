//! Account Virtualization.
//!
//! Virtual state lets protocols model logical state that spans multiple
//! Solana accounts. Use cases:
//!
//! - Protocol state larger than the 10 MiB account limit
//! - Sharded systems (order books, AMM pools, registries)
//! - Multi-account logical entities (e.g. a "Market" = OrderBook + Pool + Config)
//!
//! ## How It Works
//!
//! A `VirtualState` maps N logical slots to physical accounts in the
//! instruction's account array. At runtime it provides unified typed
//! access across all constituent accounts.
//!
//! ```text
//! +--------------+  +--------------+  +--------------+
//! |  Account 0   |  |  Account 1   |  |  Account 2   |
//! |  MarketCore  |  |  OrderBook   |  |  PoolState   |
//! +------+-------+  +------+-------+  +------+-------+
//!        |                 |                 |
//!        +-----------------+-----------------+
//!                          |
//!                +---------v---------+
//!                |  VirtualState     |
//!                |  "Market"         |
//!                |  - core: 0        |
//!                |  - orders: 1      |
//!                |  - pool: 2        |
//!                +-------------------+
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! // Define the virtual mapping
//! let vstate = VirtualState::<3>::new()
//!     .map(0, CORE_IDX)     // slot 0 -> account CORE_IDX
//!     .map(1, ORDERS_IDX)   // slot 1 -> account ORDERS_IDX
//!     .map(2, POOL_IDX);    // slot 2 -> account POOL_IDX
//!
//! // Read from any slot through the virtual view
//! let core = vstate.overlay::<MarketCore>(accounts, 0)?;
//! let book = vstate.overlay::<OrderBook>(accounts, 1)?;
//! ```

use crate::account::{FixedLayout, Pod};
use hopper_runtime::{error::ProgramError, AccountView, Address, Ref, RefMut};

// -- Virtual Slot --

/// A mapping from virtual slot index to account index.
#[derive(Clone, Copy)]
pub struct VirtualSlot {
    /// Index into the instruction's account array.
    pub account_index: u8,
    /// Expected owner (0 = skip owner check, program_id used).
    pub require_owned: bool,
    /// Whether this slot must be writable.
    pub require_writable: bool,
}

impl VirtualSlot {
    /// Create a read-only virtual slot.
    #[inline(always)]
    pub const fn read_only(account_index: u8) -> Self {
        Self {
            account_index,
            require_owned: true,
            require_writable: false,
        }
    }

    /// Create a writable virtual slot.
    #[inline(always)]
    pub const fn writable(account_index: u8) -> Self {
        Self {
            account_index,
            require_owned: true,
            require_writable: true,
        }
    }

    /// Create an unowned slot (for foreign account reads).
    #[inline(always)]
    pub const fn foreign(account_index: u8) -> Self {
        Self {
            account_index,
            require_owned: false,
            require_writable: false,
        }
    }
}

// -- Virtual State --

/// A virtual state assembly mapping `N` slots to accounts.
///
/// Stack-allocated, const-generic. No heap, no alloc.
pub struct VirtualState<const N: usize> {
    slots: [VirtualSlot; N],
    count: usize,
}

impl<const N: usize> VirtualState<N> {
    /// Create a new empty virtual state.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            slots: [VirtualSlot {
                account_index: 0,
                require_owned: false,
                require_writable: false,
            }; N],
            count: 0,
        }
    }

    /// Map a virtual slot to an account index (read-only owned).
    #[inline(always)]
    pub const fn map(mut self, slot: usize, account_index: u8) -> Self {
        assert!(slot < N, "slot index out of bounds");
        self.slots[slot] = VirtualSlot::read_only(account_index);
        if slot >= self.count {
            self.count = slot + 1;
        }
        self
    }

    /// Map a writable virtual slot.
    #[inline(always)]
    pub const fn map_mut(mut self, slot: usize, account_index: u8) -> Self {
        assert!(slot < N, "slot index out of bounds");
        self.slots[slot] = VirtualSlot::writable(account_index);
        if slot >= self.count {
            self.count = slot + 1;
        }
        self
    }

    /// Map a foreign (unowned) virtual slot.
    #[inline(always)]
    pub const fn map_foreign(mut self, slot: usize, account_index: u8) -> Self {
        assert!(slot < N, "slot index out of bounds");
        self.slots[slot] = VirtualSlot::foreign(account_index);
        if slot >= self.count {
            self.count = slot + 1;
        }
        self
    }

    /// Set a slot directly. Used by the `hopper_virtual!` macro for
    /// custom slot configurations that don't fit the standard map/map_mut/map_foreign
    /// builder methods (e.g., writable but unowned).
    #[inline(always)]
    pub const fn set_slot(mut self, slot: usize, vs: VirtualSlot) -> Self {
        assert!(slot < N, "slot index out of bounds");
        self.slots[slot] = vs;
        if slot >= self.count {
            self.count = slot + 1;
        }
        self
    }

    /// Number of mapped slots (highest slot index + 1).
    #[inline(always)]
    pub const fn slot_count(&self) -> usize {
        self.count
    }

    /// Validate all slots against the instruction accounts.
    ///
    /// Checks: account bounds, ownership, writability.
    #[inline]
    pub fn validate(
        &self,
        accounts: &[AccountView<'_>],
        program_id: &Address,
    ) -> Result<(), ProgramError> {
        let mut i = 0;
        while i < self.count {
            let slot = &self.slots[i];
            let idx = slot.account_index as usize;
            if idx >= accounts.len() {
                return Err(ProgramError::NotEnoughAccountKeys);
            }
            let acc = &accounts[idx];

            if slot.require_owned {
                crate::check::check_owner(acc, program_id)?;
            }
            if slot.require_writable {
                crate::check::check_writable(acc)?;
            }
            i += 1;
        }
        Ok(())
    }

    /// Get a typed immutable overlay from a virtual slot.
    #[inline]
    pub fn overlay<'a, T: Pod + FixedLayout>(
        &self,
        accounts: &'a [AccountView<'a>],
        slot: usize,
    ) -> Result<Ref<'a, T>, ProgramError> {
        if slot >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = self.slots[slot].account_index as usize;
        if idx >= accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let acc = &accounts[idx];
        // SAFETY: Pod + FixedLayout types have valid bit patterns;
        // the backend borrow guard still enforces per-account aliasing.
        unsafe { acc.raw_ref::<T>() }
    }

    /// Get a typed mutable overlay from a virtual slot.
    ///
    /// # Safety rationale for `mut_from_ref`
    /// The `&self` receiver is sound because hopper-native's `AccountView` uses
    /// Solana runtime interior mutability (pointer-based access to account data).
    /// The slot's `require_writable` flag is checked to ensure we only mutate
    /// accounts the runtime has granted write access to.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub fn overlay_mut<'a, T: Pod + FixedLayout>(
        &self,
        accounts: &'a [AccountView<'a>],
        slot: usize,
    ) -> Result<RefMut<'a, T>, ProgramError> {
        if slot >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let vs = &self.slots[slot];
        if !vs.require_writable {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = vs.account_index as usize;
        if idx >= accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let acc = &accounts[idx];
        // SAFETY: Pod + FixedLayout types have valid bit patterns;
        // writable check is done above; backend borrow guard enforces aliasing.
        unsafe { acc.raw_mut::<T>() }
    }

    /// Get raw immutable data from a virtual slot.
    #[inline]
    pub fn data<'a>(
        &self,
        accounts: &'a [AccountView<'a>],
        slot: usize,
    ) -> Result<Ref<'a, [u8]>, ProgramError> {
        if slot >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = self.slots[slot].account_index as usize;
        if idx >= accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        accounts[idx].try_borrow()
    }

    /// Get the AccountView for a virtual slot.
    #[inline]
    pub fn account<'a>(
        &self,
        accounts: &'a [AccountView<'a>],
        slot: usize,
    ) -> Result<&'a AccountView<'a>, ProgramError> {
        if slot >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = self.slots[slot].account_index as usize;
        accounts.get(idx).ok_or(ProgramError::NotEnoughAccountKeys)
    }
}

impl<const N: usize> Default for VirtualState<N> {
    fn default() -> Self {
        Self::new()
    }
}

// -- Sharded Collection --

/// A sharded collection that distributes entries across multiple accounts.
///
/// Each shard is an account containing a `FixedVec<T>`. The shard index
/// is determined by a key hash.
///
/// This enables collections that exceed single-account size limits.
pub struct ShardedAccess<'a, const SHARDS: usize> {
    accounts: &'a [AccountView<'a>],
    shard_indices: [u8; SHARDS],
    shard_count: usize,
}

impl<'a, const SHARDS: usize> ShardedAccess<'a, SHARDS> {
    /// Create a sharded access from account indices.
    #[inline]
    pub fn new(
        accounts: &'a [AccountView<'a>],
        shard_indices: &[u8],
    ) -> Result<Self, ProgramError> {
        // Reject an empty shard set: `shard_count` would be 0 and
        // `shard_for_key`'s `hash % self.shard_count` would divide by
        // zero (panic / DoS). A sharded collection needs ≥ 1 shard.
        if shard_indices.is_empty() || shard_indices.len() > SHARDS {
            return Err(ProgramError::InvalidArgument);
        }
        let mut indices = [0u8; SHARDS];
        let mut i = 0;
        while i < shard_indices.len() {
            if shard_indices[i] as usize >= accounts.len() {
                return Err(ProgramError::NotEnoughAccountKeys);
            }
            indices[i] = shard_indices[i];
            i += 1;
        }
        Ok(Self {
            accounts,
            shard_indices: indices,
            shard_count: shard_indices.len(),
        })
    }

    /// Determine which shard a key maps to (simple modular hashing).
    #[inline(always)]
    pub fn shard_for_key(&self, key: &[u8]) -> usize {
        // FNV-1a hash for shard selection
        let mut hash: u32 = 0x811c_9dc5;
        let mut i = 0;
        while i < key.len() {
            hash ^= key[i] as u32;
            hash = hash.wrapping_mul(0x0100_0193);
            i += 1;
        }
        (hash as usize) % self.shard_count
    }

    /// Get the account for a given shard index.
    #[inline]
    pub fn shard_account(&self, shard: usize) -> Result<&'a AccountView<'a>, ProgramError> {
        if shard >= self.shard_count {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = self.shard_indices[shard] as usize;
        self.accounts
            .get(idx)
            .ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get the account data for the shard that owns a given key.
    #[inline]
    pub fn data_for_key(&self, key: &[u8]) -> Result<Ref<'a, [u8]>, ProgramError> {
        let shard = self.shard_for_key(key);
        let acc = self.shard_account(shard)?;
        acc.try_borrow()
    }

    /// Number of shards.
    #[inline(always)]
    pub fn shard_count(&self) -> usize {
        self.shard_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    fn make(seed: u8) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + 8];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: backing sized for header + data, outlives the view.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([seed; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports: 1,
                data_len: 8,
            });
        }
        // SAFETY: raw points at a fully initialized RuntimeAccount.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        // SAFETY: hopper-runtime AccountView is repr(transparent) over the
        // native view under the native backend (same pattern as the
        // frame audit tests).
        let view = unsafe { core::mem::transmute::<NativeAccountView, AccountView>(backend) };
        (backing, view)
    }

    #[test]
    fn empty_shard_set_is_rejected_not_div_by_zero() {
        let (_b, a) = make(1);
        let accounts = [a];
        // Pre-fix, `new(.., &[])` gave shard_count 0 and the first
        // `shard_for_key` divided by zero. Now it's refused up front.
        assert!(ShardedAccess::<4>::new(&accounts, &[]).is_err());
    }

    #[test]
    fn sharding_routes_keys_within_bounds() {
        let (_b0, a0) = make(0);
        let (_b1, a1) = make(1);
        let accounts = [a0, a1];
        let sharded = ShardedAccess::<2>::new(&accounts, &[0, 1]).unwrap();
        for key in [b"alice".as_slice(), b"bob", b"", b"a-very-long-key-value"] {
            assert!(sharded.shard_for_key(key) < sharded.shard_count());
        }
    }
}
