//! Account relationship graph.
//!
//! Builds a directed graph of relationships between accounts in a
//! `#[derive(Accounts)]` struct.  Edges come from explicit constraints
//! (`has_one`, `token::mint`, PDA seeds, etc.) and missing edges are
//! inferred from the type registry so downstream rules can flag them.

use {
    super::types::TypeRegistry,
    crate::{
        lint::constraints::{FieldClass, FieldConstraints},
        parser::accounts::RawAccountsStruct,
    },
    std::collections::{HashSet, VecDeque},
};

// ---------------------------------------------------------------------------
// Edge types
// ---------------------------------------------------------------------------

/// The kind of relationship between two accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeKind {
    HasOne,
    TokenMint,
    TokenAuthority,
    AssociatedTokenMint,
    AssociatedTokenAuthority,
    PdaSeed,
    Payer,
}

impl EdgeKind {
    /// Human-readable label used in diagnostics and graph output.
    pub fn label(&self) -> &'static str {
        match self {
            Self::HasOne => "has_one",
            Self::TokenMint => "token::mint",
            Self::TokenAuthority => "token::authority",
            Self::AssociatedTokenMint => "associated_token::mint",
            Self::AssociatedTokenAuthority => "associated_token::authority",
            Self::PdaSeed => "pda_seed",
            Self::Payer => "payer",
        }
    }
}

/// A directed edge in the account graph.
#[derive(Debug, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A node representing a single account field.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub field_class: FieldClass,
    pub inner_type_name: Option<String>,
    pub constraints: FieldConstraints,
    pub writable: bool,
}

// ---------------------------------------------------------------------------
// Missing edge
// ---------------------------------------------------------------------------

/// An edge that *should* exist according to the type registry but is absent
/// from the explicit constraints.
#[derive(Debug, Clone)]
pub struct MissingEdge {
    pub from: String,
    pub to: String,
    pub expected_kind: EdgeKind,
    /// The Address field on the inner type that motivated this expected edge.
    pub address_field: String,
}

// ---------------------------------------------------------------------------
// AccountGraph
// ---------------------------------------------------------------------------

/// The full account relationship graph for a single `#[derive(Accounts)]`
/// struct.
pub struct AccountGraph {
    pub struct_name: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub missing_edges: Vec<MissingEdge>,
}

impl AccountGraph {
    /// Build the graph from a parsed accounts struct and type registry.
    pub fn build(accounts: &RawAccountsStruct, registry: &TypeRegistry) -> Self {
        let nodes: Vec<Node> = accounts
            .fields
            .iter()
            .map(|f| Node {
                name: f.name.clone(),
                field_class: f.field_class.clone(),
                inner_type_name: f.inner_type_name.clone(),
                constraints: f.constraints.clone(),
                writable: f.writable,
            })
            .collect();

        let field_names: HashSet<&str> = nodes.iter().map(|n| n.name.as_str()).collect();

        // ---- Collect explicit edges ----
        let mut edges = Vec::new();

        for node in &nodes {
            let c = &node.constraints;

            // has_one edges
            for target in &c.has_ones {
                if field_names.contains(target.as_str()) {
                    edges.push(Edge {
                        from: node.name.clone(),
                        to: target.clone(),
                        kind: EdgeKind::HasOne,
                    });
                }
            }

            // token::mint
            if let Some(ref target) = c.token_mint {
                if field_names.contains(target.as_str()) {
                    edges.push(Edge {
                        from: node.name.clone(),
                        to: target.clone(),
                        kind: EdgeKind::TokenMint,
                    });
                }
            }

            // token::authority
            if let Some(ref target) = c.token_authority {
                if field_names.contains(target.as_str()) {
                    edges.push(Edge {
                        from: node.name.clone(),
                        to: target.clone(),
                        kind: EdgeKind::TokenAuthority,
                    });
                }
            }

            // associated_token::mint
            if let Some(ref target) = c.associated_token_mint {
                if field_names.contains(target.as_str()) {
                    edges.push(Edge {
                        from: node.name.clone(),
                        to: target.clone(),
                        kind: EdgeKind::AssociatedTokenMint,
                    });
                }
            }

            // associated_token::authority
            if let Some(ref target) = c.associated_token_authority {
                if field_names.contains(target.as_str()) {
                    edges.push(Edge {
                        from: node.name.clone(),
                        to: target.clone(),
                        kind: EdgeKind::AssociatedTokenAuthority,
                    });
                }
            }

            // PDA seed refs
            for target in &c.seeds_account_refs {
                if field_names.contains(target.as_str()) {
                    edges.push(Edge {
                        from: node.name.clone(),
                        to: target.clone(),
                        kind: EdgeKind::PdaSeed,
                    });
                }
            }

            // payer
            if let Some(ref target) = c.payer {
                if field_names.contains(target.as_str()) {
                    edges.push(Edge {
                        from: node.name.clone(),
                        to: target.clone(),
                        kind: EdgeKind::Payer,
                    });
                }
            }
        }

        // ---- Detect missing edges ----
        let missing_edges = detect_missing_edges(&nodes, &edges, registry);

        Self {
            struct_name: accounts.name.clone(),
            nodes,
            edges,
            missing_edges,
        }
    }

