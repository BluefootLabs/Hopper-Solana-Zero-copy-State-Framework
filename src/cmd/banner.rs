//! `hopper init` opening banner.
//!
//! Quasar prints an animated FIGlet "Quasar" reveal under a sweeping
//! blue nebula. We do something distinct that fits Hopper's identity
//! — a **leap reveal**: each row of the FIGlet "HOPPER" arrives from
//! below with a small bounce, settling in, while the background fills
//! with a gradient of green dots ("grass") that the H jumped through.
//!
//! The animation is gated on three checks, all of which must pass:
//!
//! 1. `globals.ui.animation == true` (user-toggleable, defaults on).
//! 2. stdout is a TTY (no terminal codes when piped to a file).
//! 3. We're not running under `NO_COLOR` (de-facto disable signal).
//!
//! On fallback, we print a single-line plain text title so the
//! wizard still gets a header, just without the choreography.
//!
//! No external dependencies. The choreography is twenty-ish frames at
//! 50 ms each — under one second total — which is fast enough not to
//! annoy power users running `hopper init` repeatedly during plugin
//! development. After the banner runs once, `globals.ui.animation` is
//! flipped to false in the saved defaults so the second run is silent
//! by default; mirrors Quasar's "be polite on the second run" rule.

use std::io::{self, IsTerminal, Write};

const PALETTE_GRASS: &[u8] = &[28, 34, 40, 46, 82, 118, 154, 190]; // dark→bright green

/// Run the leap-reveal animation. Falls through to a plain header
/// when animation can't run (no TTY, NO_COLOR, etc.).
pub fn print_banner(animation_enabled: bool) {
    let stdout = io::stdout();
    let no_color = std::env::var_os("NO_COLOR").is_some();
    if !animation_enabled || !stdout.is_terminal() || no_color {
        plain_header();
        return;
    }
    if !animate(stdout.lock()).is_ok() {
        // Anything went wrong mid-frame (broken pipe, weird terminal):
        // fall back to the plain header so the wizard still has its
        // intro line.
        plain_header();
    }
}

fn plain_header() {
    println!();
    println!("  hopper init — interactive scaffold");
    println!("  zero-copy Solana state, in 60 seconds");
    println!();
}

