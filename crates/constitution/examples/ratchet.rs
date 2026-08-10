//! What the duplication gate would say right now, as paste-ready waivers.
//!
//! ```text
//! cargo run -p constitution --example ratchet
//! ```
//!
//! Prints every *unwaived* duplication finding as a `[[waiver]]` skeleton for
//! `crates/constitution/waivers.toml`, and every problem with the waiver file
//! itself as a comment. Silence means the gate is green. This is the tool for
//! whoever the gate just fired on: fix the duplication, or paste the entry
//! and write the justification it deliberately arrives without.

use constitution::corpus::{Corpus, workspace_root};
use constitution::finding::Rule;
use constitution::{duplication, waiver};

/// Where the waiver file lives, relative to the workspace root.
const WAIVER_FILE: &str = "crates/constitution/waivers.toml";

fn main() {
    let root = workspace_root();
    let corpus = Corpus::walk(&root).expect("the workspace should be readable");
    let sources = duplication::scope(&corpus);

    let mut findings = duplication::literals(&sources, duplication::LITERAL_FLOOR);
    findings.extend(duplication::runs(&sources, duplication::RUN_LINES));

    let waiver_text = std::fs::read_to_string(root.join(WAIVER_FILE)).unwrap_or_default();
    let waivers = waiver::parse(&waiver_text).expect("the waiver file should parse");

    let remaining = waiver::apply(&findings, &waivers, WAIVER_FILE);
    for finding in &remaining {
        if finding.rule == Rule::Waiver {
            println!("# {WAIVER_FILE}: {} — {}", finding.what, finding.why);
            println!();
            continue;
        }
        println!("# {finding}");
        println!("[[waiver]]");
        println!("key = {}", toml_string(&finding.key()));
        println!("justification = \"\"");
        println!();
    }
}

/// One string, escaped the way the waiver file will need it.
fn toml_string(text: &str) -> String {
    toml::Value::String(text.to_string()).to_string()
}
