//! Filesystem watcher for `hopper build --watch` and `hopper test --watch`.
//!
//! Poll-based, no external crate dependency. On every tick it walks
//! `src/` and the top-level `Cargo.toml` collecting the max mtime.
//! When that mtime changes, the user-supplied closure re-runs. A
//! 150-ms debounce swallows editor save bursts (vim uses write +
//! rename, VS Code uses atomic-replace) so we do not thrash the
//! build.
//!
//! Watching stops on Ctrl-C; the OS delivers SIGINT to the process,
//! terminates the spawned cargo child (if any), and exits cleanly.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

/// Roots to watch relative to a project root. Exactly the files a
/// Rust program re-reads when the source changes. Includes the
/// workspace `Cargo.toml` because changing dependencies should also
/// re-trigger a build.
const WATCH_FILES: &[&str] = &["Cargo.toml", "Cargo.lock"];
const WATCH_DIRS: &[&str] = &["src", "tests", "benches", "examples"];

/// Poll interval. Slower than a native inotify watcher but avoids the
/// dependency, and the stat cost is trivial for the tiny file set we
/// walk.
const POLL: Duration = Duration::from_millis(200);
/// Debounce window after a detected change. Editors often save via
/// atomic rename, which reads as several distinct mtime hops within a
/// few milliseconds; this coalesces them into one run.
const DEBOUNCE: Duration = Duration::from_millis(150);

/// Run `f` once, then again every time the watched tree changes.
pub fn watch<F>(project_root: &Path, mut f: F)
where
    F: FnMut(),
{
    // Initial run.
    banner(project_root, "initial build");
    f();

    let mut last_seen = max_mtime(project_root).unwrap_or(SystemTime::UNIX_EPOCH);
    loop {
        thread::sleep(POLL);
        let now = match max_mtime(project_root) {
            Some(t) => t,
            None => continue,
        };
        if now > last_seen {
            // Debounce the burst: wait DEBOUNCE, then take the
            // max-mtime snapshot again so every save within the
            // window collapses into one run.
            thread::sleep(DEBOUNCE);
            last_seen = max_mtime(project_root).unwrap_or(now);
            banner(project_root, "change detected, rebuilding");
            f();
        }
    }
}

fn banner(project_root: &Path, msg: &str) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(
        stdout,
        "\n[hopper --watch] {} ({})",
        msg,
        project_root.display()
    );
    let _ = stdout.flush();
}

fn max_mtime(root: &Path) -> Option<SystemTime> {
    let mut best: Option<SystemTime> = None;
    for name in WATCH_FILES {
        if let Some(t) = mtime(&root.join(name)) {
            best = Some(best.map(|b| b.max(t)).unwrap_or(t));
        }
    }
    for dir in WATCH_DIRS {
        walk_mtimes(&root.join(dir), &mut best);
    }
    best
}

fn walk_mtimes(dir: &Path, best: &mut Option<SystemTime>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip target/ and anything that starts with a dot.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || name == "target" {
                    continue;
                }
            }
            walk_mtimes(&path, best);
            continue;
        }
        if let Some(t) = mtime(&path) {
            *best = Some(best.map(|b| b.max(t)).unwrap_or(t));
        }
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Strip `--watch` out of a cargo arg list and report whether it was
/// present. Used by `hopper build` / `hopper test` so the watcher
/// wraps a vanilla cargo invocation without passing the flag through.
pub fn extract_watch_flag(args: &mut Vec<String>) -> bool {
    let before = args.len();
    args.retain(|a| a != "--watch");
    args.len() != before
}

/// Resolve the nearest project root that a watcher should key on.
/// Right now this is just the directory passed in, but the indirection
/// gives us a seam to grow workspace-aware logic later without
/// touching every call site.
pub fn project_watch_root(root: &Path) -> PathBuf {
    root.to_path_buf()
}
