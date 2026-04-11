use core::convert::TryInto;
use core::mem::size_of;

use hopper_core::abi::{
    TypedAddress, UntypedAddress, WireBool, WireI16, WireI32, WireI64, WireI128, WireU16,
    WireU32, WireU64, WireU128,
};
use hopper_runtime::{Address, ProgramError};

/// Bounded decoder over the instruction payload after the discriminator byte.
pub struct Decoder<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    #[inline(always)]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    #[inline(always)]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    #[inline(always)]
    pub fn remaining(&self) -> &'a [u8] {
        &self.data[self.offset..]
    }

    #[inline]
    pub fn finish(&self) -> Result<(), ProgramError> {
        if self.offset == self.data.len() {
            Ok(())
        } else {
            Err(ProgramError::InvalidInstructionData)
        }
    }

    #[inline]
    pub fn take_remaining(&mut self) -> &'a [u8] {
        let remaining = self.remaining();
        self.offset = self.data.len();
        remaining
    }

    #[inline]
    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], ProgramError> {
        self.take(N)?
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)
    }

    #[inline]
    pub fn read_array_ref<const N: usize>(&mut self) -> Result<&'a [u8; N], ProgramError> {
        self.take(N)?
            .try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)
    }

    #[inline]
    fn take(&mut self, len: usize) -> Result<&'a [u8], ProgramError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end > self.data.len() {
            return Err(ProgramError::InvalidInstructionData);
        }
        let slice = &self.data[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    #[inline]
    fn read_copy<T: Copy>(&mut self) -> Result<T, ProgramError> {
        let bytes = self.take(size_of::<T>())?;
        Ok(unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const T) })
    }
}

/// Decode a single authored handler argument from instruction bytes.
pub trait DecodeInstructionArg<'a>: Sized {
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError>;
}

macro_rules! impl_decode_copy {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<'a> DecodeInstructionArg<'a> for $ty {
                #[inline(always)]
                fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
                    decoder.read_copy::<Self>()
                }
            }
        )*
    };
}

impl_decode_copy!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

impl<'a> DecodeInstructionArg<'a> for bool {
    #[inline]
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
        match u8::decode(decoder)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

macro_rules! impl_decode_wire_int {
    ($($wire:ty => $native:ty),* $(,)?) => {
        $(
            impl<'a> DecodeInstructionArg<'a> for $wire {
                #[inline(always)]
                fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
                    Ok(<$wire>::new(<$native as DecodeInstructionArg<'a>>::decode(decoder)?))
                }
            }
        )*
    };
}

impl_decode_wire_int!(
    WireU16 => u16,
    WireU32 => u32,
    WireU64 => u64,
    WireU128 => u128,
    WireI16 => i16,
    WireI32 => i32,
    WireI64 => i64,
    WireI128 => i128,
);

impl<'a> DecodeInstructionArg<'a> for WireBool {
    #[inline(always)]
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
        Ok(WireBool::new(bool::decode(decoder)?))
    }
}

impl<'a, T> DecodeInstructionArg<'a> for TypedAddress<T> {
    #[inline(always)]
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
        Ok(Self::new(decoder.read_array::<32>()?))
    }
}

impl<'a> DecodeInstructionArg<'a> for UntypedAddress {
    #[inline(always)]
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
        Ok(Self(decoder.read_array::<32>()?))
    }
}

impl<'a> DecodeInstructionArg<'a> for Address {
    #[inline(always)]
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
        Ok(Address::new(decoder.read_array::<32>()?))
    }
}

impl<'a, const N: usize> DecodeInstructionArg<'a> for [u8; N] {
    #[inline(always)]
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
        decoder.read_array::<N>()
    }
}

impl<'a, const N: usize> DecodeInstructionArg<'a> for &'a [u8; N] {
    #[inline(always)]
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
        decoder.read_array_ref::<N>()
    }
}

impl<'a> DecodeInstructionArg<'a> for &'a [u8] {
    #[inline(always)]
    fn decode(decoder: &mut Decoder<'a>) -> Result<Self, ProgramError> {
        Ok(decoder.take_remaining())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_scalars_in_order() {
        let mut data = [0u8; 10];
        data[..2].copy_from_slice(&7u16.to_le_bytes());
        data[2..10].copy_from_slice(&42u64.to_le_bytes());

        let mut decoder = Decoder::new(&data);
        assert_eq!(u16::decode(&mut decoder).unwrap(), 7);
        assert_eq!(u64::decode(&mut decoder).unwrap(), 42);
        assert!(decoder.finish().is_ok());
    }

    #[test]
    fn remainder_slice_consumes_tail() {
        let data = [1u8, 2, 3, 4];
        let mut decoder = Decoder::new(&data);
        let head = u8::decode(&mut decoder).unwrap();
        let tail = <&[u8]>::decode(&mut decoder).unwrap();

        assert_eq!(head, 1);
        assert_eq!(tail, &[2, 3, 4]);
        assert!(decoder.finish().is_ok());
    }

    #[test]
    fn typed_address_decodes() {
        let data = [9u8; 32];
        let mut decoder = Decoder::new(&data);
        let address = TypedAddress::<()>::decode(&mut decoder).unwrap();

        assert_eq!(address.as_bytes(), &[9u8; 32]);
        assert!(decoder.finish().is_ok());
    }

    #[test]
    fn finish_rejects_trailing_bytes() {
        let data = [1u8, 2, 3];
        let mut decoder = Decoder::new(&data);
        assert_eq!(u8::decode(&mut decoder).unwrap(), 1);
        assert_eq!(decoder.finish(), Err(ProgramError::InvalidInstructionData));
    }
}