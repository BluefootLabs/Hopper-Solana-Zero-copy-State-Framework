//! Runtime-typed lazy account parsing.
//!
//! The substrate's [`hopper_native::LazyContext`] defers account parsing
//! until a handler asks for each account — the R3 bench lab measured
//! where that pays (0–2-account dispatch paths) and where it does not
//! (see `BENCHMARKS.md`). Its API, however, speaks SUBSTRATE types
//! (native `AccountView`, native `ProgramError`), which left a layering
//! hole: the runtime's `hopper_lazy_entrypoint!` handed handlers the
//! raw substrate context, while the eager `hopper_fast_entrypoint!`
//! bridges to runtime types — so lazy programs needed hand-written glue
//! that eager programs never did (found the honest way: our own bench
//! target would not compile).
//!
//! This module is the missing bridge, mirroring the eager macro's
//! layering exactly: a [`LazyContext`] wrapper whose every method
//! speaks RUNTIME types. The casts are the same layout facts the eager
//! macro relies on — runtime [`AccountView`](crate::AccountView) is
//! `repr(transparent)` over the native view, and the two `ProgramError`
//! enums are layout twins with a `From` glue in both directions.
//!
//! ```ignore
//! hopper::lazy_entrypoint!(process);
//!
//! fn process(ctx: &mut hopper::prelude::LazyContext) -> ProgramResult {
//!     let payer = ctx.next_signer()?;          // runtime AccountView
//!     let vault = ctx.next_writable()?;
//!     match ctx.instruction_data().first() {
//!         Some(0) => ping(),
//!         _ => Err(ProgramError::InvalidInstructionData),
//!     }
//! }
//! ```
//!
//! Substrate authors who want the native-typed context keep it: the
//! substrate macro (`hopper::substrate`-level `hopper_lazy_entrypoint!`)
//! is unchanged; only the runtime/facade spelling bridges.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;

/// Runtime-typed view over a native lazy parsing context.
///
/// Construction happens inside the runtime's `hopper_lazy_entrypoint!`
/// expansion ([`LazyContext::from_native`]); handlers only ever see this
/// wrapper. Every method delegates to the substrate implementation and
/// converts at the boundary — no re-parsing, no copies of account data.
pub struct LazyContext<'a, 'info> {
    inner: &'a mut hopper_native::LazyContext<'info>,
}

impl<'a, 'info> LazyContext<'a, 'info> {
    /// Wrap a substrate lazy context. Called by the runtime's
    /// `hopper_lazy_entrypoint!` expansion; public so custom entrypoint
    /// plumbing can bridge the same way.
    #[inline(always)]
    pub fn from_native(inner: &'a mut hopper_native::LazyContext<'info>) -> Self {
        Self { inner }
    }

    /// Instruction data for this invocation. Available at any time,
    /// including before any account is consumed.
    #[inline(always)]
    pub fn instruction_data(&self) -> &[u8] {
        self.inner.instruction_data()
    }

