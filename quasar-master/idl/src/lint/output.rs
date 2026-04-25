//! Terminal formatting and security score dashboard.
use super::types::{LintReport, Severity};

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

pub fn print_report(report: &LintReport) {
    print_score_dashboard(report);
    print_diagnostics(report);
    print_summary(report);
}

fn print_score_dashboard(report: &LintReport) {
    if report.instruction_scores.is_empty() {
        return;
    }

    // Use the program_name from the first instruction score for the header.
    let program_name = &report.instruction_scores[0].program_name;
    let num_instructions = report.instruction_scores.len();
    println!(
        "\n  Program: {} ({} instruction{})\n",
        program_name,
        num_instructions,
        if num_instructions == 1 { "" } else { "s" }
    );

    let mut total_constrained = 0usize;
    let mut total_edges = 0usize;

    for score in &report.instruction_scores {
        total_constrained += score.constrained_edges;
        total_edges += score.total_edges;

        let (icon, color) =
            if score.total_edges == 0 || score.constrained_edges == score.total_edges {
                ("\u{2714}", GREEN) // checkmark
            } else if score.constrained_edges * 2 > score.total_edges {
                ("\u{26A0}", YELLOW) // warning sign
            } else {
                ("\u{2718}", RED) // cross
            };

        let missing = score.total_edges - score.constrained_edges;
        let suffix = if missing > 0 {
            format!("  ({} missing)", missing)
        } else {
            String::new()
        };

        println!(
            "    {:<20} {}/{} edges   {}{}{}{}",
            score.instruction_name,
            score.constrained_edges,
            score.total_edges,
            color,
            icon,
            RESET,
            suffix,
        );
    }

    let pct = (total_constrained * 100)
        .checked_div(total_edges)
        .unwrap_or(100);

    println!(
        "\n  Overall: {}/{} edges constrained ({}%)\n",
        total_constrained, total_edges, pct
    );
}

fn print_diagnostics(report: &LintReport) {
    for diag in &report.diagnostics {
        let (icon, color) = match diag.severity {
            Severity::Error => ("\u{2718}", RED),
            Severity::Warning => ("\u{26A0}", YELLOW),
        };

        let field_part = match &diag.field {
            Some(f) => format!(" on `{}`", f),
            None => String::new(),
        };

        println!(
            "  {}{}{} {} in {}{}: {}",
            color,
            icon,
            RESET,
            diag.rule.code(),
            diag.accounts_struct,
            field_part,
            diag.message
        );

        if let Some(suggestion) = &diag.suggestion {
            println!("    \u{2192} {}", suggestion);
        }

        println!("    Suppress: #[allow({})]\n", diag.rule.suppression_attr());
    }
}

fn print_summary(report: &LintReport) {
    let errors = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();

    if errors == 0 && warnings == 0 {
        let num_instructions = report.instruction_scores.len();
        println!(
            "  {}\u{2714}{} Account graphs verified ({} instruction{}, 0 issues)",
            GREEN,
            RESET,
            num_instructions,
            if num_instructions == 1 { "" } else { "s" }
        );
    } else {
        println!("  {} error(s), {} warning(s)", errors, warnings);
    }
}
