//! Evolution loop phase-2 replay + metrics contract tests (#43c, SDD
//! spec: `specs/task-req-olp-evo-p2.spec.md`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn harvest() -> PathBuf {
    repo_root().join("scripts/olp-evo-harvest.sh")
}

fn retro() -> PathBuf {
    repo_root().join("scripts/olp-evo-retro.sh")
}

fn replay_dir() -> PathBuf {
    repo_root().join("fixtures/evolution/replay")
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Sb {
    #[allow(dead_code)]
    root: PathBuf,
    repo: PathBuf,
    state_root: PathBuf,
}

impl Sb {
    fn new(tag: &str) -> Self {
        let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("olp-p2-{tag}-{}-{seq}", std::process::id()));
        let repo = root.join("repo");
        let state_root = root.join("state");
        std::fs::create_dir_all(repo.join(".octos")).unwrap();
        std::fs::create_dir_all(&state_root).unwrap();
        Self {
            root,
            repo,
            state_root,
        }
    }
}

impl Drop for Sb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Scenario: 回放夹具产出与期望一致
#[test]
fn olp_evo_replay_matches_expected() {
    let sb = Sb::new("replay");
    let fx = replay_dir();
    // review-board is the LIVE board; events/mcp copied beside the repo.
    std::fs::copy(
        fx.join("review-board.md"),
        sb.repo.join(".octos/OUTER_LOOP_REVIEW.md"),
    )
    .unwrap();
    let events = sb.root.join("events.jsonl");
    let mcp = sb.root.join("mcp-board.md");
    std::fs::copy(fx.join("events.jsonl"), &events).unwrap();
    std::fs::copy(fx.join("mcp-board.md"), &mcp).unwrap();
    let env = |cmd: &mut Command| {
        cmd.env("OLP_EVO_STATE", &sb.state_root)
            .env("OLP_EVO_EVENTS", &events)
            .env("OLP_EVO_MCP_BOARD", &mcp)
            .current_dir(&sb.repo);
    };
    let mut h = Command::new("bash");
    h.arg(harvest()).arg(&sb.repo);
    env(&mut h);
    let out = h.output().unwrap();
    assert!(
        out.status.success(),
        "harvest stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut r = Command::new("bash");
    r.arg(retro()).arg(&sb.repo);
    env(&mut r);
    let out = r.output().unwrap();
    assert!(
        out.status.success(),
        "retro stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let board_text = std::fs::read_to_string(sb.repo.join(".octos/EVOLUTION.md")).unwrap();
    let cards = board_text
        .lines()
        .filter(|l| l.starts_with("### EVO-"))
        .count();
    let mut by_trigger: std::collections::BTreeMap<String, usize> = Default::default();
    let mut by_source: std::collections::BTreeMap<String, usize> = Default::default();
    #[allow(unused_assignments)]
    let mut trigger = String::new();
    for line in board_text.lines() {
        if let Some(v) = line.strip_prefix("trigger: ") {
            trigger = v.to_string();
            *by_trigger.entry(trigger.clone()).or_default() += 1;
        }
        if let Some(v) = line.strip_prefix("source: ") {
            let s = v.split(' ').next().unwrap_or("?");
            *by_source.entry(s.to_string()).or_default() += 1;
        }
    }

    // retro candidates + recurrence from the brief
    let retro_json = find_state_file(&sb.state_root, "retro.json");
    let brief = latest_brief(&retro_json);
    let brief_text = std::fs::read_to_string(&brief).unwrap();
    let candidates = brief_text.lines().filter(|l| l.starts_with("## C")).count();

    let expected_text = std::fs::read_to_string(fx.join("expected.json")).unwrap();
    let expected: serde_json::Value = serde_json::from_str(&expected_text).unwrap();

    assert_eq!(cards, expected["cards"].as_u64().unwrap() as usize, "cards");
    for (k, v) in expected["by_trigger"].as_object().unwrap() {
        let got = by_trigger.get(k).copied().unwrap_or(0);
        assert_eq!(got, v.as_u64().unwrap() as usize, "by_trigger[{k}]");
    }
    for (k, v) in expected["by_source"].as_object().unwrap() {
        let got = by_source.get(k).copied().unwrap_or(0);
        assert_eq!(got, v.as_u64().unwrap() as usize, "by_source[{k}]");
    }
    assert_eq!(
        candidates,
        expected["candidates"].as_u64().unwrap() as usize,
        "candidates"
    );
    // per-candidate recurrence_hint (key → hint). Sections print the
    // `## C<k> … recurrence_hint=<n>` header BEFORE the `key:` line, so
    // attach the CURRENT section's hint to each key as it appears.
    let mut hints: std::collections::HashMap<String, usize> = Default::default();
    let mut cur_hint = 0usize;
    for line in brief_text.lines() {
        if line.starts_with("## C") {
            cur_hint = line
                .find("recurrence_hint=")
                .and_then(|pos| {
                    line[pos + 16..]
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse().ok())
                })
                .unwrap_or(0);
        }
        if let Some(v) = line.strip_prefix("key: ") {
            hints.insert(v.to_string(), cur_hint);
        }
    }
    for (key, want) in expected["recurrence"].as_object().unwrap() {
        let want = want.as_u64().unwrap() as usize;
        let got = hints.get(key.as_str()).copied().unwrap_or(0);
        assert_eq!(
            got,
            want,
            "recurrence for {key} (key in brief: {})",
            hints.contains_key(key.as_str())
        );
    }
}

/// Scenario: 回放夹具已脱敏
#[test]
fn olp_evo_replay_fixture_is_scrubbed() {
    for entry in std::fs::read_dir(replay_dir()).unwrap().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !text.contains("/home/alexzhang"),
            "{} leaks a home path",
            path.display()
        );
        // api_key as an identifier in technical prose is fine; a VALUE
        // (api_key=<non-redacted> / api_key: <literal>) is not.
        for (idx, _) in text.match_indices("api_key") {
            let rest = &text[idx + "api_key".len()..];
            let rest = rest.trim_start_matches("_env");
            if rest.starts_with("=") {
                assert!(
                    rest.starts_with("=[redacted]"),
                    "{} leaks api_key value at byte {idx}",
                    path.display()
                );
            }
        }
        assert!(!text.contains("token="), "{} leaks token=", path.display());
        // A real audit-line VALUE must be [redacted]; the README's own
        // scrub-notation (`question=`…改 [redacted]) quotes the token in
        // backticks — allow that documented self-reference.
        for (idx, _) in text.match_indices("question=") {
            let before = &text[..idx];
            if before.ends_with("`") {
                continue;
            }
            let rest = &text[idx + "question=".len()..];
            assert!(
                rest.starts_with("[redacted]"),
                "{} has non-redacted question= at byte {idx}",
                path.display()
            );
        }
    }
}

fn find_state_file(state_root: &Path, name: &str) -> PathBuf {
    let mut out = PathBuf::new();
    fn walk(dir: &Path, name: &str, out: &mut PathBuf) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, name, out);
            } else if p.file_name().map(|n| n == name).unwrap_or(false)
                && out.as_os_str().is_empty()
            {
                *out = p;
            }
        }
    }
    walk(state_root, name, &mut out);
    out
}

fn latest_brief(retro_json: &Path) -> PathBuf {
    let text = std::fs::read_to_string(retro_json).unwrap_or_default();
    let mut best = PathBuf::new();
    if let Some(pos) = text.find("\"brief\": \"") {
        let rest = &text[pos + 10..];
        if let Some(end) = rest.find('"') {
            best = PathBuf::from(&rest[..end]);
        }
    }
    best
}
