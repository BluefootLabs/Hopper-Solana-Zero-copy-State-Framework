//! The CPI frame tree of one processed instruction, parsed from the
//! runtime's program logs.
//!
//! The runtime logs `Program <id> invoke [<depth>]` when a frame starts,
//! `Program <id> consumed <n> of <budget> compute units` when it ends, and
//! `Program <id> success` or `Program <id> failed: <reason>` for its result;
//! everything between an invoke and its result at the next depth belongs to
//! that frame. [`parse_frames`] turns those lines into a tree with the
//! compute units each frame consumed, including what its own callees cost,
//! and the units the frame spent itself (`own_units`). Capture the logs with
//! [`crate::LiteSvmHarness::capture_logs`] before `process`.

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;

/// One program invocation in the frame tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The program that ran, as the log printed it.
    pub program: String,
    /// Nesting depth: 1 for the top-level instruction, 2 for its CPIs.
    pub depth: u32,
    /// Compute units the frame consumed, callees included, when the runtime
    /// logged them (a frame that aborts may have no `consumed` line).
    pub consumed: Option<u64>,
    /// The budget the `consumed` line reported.
    pub budget: Option<u64>,
    /// `Some(true)` on `success`, `Some(false)` on `failed`, `None` when the
    /// log ended inside the frame.
    pub success: Option<bool>,
    /// The failure text after `failed: `, when the frame failed.
    pub error: Option<String>,
    /// Program log lines (`Program log: ...`) emitted directly by this frame.
    pub logs: Vec<String>,
    /// Frames this frame invoked, in order.
    pub children: Vec<Frame>,
}

impl Frame {
    /// Compute units the frame spent itself: `consumed` minus what its
    /// direct callees consumed. `None` when any of those is unknown.
    pub fn own_units(&self) -> Option<u64> {
        let total = self.consumed?;
        let mut callees = 0u64;
        for child in &self.children {
            callees = callees.checked_add(child.consumed?)?;
        }
        total.checked_sub(callees)
    }

    /// Every frame in the subtree, pre-order.
    pub fn walk(&self) -> Vec<&Frame> {
        let mut out = Vec::new();
        self.push_walk(&mut out);
        out
    }

    fn push_walk<'a>(&'a self, out: &mut Vec<&'a Frame>) {
        out.push(self);
        for child in &self.children {
            child.push_walk(out);
        }
    }

    /// Number of frames in the subtree that ran `program`.
    pub fn invocations_of(&self, program: &str) -> usize {
        self.walk().iter().filter(|f| f.program == program).count()
    }

    fn write_json(&self, s: &mut String, indent: usize) {
        let pad = " ".repeat(indent);
        let _ = writeln!(s, "{pad}{{");
        let _ = writeln!(s, "{pad}  \"program\": \"{}\",", self.program);
        let _ = writeln!(s, "{pad}  \"depth\": {},", self.depth);
        let _ = writeln!(s, "{pad}  \"consumed\": {},", opt(self.consumed));
        let _ = writeln!(s, "{pad}  \"ownUnits\": {},", opt(self.own_units()));
        let _ = writeln!(
            s,
            "{pad}  \"success\": {},",
            match self.success {
                Some(true) => "true",
                Some(false) => "false",
                None => "null",
            }
        );
        let _ = writeln!(s, "{pad}  \"children\": [");
        for (i, child) in self.children.iter().enumerate() {
            child.write_json(s, indent + 4);
            let _ = writeln!(s, "{}", if i + 1 < self.children.len() { "," } else { "" });
        }
        let _ = writeln!(s, "{pad}  ]");
        let _ = write!(s, "{pad}}}");
    }

    /// The subtree as pretty-printed JSON.
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        self.write_json(&mut s, 0);
        s
    }
}

fn opt(value: Option<u64>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "null".to_string(),
    }
}

