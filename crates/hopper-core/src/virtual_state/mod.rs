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
//! let core: &MarketCore = vstate.overlay::<MarketCore>(accounts, 0)?;
//! let book: &OrderBook = vstate.overlay::<OrderBook>(accounts, 1)?;
//! ```

use hopper_runtime::{error::ProgramError, AccountView, Address};
use crate::account::{Pod, FixedLayout};

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
            slots: [VirtualSlot { account_index: 0, require_owned: false, require_writable: false }; N],
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
        accounts: &[AccountView],
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
        accounts: &'a [AccountView],
        slot: usize,
    ) -> Result<&'a T, ProgramError> {
        if slot >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = self.slots[slot].account_index as usize;
        if idx >= accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let acc = &accounts[idx];
        // SAFETY: Frame/caller ensures no conflicting mutable borrows.
        let data = unsafe { acc.borrow_unchecked() };
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: T: Pod, alignment-1, size checked.
        Ok(unsafe { &*(data.as_ptr() as *const T) })
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
        accounts: &'a [AccountView],
        slot: usize,
    ) -> Result<&'a mut T, ProgramError> {
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
        // SAFETY: Caller ensures exclusive access. Slot validated as writable.
        let data = unsafe { acc.borrow_unchecked_mut() };
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: T: Pod, alignment-1, size checked. Exclusive access.
        Ok(unsafe { &mut *(data.as_mut_ptr() as *mut T) })
    }

    /// Get raw immutable data from a virtual slot.
    #[inline]
    pub fn data<'a>(
        &self,
        accounts: &'a [AccountView],
        slot: usize,
    ) -> Result<&'a [u8], ProgramError> {
        if slot >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = self.slots[slot].account_index as usize;
        if idx >= accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        // SAFETY: Frame ensures no conflicting borrows.
        Ok(unsafe { accounts[idx].borrow_unchecked() })
    }

    /// Get the AccountView for a virtual slot.
    #[inline]
    pub fn account<'a>(
        &self,
        accounts: &'a [AccountView],
        slot: usize,
    ) -> Result<&'a AccountView, ProgramError> {
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
    accounts: &'a [AccountView],
    shard_indices: [u8; SHARDS],
    shard_count: usize,
}

impl<'a, const SHARDS: usize> ShardedAccess<'a, SHARDS> {
    /// Create a sharded access from account indices.
    #[inline]
    pub fn new(
        accounts: &'a [AccountView],
        shard_indices: &[u8],
    ) -> Result<Self, ProgramError> {
        if shard_indices.len() > SHARDS {
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
    pub fn shard_account(&self, shard: usize) -> Result<&'a AccountView, ProgramError> {
        if shard >= self.shard_count {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = self.shard_indices[shard] as usize;
        self.accounts.get(idx).ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get the account data for the shard that owns a given key.
    #[inline]
    pub fn data_for_key(&self, key: &[u8]) -> Result<&'a [u8], ProgramError> {
        let shard = self.shard_for_key(key);
        let acc = self.shard_account(shard)?;
        // SAFETY: Frame ensures no conflicting borrows.
        Ok(unsafe { acc.borrow_unchecked() })
    }

    /// Number of shards.
    #[inline(always)]
    pub fn shard_count(&self) -> usize {
        self.shard_count
    }
}
