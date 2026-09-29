//! Zero-allocation logging via Solana syscalls.

/// Log a UTF-8 message to the runtime log.
#[inline(always)]
pub fn log(message: &str) {
    #[cfg(target_os = "solana")]
    // SAFETY: The pointer and the length come from one live slice, which
    // outlives the synchronous syscall.
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
    // SAFETY: The syscall takes no pointer and has no memory precondition.
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
    // SAFETY: The syscall takes no pointer and has no memory precondition.
    unsafe {
        crate::syscalls::sol_log_compute_units_();
    }
}

/// Emit structured data segments via `sol_log_data` (for events).
#[inline(always)]
pub fn log_data(segments: &[&[u8]]) {
    #[cfg(target_os = "solana")]
    // SAFETY: `segments` is a live `&[&[u8]]`; on SBF each element is the
    // 16-byte (pointer, length) pair the syscall reads, and every inner slice
    // outlives the synchronous call.
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
/// On BPF, this formats into a 256-byte stack buffer and calls
/// `sol_log_`. A longer message is cut at the last whole character
/// that fits.
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
            $crate::log::log(wrapper.as_str());
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
    truncated: bool,
}

impl<'a> StackWriter<'a> {
    /// Create a new writer over the given buffer.
    #[inline(always)]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            truncated: false,
        }
    }

    /// Number of bytes written.
    #[inline(always)]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Whether the message did not fit and was cut.
    #[inline(always)]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// The text written so far. A message that did not fit ends at the
    /// last whole character that did.
    #[inline(always)]
    pub fn as_str(&self) -> &str {
        // SAFETY: every byte in `buf[..pos]` was copied from a `&str` by
        // `write_str`, which copies whole strings or cuts on a character
        // boundary and then accepts nothing more, so the prefix is a
        // concatenation of valid UTF-8 strings.
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.pos]) }
    }
}

impl core::fmt::Write for StackWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // After a cut nothing more is accepted: text appended behind a
        // dropped tail would read as if nothing were missing.
        if self.truncated {
            return Ok(());
        }
        let remaining = self.buf.len().saturating_sub(self.pos);
        let mut to_write = s.len();
        if to_write > remaining {
            // Cut on a character boundary: the log syscall refuses bytes
            // that are not UTF-8 and fails the transaction.
            to_write = remaining;
            while !s.is_char_boundary(to_write) {
                to_write -= 1;
            }
            self.truncated = true;
        }
        self.buf[self.pos..self.pos + to_write].copy_from_slice(&s.as_bytes()[..to_write]);
        self.pos += to_write;
        Ok(())
    }
}

#[cfg(test)]
mod stack_writer_tests {
    use super::StackWriter;
    use core::fmt::Write;

    #[test]
    fn a_message_that_fits_is_written_whole() {
        let mut buf = [0u8; 16];
        let mut writer = StackWriter::new(&mut buf);
        write!(writer, "slot {}", 42).unwrap();
        assert_eq!(writer.as_str(), "slot 42");
        assert!(!writer.truncated());
    }

    #[test]
    fn a_cut_never_splits_a_character() {
        // Every buffer length against text with 1, 2, 3 and 4 byte
        // characters: the result is always a prefix made of whole
        // characters, and the longest one that fits.
        let text = "a\u{e9}\u{20ac}\u{1f980}z\u{e9}\u{1f980}";
        for len in 0..=text.len() + 2 {
            let mut buf = [0xffu8; 32];
            let mut writer = StackWriter::new(&mut buf[..len]);
            write!(writer, "{text}").unwrap();
            let written = writer.as_str();
            assert!(text.starts_with(written), "len {len}");
            assert!(core::str::from_utf8(written.as_bytes()).is_ok());
            let next = text[written.len()..].chars().next();
            match next {
                Some(c) => {
                    assert!(writer.truncated());
                    assert!(
                        written.len() + c.len_utf8() > len,
                        "len {len}: room was left"
                    );
                }
                None => assert!(!writer.truncated()),
            }
        }
    }

    #[test]
    fn nothing_is_appended_after_a_cut() {
        let mut buf = [0u8; 4];
        let mut writer = StackWriter::new(&mut buf);
        // The second argument would fit in the byte the first one left.
        let (head, tail) = ("ab\u{20ac}", "c");
        write!(writer, "{head}{tail}").unwrap();
        assert_eq!(writer.as_str(), "ab");
        assert!(writer.truncated());
    }
}