    /// The executing program's id.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        let native: &hopper_native::Address = self.inner.program_id();
        // SAFETY: `Address` is a transparent 32-byte wrapper shared by
        // the native and runtime layers; the reinterpret is
        // layout-identical (same cast the eager entrypoint macro makes).
        unsafe { &*(native as *const hopper_native::Address as *const Address) }
    }

    /// Total accounts the instruction declared.
    #[inline(always)]
    pub fn total_accounts(&self) -> usize {
        self.inner.total_accounts()
    }

    /// How many accounts have been parsed so far.
    #[inline(always)]
    pub fn parsed_count(&self) -> usize {
        self.inner.parsed_count()
    }

    /// How many declared accounts remain unparsed.
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.inner.remaining()
    }

    /// Parse and return the next account.
    #[inline(always)]
    pub fn next_account(&mut self) -> Result<AccountView<'info>, ProgramError> {
        self.inner
            .next_account()
            .map(AccountView::from_inner)
            .map_err(ProgramError::from)
    }

    /// Parse the next account and require it to be a signer.
    #[inline(always)]
    pub fn next_signer(&mut self) -> Result<AccountView<'info>, ProgramError> {
        self.inner
            .next_signer()
            .map(AccountView::from_inner)
            .map_err(ProgramError::from)
    }

    /// Parse the next account and require it to be writable.
    #[inline(always)]
    pub fn next_writable(&mut self) -> Result<AccountView<'info>, ProgramError> {
        self.inner
            .next_writable()
            .map(AccountView::from_inner)
            .map_err(ProgramError::from)
    }

    /// Parse the next account and require signer + writable (a fee
    /// payer shape).
    #[inline(always)]
    pub fn next_payer(&mut self) -> Result<AccountView<'info>, ProgramError> {
        self.inner
            .next_payer()
            .map(AccountView::from_inner)
            .map_err(ProgramError::from)
    }

    /// Parse the next account and require its owner to be `program`.
    #[inline(always)]
    pub fn next_owned_by(&mut self, program: &Address) -> Result<AccountView<'info>, ProgramError> {
        // SAFETY: transparent 32-byte address reinterpret, as above.
        let native = unsafe { &*(program as *const Address as *const hopper_native::Address) };
        self.inner
            .next_owned_by(native)
            .map(AccountView::from_inner)
            .map_err(ProgramError::from)
    }

    /// Skip the next `n` accounts without exposing them.
    #[inline(always)]
    pub fn skip(&mut self, n: usize) -> Result<(), ProgramError> {
        self.inner.skip(n).map_err(ProgramError::from)
    }

    /// Parse every remaining account and return them as a slice.
    #[inline(always)]
    pub fn drain_remaining(&mut self) -> Result<&[AccountView<'info>], ProgramError> {
        match self.inner.drain_remaining() {
            Ok(native) => {
                // SAFETY: runtime `AccountView` is `repr(transparent)`
                // over the native view (the eager entrypoint macro makes
                // the identical whole-slice reinterpret).
                let views = unsafe {
                    core::slice::from_raw_parts(
                        native.as_ptr() as *const AccountView<'info>,
                        native.len(),
                    )
                };
                Ok(views)
            }
            Err(e) => Err(ProgramError::from(e)),
        }
    }

    /// Already-parsed account at `index`, if it has been consumed.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<&AccountView<'info>> {
        self.inner.get(index).map(|native| {
            // SAFETY: transparent single-view reinterpret, as above.
            unsafe {
                &*(native as *const hopper_native::AccountView<'info> as *const AccountView<'info>)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    /// Build a minimal loader input frame: `count` fresh accounts of
    /// `data_len` bytes each (first one a signer), then instruction
    /// data, then a program id — the same layout the native lazy tests
    /// construct, reduced to what these bridge tests need.
    fn build_frame(
        count: u64,
        data_len: usize,
        ix_data: &[u8],
        program_id: [u8; 32],
    ) -> std::vec::Vec<u8> {
        use hopper_native::RuntimeAccount;
        let mut buf: std::vec::Vec<u8> = std::vec::Vec::new();
        buf.extend_from_slice(&count.to_le_bytes());
        for i in 0..count {
            let start = buf.len();
            // Marker byte 0xFF (fresh) leads the RuntimeAccount header.
            let mut header = [0u8; core::mem::size_of::<RuntimeAccount>()];
            header[0] = 0xFF;
            // SAFETY: header is sized exactly for RuntimeAccount; we
            // construct the value then copy its bytes (test-only).
            let acct = RuntimeAccount {
                borrow_state: 0xFF,
                is_signer: (i == 0) as u8,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: hopper_native::Address::new_from_array([i as u8 + 1; 32]),
                owner: hopper_native::Address::new_from_array([9; 32]),
                lamports: 5,
                data_len: data_len as u64,
            };
            // SAFETY: plain-data struct viewed as bytes, test-only.
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    &acct as *const RuntimeAccount as *const u8,
                    core::mem::size_of::<RuntimeAccount>(),
                )
            };
            header.copy_from_slice(bytes);
            header[0] = 0xFF;
            buf.extend_from_slice(&header);
            buf.extend_from_slice(&std::vec![0u8; data_len]);
            // 10 KiB realloc reserve + pad to 8.
            buf.extend_from_slice(&std::vec![0u8; 10 * 1024]);
            let advanced = buf.len() - start;
            let pad = (8 - (advanced % 8)) % 8;
            buf.extend_from_slice(&std::vec![0u8; pad]);
            // Rent epoch.
            buf.extend_from_slice(&u64::MAX.to_le_bytes());
        }
        buf.extend_from_slice(&(ix_data.len() as u64).to_le_bytes());
        buf.extend_from_slice(ix_data);
        buf.extend_from_slice(&program_id);
        buf
    }

    #[test]
    fn bridge_yields_runtime_types_end_to_end() {
        let mut frame = build_frame(2, 8, &[7, 1, 2], [42; 32]);
        // SAFETY: `frame` is a well-formed loader input buffer built
        // above and outlives the context (held for the whole test).
        let mut native = unsafe { hopper_native::lazy::lazy_deserialize(frame.as_mut_ptr()) };
        let mut ctx = LazyContext::from_native(&mut native);

        assert_eq!(ctx.total_accounts(), 2);
        assert_eq!(ctx.remaining(), 2);
        assert_eq!(ctx.instruction_data(), &[7, 1, 2]);
        assert_eq!(ctx.program_id(), &Address::new([42u8; 32]));

        // Runtime-typed views come back with the right identities.
        let first = ctx.next_signer().expect("first is a signer");
        assert_eq!(first.address(), &Address::new([1u8; 32]));
        assert!(first.is_signer());
        let second = ctx.next_account().expect("second parses");
        assert_eq!(second.address(), &Address::new([2u8; 32]));
        assert_eq!(ctx.parsed_count(), 2);
        assert_eq!(ctx.remaining(), 0);

        // get() re-exposes parsed views by index, runtime-typed.
        assert_eq!(ctx.get(0).unwrap().address(), &Address::new([1u8; 32]));
        assert!(ctx.get(2).is_none());

        // Errors arrive as RUNTIME ProgramError variants.
        assert_eq!(
            ctx.next_account().unwrap_err(),
            ProgramError::NotEnoughAccountKeys,
            "exhaustion maps through the layout-twin glue"
        );
    }

    #[test]
    fn signer_requirement_maps_to_the_runtime_error() {
        let mut frame = build_frame(2, 8, &[0], [1; 32]);
        // SAFETY: as above — well-formed frame, outlives the context.
        let mut native = unsafe { hopper_native::lazy::lazy_deserialize(frame.as_mut_ptr()) };
        let mut ctx = LazyContext::from_native(&mut native);
        ctx.skip(1).expect("skip the signer");
        assert_eq!(
            ctx.next_signer().unwrap_err(),
            ProgramError::MissingRequiredSignature
        );
    }

    #[test]
    fn drain_remaining_casts_the_whole_slice() {
        let mut frame = build_frame(3, 4, &[0], [1; 32]);
        // SAFETY: as above.
        let mut native = unsafe { hopper_native::lazy::lazy_deserialize(frame.as_mut_ptr()) };
        let mut ctx = LazyContext::from_native(&mut native);
        let _first = ctx.next_account().unwrap();
        let rest = ctx.drain_remaining().expect("drains");
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].address(), &Address::new([2u8; 32]));
        assert_eq!(rest[1].address(), &Address::new([3u8; 32]));
    }
}
