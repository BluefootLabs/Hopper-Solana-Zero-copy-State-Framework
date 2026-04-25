use std::process::Command;

/// Check whether sbpf-linker is reachable on PATH.
pub fn has_sbpf_linker() -> bool {
    Command::new("sbpf-linker")
        .arg("--version")
        .output()
        .ok()
        .is_some_and(|o| o.status.success())
}
