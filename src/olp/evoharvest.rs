//! `octoscode olp evo harvest <repo> [--dry-run]` — the evolution loop's
//! harvest: collect friction signals from three sources and append one
//! identity-stable symptom card for each to the evolution board. It never
//! writes the review board.
//!
//! Sources: the review board (blocked / wontdo ACKs, overrides, R2
//! records, returns, unit alerts, and a unit's forwarded MCP audit lines),
//! the events stream (`OLP_EVO_EVENTS`), and the MCP audit board. Only
//! newline-terminated lines count. A signal already on the evolution board
//! (by its identity) is not carded again.
//!
//! State lives outside the repository, under
//! `${OLP_EVO_STATE:-~/.octos/outer/evo}/<project key>/state.json`. A dry
//! run prints the pending cards and creates nothing. On a multi-machine
//! deployment only the hub harvests.
//!
//! Exit codes: 0; 1 error; 2 repo or review board missing; 3 not the hub;
//! 70 an injected fault (tests).

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use fancy_regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::evolib::{Json, hub_guard};

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a pattern written in this file")
}

const WS: &str = r"[ \t\n\r\x0b\x0c]";
const NWS: &str = r"[^ \t\n\r\x0b\x0c]";

static HEADING: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^###{WS}+([0-9]+)")));
static OVERRIDE: LazyLock<Regex> = LazyLock::new(|| re(r"^外环\([^)]+\)·改判\("));
static R2: LazyLock<Regex> = LazyLock::new(|| re(r"^外环\([^)]+\)·R2 记档\("));
static INNER: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^内环\([^)]+\)·(#([0-9]+){WS}*)?ACK\((blocked|wontdo)\)"
    ))
});
static INNER_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"ACK\((blocked|wontdo)\)(\([^)]*\))?(:|：){WS}*#([0-9]+)"
    ))
});
static RETURNED: LazyLock<Regex> = LazyLock::new(|| re(r"^外环\([^)]+\)·打回\(#([0-9]+)\)"));
static ALERT: LazyLock<Regex> = LazyLock::new(|| re(r"^单元\([^)]+\)·负哨\(#?([^)]*)\)"));
static UNIT_MCP: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^单元\(([^)]+)\)·-{WS}+({NWS}+{WS}+{NWS}+){WS}+MCP\(ask_outer\){WS}+(blocked|timeout):{WS}+(.*)$"
    ))
});
static MCP_LINE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^- ({NWS}+ {NWS}+) MCP\(ask_outer\) (blocked|timeout): (.*)$"
    ))
});
static ASK_ID: LazyLock<Regex> = LazyLock::new(|| re(r"id=([0-9a-fA-F]+)"));
static NEXT_KEY: LazyLock<Regex> = LazyLock::new(|| re(r"\s[a-z_]+="));
static LEADING_ID: LazyLock<Regex> = LazyLock::new(|| re(r"^id=\S+\s*"));

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn take(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn real(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// One signal found in a source.
struct Found {
    trigger: String,
    source: &'static str,
    path: String,
    identity: String,
    line: usize,
    offset: usize,
    symptom: String,
}

/// The newline-terminated lines of a file: (1-based number, byte offset,
/// bytes). A trailing fragment without its newline is not a line yet.
fn complete_lines(data: &[u8]) -> Vec<(usize, usize, &[u8])> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(len) = data[start..].iter().position(|b| *b == b'\n') {
        out.push((out.len() + 1, start, &data[start..start + len]));
        start += len + 1;
    }
    out
}

/// The reason in an MCP audit line's detail: what follows `reason=` up to
/// the next ` key=`; a timeout line has none, so the text after its id.
fn mcp_reason(detail: &str) -> String {
    let reason = match detail.find("reason=") {
        Some(at) => {
            let rest = &detail[at + "reason=".len()..];
            match NEXT_KEY.find(rest) {
                Ok(Some(next)) => &rest[..next.start()],
                _ => rest,
            }
            .to_string()
        }
        None => LEADING_ID.replace(detail, "").into_owned(),
    };
    take(&reason, 80)
}

fn ascii_trim_start(text: &str) -> &str {
    text.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
}

fn harvest_board(data: &[u8], path: &str, out: &mut Vec<Found>) {
    let mut entry = "-1".to_string();
    for (number, offset, raw) in complete_lines(data) {
        let line = String::from_utf8_lossy(raw);
        if let Ok(Some(found)) = HEADING.captures(&line) {
            entry = found[1].to_string();
        }
        let trimmed = ascii_trim_start(&line);
        let mut trigger = String::new();
        let mut rest: Option<&str> = None;
        if trimmed.starts_with("ACK(blocked):") || trimmed.starts_with("ACK(blocked)：") {
            trigger = "ack_blocked".to_string();
            rest = Some(trimmed);
        } else if trimmed.starts_with("ACK(wontdo):") || trimmed.starts_with("ACK(wontdo)：") {
            trigger = "ack_wontdo".to_string();
            rest = Some(trimmed);
        }
        // Signed forms: a copy of the line without leading `> `, `**` and
        // whitespace, matched from its start.
        let mut signed = trimmed;
        loop {
            if let Some(next) = signed.strip_prefix("> ") {
                signed = next;
            } else if let Some(next) = signed.strip_prefix("**") {
                signed = next;
            } else {
                break;
            }
        }
        let signed = ascii_trim_start(signed);
        if trigger.is_empty() {
            if OVERRIDE.is_match(signed).unwrap_or(false) {
                trigger = "override".to_string();
            } else if R2.is_match(signed).unwrap_or(false) {
                trigger = "r2_record".to_string();
            }
        }
        // Cluster forms name their entry as #N: a unit cannot write under
        // an entry, so these lines sit at the end of the board.
        let mut card_entry = entry.clone();
        if trigger.is_empty() {
            if let Ok(Some(found)) = INNER.captures(signed) {
                trigger = format!("ack_{}", &found[3]);
                match found.get(2) {
                    Some(n) => card_entry = n.as_str().to_string(),
                    None => {
                        if let Ok(Some(tail)) = INNER_TAIL.captures(signed) {
                            card_entry = tail[4].to_string();
                        }
                    }
                }
                rest = Some(trimmed);
            } else if let Ok(Some(found)) = RETURNED.captures(signed) {
                trigger = "returned".to_string();
                card_entry = found[1].to_string();
            } else if let Ok(Some(found)) = ALERT.captures(signed) {
                trigger = "unit_alert".to_string();
                let task = &found[1];
                if !task.is_empty() && task.bytes().all(|b| b.is_ascii_digit()) {
                    card_entry = task.to_string();
                }
            }
        }
        // A unit's MCP audit lines, signed by the unit: the same signals
        // the MCP board yields on a single machine, in the same shape.
        if trigger.is_empty() {
            if let Ok(Some(found)) = UNIT_MCP.captures(signed) {
                let (unit, kind, detail) = (&found[1], &found[3], &found[4]);
                let id = match ASK_ID.captures(detail) {
                    Ok(Some(id)) => id[1].to_string(),
                    _ => "-".to_string(),
                };
                out.push(Found {
                    trigger: if kind == "blocked" {
                        "report_blocked"
                    } else {
                        "ask_outer_timeout"
                    }
                    .to_string(),
                    source: "review",
                    path: path.to_string(),
                    identity: format!("board:{path}#unit:{unit}#mcp_{kind}#{}", sha(raw)),
                    line: number,
                    offset,
                    symptom: format!(
                        "kind={kind} id={id} unit={unit} reason={}",
                        mcp_reason(detail)
                    ),
                });
            }
        }
        if trigger.is_empty() {
            continue;
        }
        let kind = match trigger.as_str() {
            "ack_wontdo" => "wontdo",
            "override" => "override",
            "r2_record" => "r2",
            "returned" => "returned",
            "unit_alert" => "alert",
            _ => "blocked",
        };
        // Signed forms report the original line.
        let base = match trigger.as_str() {
            "override" | "r2_record" | "returned" | "unit_alert" => trimmed,
            _ => rest.unwrap_or(trimmed),
        };
        out.push(Found {
            identity: format!("board:{path}#{card_entry}#{kind}#{}", sha(raw)),
            trigger,
            source: "review",
            path: path.to_string(),
            line: number,
            offset,
            symptom: take(base, 200),
        });
    }
}

fn harvest_mcp(data: &[u8], path: &str, out: &mut Vec<Found>) {
    for (number, offset, raw) in complete_lines(data) {
        let line = String::from_utf8_lossy(raw);
        let Ok(Some(found)) = MCP_LINE.captures(&line) else {
            continue;
        };
        let (stamp, kind, detail) = (&found[1], &found[2], &found[3]);
        let id = match ASK_ID.captures(detail) {
            Ok(Some(id)) => id[1].to_string(),
            _ => "-".to_string(),
        };
        out.push(Found {
            trigger: if kind == "blocked" {
                "report_blocked"
            } else {
                "ask_outer_timeout"
            }
            .to_string(),
            source: "mcp",
            path: path.to_string(),
            identity: format!("mcp:{path}#{stamp}#{kind}#{id}#{}", sha(raw)),
            line: number,
            offset,
            // Never the question, its context or what was tried.
            symptom: format!("kind={kind} id={id} reason={}", mcp_reason(detail)),
        });
    }
}

/// A JSON value as the old tool would print it into an identity.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        other => other.to_string(),
    }
}

