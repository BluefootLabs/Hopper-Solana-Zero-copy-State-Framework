use {
    crate::{
        config::{CommandSpec, QuasarConfig},
        error::{CliError, CliResult},
        style,
    },
    std::process::Command,
};

pub fn run(
    debug: bool,
    filter: Option<String>,
    watch: bool,
    no_build: bool,
    features: Option<String>,
) -> CliResult {
    if watch {
        run_watch(debug, filter, no_build, features);
    }
    run_once(debug, filter.as_deref(), no_build, features.as_deref())
}

fn run_once(
    debug: bool,
    filter: Option<&str>,
    no_build: bool,
    features: Option<&str>,
) -> CliResult {
    let config = QuasarConfig::load()?;

    if !no_build {
        crate::build::run(debug, false, features.map(String::from), false)?;
    }

    if config.has_typescript_tests() {
        run_typescript_tests(&config, filter)
    } else if config.has_rust_tests() {
        run_rust_tests(&config, filter)
    } else {
        println!("  {}", style::warn("no test framework configured"));
        Ok(())
    }
}

fn run_watch(debug: bool, filter: Option<String>, no_build: bool, features: Option<String>) -> ! {
    crate::build::watch_loop(|| run_once(debug, filter.as_deref(), no_build, features.as_deref()))
}

// ---------------------------------------------------------------------------
// TypeScript (vitest)
// ---------------------------------------------------------------------------

fn run_typescript_tests(config: &QuasarConfig, filter: Option<&str>) -> CliResult {
    let ts = config.testing.typescript.as_ref();
    let default_install = CommandSpec::new("npm", ["install"]);
    let default_test = CommandSpec::new("npx", ["vitest", "run"]);
    let install_cmd = ts.map(|t| &t.install).unwrap_or(&default_install);
    let test_cmd = ts.map(|t| &t.test).unwrap_or(&default_test);

    if !std::path::Path::new("node_modules").exists() {
        run_command(install_cmd)?;
    }

    run_test_cmd(test_cmd, filter)
}

// ---------------------------------------------------------------------------
// Rust (cargo test)
// ---------------------------------------------------------------------------

fn run_rust_tests(config: &QuasarConfig, filter: Option<&str>) -> CliResult {
    let default_test = CommandSpec::new("cargo", ["test", "tests::"]);
    let test_cmd = config
        .testing
        .rust
        .as_ref()
        .map(|r| &r.test)
        .unwrap_or(&default_test);

    run_test_cmd(test_cmd, filter)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn run_command(command: &CommandSpec) -> CliResult {
    let status = Command::new(&command.program).args(&command.args).status();

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(CliError::process_failure(
            format!("{} failed", command.display()),
            s.code().unwrap_or(1),
        )),
        Err(e) => Err(CliError::message(format!(
            "failed to run {}: {e}",
            command.display()
        ))),
    }
}

fn run_test_cmd(test_cmd: &CommandSpec, filter: Option<&str>) -> CliResult {
    let mut cmd = Command::new(&test_cmd.program);
    cmd.args(&test_cmd.args);

    if let Some(pattern) = filter {
        // cargo test uses a positional filter; vitest/jest use -t
        if test_cmd.program == "cargo" {
            cmd.arg(pattern);
        } else {
            cmd.args(["-t", pattern]);
        }
    }

    let status = cmd.status();

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(CliError::process_failure(
            format!("{} failed", test_cmd.display()),
            s.code().unwrap_or(1),
        )),
        Err(e) => Err(CliError::message(format!(
            "failed to run {}: {e}",
            test_cmd.display()
        ))),
    }
}
