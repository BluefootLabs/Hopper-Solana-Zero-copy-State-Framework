//! Process-protocol fixture for `hopper fuzz run` integration tests.
//!
//! This deliberately does not execute a program. It proves that the CLI sends
//! the generated request to a separate process and enforces that process's
//! response. Production adapters must replace the echo behavior with SVM or
//! program-test execution and real invariant checks.

use std::env;
use std::io::{self, Read};

fn main() {
    let mut request = String::new();
    io::stdin().read_to_string(&mut request).unwrap();

    let commitment = extract_string(&request, "contractCommitment");
    let id = extract_string(&request, "id");
    let mut invariants = extract_string_array(&request, "requiredInvariants");
    if env::args().any(|arg| arg == "--omit-invariant") {
        invariants.pop();
    }
    let checked = invariants
        .iter()
        .map(|invariant| format!("\"{invariant}\""))
        .collect::<Vec<_>>()
        .join(",");

    println!(
        "{{\"schema\":\"hopper.manifest-fuzz-response.v1\",\"contractCommitment\":\"{commitment}\",\"results\":[{{\"id\":\"{id}\",\"outcome\":\"passed\",\"checkedInvariants\":[{checked}],\"detail\":\"fixture transport only\"}}]}}"
    );
}

fn extract_string(input: &str, key: &str) -> String {
    let marker = format!("\"{key}\":\"");
    let start = input.find(&marker).unwrap() + marker.len();
    let tail = &input[start..];
    let end = tail.find('"').unwrap();
    tail[..end].to_string()
}

fn extract_string_array(input: &str, key: &str) -> Vec<String> {
    let marker = format!("\"{key}\":[");
    let start = input.find(&marker).unwrap() + marker.len();
    let tail = &input[start..];
    let end = tail.find(']').unwrap();
    let body = &tail[..end];
    if body.is_empty() {
        return Vec::new();
    }
    body.split(',')
        .map(|value| value.trim_matches('"').to_string())
        .collect()
}