/// Parse the runtime's log lines into the top-level frames they describe
/// (one per top-level instruction in the log; a single `process` yields
/// one). Lines the parser does not recognize are ignored, so the input can
/// be the whole captured log.
pub fn parse_frames(logs: &[String]) -> Vec<Frame> {
    let mut roots: Vec<Frame> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    for line in logs {
        let Some(rest) = line.strip_prefix("Program ") else {
            continue;
        };
        if let Some(text) = rest.strip_prefix("log: ") {
            if let Some(top) = stack.last_mut() {
                top.logs.push(text.to_string());
            }
            continue;
        }
        if rest.starts_with("data: ") || rest.starts_with("return: ") {
            continue;
        }
        let Some((program, tail)) = rest.split_once(' ') else {
            continue;
        };
        if let Some(depth) = tail
            .strip_prefix("invoke [")
            .and_then(|d| d.strip_suffix(']'))
        {
            let depth = depth.parse().unwrap_or(stack.len() as u32 + 1);
            stack.push(Frame {
                program: program.to_string(),
                depth,
                consumed: None,
                budget: None,
                success: None,
                error: None,
                logs: Vec::new(),
                children: Vec::new(),
            });
            continue;
        }
        let Some(top) = stack.last_mut() else {
            continue;
        };
        if top.program != program {
            continue;
        }
        if let Some(units) = tail.strip_prefix("consumed ") {
            let mut parts = units.split(' ');
            top.consumed = parts.next().and_then(|n| n.parse().ok());
            top.budget = parts.nth(1).and_then(|n| n.parse().ok());
            continue;
        }
        let finished = if tail == "success" {
            top.success = Some(true);
            true
        } else if let Some(reason) = tail.strip_prefix("failed: ") {
            top.success = Some(false);
            top.error = Some(reason.to_string());
            true
        } else {
            false
        };
        if finished {
            let frame = stack.pop().expect("frame on the stack");
            match stack.last_mut() {
                Some(parent) => parent.children.push(frame),
                None => roots.push(frame),
            }
        }
    }
    // An aborted transaction can end the log inside a frame; keep what was
    // seen so the caller can still inspect the partial tree.
    while let Some(frame) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(frame),
            None => roots.push(frame),
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(|l| l.trim().to_string()).collect()
    }

    #[test]
    fn nested_frames_carry_their_own_units() {
        let logs = lines(
            "Program Lab invoke [1]
             Program log: hello
             Program 11111111111111111111111111111111 invoke [2]
             Program 11111111111111111111111111111111 success
             Program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA invoke [2]
             Program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA consumed 454 of 1398000 compute units
             Program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA success
             Program return: Lab AQID
             Program Lab consumed 2472 of 200000 compute units
             Program Lab success",
        );
        let frames = parse_frames(&logs);
        assert_eq!(frames.len(), 1);
        let root = &frames[0];
        assert_eq!(root.program, "Lab");
        assert_eq!(root.depth, 1);
        assert_eq!(root.consumed, Some(2472));
        assert_eq!(root.budget, Some(200000));
        assert_eq!(root.success, Some(true));
        assert_eq!(root.logs, vec!["hello".to_string()]);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].program, "11111111111111111111111111111111");
        assert_eq!(
            root.children[0].consumed, None,
            "builtins log no consumed line"
        );
        assert_eq!(root.children[1].consumed, Some(454));
        assert_eq!(root.own_units(), None, "unknown callee cost stays unknown");
        assert_eq!(root.children[1].own_units(), Some(454));
        assert_eq!(
            root.invocations_of("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
            1
        );
        assert!(root.to_json().contains("\"consumed\": 2472"));
    }

    #[test]
    fn a_failed_frame_keeps_its_reason_and_an_aborted_log_keeps_the_partial_tree() {
        let logs = lines(
            "Program Lab invoke [1]
             Program Tokenz invoke [2]
             Program Tokenz consumed 1417 of 1397999 compute units
             Program Tokenz failed: custom program error: 0xc
             Program Lab consumed 5575 of 200000 compute units
             Program Lab failed: custom program error: 0xc",
        );
        let frames = parse_frames(&logs);
        assert_eq!(frames[0].success, Some(false));
        assert_eq!(
            frames[0].children[0].error.as_deref(),
            Some("custom program error: 0xc")
        );
        assert_eq!(frames[0].own_units(), Some(5575 - 1417));

        let partial = parse_frames(&lines("Program Lab invoke [1]\nProgram Inner invoke [2]"));
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].children.len(), 1);
        assert_eq!(partial[0].success, None);
    }
}
