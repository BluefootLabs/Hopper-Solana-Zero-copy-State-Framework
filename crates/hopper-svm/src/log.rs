//! Log capture buffer.
//!
//! Hopper-native log buffer that mirrors what the on-chain runtime
//! writes when a program calls `sol_log_*`. Phase 1 collects logs
//! emitted by built-in programs through the [`InvokeContext`]
//! interface. Phase 2 will additionally collect `sol_log` /
//! `sol_log_64` / `sol_log_pubkey` / `sol_log_compute_units`
//! syscall output from BPF programs.
//!
//! Log-line format:
//!
//! ```text
//! Program <id> invoke [<depth>]
//! Program log: <message>
//! Program <id> consumed <consumed> of <budget> compute units
//! Program <id> success
//! ```
//!
//! …matching what the actual Solana runtime emits, so tests that
//! snapshot log strings can assert against the exact same shape they
//! see in production.

use solana_sdk::pubkey::Pubkey;

/// Captured execution logs.
///
/// Stored as a `Vec<String>` for easy snapshot testing. The
/// [`section`] helper inserts a sentinel for chained instructions
/// so `result.all_logs()` reads as one coherent transcript across
/// `process_instruction_chain` calls.
#[derive(Debug, Default, Clone)]
pub struct LogCapture {
    lines: Vec<String>,
    /// Current invoke depth — used to indent CPI log lines once
    /// Phase 2 wires CPI dispatch.
    depth: u32,
}

impl LogCapture {
    /// Append a raw line. Used by built-ins via the [`InvokeContext`]
    /// interface; in tests, called directly to seed expected output.
    pub fn line(&mut self, s: impl Into<String>) {
        self.lines.push(s.into());
    }

    /// Append a `Program log: <msg>` line — the standard form Solana
    /// programs use to emit human-readable logs. Built-ins should
    /// call this instead of [`line`] so the prefix is consistent
    /// across the transcript.
    pub fn program_log(&mut self, msg: impl AsRef<str>) {
        self.lines.push(format!("Program log: {}", msg.as_ref()));
    }

    /// Append the `Program <id> invoke [<depth>]` framing line
    /// emitted by the runtime when a program starts executing.
    pub fn invoke(&mut self, program_id: &Pubkey) {
        self.depth += 1;
        self.lines
            .push(format!("Program {program_id} invoke [{}]", self.depth));
    }

    /// Append the `Program <id> consumed <consumed> of <budget> CUs`
    /// + `Program <id> success` framing lines emitted when a program
    /// returns Ok. The CU lines mirror the runtime's wire format so
    /// downstream snapshot tests don't have to special-case Hopper.
    pub fn success(&mut self, program_id: &Pubkey, consumed: u64, budget: u64) {
        self.lines.push(format!(
            "Program {program_id} consumed {consumed} of {budget} compute units"
        ));
        self.lines.push(format!("Program {program_id} success"));
        self.depth = self.depth.saturating_sub(1);
    }

    /// Append a failed-program framing line. Carries the error
    /// description verbatim so test-side regex matching can pin on a
    /// specific error code.
    pub fn failure(
        &mut self,
        program_id: &Pubkey,
        consumed: u64,
        budget: u64,
        err: &dyn std::fmt::Display,
    ) {
        self.lines.push(format!(
            "Program {program_id} consumed {consumed} of {budget} compute units"
        ));
        self.lines
            .push(format!("Program {program_id} failed: {err}"));
        self.depth = self.depth.saturating_sub(1);
    }

    /// Insert a chained-instruction divider — a comment line that
    /// marks the boundary between two instructions in a chain. Not
    /// emitted by the on-chain runtime; this is a Hopper-specific
    /// convenience so multi-instruction transcripts stay legible.
    pub fn section(&mut self, label: &str) {
        if !self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.lines.push(format!("# {label}"));
    }

    /// Consume the buffer and return the line vector.
    pub fn into_lines(self) -> Vec<String> {
        self.lines
    }

    /// Borrow the line vector.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `program_log` must prefix the line with `Program log: ` so
    /// downstream snapshot tests see the same shape the runtime
    /// emits. Pin this — many Solana tests grep for that exact
    /// prefix.
    #[test]
    fn program_log_uses_runtime_prefix() {
        let mut buf = LogCapture::default();
        buf.program_log("hello world");
        assert_eq!(buf.lines(), &["Program log: hello world".to_string()]);
    }

    /// invoke / success bookend must include depth and CU framing
    /// in the exact runtime format. Pin against a known-good
    /// transcript so regression on the framing string is caught.
    #[test]
    fn invoke_success_framing_matches_runtime() {
        let pid = Pubkey::new_unique();
        let mut buf = LogCapture::default();
        buf.invoke(&pid);
        buf.program_log("did the thing");
        buf.success(&pid, 42, 200_000);
        let got = buf.lines();
        assert_eq!(got[0], format!("Program {pid} invoke [1]"));
        assert_eq!(got[1], "Program log: did the thing".to_string());
        assert_eq!(
            got[2],
            format!("Program {pid} consumed 42 of 200000 compute units")
        );
        assert_eq!(got[3], format!("Program {pid} success"));
    }
}