    /// Returns true if an explicit edge exists from `from` to `to`.
    pub fn has_edge(&self, from: &str, to: &str) -> bool {
        self.edges.iter().any(|e| e.from == from && e.to == to)
    }

    /// Total number of edges that should exist (explicit + missing).
    pub fn expected_edge_count(&self) -> usize {
        self.edges.len() + self.missing_edges.len()
    }

    /// Number of edges that are explicitly constrained.
    pub fn constrained_edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Compute connected components via BFS on an undirected view of the
    /// graph.  Self-constrained nodes (Signer, Program, Sysvar) are excluded
    /// from the component analysis since they don't need relationship edges.
    pub fn connected_components(&self) -> Vec<Vec<String>> {
        let non_self: Vec<&str> = self
            .nodes
            .iter()
            .filter(|n| !n.field_class.is_self_constrained())
            .map(|n| n.name.as_str())
            .collect();

        if non_self.is_empty() {
            return Vec::new();
        }

        // Build adjacency list (undirected) among non-self-constrained nodes
        let node_set: HashSet<&str> = non_self.iter().copied().collect();
        let mut adj: std::collections::HashMap<&str, Vec<&str>> =
            non_self.iter().map(|&n| (n, Vec::new())).collect();

        for edge in &self.edges {
            let from = edge.from.as_str();
            let to = edge.to.as_str();
            if node_set.contains(from) && node_set.contains(to) {
                adj.get_mut(from).unwrap().push(to);
                adj.get_mut(to).unwrap().push(from);
            }
        }

        let mut visited: HashSet<&str> = HashSet::new();
        let mut components = Vec::new();

        for &start in &non_self {
            if visited.contains(start) {
                continue;
            }

            let mut component = Vec::new();
            let mut queue = VecDeque::new();
            queue.push_back(start);
            visited.insert(start);

            while let Some(current) = queue.pop_front() {
                component.push(current.to_string());
                for &neighbor in &adj[current] {
                    if !visited.contains(neighbor) {
                        visited.insert(neighbor);
                        queue.push_back(neighbor);
                    }
                }
            }

            components.push(component);
        }

        components
    }

    /// Count the number of edges touching a given node (in either direction).
    pub fn node_degree(&self, name: &str) -> usize {
        self.edges
            .iter()
            .filter(|e| e.from == name || e.to == name)
            .count()
    }
}

// ---------------------------------------------------------------------------
// Missing edge detection
// ---------------------------------------------------------------------------

/// Returns true if `node_name` is already connected to `target_name` via an
/// explicit edge that covers the relationship (has_one, seed ref, or any
/// other kind).
fn is_covered(edges: &[Edge], node_name: &str, target_name: &str) -> bool {
    edges
        .iter()
        .any(|e| e.from == node_name && e.to == target_name)
}

fn detect_missing_edges(
    nodes: &[Node],
    edges: &[Edge],
    registry: &TypeRegistry,
) -> Vec<MissingEdge> {
    let mut missing = Vec::new();

    // Helper: check if a node with a given field class exists in the struct
    let has_class = |class_pred: fn(&FieldClass) -> bool| -> Option<String> {
        nodes
            .iter()
            .find(|n| class_pred(&n.field_class))
            .map(|n| n.name.clone())
    };

    for node in nodes {
        match &node.field_class {
            // For Account<T>: look up T's Address fields in the registry.
            // Each Address field suggests a relationship to another account in
            // the struct.  Skip init accounts — their Address fields are being
            // set, not verified.
            FieldClass::Account { inner_type } if !node.constraints.is_init => {
                let addr_fields = registry.get_address_fields(inner_type);
                for addr_field in &addr_fields {
                    // The Address field name should correspond to another
                    // account field in the struct.
                    let target_exists = nodes.iter().any(|n| n.name == *addr_field);
                    if target_exists && !is_covered(edges, &node.name, addr_field) {
                        missing.push(MissingEdge {
                            from: node.name.clone(),
                            to: addr_field.clone(),
                            expected_kind: EdgeKind::HasOne,
                            address_field: addr_field.clone(),
                        });
                    }
                }
            }

            // For TokenAccount: if no token::mint constraint and a Mint
            // account exists -> missing mint edge.  If writable and no
            // token::authority and a Signer exists -> missing authority edge.
            FieldClass::TokenAccount => {
                if node.constraints.token_mint.is_none()
                    && node.constraints.associated_token_mint.is_none()
                {
                    if let Some(mint_name) = has_class(|c| matches!(c, FieldClass::Mint)) {
                        if !is_covered(edges, &node.name, &mint_name) {
                            missing.push(MissingEdge {
                                from: node.name.clone(),
                                to: mint_name,
                                expected_kind: EdgeKind::TokenMint,
                                address_field: "mint".to_string(),
                            });
                        }
                    }
                }

                if node.writable
                    && node.constraints.token_authority.is_none()
                    && node.constraints.associated_token_authority.is_none()
                {
                    if let Some(signer_name) = has_class(|c| matches!(c, FieldClass::Signer)) {
                        if !is_covered(edges, &node.name, &signer_name) {
                            missing.push(MissingEdge {
                                from: node.name.clone(),
                                to: signer_name,
                                expected_kind: EdgeKind::TokenAuthority,
                                address_field: "owner".to_string(),
                            });
                        }
                    }
                }
            }

            _ => {}
        }
    }

    missing
}
