//! Backend-neutral logging helpers.

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