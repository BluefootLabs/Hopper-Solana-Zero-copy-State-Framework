use {
    crate::{config::QuasarConfig, error::CliResult, utils, LintCommand},
    quasar_idl::lint,
    std::path::Path,
};

pub fn run(cmd: LintCommand) -> CliResult {
    let config = QuasarConfig::load()?;
    let crate_root = utils::find_program_crate(&config);

    let lint_config = lint::LintConfig {
        fix: cmd.fix,
        graph: cmd.graph.as_deref().map(parse_graph_format),
    };

    run_lint_from_path(&crate_root, &lint_config)
}

/// Run lint from a crate path (standalone `quasar lint`).
pub fn run_lint_from_path(crate_path: &Path, lint_config: &lint::LintConfig) -> CliResult {
    let parsed = quasar_idl::parser::parse_program(crate_path);
    run_lint_on_parsed(&parsed, lint_config)
}

/// Run lint on an already-parsed program (called from build to avoid
/// double-parse).
pub fn run_lint_on_parsed(
    parsed: &quasar_idl::parser::ParsedProgram,
    lint_config: &lint::LintConfig,
) -> CliResult {
    let report = lint::run_lint(parsed, lint_config);
    lint::output::print_report(&report);

    if report.has_errors() {
        Err(crate::error::CliError::process_failure(
            "lint check failed".to_string(),
            1,
        ))
    } else {
        Ok(())
    }
}

fn parse_graph_format(s: &str) -> lint::GraphFormat {
    match s {
        "mermaid" => lint::GraphFormat::Mermaid,
        "dot" => lint::GraphFormat::Dot,
        "json" => lint::GraphFormat::Json,
        _ => lint::GraphFormat::Ascii,
    }
}
