//! Instruction dispatch -- tag-based routing with zero allocation.
//!
//! Hopper uses a 1-byte or 2-byte tag at the start of instruction data
//! to route to handler functions. The `dispatch!` macro generates an
//! efficient match statement.
//!
//! ## Dispatch variants
//!
//! - `hopper_dispatch!`, Standard: receives `(program_id, accounts, data)`
//! - `hopper_dispatch_lazy!`, Lazy: receives `LazyContext`, parses accounts on-demand
//! - `hopper_dispatch_8!`, 8-byte discriminator (Anchor/Quasar compatible)

use hopper_runtime::error::ProgramError;

/// Read a 1-byte dispatch tag from instruction data.
#[inline(always)]
pub fn dispatch_instruction(data: &[u8]) -> Result<(u8, &[u8]), ProgramError> {
    if data.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok((data[0], &data[1..]))
}

/// Read a 2-byte dispatch tag (for programs with >256 instructions).
#[inline(always)]
pub fn dispatch_instruction_u16(data: &[u8]) -> Result<(u16, &[u8]), ProgramError> {
    if data.len() < 2 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let tag = u16::from_le_bytes([data[0], data[1]]);
    Ok((tag, &data[2..]))
}

/// Read an 8-byte discriminator (Anchor/Quasar compatible).
#[inline(always)]
pub fn dispatch_instruction_8(data: &[u8]) -> Result<([u8; 8], &[u8]), ProgramError> {
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&data[..8]);
    Ok((disc, &data[8..]))
}

/// Macro for instruction dispatch.
///
/// ```ignore
/// hopper_dispatch! {
///     program_id, accounts, instruction_data;
///     0 => process_init,
///     1 => process_deposit,
///     2 => process_withdraw,
/// }
/// ```
#[macro_export]
macro_rules! hopper_dispatch {
    (
        $program_id:expr, $accounts:expr, $data:expr;
        $( $tag:literal => $handler:expr ),+ $(,)?
    ) => {{
        let (tag, remaining) = $crate::dispatch::dispatch_instruction($data)?;
        match tag {
            $( $tag => $handler($program_id, $accounts, remaining), )+
            _ => Err($crate::__runtime::error::ProgramError::InvalidInstructionData),
        }
    }};
}

/// Lazy dispatch -- routes on instruction data before parsing any accounts.
///
/// Each handler receives a `&mut LazyContext` and can parse only the accounts
/// it needs, saving CU on instructions that touch few accounts.
///
/// ```ignore
/// hopper_dispatch_lazy! {
///     ctx;
///     0 => process_init,
///     1 => process_deposit,
///     2 => process_withdraw,
/// }
/// ```
#[macro_export]
macro_rules! hopper_dispatch_lazy {
    (
        $ctx:expr;
        $( $tag:literal => $handler:expr ),+ $(,)?
    ) => {{
        let data = $ctx.instruction_data();
        if data.is_empty() {
            return Err($crate::__runtime::error::ProgramError::InvalidInstructionData);
        }
        let tag = data[0];
        match tag {
            $( $tag => $handler($ctx), )+
            _ => Err($crate::__runtime::error::ProgramError::InvalidInstructionData),
        }
    }};
}

/// 8-byte discriminator dispatch (Anchor/Quasar compatible).
///
/// Uses 8-byte discriminators instead of 1-byte tags. This allows
/// interoperability with Anchor IDLs and Quasar programs.
///
/// ```ignore
/// hopper_dispatch_8! {
///     program_id, accounts, instruction_data;
///     [0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d] => initialize,
///     [0xf2, 0x23, 0xc6, 0x89, 0x52, 0xe1, 0xf2, 0xb6] => deposit,
/// }
/// ```
#[macro_export]
macro_rules! hopper_dispatch_8 {
    (
        $program_id:expr, $accounts:expr, $data:expr;
        $( [ $($disc:literal),+ ] => $handler:expr ),+ $(,)?
    ) => {{
        let (disc, remaining) = $crate::dispatch::dispatch_instruction_8($data)?;
        match disc {
            $( [ $($disc),+ ] => $handler($program_id, $accounts, remaining), )+
            _ => Err($crate::__runtime::error::ProgramError::InvalidInstructionData),
        }
    }};
}

#[cfg(test)]
mod tests {
    use hopper_runtime::error::ProgramError;

    fn standard_handler(_: (), _: (), _: &[u8]) -> Result<(), ProgramError> {
        Ok(())
    }

    fn dispatch_standard(data: &[u8]) -> Result<(), ProgramError> {
        crate::hopper_dispatch! {
            (), (), data;
            0 => standard_handler,
        }
    }

    struct LazyFixture<'a> {
        data: &'a [u8],
    }

    impl LazyFixture<'_> {
        fn instruction_data(&self) -> &[u8] {
            self.data
        }
    }

    fn lazy_handler(_: &LazyFixture<'_>) -> Result<(), ProgramError> {
        Ok(())
    }

    fn dispatch_lazy(ctx: &LazyFixture<'_>) -> Result<(), ProgramError> {
        crate::hopper_dispatch_lazy! {
            ctx;
            0 => lazy_handler,
        }
    }

    fn wide_handler(_: (), _: (), _: &[u8]) -> Result<(), ProgramError> {
        Ok(())
    }

    fn dispatch_wide(data: &[u8]) -> Result<(), ProgramError> {
        crate::hopper_dispatch_8! {
            (), (), data;
            [0, 0, 0, 0, 0, 0, 0, 0] => wide_handler,
        }
    }

    #[test]
    fn legacy_unauthenticated_event_marker_is_rejected() {
        let marker = [0xFF, 0xFE, 7];
        assert_eq!(
            dispatch_standard(&marker),
            Err(ProgramError::InvalidInstructionData)
        );
        assert_eq!(
            dispatch_lazy(&LazyFixture { data: &marker }),
            Err(ProgramError::InvalidInstructionData)
        );
        assert_eq!(
            dispatch_wide(&marker),
            Err(ProgramError::InvalidInstructionData)
        );
    }
}
