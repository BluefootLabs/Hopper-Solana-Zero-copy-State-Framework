//! Zero-allocation logging via Solana syscalls.

/// Log a UTF-8 message to the runtime log.
#[inline(always)]
pub fn log(message: &str) {
    #[cfg(target_os = "solana")]
    unsafe {
        crate::syscalls::sol_log_(message.as_ptr(), message.len() as u64);
    }
    #[cfg(not(target_os = "solana"))]
    {
        // No-op off-chain; test harnesses capture logs separately.
        let _ = message;
    }
}

/// Log five u64 values for quick debugging.
#[inline(always)]
pub fn log_64(a: u64, b: u64, c: u64, d: u64, e: u64) {
    #[cfg(target_os = "solana")]
    unsafe {
        crate::syscalls::sol_log_64_(a, b, c, d, e);
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (a, b, c, d, e);
    }
}

/// Log the current compute unit consumption.
#[inline(always)]
pub fn log_compute_units() {
    #[cfg(target_os = "solana")]
    unsafe {
        crate::syscalls::sol_log_compute_units_();
    }
}

/// Emit structured data segments via `sol_log_data` (for events).
#[inline(always)]
pub fn log_data(segments: &[&[u8]]) {
    #[cfg(target_os = "solana")]
    unsafe {
        crate::syscalls::sol_log_data(segments.as_ptr() as *const u8, segments.len() as u64);
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = segments;
    }
}

/// Convenience macro for logging. Equivalent to `hopper_native::log::log(msg)`.
///
/// Usage: `msg!("Hello, {}", name);`
///
/// On BPF, this formats into a stack buffer and calls `sol_log_`.
/// For simple string literals, prefer `hopper_native::log::log("...")` directly.
#[macro_export]
macro_rules! msg {
    ( $literal:expr ) => {
        $crate::log::log($literal)
    };
    ( $fmt:expr, $($arg:tt)* ) => {{
        // On BPF we have limited stack, so use a fixed 256-byte buffer.
        // For string literals, the branch above avoids this entirely.
        #[cfg(target_os = "solana")]
        {
            use core::fmt::Write;
            let mut buf = [0u8; 256];
            let mut wrapper = $crate::log::StackWriter::new(&mut buf);
            let _ = write!(wrapper, $fmt, $($arg)*);
            let len = wrapper.pos();
            $crate::log::log(
                // SAFETY: Write only produces valid UTF-8 from fmt::Display impls.
                unsafe { core::str::from_utf8_unchecked(&buf[..len]) }
            );
        }
        #[cfg(not(target_os = "solana"))]
        {
            let _ = ($fmt, $($arg)*);
        }
    }};
}

/// Stack-allocated write buffer for formatted log messages on BPF.
pub struct StackWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> StackWriter<'a> {
    /// Create a new writer over the given buffer.
    #[inline(always)]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Number of bytes written.
    #[inline(always)]
    pub fn pos(&self) -> usize {
        self.pos
    }
}

impl core::fmt::Write for StackWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len() - self.pos;
        let to_write = bytes.len().min(remaining);
        self.buf[self.pos..self.pos + to_write].copy_from_slice(&bytes[..to_write]);
        self.pos += to_write;
        Ok(())
    }
}