fn falsy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => s.is_empty(),
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        Value::Bool(true) => false,
    }
}

fn harvest_events(data: &[u8], file: &str, path: &str, out: &mut Vec<Found>) {
    let text = String::from_utf8_lossy(data);
    let mut lines: Vec<&str> = text.split('\n').collect();
    // A trailing line without its newline is not a record yet.
    if !data.is_empty() && !data.ends_with(b"\n") {
        lines.pop();
    }
    let mut offset = 0;
    for (i, line) in lines.iter().enumerate() {
        let start = offset;
        offset += line.len() + 1;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(Value::Object(record)) = serde_json::from_str::<Value>(line) else {
            eprintln!("malformed: {file}:{}", i + 1);
            continue;
        };
        let kind = record.get("kind").map(text_of).unwrap_or_default();
        let trigger = match kind.as_str() {
            "escalation" | "turn_error" | "fallback_switch" | "malformed_exhausted" => kind.clone(),
            "goal_transition" => match record.get("detail").and_then(Value::as_str) {
                Some("goal transitioned to `blocked`") => "goal_blocked".to_string(),
                Some("goal transitioned to `budget_limited`") => "goal_budget_limited".to_string(),
                _ => continue,
            },
            _ => continue,
        };
        let pick = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| record.get(*k).filter(|v| !falsy(v)))
                .map(text_of)
                .unwrap_or_else(|| "-".to_string())
        };
        // A missing ts must not become "now": the event would never dedup.
        let stamp = pick(&["ts"]);
        let reference = pick(&["goal_id", "slug", "session"]);
        out.push(Found {
            trigger,
            source: "events",
            path: path.to_string(),
            identity: format!(
                "events:{path}#{stamp}#{kind}#{reference}#{}",
                sha(line.as_bytes())
            ),
            line: i + 1,
            offset: start,
            symptom: take(line, 200),
        });
    }
}

