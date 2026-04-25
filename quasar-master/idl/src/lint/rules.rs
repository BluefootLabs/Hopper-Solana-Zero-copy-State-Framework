//! Lint rules L001-L007.

use {
    super::{
        constraints::FieldClass,
        graph::{AccountGraph, EdgeKind},
        types::{Diagnostic, LintRule, Severity},
    },
    std::collections::{HashSet, VecDeque},
};

/// Run all lint rules against the given account graph.
pub fn run_all(graph: &AccountGraph, diagnostics: &mut Vec<Diagnostic>) {
    l001_island_detection(graph, diagnostics);
    l002_disconnected_subgraph(graph, diagnostics);
    l003_missing_has_one(graph, diagnostics);
    l004_unvalidated_token_mint(graph, diagnostics);
    l005_unvalidated_token_authority(graph, diagnostics);
    l006_writable_without_authority(graph, diagnostics);
    l007_unchecked_account(graph, diagnostics);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_suppressed(suppressions: &[String], rule: LintRule) -> bool {
    suppressions.iter().any(|s| s == rule.suppression_attr())
}

/// BFS through edges (both directions) checking if any reachable node is a
/// Signer.
fn has_path_to_signer(graph: &AccountGraph, start_name: &str) -> bool {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    visited.insert(start_name.to_string());
    queue.push_back(start_name.to_string());

    while let Some(current) = queue.pop_front() {
        // Check if this node is a Signer
        if let Some(node) = graph.nodes.iter().find(|n| n.name == current) {
            if node.field_class == FieldClass::Signer {
                return true;
            }
        }

        // Traverse edges in both directions
        for edge in &graph.edges {
            let neighbor = if edge.from == current {
                &edge.to
            } else if edge.to == current {
                &edge.from
            } else {
                continue;
            };

            if !visited.contains(neighbor.as_str()) {
                visited.insert(neighbor.clone());
                queue.push_back(neighbor.clone());
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// L001 — Island Detection
// ---------------------------------------------------------------------------

/// Account with degree 0, not self-constrained, no `has_address`, no
/// `has_constraint`, not (init + payer), and not suppressed.
fn l001_island_detection(graph: &AccountGraph, diagnostics: &mut Vec<Diagnostic>) {
    for node in &graph.nodes {
        if node.field_class.is_self_constrained() {
            continue;
        }
        if is_suppressed(&node.constraints.suppressions, LintRule::L001) {
            continue;
        }
        if node.constraints.has_address || node.constraints.has_constraint {
            continue;
        }
        if node.constraints.is_init && node.constraints.payer.is_some() {
            continue;
        }
        if graph.node_degree(&node.name) > 0 {
            continue;
        }

        diagnostics.push(Diagnostic {
            rule: LintRule::L001,
            severity: LintRule::L001.default_severity(),
            accounts_struct: graph.struct_name.clone(),
            field: Some(node.name.clone()),
            message: format!(
                "account `{}` has no relationship edges and no self-constraint",
                node.name
            ),
            suggestion: Some(
                "add a `has_one`, `seeds`, `address`, or `constraint` directive".to_string(),
            ),
        });
    }
}

// ---------------------------------------------------------------------------
// L002 — Disconnected Subgraph
// ---------------------------------------------------------------------------

/// `connected_components()` returns 2+ components.  Struct-level suppression
/// (any node has the suppression).
fn l002_disconnected_subgraph(graph: &AccountGraph, diagnostics: &mut Vec<Diagnostic>) {
    let components = graph.connected_components();
    if components.len() < 2 {
        return;
    }

    // Struct-level suppression: any node having the suppression silences this
    let suppressed = graph
        .nodes
        .iter()
        .any(|n| is_suppressed(&n.constraints.suppressions, LintRule::L002));
    if suppressed {
        return;
    }

    let component_labels: Vec<String> = components
        .iter()
        .map(|c| format!("{{{}}}", c.join(", ")))
        .collect();

    diagnostics.push(Diagnostic {
        rule: LintRule::L002,
        severity: LintRule::L002.default_severity(),
        accounts_struct: graph.struct_name.clone(),
        field: None,
        message: format!(
            "accounts struct has {} disconnected subgraphs: {}",
            components.len(),
            component_labels.join(", ")
        ),
        suggestion: Some("add constraints to connect all account clusters".to_string()),
    });
}

// ---------------------------------------------------------------------------
// L003 — Missing has_one
// ---------------------------------------------------------------------------

/// Iterate `graph.missing_edges` where `expected_kind == EdgeKind::HasOne`.
/// Check suppression on the source node.
fn l003_missing_has_one(graph: &AccountGraph, diagnostics: &mut Vec<Diagnostic>) {
    for missing in &graph.missing_edges {
        if missing.expected_kind != EdgeKind::HasOne {
            continue;
        }

        let source_node = match graph.nodes.iter().find(|n| n.name == missing.from) {
            Some(n) => n,
            None => continue,
        };

        if is_suppressed(&source_node.constraints.suppressions, LintRule::L003) {
            continue;
        }

        // Also skip if the source node's PDA seeds already reference the target
        if source_node
            .constraints
            .seeds_account_refs
            .iter()
            .any(|r| r == &missing.to)
        {
            continue;
        }

        diagnostics.push(Diagnostic {
            rule: LintRule::L003,
            severity: LintRule::L003.default_severity(),
            accounts_struct: graph.struct_name.clone(),
            field: Some(missing.from.clone()),
            message: format!(
                "`{}` has address field `{}` but no `has_one = {}` constraint",
                missing.from, missing.address_field, missing.to
            ),
            suggestion: Some(format!("add `has_one = {}`", missing.to)),
        });
    }
}

// ---------------------------------------------------------------------------
// L004 — Unvalidated Token Mint
// ---------------------------------------------------------------------------

/// Iterate `graph.missing_edges` where `expected_kind == EdgeKind::TokenMint`.
/// Check suppression on source node.
fn l004_unvalidated_token_mint(graph: &AccountGraph, diagnostics: &mut Vec<Diagnostic>) {
    for missing in &graph.missing_edges {
        if missing.expected_kind != EdgeKind::TokenMint {
            continue;
        }

        let source_node = match graph.nodes.iter().find(|n| n.name == missing.from) {
            Some(n) => n,
            None => continue,
        };

        if is_suppressed(&source_node.constraints.suppressions, LintRule::L004) {
            continue;
        }

        diagnostics.push(Diagnostic {
            rule: LintRule::L004,
            severity: LintRule::L004.default_severity(),
            accounts_struct: graph.struct_name.clone(),
            field: Some(missing.from.clone()),
            message: format!(
                "token account `{}` has no `token::mint` constraint",
                missing.from
            ),
            suggestion: Some(format!("add `token::mint = {}`", missing.to)),
        });
    }
}

// ---------------------------------------------------------------------------
// L005 — Unvalidated Token Authority
// ---------------------------------------------------------------------------

/// Iterate `graph.missing_edges` where `expected_kind ==
/// EdgeKind::TokenAuthority`.  Severity is Error if the node is writable,
/// Warning otherwise.  Check suppression.
fn l005_unvalidated_token_authority(graph: &AccountGraph, diagnostics: &mut Vec<Diagnostic>) {
    for missing in &graph.missing_edges {
        if missing.expected_kind != EdgeKind::TokenAuthority {
            continue;
        }

        let source_node = match graph.nodes.iter().find(|n| n.name == missing.from) {
            Some(n) => n,
            None => continue,
        };

        if is_suppressed(&source_node.constraints.suppressions, LintRule::L005) {
            continue;
        }

        let severity = if source_node.writable {
            Severity::Error
        } else {
            Severity::Warning
        };

        diagnostics.push(Diagnostic {
            rule: LintRule::L005,
            severity,
            accounts_struct: graph.struct_name.clone(),
            field: Some(missing.from.clone()),
            message: format!(
                "token account `{}` has no `token::authority` constraint",
                missing.from
            ),
            suggestion: Some(format!("add `token::authority = {}`", missing.to)),
        });
    }
}

// ---------------------------------------------------------------------------
// L006 — Writable Without Authority
// ---------------------------------------------------------------------------

/// Mutable non-self-constrained non-SystemAccount node with no `has_address`,
/// no `has_constraint`, and no path to a Signer through edges (BFS).
/// Check suppression.
fn l006_writable_without_authority(graph: &AccountGraph, diagnostics: &mut Vec<Diagnostic>) {
    for node in &graph.nodes {
        if !node.writable {
            continue;
        }
        if node.field_class.is_self_constrained() {
            continue;
        }
        if matches!(node.field_class, FieldClass::SystemAccount) {
            continue;
        }
        if node.constraints.has_address || node.constraints.has_constraint {
            continue;
        }
        if is_suppressed(&node.constraints.suppressions, LintRule::L006) {
            continue;
        }
        if has_path_to_signer(graph, &node.name) {
            continue;
        }

        diagnostics.push(Diagnostic {
            rule: LintRule::L006,
            severity: LintRule::L006.default_severity(),
            accounts_struct: graph.struct_name.clone(),
            field: Some(node.name.clone()),
            message: format!("writable account `{}` has no path to a signer", node.name),
            suggestion: Some(
                "add an authority constraint or connect through has_one to a signer".to_string(),
            ),
        });
    }
}

// ---------------------------------------------------------------------------
// L007 — Unchecked Account
// ---------------------------------------------------------------------------

/// FieldClass::Unchecked with no `has_address`, no `has_constraint`, and
/// empty `seeds_account_refs`.  Check suppression.
fn l007_unchecked_account(graph: &AccountGraph, diagnostics: &mut Vec<Diagnostic>) {
    for node in &graph.nodes {
        if node.field_class != FieldClass::Unchecked {
            continue;
        }
        if node.constraints.has_address || node.constraints.has_constraint {
            continue;
        }
        if !node.constraints.seeds_account_refs.is_empty() {
            continue;
        }
        if is_suppressed(&node.constraints.suppressions, LintRule::L007) {
            continue;
        }
        // Skip if another account's constraint validates this one (inbound edge)
        if graph.node_degree(&node.name) > 0 {
            continue;
        }

        diagnostics.push(Diagnostic {
            rule: LintRule::L007,
            severity: LintRule::L007.default_severity(),
            accounts_struct: graph.struct_name.clone(),
            field: Some(node.name.clone()),
            message: format!(
                "unchecked account `{}` has no validation constraints",
                node.name
            ),
            suggestion: Some("add an `address`, `constraint`, or `seeds` directive".to_string()),
        });
    }
}
