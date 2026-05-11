//! Shared terminal styling helpers for the `hopper` CLI.
//!
//! Centralised so output across `init`, `add`, `clean`, `doctor`, etc.
//! looks consistent: same checkmark, same dim grey for paths, same
//! cyan accent for arrows. Mirrors Quasar's `style.rs` shape so output
//! reads similarly to developers porting between the two frameworks,
//! but the colour palette is Hopper's (cyan + lime - Hopper *jumps*,
//! Quasar drifts through a blue nebula).
//!
//! Behavior:
//!
//! - Colour is auto-disabled when `NO_COLOR` is set (the de-facto
//!   standard) or when stdout is not a TTY (output redirected to a
//!   file). Both checks are cached after the first call so we never
//!   probe `isatty` per-line in a tight loop.
//! - `init(color)` is the explicit override, called from `main()` once
//!   the global config is loaded so the user's `ui.color = false`
//!   preference wins over the auto-detect.
//!
//! No new dependencies - just `std::io::IsTerminal` (stable since
//! 1.70) and a couple of `AtomicBool`s.
//!
//! ## Adding new helpers
//!
//! New helpers go here, not in individual modules. The contract is:
//! every helper returns a `String` (or writes ANSI to a `Write`),
//! every helper degrades to a plain ASCII fallback when colour is
//! off, and no helper introduces a new dependency. Keep the palette
//! tight: cyan (45) for accents, lime (83) for success, red (196) for
//! errors, yellow (208) for warnings, dim (2) for paths.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicU8, Ordering};

const COLOR_OFF: u8 = 0;
const COLOR_ON: u8 = 1;
const COLOR_UNINITIALIZED: u8 = 2;

static COLOR_STATE: AtomicU8 = AtomicU8::new(COLOR_UNINITIALIZED);

/// Explicit override. Call once at startup after loading the global
/// config so a user-set `ui.color = false` beats the auto-detect.
pub fn init(color: bool) {
    COLOR_STATE.store(if color { COLOR_ON } else { COLOR_OFF }, Ordering::Release);
}

/// Auto-detect rule: respect `NO_COLOR`, then check stdout TTY.
fn auto_detect() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

fn enabled() -> bool {
    match COLOR_STATE.load(Ordering::Acquire) {
        COLOR_OFF => false,
        COLOR_ON => true,
        _ => {
            let detected = if auto_detect() { COLOR_ON } else { COLOR_OFF };
            match COLOR_STATE.compare_exchange(
                COLOR_UNINITIALIZED,
                detected,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => detected == COLOR_ON,
                Err(current) => current == COLOR_ON,
            }
        }
    }
}

/// Lime checkmark followed by a message. Falls back to `[ok] ...`.
pub fn success(msg: &str) -> String {
    if enabled() {
        format!("\x1b[38;5;83m\u{2714}\x1b[0m {msg}")
    } else {
        format!("[ok] {msg}")
    }
}

/// Red cross. Falls back to `[error] ...`.
pub fn fail(msg: &str) -> String {
    if enabled() {
        format!("\x1b[38;5;196m\u{2718}\x1b[0m {msg}")
    } else {
        format!("[error] {msg}")
    }
}

/// Cyan arrow used for "in-progress" or "next-up" lines.
pub fn step(msg: &str) -> String {
    if enabled() {
        format!("\x1b[38;5;45m\u{276f}\x1b[0m {msg}")
    } else {
        format!("> {msg}")
    }
}

/// Yellow warning triangle.
pub fn warn(msg: &str) -> String {
    if enabled() {
        format!("\x1b[38;5;208m\u{26a0}\x1b[0m {msg}")
    } else {
        format!("[warn] {msg}")
    }
}

/// Bold.
pub fn bold(s: &str) -> String {
    if enabled() {
        format!("\x1b[1m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Dim (grey) - for paths, hints, secondary info.
pub fn dim(s: &str) -> String {
    if enabled() {
        format!("\x1b[2m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Arbitrary 256-colour foreground.
pub fn color(code: u8, s: &str) -> String {
    if enabled() {
        format!("\x1b[38;5;{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Format a byte count using KiB/MiB. Matches what `hopper build`
/// already prints, lifted out so other commands (clean, doctor) can
/// share the same formatting and units.
pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_off_strips_ansi() {
        init(false);
        let s = success("ok");
        assert!(
            !s.contains("\x1b["),
            "expected no ANSI when disabled, got {s:?}"
        );
        assert!(s.starts_with("[ok]"));
    }

    #[test]
    fn human_size_chooses_units() {
        // ordering matters - these run in the same process, so
        // disable colour first to keep tests independent of TTY.
        init(false);
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1024), "1.00 KiB");
        assert_eq!(human_size(2_500_000), "2.38 MiB");
    }
}