fn file_id(path: &Path) -> (u64, u64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        path.metadata()
            .map(|m| (m.dev(), m.ino()))
            .unwrap_or((0, 0))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        (0, 0)
    }
}

/// sha256 of the up-to-64 bytes before `offset`.
fn prefix_sha(data: &[u8], offset: usize) -> String {
    let end = offset.min(data.len());
    sha(&data[end.saturating_sub(64)..end])
}

/// Say `reset:` when a source is no longer the file the cursor was taken
/// on: shorter, another inode, or other bytes before the cursor.
fn note_reset(state: &Value, key: &str, path: &Path, data: &[u8]) {
    let Some(source) = state
        .pointer(&format!("/sources/{key}"))
        .filter(|s| s.is_object())
    else {
        return;
    };
    let previous = source["offset"].as_u64().unwrap_or(0) as usize;
    if previous == 0 {
        return;
    }
    let (dev, ino) = file_id(path);
    let moved = data.len() < previous
        || source["dev"].as_u64().unwrap_or(0) != dev
        || source["ino"].as_u64().unwrap_or(0) != ino;
    let prefix = source["prefix_sha256"].as_str().unwrap_or("");
    if moved || (!prefix.is_empty() && prefix != prefix_sha(data, previous)) {
        eprintln!("reset: {}", path.display());
    }
}

/// The first value of `key` anywhere in a JSON document.
fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .iter()
            .find_map(|(k, v)| if k == key { Some(v) } else { find_key(v, key) }),
        Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}

