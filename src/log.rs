//! Backend-neutral logging helpers.
//!
//! Two tiers are exposed:
//!
//! - [`log`] for arbitrary UTF-8 text through the active backend's
//!   `sol_log_` syscall.
//! - [`log_64`] for integer-heavy logs through the five-u64 `sol_log_64_`
//!   syscall, which is the cheapest structured-log path on Solana. This
//!   backs the `hopper_log!` macro's "label + values" form and lets
//!   hot handlers emit telemetry without the `core::fmt::Write` setup
//!   cost that `msg!` pays.

/// Log a UTF-8 message through the active backend.
#[inline(always)]
pub fn log(message: &str) {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    unsafe {
        hopper_native::syscalls::sol_log_(message.as_ptr(), message.len() as u64);
    }

    #[cfg(all(target_os = "solana", feature = "pinocchio-backend"))]
    unsafe {
        pinocchio::syscalls::sol_log_(message.as_ptr(), message.len() as u64);
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    {
        ::solana_program::log::sol_log(message);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = message;
    }
}

/// Log up to five `u64` values through the `sol_log_64_` syscall.
///
/// One syscall, no allocation, no format parsing. Pad unused slots
/// with zero. The Solana runtime renders the five values as a single
/// line "Program log: 0x... 0x... ...". Use this as the tight-loop
/// escape hatch when the output is going to be grep'd, not read.
///
/// ```ignore
/// // Emit "balance, delta, new_balance":
/// hopper_runtime::log::log_64(balance, delta, new_balance, 0, 0);
/// ```
#[inline(always)]
pub fn log_64(a: u64, b: u64, c: u64, d: u64, e: u64) {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    unsafe {
        hopper_native::syscalls::sol_log_64_(a, b, c, d, e);
    }

    #[cfg(all(target_os = "solana", feature = "pinocchio-backend"))]
    unsafe {
        pinocchio::syscalls::sol_log_64_(a, b, c, d, e);
    }

    #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
    {
        ::solana_program::log::sol_log_64(a, b, c, d, e);
    }

    #[cfg(not(target_os = "solana"))]
    {
        let _ = (a, b, c, d, e);
    }
}

/// Stack-allocated write buffer for formatted log messages.
pub struct StackWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> StackWriter<'a> {
    #[inline(always)]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    #[inline(always)]
    pub fn pos(&self) -> usize {
        self.pos
    }
}

impl core::fmt::Write for StackWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len().saturating_sub(self.pos);
        let to_write = bytes.len().min(remaining);
        self.buf[self.pos..self.pos + to_write].copy_from_slice(&bytes[..to_write]);
        self.pos += to_write;
        Ok(())
    }
}