/// Inner animator — returns `io::Result` so a broken pipe (someone
/// piped `hopper init` into `head`) is recoverable, not a panic.
fn animate(mut out: impl Write) -> io::Result<()> {
    use std::{thread, time::Duration};

    // FIGlet "HOPPER" — block style, 6 lines tall plus a blank.
    #[rustfmt::skip]
    let figlet: [&str; 6] = [
        "██╗  ██╗ ██████╗ ██████╗ ██████╗ ███████╗██████╗ ",
        "██║  ██║██╔═══██╗██╔══██╗██╔══██╗██╔════╝██╔══██╗",
        "███████║██║   ██║██████╔╝██████╔╝█████╗  ██████╔╝",
        "██╔══██║██║   ██║██╔═══╝ ██╔═══╝ ██╔══╝  ██╔══██╗",
        "██║  ██║╚██████╔╝██║     ██║     ███████╗██║  ██║",
        "╚═╝  ╚═╝ ╚═════╝ ╚═╝     ╚═╝     ╚══════╝╚═╝  ╚═╝",
    ];
    let fig: Vec<Vec<char>> = figlet.iter().map(|l| l.chars().collect()).collect();
    let fig_w = fig.iter().map(|l| l.len()).max().unwrap_or(0);
    let fig_h = fig.len();

    let canvas_w: usize = 70;
    let fig_off = canvas_w.saturating_sub(fig_w) / 2;

    let tagline = "zero-copy Solana state, in 60 seconds";
    let tag_chars: Vec<char> = tagline.chars().collect();
    let tag_off = canvas_w.saturating_sub(tag_chars.len()) / 2;

    let byline = "by hopperzero.dev";
    let by_chars: Vec<char> = byline.chars().collect();
    let by_off = canvas_w.saturating_sub(by_chars.len()) / 2;

    // Layout (h = total lines we redraw each frame):
    //   line 0:        blank
    //   lines 1..=6:   figlet (6 rows)
    //   line 7:        blank
    //   line 8:        tagline
    //   line 9:        byline
    let h: usize = 10;
    let n_frames: usize = 18;

    // Reserve space + hide cursor.
    write!(out, "\x1b[?25l")?;
    writeln!(out)?;
    for _ in 0..h {
        writeln!(out)?;
    }
    out.flush()?;

    for frame in 0..n_frames {
        let is_final = frame == n_frames - 1;
        // Move cursor up `h` rows so we can redraw in place.
        write!(out, "\x1b[{h}A")?;

        // The "leap" t parameter: 0 = letters resting below the line,
        // 1 = settled. We use a damped quadratic ease-out so the H
        // overshoots slightly then settles (the bounce).
        let t_raw = frame as f32 / (n_frames - 1).max(1) as f32;
        let t = ease_out_back(t_raw);

        for li in 0..h {
            // Clear the current line, indent two cols.
            write!(out, "\x1b[2K  ")?;
            match li {
                1..=6 => {
                    // Each figlet row arrives with a per-row delay so
                    // the leftmost column lands first and the rightmost
                    // is still mid-arc. That's the "running jump"
                    // feel.
                    let row_idx = li - 1;
                    let row = &fig[row_idx];
                    // Compute a bound for "how many chars are visible
                    // so far" along this row, easing in left→right.
                    let chars_visible_f = t * (fig_w as f32 + 8.0) - row_idx as f32 * 1.5;
                    let chars_visible = chars_visible_f.max(0.0) as usize;

                    for _ in 0..fig_off {
                        write!(out, " ")?;
                    }
                    for (col, &ch) in row.iter().enumerate() {
                        if col < chars_visible {
                            // Settled letter — cyan accent for HOPPER.
                            if ch != ' ' {
                                write!(out, "\x1b[38;5;45m{ch}\x1b[0m")?;
                            } else {
                                write!(out, " ")?;
                            }
                        } else if col < chars_visible + 4 {
                            // The mid-jump frontier: random green
                            // grass dust ahead of the letter to suggest
                            // motion.
                            let g_idx = (frame + col + row_idx * 3) % PALETTE_GRASS.len();
                            let code = PALETTE_GRASS[g_idx];
                            // Use a tiny dot character; never overrun
                            // the figlet row width.
                            if fig_h > 0 && row_idx == fig_h - 1 {
                                write!(out, "\x1b[38;5;{code}m·\x1b[0m")?;
                            } else {
                                write!(out, " ")?;
                            }
                        } else {
                            write!(out, " ")?;
                        }
                    }
                }
                8 if is_final => {
                    for _ in 0..tag_off {
                        write!(out, " ")?;
                    }
                    write!(out, "\x1b[1m{tagline}\x1b[0m")?;
                }
                9 if is_final => {
                    for _ in 0..by_off {
                        write!(out, " ")?;
                    }
                    write!(out, "\x1b[2mby \x1b[38;5;46mhopperzero.dev\x1b[0m")?;
                }
                _ => {}
            }
            writeln!(out)?;
        }
        out.flush()?;

        if !is_final {
            thread::sleep(Duration::from_millis(50));
        }
    }

    // Restore cursor.
    write!(out, "\x1b[?25h")?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

/// Ease-out-back: settles past 1.0 then bounces back. Standard easing
/// curve, parameters tuned so the overshoot is gentle (~6 %).
fn ease_out_back(t: f32) -> f32 {
    let c1 = 1.70158_f32;
    let c3 = c1 + 1.0;
    let p = t - 1.0;
    1.0 + c3 * p * p * p + c1 * p * p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_back_hits_one_at_end() {
        // Final frame must land at exactly 1.0 (or epsilon-close)
        // so the figlet ends fully revealed, no half-drawn letters.
        let v = ease_out_back(1.0);
        assert!((v - 1.0).abs() < 1e-5, "got {v}");
    }

    #[test]
    fn ease_out_back_overshoots() {
        // Curve must overshoot near the end — that's what gives the
        // bounce its character. Sample around t=0.7.
        let v = ease_out_back(0.7);
        assert!(v > 0.7, "expected overshoot past linear, got {v}");
    }
}