/// `argv` is what follows `olp evo harvest`.
pub fn run(argv: &[String]) -> i32 {
    let Some(repo) = argv.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: octoscode olp evo harvest <repo-root> [--dry-run]");
        return 1;
    };
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        println!(
            "用法: octoscode olp evo harvest <repo-root> [--dry-run]\n\n从审查黑板、事件流、MCP 审计板采集摩擦信号,每条在进化板上落一张身份稳定的症状卡。不写审查黑板。\n--dry-run 只打印待落的卡,不建任何文件。多机部署时只在 hub 上运行。\n退出码:0;1 出错;2 仓库或审查黑板不存在;3 不是 hub"
        );
        return 0;
    }
    let dry_run = argv.get(1).is_some_and(|a| a == "--dry-run");
    let repo = Path::new(repo);
    let env_path = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let board = env_path("OLP_EVO_REVIEW_BOARD")
        .unwrap_or_else(|| repo.join(".octos/OUTER_LOOP_REVIEW.md"));
    let events = env_path("OLP_EVO_EVENTS");
    let mcp_board = env_path("OLP_EVO_MCP_BOARD")
        .unwrap_or_else(|| home.join(".octos/outer/OUTER_LOOP_MCP.md"));
    let evo_board = env_path("OLP_EVO_BOARD").unwrap_or_else(|| repo.join(".octos/EVOLUTION.md"));
    // Before anything is created: a missing repository or board leaves no
    // trace.
    if !repo.exists() {
        eprintln!("error: repo-root not found: {}", repo.display());
        return 2;
    }
    if !board.is_file() {
        eprintln!("error: review board not found: {}", board.display());
        return 2;
    }
    if !dry_run {
        if let Err(why) = hub_guard("olp-evo-harvest") {
            eprintln!("{why}");
            return 3;
        }
    }
    let key: String = sha(real(repo).as_bytes()).chars().take(16).collect();
    let state_dir = env_path("OLP_EVO_STATE")
        .unwrap_or_else(|| home.join(".octos/outer/evo"))
        .join(key);
    let state_file = state_dir.join("state.json");
    // One timestamp for the whole run: a card's title and its envelope
    // must agree even across a second boundary.
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let _lock = if dry_run {
        None
    } else {
        if std::fs::create_dir_all(&state_dir).is_err() {
            eprintln!("error: cannot create state dir {}", state_dir.display());
            return 1;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(state_dir.join("harvest.lock"))
            .and_then(|file| fs2::FileExt::lock_exclusive(&file).map(|()| file));
        match lock {
            Ok(file) => Some(file),
            Err(e) => {
                eprintln!("error: cannot lock {}: {e}", state_dir.display());
                return 1;
            }
        }
    };

    let state: Value = match std::fs::read_to_string(&state_file) {
        Err(_) => Value::Null,
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("state-read-failed: {e}");
            Value::Null
        }),
    };
    let mut next_id = find_key(&state, "next_id")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    // What has been carded: the state's record and the evolution board
    // itself, which is the ledger. Both as sha256 of the identity.
    let mut seen: Vec<String> = state["seen"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect();
    let ledger = std::fs::read(&evo_board).unwrap_or_default();
    for (_, _, raw) in complete_lines(&ledger) {
        let line = String::from_utf8_lossy(raw);
        if let Some(rest) = line.strip_prefix("### EVO-") {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            match digits.parse::<u64>() {
                Ok(n) if n >= next_id => next_id = n + 1,
                Ok(_) => {}
                Err(_) => eprintln!("warn: non-numeric EVO id ignored in reconcile: {line}"),
            }
        } else if line.starts_with("identity:") {
            let identity = line.strip_prefix("identity: ").unwrap_or(&line);
            seen.push(sha(identity.as_bytes()));
        }
    }

    // Sources, in the order their cards come out: board, MCP, events.
    let mut found = Vec::new();
    let board_data = std::fs::read(&board).unwrap_or_default();
    note_reset(&state, "review", &board, &board_data);
    harvest_board(&board_data, &real(&board), &mut found);
    let events_data = match &events {
        None => None,
        Some(path) => match std::fs::read(path).ok().filter(|_| path.is_file()) {
            Some(data) => {
                note_reset(&state, "events", path, &data);
                Some(data)
            }
            None => {
                eprintln!("skip: {}", path.display());
                None
            }
        },
    };
    let mcp_data = match std::fs::read(&mcp_board)
        .ok()
        .filter(|_| mcp_board.is_file())
    {
        Some(data) => {
            note_reset(&state, "mcp", &mcp_board, &data);
            harvest_mcp(&data, &real(&mcp_board), &mut found);
            Some(data)
        }
        None => {
            eprintln!("skip: {}", mcp_board.display());
            None
        }
    };
    if let (Some(path), Some(data)) = (&events, &events_data) {
        harvest_events(data, &path.to_string_lossy(), &real(path), &mut found);
    }

    let mut cards = String::new();
    let mut added = 0u64;
    for signal in &found {
        let identity = signal.identity.trim_end_matches('\r');
        if seen.contains(&sha(identity.as_bytes())) {
            continue;
        }
        cards.push_str(&format!(
            "### EVO-{next_id:04}（{stamp}，harvest）\ntrigger: {}\nsource: {} {}\nidentity: {}\nenvelope: line={} offset={} ts={stamp}\nsymptom: {}\n",
            signal.trigger,
            signal.source,
            signal.path,
            signal.identity,
            signal.line,
            signal.offset,
            signal.symptom
        ));
        next_id += 1;
        added += 1;
    }
    if dry_run {
        print!("{cards}");
        return 0;
    }
    let fault = std::env::var("OLP_EVO_TEST").as_deref() == Ok("1")
        && std::env::var("OLP_EVO_FAULT").as_deref() == Ok("after-append");
    if !cards.is_empty() {
        let appended = super::append::append_entry(&evo_board, None, cards.clone().into_bytes());
        if fault {
            // Test hook: the cards landed, the state was not committed.
            eprintln!("fault-injected: after-append");
            return 70;
        }
        if let Err((code, why)) = appended {
            eprintln!("error: {why}");
            return if code == 6 { 6 } else { 1 };
        }
    }

    // Commit the state: every identity on the ledger is seen, and each
    // source's cursor is the end of its last complete line.
    let ledger = std::fs::read(&evo_board).unwrap_or_default();
    for line in String::from_utf8_lossy(&ledger).split('\n') {
        if let Some(identity) = line.strip_prefix("identity: ") {
            seen.push(sha(identity.as_bytes()));
        }
    }
    seen.sort();
    seen.dedup();
    let mut sources = Vec::new();
    for (name, path, data) in [
        ("review", Some(&board), Some(&board_data)),
        ("events", events.as_ref(), events_data.as_ref()),
        ("mcp", Some(&mcp_board), mcp_data.as_ref()),
    ] {
        let (Some(path), Some(_)) = (path, data) else {
            continue;
        };
        // Read again: the cursor is where the file ends now.
        let now = std::fs::read(path).unwrap_or_default();
        let offset = if now.ends_with(b"\n") {
            now.len()
        } else {
            now.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)
        };
        let (dev, ino) = file_id(path);
        sources.push((
            name.to_string(),
            Json::Obj(vec![
                ("path".to_string(), Json::Str(real(path))),
                ("offset".to_string(), Json::Int(offset as i64)),
                ("dev".to_string(), Json::Int(dev as i64)),
                ("ino".to_string(), Json::Int(ino as i64)),
                (
                    "prefix_sha256".to_string(),
                    Json::Str(if offset > 0 {
                        prefix_sha(&now, offset)
                    } else {
                        String::new()
                    }),
                ),
            ]),
        ));
    }
    let next = Json::Obj(vec![
        ("next_id".to_string(), Json::Int(next_id as i64)),
        (
            "seen".to_string(),
            Json::Arr(seen.into_iter().map(Json::Str).collect()),
        ),
        ("sources".to_string(), Json::Obj(sources)),
    ]);
    let written = tempfile::Builder::new()
        .prefix("state.json.tmp.")
        .tempfile_in(&state_dir)
        .and_then(|mut tmp| {
            std::io::Write::write_all(&mut tmp, next.line().as_bytes())?;
            tmp.as_file().sync_all()?;
            tmp.persist(&state_file).map_err(|e| e.error)?;
            Ok(())
        });
    if let Err(e) = written {
        eprintln!("error: {}: {e}", state_file.display());
        return 1;
    }
    eprintln!("harvested: {added} new card(s), next_id={next_id}");
    0
}
