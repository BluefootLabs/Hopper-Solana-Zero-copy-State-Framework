//! Instruction dispatch -- tag-based routing with zero allocation.
//!
//! Hopper uses a 1-byte or 2-byte tag at the start of instruction data
//! to route to handler functions. The `dispatch!` macro generates an
//! efficient match statement.

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
