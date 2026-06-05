//! Known external oracle bytes without forcing a Hopper account header.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

pub const ORACLE_PROGRAM_ID: Address = Address::new_from_array([42; 32]);

const PRICE_OFFSET: usize = 8;
const CONFIDENCE_OFFSET: usize = 16;
const SLOT_OFFSET: usize = 24;

pub struct PythLikePrice;

pub struct PythLikePriceView<'a> {
    data: Ref<'a, [u8]>,
}

impl PythLikePriceView<'_> {
    #[inline]
    pub fn price(&self) -> Result<u64> {
        read_u64(self.data.as_ref(), PRICE_OFFSET)
    }

    #[inline]
    pub fn confidence(&self) -> Result<u64> {
        read_u64(self.data.as_ref(), CONFIDENCE_OFFSET)
    }

    #[inline]
    pub fn published_slot(&self) -> Result<u64> {
        read_u64(self.data.as_ref(), SLOT_OFFSET)
    }

    #[inline]
    pub fn checked_price(&self, max_confidence: u64) -> Result<u64> {
        if self.confidence()? > max_confidence {
            return Err(ProgramError::InvalidAccountData);
        }
        self.price()
    }
}

impl ExternalZeroCopy for PythLikePrice {
    type View<'a> = PythLikePriceView<'a>;

    const OWNER: Option<Address> = Some(ORACLE_PROGRAM_ID);
    const DISCRIMINATOR: Option<&'static [u8]> = Some(b"PX");
    const MIN_LEN: usize = 32;

    #[inline]
    fn view<'a>(data: Ref<'a, [u8]>) -> Result<Self::View<'a>> {
        Ok(PythLikePriceView { data })
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 11, version = 1)]
pub struct Market {
    pub authority: Address,
    pub last_price: WireU64,
    pub last_confidence: WireU64,
    pub last_oracle_slot: WireU64,
}

impl Market {
    #[inline]
    pub fn update_from_oracle(&mut self, oracle: &PythLikePriceView<'_>) -> ProgramResult {
        self.last_price.set(oracle.checked_price(50)?);
        self.last_confidence.set(oracle.confidence()?);
        self.last_oracle_slot.set(oracle.published_slot()?);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct UpdateMarket<'info> {
    pub oracle: ExternalAccount<'info, PythLikePrice>,

    #[account(mut)]
    pub market: Account<'info, Market>,
}

#[program(profile = "tiny")]
mod external_oracle_program {
    use super::*;

    #[instruction(0)]
    pub fn update_market(ctx: Ctx<UpdateMarket>) -> ProgramResult {
        let before = ctx.accounts.oracle.snapshot_hash()?;
        let oracle = ctx.accounts.oracle.view()?;
        ctx.accounts
            .market
            .with_mut(|market| market.update_from_oracle(&oracle))?;
        ctx.accounts.oracle.assert_snapshot(&before)
    }

    #[instruction(1)]
    pub fn update_market_from_remaining(ctx: Ctx<UpdateMarket>, oracle_index: u8) -> ProgramResult {
        let oracle = ctx
            .remaining_lazy()
            .at(oracle_index as usize)?
            .external::<PythLikePrice>()?;
        let price = oracle.lens::<u64, PRICE_OFFSET>()?.get();

        ctx.accounts.market.with_mut(|market| {
            market.last_price.set(price);
            Ok(())
        })
    }
}

#[inline]
fn read_u64(data: &[u8], offset: usize) -> Result<u64> {
    let end = offset
        .checked_add(core::mem::size_of::<u64>())
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if end > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&data[offset..end]);
    Ok(u64::from_le_bytes(raw))
}
