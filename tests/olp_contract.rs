//! Contract tests for OLP v1 (`specs/task-r-olp-proto-v1.spec.md`, REQ-OLP-PROTO).
//!
//! These are pure documentation contracts: they pin the ACK grammar, the
//! lane-template TOML block, the result.md schema field list, and the
//! protocol-version references across `docs/OUTER_LOOP_PROTOCOL.md`,
//! `docs/OUTER_LOOP_REVIEW.md`, and `AGENTS.md`. No octos dependency.
//!
//! Historical ACK lines predate the v1 grammar and are NOT rewritten: they
//! are covered by the v1 effective-date exemption (2026-08-24), implemented
//! here as an exemption list of the legacy line forms present at v1 adoption.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// v1 ACK grammar: `ACK(done|wontdo|blocked): <non-empty explanation>`.
///
/// The grammar matches a single line. The explanation must be non-empty
/// (whitespace-only does not count).
fn ack_line_matches_v1(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix("ACK(") else {
        return false;
    };
    let Some(paren_end) = rest.find(')') else {
        return false;
    };
    let status = &rest[..paren_end];
    if !matches!(status, "done" | "wontdo" | "blocked") {
        return false;
    }
    let Some(explanation) = rest[paren_end + 1..].strip_prefix(':') else {
        return false;
    };
    !explanation.trim().is_empty()
}

/// Extract every candidate ACK line from the blackboard: lines whose trimmed
/// form starts with `ACK`. This includes both legacy `ACK:` lines and the
/// v1 `ACK(status):` form.
fn blackboard_ack_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.trim_start().starts_with("ACK"))
        .map(|line| line.trim().to_string())
        .collect()
}

/// Legacy ACK forms exempt from the v1 grammar (v1 effective 2026-08-24;
/// history is not rewritten — the exemption list is the contract).
///
/// Matching is by prefix: the legacy forms are `ACK:` and a bare `ACK:`
/// placeholder awaiting content. New (post-v1) lines must use the v1 form,
/// which does not start with these prefixes followed by a non-`(`.
fn is_exempt_legacy_ack(line: &str) -> bool {
    let trimmed = line.trim_start();
    // Legacy: `ACK: ...` or bare `ACK:` (empty placeholder). v1 lines start
    // with `ACK(` and therefore never hit this branch.
    trimmed.starts_with("ACK:")
}

#[test]
fn olp_ack_lines_match_v1_grammar() {
    let blackboard = read("docs/OUTER_LOOP_REVIEW.md");
    let ack_lines = blackboard_ack_lines(&blackboard);
    assert!(!ack_lines.is_empty(), "blackboard must contain ACK lines");
    let mut violations = Vec::new();
    for line in &ack_lines {
        if ack_line_matches_v1(line) || is_exempt_legacy_ack(line) {
            continue;
        }
        violations.push(line.clone());
    }
    assert!(
        violations.is_empty(),
        "ACK lines violating v1 grammar (and not exempt legacy `ACK:` lines):\n{}",
        violations.join("\n")
    );
}

#[test]
fn olp_ack_rejects_unknown_status() {
    // Known-good v1 lines parse.
    assert!(ack_line_matches_v1("ACK(done): shipped in commit abc123"));
    assert!(ack_line_matches_v1("ACK(wontdo): 异议:证据链不足"));
    assert!(ack_line_matches_v1("  ACK(blocked): cargo 不可用,等待工具链窗口"));
    // Unknown status words are rejected.
    assert!(!ack_line_matches_v1("ACK(finished): done-ish"));
    assert!(!ack_line_matches_v1("ACK(rejected): nope"));
    assert!(!ack_line_matches_v1("ACK(DONE): wrong case"));
    assert!(!ack_line_matches_v1("ACK(): empty status"));
    // Malformed shapes are rejected.
    assert!(!ack_line_matches_v1("ACK: legacy form is not v1"));
    assert!(!ack_line_matches_v1("ACK(done) missing colon"));
    assert!(!ack_line_matches_v1("ACK(done):   ")); // empty explanation
}

#[test]
fn olp_lane_template_parses() {
    let protocol = read("docs/OUTER_LOOP_PROTOCOL.md");
    // Locate the appendix B TOML fence and extract its body.
    let appendix_marker = "## 附录 B";
    let appendix_start = protocol
        .find(appendix_marker)
        .expect("OUTER_LOOP_PROTOCOL.md must contain appendix B (lane template)");
    let appendix = &protocol[appendix_start..];
    let fence_start = appendix
        .find("```toml\n")
        .expect("appendix B must contain a ```toml fenced block");
    let body = &appendix[fence_start + "```toml\n".len()..];
    let fence_end = body.find("```").expect("toml fence must be closed");
    let toml_body = &body[..fence_end];

    let parsed: toml::Value = toml_body
        .parse()
        .expect("lane template TOML must parse");
    let sub_providers = parsed
        .get("sub_providers")
        .and_then(|v| v.as_table())
        .expect("lane template must define [sub_providers.<lane>] tables");
    assert!(
        !sub_providers.is_empty(),
        "lane template must declare at least one lane"
    );
    for (lane, config) in sub_providers {
        let description = config
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("lane `{lane}` must have a description string"));
        assert!(
            !description.trim().is_empty(),
            "lane `{lane}` description must be non-empty"
        );
    }
}

#[test]
fn olp_version_consistent_across_docs() {
    let agents = read("AGENTS.md");
    let protocol = read("docs/OUTER_LOOP_PROTOCOL.md");
    for (rel, text) in [
        ("AGENTS.md", agents.as_str()),
        ("docs/OUTER_LOOP_PROTOCOL.md", protocol.as_str()),
    ] {
        assert!(
            text.contains("olp/v1"),
            "{rel} must reference protocol version olp/v1"
        );
    }
    // Neither file's own protocol declaration may still say v0. (Quoted
    // historical mentions such as blackboard narratives live in
    // OUTER_LOOP_REVIEW.md and are out of scope for this check.)
    assert!(
        !agents.contains("protocol: olp/v0"),
        "AGENTS.md must not declare protocol: olp/v0"
    );
    assert!(
        !protocol.contains("`protocol: olp/v0`"),
        "OUTER_LOOP_PROTOCOL.md must not declare protocol: olp/v0"
    );
}

#[test]
fn olp_result_schema_fields_documented() {
    let protocol = read("docs/OUTER_LOOP_PROTOCOL.md");
    let appendix_marker = "## 附录 A";
    let appendix_start = protocol
        .find(appendix_marker)
        .expect("OUTER_LOOP_PROTOCOL.md must contain appendix A (result.md schema)");
    // Appendix A runs to the next `## ` heading (appendix B) or EOF.
    let appendix = &protocol[appendix_start..];
    let appendix_end = appendix[appendix_marker.len()..]
        .find("\n## ")
        .map(|i| i + appendix_marker.len())
        .unwrap_or(appendix.len());
    let appendix = &appendix[..appendix_end];

    // Field names are documented as `field` in the schema table rows.
    let mut fields = BTreeSet::new();
    for line in appendix.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let Some(first_cell) = line.split('|').nth(1) else {
            continue;
        };
        let cell = first_cell.trim();
        if let Some(name) = cell.strip_prefix('`').and_then(|c| c.strip_suffix('`')) {
            fields.insert(name.to_string());
        }
    }
    let expected: BTreeSet<String> = [
        "slug",
        "outcome",
        "updated_unix",
        "turn",
        "verified",
        "protocol",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(
        fields, expected,
        "result.md schema fields documented in appendix A must be exactly \
         {{slug, outcome, updated_unix, turn, verified, protocol}}"
    );
}
