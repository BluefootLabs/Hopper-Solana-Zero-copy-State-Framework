//! Auto-fix for missing account constraints.

use super::graph::{AccountGraph, EdgeKind};

/// A suggested fix: insert `directive` into the `#[account(...)]` attribute
/// on `field_name`.
pub struct Fix {
    pub field_name: String,
    pub directive: String,
}

/// Generate fixes from the missing edges in an [`AccountGraph`].
///
/// `PdaSeed` and `Payer` edges are skipped because they are not auto-fixable
/// (they require seeds arrays or init configuration).
pub fn generate_fixes(graph: &AccountGraph) -> Vec<Fix> {
    graph
        .missing_edges
        .iter()
        .filter_map(|edge| {
            let directive = match &edge.expected_kind {
                EdgeKind::HasOne => format!("has_one = {}", edge.to),
                EdgeKind::TokenMint => format!("token::mint = {}", edge.to),
                EdgeKind::TokenAuthority => format!("token::authority = {}", edge.to),
                EdgeKind::AssociatedTokenMint => format!("associated_token::mint = {}", edge.to),
                EdgeKind::AssociatedTokenAuthority => {
                    format!("associated_token::authority = {}", edge.to)
                }
                EdgeKind::PdaSeed | EdgeKind::Payer => return None,
            };
            Some(Fix {
                field_name: edge.from.clone(),
                directive,
            })
        })
        .collect()
}

/// Apply fixes to Rust source text, inserting directives into existing
/// `#[account(...)]` attributes.
///
/// Fixes are applied in reverse positional order so that byte offsets remain
/// valid across insertions.  If a field does not have an `#[account(...)]`
/// attribute the fix is silently skipped.
pub fn apply_fixes(source: &str, fixes: &[Fix]) -> String {
    // Resolve each fix to a byte-offset insertion point.
    let mut insertions: Vec<(usize, &str)> = Vec::new();

    for fix in fixes {
        // Find `pub {field_name} :` or `pub {field_name}:` in source.
        let field_pos = find_field(source, &fix.field_name);
        let field_pos = match field_pos {
            Some(p) => p,
            None => continue,
        };

        // Walk backwards to find `#[account(`.
        let attr_open = find_account_attr_before(source, field_pos);
        let attr_open = match attr_open {
            Some(p) => p, // points at the `(` in `#[account(`
            None => continue,
        };

        // Find the matching `)` for that `(`.
        let close = find_matching_close(source, attr_open);
        let close = match close {
            Some(p) => p,
            None => continue,
        };

        insertions.push((close, &fix.directive));
    }

    // Sort descending by position so we can insert without invalidating
    // earlier offsets.
    insertions.sort_by_key(|b| std::cmp::Reverse(b.0));

    let mut result = source.to_string();
    for (pos, directive) in insertions {
        let insert = format!(", {}", directive);
        result.insert_str(pos, &insert);
    }

    result
}

/// Find byte offset of `pub {name} :` or `pub {name}:` in source.
fn find_field(source: &str, name: &str) -> Option<usize> {
    // Try both patterns: with and without space before colon.
    let patterns = [format!("pub {} :", name), format!("pub {}:", name)];
    for pat in &patterns {
        if let Some(pos) = source.find(pat) {
            return Some(pos);
        }
    }
    None
}

/// Walk backwards from `before` looking for `#[account(` and return the byte
/// offset of the `(`.
fn find_account_attr_before(source: &str, before: usize) -> Option<usize> {
    let prefix = &source[..before];
    let needle = "#[account(";
    let attr_start = prefix.rfind(needle)?;
    // Return position of the `(` which is at attr_start + "#[account".len()
    Some(attr_start + needle.len() - 1)
}

/// Find the matching `)` for a `(` at `open_pos`, handling nesting.
fn find_matching_close(source: &str, open_pos: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open_pos).copied() != Some(b'(') {
        return None;
    }
    let mut depth = 1u32;
    let mut i = open_pos + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
