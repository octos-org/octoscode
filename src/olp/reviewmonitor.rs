//! `octoscode olp review monitor <互审目录> [--runtime-dir D] [--board 板] …`
//! — a read-only view of a review in progress, for a herdr pane.
//!
//! Four blocks that are never mixed: lifecycle (frozen, challenge, cross),
//! current turn (what each peer is doing now), last outcome (how each
//! peer's most recent turn ended) and deliverables (files, never called
//! accepted because they exist). Everything it cannot prove it shows as
//! `unknown`, with the reason:
//!
//! - `review-state.json` is only read, also when it is damaged.
//! - A peer's `lifetime.json` is believed only at
//!   `<runtime>/profiles/<profile>/data/peers/<slug>/lifetime.json`, not
//!   through a symlink, with version 1, a task id, the registry key of
//!   this profile and slug, a master equal to the `originator` file (and
//!   to `--session` when given) and a turn id that fits the phase; an idle
//!   peer's `result.md` must hash to the recorded digest.
//! - Without that, a peer is running only when its own unfinished thread
//!   is bound to this session and its `originator` and `goal` files prove
//!   it belongs here. One peer's activity never shows on another.
//! - The last outcome needs `result-N.md` and `turns.txt` to agree. What a
//!   snapshot says about itself is a note, not an outcome.
//! - Blocked transitions and escalations come from this runtime's and this
//!   profile's `events.jsonl` only.
//!
//! The one file it writes is its own `monitor-state.json` (replaced
//! atomically): the board's ACK lines by content, so that an added, an
//! edited and a removed ACK are told apart.
//!
//! Exit codes: 0; 2 usage.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::evolib::Json;

const PROTOCOL: &str = "olp-review-monitor/v1";
const DEFAULT_PROFILE: &str = "octos";

const USAGE: &str = "用法: octoscode olp review monitor <互审目录> [--runtime-dir 目录] [--board 板] [--profile 名] [--session 会话] [--format json|human] [--watch] [--interval 秒]

只读的互审进度视图:lifecycle / current-turn / last-outcome / deliverables 四块分开显示,
证明不了的一律 unknown 并给出原因。只写自己的 monitor-state.json(板上 ACK 行的基线)。
--watch 持续运行,内容变了才输出;--interval 默认 2 秒。退出码:0;2 用法";

static FRONTMATTER: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(r"(?s)\A---\s*\n(.*?)\n---\s*\n").expect("a pattern written here")
});

// ---------------------------------------------------------------------------
// text and values, read the way the files' writers mean them
// ---------------------------------------------------------------------------

fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

fn words(text: &str) -> Vec<&str> {
    text.split(is_space).filter(|w| !w.is_empty()).collect()
}

/// Lines without their endings; every line break Unicode knows ends one.
fn lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if !matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            continue;
        }
        out.push(&text[start..at]);
        start = at + c.len_utf8();
        if c == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
            chars.next();
            start += 1;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
}

/// A file's text: invalid UTF-8 replaced, line endings made `\n`.
fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(
        String::from_utf8_lossy(&bytes)
            .replace("\r\n", "\n")
            .replace('\r', "\n"),
    )
}

/// The text of a regular file; none for a symlink or anything else.
fn read_regular(path: &Path) -> Option<String> {
    if is_symlink(path) || !path.is_file() {
        return None;
    }
    read_text(path)
}

/// A path argument without `.` parts, doubled or trailing slashes: the
/// form a runtime is named by in snapshots and events.
fn path_text(arg: &str) -> String {
    let lead = if arg.starts_with("//") && !arg.starts_with("///") {
        "//"
    } else if arg.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts: Vec<&str> = arg
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let joined = format!("{lead}{}", parts.join("/"));
    if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// A value as the script's messages show it: `None`, `'text'`, `999`.
fn repr(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::Bool(true)) => "True".to_string(),
        Some(Value::Bool(false)) => "False".to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => {
            let quote = if s.contains('\'') && !s.contains('"') {
                '"'
            } else {
                '\''
            };
            let mut out = String::from(quote);
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c == quote => {
                        out.push('\\');
                        out.push(c);
                    }
                    c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                        out.push_str(&format!("\\x{:02x}", c as u32));
                    }
                    c => out.push(c),
                }
            }
            out.push(quote);
            out
        }
        Some(Value::Array(items)) => {
            let inner: Vec<String> = items.iter().map(|v| repr(Some(v))).collect();
            format!("[{}]", inner.join(", "))
        }
        Some(Value::Object(map)) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {}",
                        repr(Some(&Value::String(k.clone()))),
                        repr(Some(v))
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

/// A value inside a line of text: a string as it is, anything else as
/// [`repr`] shows it.
fn shown(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => repr(Some(other)),
    }
}

fn text_of(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn flag(value: bool) -> Json {
    Json::Raw(Value::Bool(value))
}

fn text(value: impl Into<String>) -> Json {
    Json::Str(value.into())
}

fn optional(value: &Option<String>) -> Json {
    value.clone().map_or(Json::Null, Json::Str)
}

fn list(items: &[String]) -> Json {
    Json::Arr(items.iter().cloned().map(Json::Str).collect())
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

// ---------------------------------------------------------------------------
// the point of view: which runtime, profile, session and goal are "here"
// ---------------------------------------------------------------------------

/// The session in force. One read from `review-state.json` may be
/// something other than a string; it then equals nothing.
struct Session {
    raw: Value,
}

struct View {
    runtime: Option<PathBuf>,
    /// The runtime's name in snapshots and events.
    runtime_tag: Option<String>,
    profile: String,
    session: Option<Session>,
    goal: Option<String>,
}

impl View {
    /// The session, when it is known and a string.
    fn session_text(&self) -> Option<&str> {
        self.session.as_ref().and_then(|s| s.raw.as_str())
    }

    /// A session is in force and `other` is not it.
    fn session_differs(&self, other: &str) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.raw.as_str() != Some(other))
    }
}

// ---------------------------------------------------------------------------
// wire session ids
// ---------------------------------------------------------------------------

/// Where a session id's working-directory suffix starts: a NUL (or the
/// four characters `\x00`), `~cwd-` and letters or digits up to the end.
fn cwd_suffix_at(session: &str) -> Option<usize> {
    let hash_len = session
        .chars()
        .rev()
        .take_while(char::is_ascii_alphanumeric)
        .count();
    if hash_len == 0 {
        return None;
    }
    let run_start = session.len() - hash_len;
    let head = &session[..run_start];
    let before = head.strip_suffix("~cwd-")?;
    if before.ends_with('\0') {
        Some(before.len() - 1)
    } else if before.ends_with("\\x00") {
        Some(before.len() - 4)
    } else {
        None
    }
}

/// A session id as (what precedes the suffix, the suffix).
fn trunk_and_cwd(session: &str) -> (&str, Option<&str>) {
    match cwd_suffix_at(session) {
        Some(at) => (&session[..at], Some(&session[at..])),
        None => (session, None),
    }
}

/// Two session ids name the same originator: the same trunk, and the same
/// working directory when both carry one.
fn same_origin(a: &str, b: &str) -> bool {
    let (trunk_a, cwd_a) = trunk_and_cwd(a);
    let (trunk_b, cwd_b) = trunk_and_cwd(b);
    if trunk_a.is_empty() || trunk_a != trunk_b {
        return false;
    }
    match (cwd_a, cwd_b) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// The peer a thread's session id belongs to: `…#peer-<slug>` before the
/// suffix. None for a master session.
fn wire_peer_slug(session: &str) -> Option<&str> {
    let (trunk, _) = trunk_and_cwd(session);
    let (_, leaf) = trunk.rsplit_once('#')?;
    leaf.strip_prefix("peer-").filter(|slug| !slug.is_empty())
}

fn channel(session: &str) -> &str {
    let (trunk, _) = trunk_and_cwd(session);
    trunk.rsplit_once('#').map_or(trunk, |(chan, _)| chan)
}

// ---------------------------------------------------------------------------
// ui-protocol threads
// ---------------------------------------------------------------------------

struct Thread {
    file: String,
    corrupt: bool,
    session_id: Value,
    completed: Value,
    peer: Option<String>,
    /// Unfinished, in the real wire shape, and bound to this session.
    active: bool,
    mismatch: bool,
}

impl Thread {
    fn json(&self) -> Json {
        if self.corrupt {
            return obj(vec![("file", text(&self.file)), ("corrupt", flag(true))]);
        }
        let mut pairs = vec![
            ("file", text(&self.file)),
            ("session_id", Json::raw(&self.session_id)),
            ("completed", Json::raw(&self.completed)),
            ("peer", optional(&self.peer)),
            ("active", flag(self.active)),
        ];
        if self.mismatch {
            pairs.push(("session_mismatch", flag(true)));
        }
        obj(pairs)
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// `<runtime>/ui-protocol/*/threads/*.json`. A thread's `next_seq` counts
/// stream events and is never compared with a turn number.
fn ui_threads(view: &View) -> Vec<Thread> {
    let Some(runtime) = &view.runtime else {
        return Vec::new();
    };
    let base = runtime.join("ui-protocol");
    let mut out = Vec::new();
    for dir in names_in(&base) {
        let threads = base.join(&dir).join("threads");
        for name in names_in(&threads) {
            if !name.ends_with(".json") {
                continue;
            }
            let data = std::fs::read(threads.join(&name))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            let Some(Value::Object(data)) = data else {
                out.push(Thread {
                    file: name,
                    corrupt: true,
                    session_id: Value::Null,
                    completed: Value::Null,
                    peer: None,
                    active: false,
                    mismatch: false,
                });
                continue;
            };
            let sid = data.get("session_id").cloned().unwrap_or(Value::Null);
            let completed = data.get("completed").cloned().unwrap_or(Value::Null);
            let wire_ok =
                is_one(data.get("v")) && data.get("thread_id").is_some_and(|t| t.is_string());
            let slug = sid.as_str().and_then(wire_peer_slug).map(str::to_string);
            // Bound: the same channel and the same working directory, both
            // sides carrying one. A master without a suffix binds nothing.
            let bound = match (view.session_text(), sid.as_str(), &slug) {
                (Some(session), Some(sid), Some(_)) => {
                    let (_, ours) = trunk_and_cwd(session);
                    let (_, theirs) = trunk_and_cwd(sid);
                    !channel(session).is_empty()
                        && channel(session) == channel(sid)
                        && ours.is_some()
                        && ours == theirs
                }
                _ => false,
            };
            out.push(Thread {
                file: name,
                corrupt: false,
                active: wire_ok && slug.is_some() && bound && completed == Value::Bool(false),
                mismatch: view.session.is_some() && truthy(&sid) && !bound,
                session_id: sid,
                completed,
                peer: slug,
            });
        }
    }
    out
}

fn is_one(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_f64)
        .is_some_and(|number| number == 1.0)
}

// ---------------------------------------------------------------------------
// lifetime.json — believed only when every check holds
// ---------------------------------------------------------------------------

struct Lifetime {
    task_id: String,
    master: String,
    generation: Value,
    phase: &'static str,
    turn_id: Option<String>,
}

fn trusted_lifetime(peer_dir: &Path, slug: &str, view: &View) -> Result<Lifetime, String> {
    let path = peer_dir.join("lifetime.json");
    if !path.exists() {
        return Err("no-lifetime-json".into());
    }
    if is_symlink(&path) {
        return Err("lifetime-json-is-symlink".into());
    }
    let raw = read_regular(&path).ok_or("lifetime-json-unreadable")?;
    let rec: Value = serde_json::from_str(&raw).map_err(|_| "lifetime-json-corrupt")?;
    let rec = rec.as_object().ok_or("lifetime-json-wrong-shape")?;
    if !is_one(rec.get("version")) {
        return Err(format!(
            "lifetime-version-untrusted:{}",
            repr(rec.get("version"))
        ));
    }
    let task_id = text_of(rec.get("task_id")).ok_or("lifetime-empty-task-id")?;
    let key = format!("{}:peer:{slug}", view.profile);
    if rec.get("registry_key").and_then(Value::as_str) != Some(key.as_str()) {
        return Err("lifetime-registry-key-mismatch".into());
    }
    let master = text_of(rec.get("master")).ok_or("lifetime-empty-master")?;
    let originator = read_regular(&peer_dir.join("originator"));
    if originator.as_deref().map(strip) != Some(master) {
        return Err("lifetime-originator-mismatch".into());
    }
    // Bound to this session: the same slug under another master is not ours.
    if view.session_differs(master) {
        return Err("lifetime-master-not-current-session".into());
    }
    let phase = match rec.get("phase").and_then(Value::as_str) {
        Some("pending") => "pending",
        Some("running") => "running",
        Some("idle") => "idle",
        Some("failed") => "failed",
        _ => {
            return Err(format!(
                "lifetime-phase-untrusted:{}",
                repr(rec.get("phase"))
            ));
        }
    };
    // A pending peer may keep the turn id of the turn before it (a queued
    // follow-up) or have none; every other phase has one.
    let turn = rec.get("turn_id").filter(|v| !v.is_null());
    let turn_id = match (phase, turn) {
        ("pending", None) => None,
        (_, Some(Value::String(id))) if !id.is_empty() => Some(id.clone()),
        ("pending", _) => return Err("lifetime-pending-bad-turn-id".into()),
        _ => return Err(format!("lifetime-{phase}-without-turn-id")),
    };
    let generation = rec
        .get("generation")
        .filter(|v| v.is_i64() || v.is_u64())
        .ok_or("lifetime-generation-wrong-type")?;
    let digest = match rec.get("result_digest") {
        None | Some(Value::Null) => None,
        Some(Value::String(digest)) => Some(digest.as_str()),
        Some(_) => return Err("lifetime-result-digest-wrong-type".into()),
    };
    if phase == "idle" {
        let digest = digest
            .filter(|d| !d.is_empty())
            .ok_or("lifetime-idle-empty-digest")?;
        let result = peer_dir.join("result.md");
        if is_symlink(&result) {
            return Err("lifetime-idle-result-symlink".into());
        }
        let body = std::fs::read(&result).map_err(|_| "lifetime-idle-result-missing")?;
        if hex(&body) != digest {
            return Err("lifetime-idle-digest-mismatch".into());
        }
    }
    Ok(Lifetime {
        task_id: task_id.to_string(),
        master: master.to_string(),
        generation: generation.clone(),
        phase,
        turn_id,
    })
}

// ---------------------------------------------------------------------------
// runtime-evidence.json snapshots — each belongs to one peer
// ---------------------------------------------------------------------------

/// The parts of a snapshot's identity, each under any of its names.
const IDENTITY: [&[&str]; 5] = [
    &["runtime"],
    &["session", "session_id", "master"],
    &["goal", "goal_id"],
    &["peer", "slug"],
    &["head", "HEAD", "commit"],
];

fn first_present<'a>(snap: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names
        .iter()
        .filter_map(|name| snap.get(*name))
        .find(|v| !v.is_null())
}

/// A snapshot may be used here: it carries its whole identity (a slug
/// alone is not one) and none of it contradicts this point of view.
fn snapshot_bound_here(snap: &Map<String, Value>, view: &View) -> bool {
    let carries_identity = IDENTITY
        .iter()
        .flat_map(|names| names.iter())
        .filter(|name| **name != "slug")
        .any(|name| snap.get(*name).is_some_and(|v| !v.is_null()));
    if !carries_identity || text_of(snap.get("slug")).is_none() {
        return false;
    }
    if IDENTITY
        .iter()
        .any(|names| text_of(first_present(snap, names)).is_none())
    {
        return false;
    }
    if let (Some(theirs), Some(ours)) = (text_of(snap.get("runtime")), &view.runtime_tag) {
        if theirs != ours {
            return false;
        }
    }
    if text_of(first_present(snap, IDENTITY[1])).is_some_and(|s| view.session_differs(s)) {
        return false;
    }
    if let (Some(theirs), Some(ours)) = (text_of(first_present(snap, IDENTITY[2])), &view.goal) {
        if theirs != ours {
            return false;
        }
    }
    match text_of(snap.get("profile")) {
        Some(theirs) if !view.profile.is_empty() => theirs == view.profile,
        _ => true,
    }
}

/// Snapshots by slug, the last one of a slug kept, and notes.
fn runtime_evidence(review_dir: &Path) -> (BTreeMap<String, Map<String, Value>>, Vec<String>) {
    let mut snaps = BTreeMap::new();
    let path = review_dir.join("runtime-evidence.json");
    if !path.exists() {
        return (snaps, vec!["runtime-evidence-missing".into()]);
    }
    let Some(raw) = read_regular(&path) else {
        return (snaps, vec!["runtime-evidence-unreadable".into()]);
    };
    let Ok(data) = serde_json::from_str::<Value>(&raw) else {
        return (snaps, vec!["runtime-evidence-corrupt".into()]);
    };
    let mut notes = Vec::new();
    let flat = data.get("peers").and_then(Value::as_array);
    let nested = data
        .get("goal")
        .and_then(|g| g.get("peers"))
        .and_then(Value::as_array);
    for peer in flat.into_iter().chain(nested).flatten() {
        let Some(peer) = peer.as_object() else {
            continue;
        };
        match text_of(peer.get("slug")) {
            Some(slug) => {
                snaps.insert(slug.to_string(), peer.clone());
            }
            None => notes.push("runtime-evidence-peer-missing-slug".to_string()),
        }
    }
    (snaps, notes)
}

// ---------------------------------------------------------------------------
// a peer's own identity files
// ---------------------------------------------------------------------------

fn identity_file(peer_dir: &Path, name: &str) -> Option<String> {
    read_regular(&peer_dir.join(name))
        .map(|raw| strip(&raw).to_string())
        .filter(|value| !value.is_empty())
}

/// The peer's `originator` and `goal` files do not contradict this point
/// of view. A missing file, or a session or goal not known here, is not a
/// contradiction.
fn identity_not_foreign(peer_dir: &Path, view: &View) -> Result<(), &'static str> {
    if let Some(originator) = identity_file(peer_dir, "originator") {
        let Some(session) = view.session_text() else {
            return Ok(());
        };
        if !same_origin(&originator, session) {
            return Err("peer-originator-mismatch");
        }
    }
    if let Some(theirs) = identity_file(peer_dir, "goal") {
        let Some(ours) = &view.goal else {
            return Ok(());
        };
        if &theirs != ours {
            return Err("peer-goal-mismatch");
        }
    }
    Ok(())
}

/// The peer's identity files prove it belongs here: for a session and a
/// goal known here, the file must be present and match. Too little
/// information is not proof.
fn identity_proven(peer_dir: &Path, view: &View) -> Result<(), &'static str> {
    if let Some(session) = view.session_text() {
        match identity_file(peer_dir, "originator") {
            None => return Err("peer-originator-unproven-missing"),
            Some(o) if !same_origin(&o, session) => {
                return Err("peer-originator-unproven-mismatch");
            }
            Some(_) => {}
        }
    }
    if let Some(ours) = &view.goal {
        match identity_file(peer_dir, "goal") {
            None => return Err("peer-goal-unproven-missing"),
            Some(theirs) if &theirs != ours => return Err("peer-goal-unproven-mismatch"),
            Some(_) => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// last outcome — result-N.md and turns.txt must agree
// ---------------------------------------------------------------------------

/// How a peer's most recent turn ended, or why that is not known.
struct Outcome {
    state: String,
    /// The report's own `turn:` line; absent when no report was read.
    turn: Option<Option<String>>,
    source: String,
    notes: Option<Vec<String>>,
    turns_max: Option<u64>,
}

impl Outcome {
    fn unknown(source: &str) -> Outcome {
        Outcome {
            state: "unknown".into(),
            turn: None,
            source: source.into(),
            notes: None,
            turns_max: None,
        }
    }

    fn json(&self) -> Json {
        let mut pairs = vec![("state", text(&self.state))];
        if let Some(turn) = &self.turn {
            pairs.push(("turn", optional(turn)));
        }
        pairs.push(("source", text(&self.source)));
        if let Some(notes) = &self.notes {
            pairs.push(("notes", list(notes)));
        }
        if let Some(max) = self.turns_max {
            pairs.push(("turns_max", Json::Raw(Value::from(max))));
        }
        obj(pairs)
    }
}

fn frontmatter(path: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(body) = read_text(path) else {
        return out;
    };
    let Ok(Some(found)) = FRONTMATTER.captures(&body) else {
        return out;
    };
    let block = found.get(1).map_or("", |m| m.as_str());
    for line in lines(block) {
        if let Some((key, value)) = line.split_once(':') {
            out.insert(strip(key).to_string(), strip(value).to_string());
        }
    }
    out
}

fn number(digits: &str) -> Option<u64> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// `<round> <outcome> …` rows and notes on the bad ones. A round listed
/// twice makes the whole file untrusted: no rows.
fn turns(path: &Path) -> (Vec<(u64, String)>, Vec<String>) {
    let Some(raw) = read_regular(path) else {
        return (Vec::new(), vec!["turns-txt-missing".into()]);
    };
    let (mut rows, mut bad, mut duplicate) = (Vec::<(u64, String)>::new(), Vec::new(), false);
    for (at, line) in lines(&raw).into_iter().enumerate() {
        let parts = words(line);
        if parts.is_empty() {
            continue;
        }
        match (number(parts[0]), parts.get(1)) {
            (Some(round), Some(_)) if rows.iter().any(|(r, _)| *r == round) => {
                bad.push(format!("turns-duplicate-row:{}", at + 1));
                duplicate = true;
            }
            (Some(round), Some(outcome)) => rows.push((round, outcome.to_string())),
            _ => bad.push(format!("turns-txt-bad-row:{}", at + 1)),
        }
    }
    if duplicate {
        rows.clear();
    }
    (rows, bad)
}

fn latest_terminated(dir: &Path, slug: &str) -> Outcome {
    if !dir.exists() {
        return Outcome::unknown("native-dir-missing");
    }
    let mut best: Option<(u64, String)> = None;
    for name in names_in(dir) {
        let round = name
            .strip_prefix("result-")
            .and_then(|rest| rest.strip_suffix(".md"))
            .and_then(number);
        let Some(round) = round else {
            continue;
        };
        let path = dir.join(&name);
        if path.is_file() && !is_symlink(&path) && best.as_ref().is_none_or(|(b, _)| round > *b) {
            best = Some((round, name));
        }
    }
    let Some((round, name)) = best else {
        return Outcome::unknown("no-result-files");
    };
    let fm = frontmatter(&dir.join(&name));
    let turn = fm.get("turn").cloned();
    let mut out = Outcome {
        state: "unknown".into(),
        turn: Some(turn.clone()),
        source: name,
        notes: None,
        turns_max: None,
    };
    let mut notes = Vec::new();
    // The report must be this peer's own, and say the turn its name says.
    let verdict = (|| {
        if let Some(theirs) = fm.get("slug").filter(|theirs| *theirs != slug) {
            notes.push(format!("result-slug-mismatch:fm={theirs},peer={slug}"));
            return None;
        }
        if turn.as_deref() != Some(round.to_string().as_str()) {
            notes.push(format!(
                "result-turn-mismatch:fm={},file={round}",
                turn.as_deref().unwrap_or("None")
            ));
            return None;
        }
        let Some(outcome) = fm.get("outcome").filter(|o| !o.is_empty()) else {
            notes.push("result-frontmatter-no-outcome".to_string());
            return None;
        };
        // The four ways a turn really ends are shown as they are; anything
        // else (still running, made up) is not an outcome.
        if !["completed", "errored", "interrupted", "rate_limited"].contains(&outcome.as_str()) {
            notes.push(format!("result-outcome-untrusted:{outcome}"));
            return None;
        }
        let (rows, turn_notes) = turns(&dir.join("turns.txt"));
        notes.extend(turn_notes);
        let Some((last_round, last_outcome)) = rows.iter().max_by_key(|(r, _)| *r) else {
            notes.push("turns-txt-untrusted".to_string());
            return None;
        };
        out.turns_max = Some(*last_round);
        if *last_round != round {
            notes.push(format!("turns-txt-round-mismatch:{last_round}!={round}"));
            return None;
        }
        if last_outcome != outcome {
            notes.push(format!(
                "outcome-conflict:result={outcome},turns={last_outcome}"
            ));
            return None;
        }
        Some(outcome.clone())
    })();
    if let Some(state) = verdict {
        out.state = state;
    }
    out.notes = Some(notes);
    out
}

// ---------------------------------------------------------------------------
// peers
// ---------------------------------------------------------------------------

struct Peer {
    execution: &'static str,
    source: &'static str,
    reason: Option<String>,
    task_id: Option<String>,
    generation: Option<Value>,
    turn_id: Option<String>,
    master: Option<String>,
    last: Outcome,
}

fn dirs_in(root: &Path) -> Vec<String> {
    names_in(root)
        .into_iter()
        .filter(|name| {
            let path = root.join(name);
            path.is_dir() && !is_symlink(&path)
        })
        .collect()
}

fn collect_peers(
    review_dir: &Path,
    view: &View,
    threads: &[Thread],
) -> (Vec<(String, Peer)>, Vec<String>) {
    let (snaps, notes) = runtime_evidence(review_dir);
    let native_root = review_dir.join("native");
    let peers_root = view
        .runtime
        .as_ref()
        .map(|r| r.join("profiles").join(&view.profile).join("data/peers"))
        .filter(|root| root.exists());

    let mut slugs: Vec<String> = dirs_in(&native_root);
    if let Some(root) = &peers_root {
        slugs.extend(dirs_in(root));
    }
    slugs.extend(snaps.keys().cloned());
    slugs.sort();
    slugs.dedup();

    let mut peers = Vec::new();
    for slug in slugs {
        let mut peer = Peer {
            execution: "unknown",
            source: "none",
            reason: None,
            task_id: None,
            generation: None,
            turn_id: None,
            master: None,
            last: Outcome::unknown("runtime-evidence-self-claim"),
        };
        let peer_dir = peers_root.as_ref().map(|root| root.join(&slug));
        let snap = snaps.get(&slug);
        let own_thread = threads
            .iter()
            .any(|t| t.active && t.peer.as_deref() == Some(slug.as_str()));
        // Activity belongs to exactly one peer: its snapshot's own claim
        // or its own bound thread.
        let active =
            |snap: &Map<String, Value>| text_of(snap.get("active_thread")).is_some() || own_thread;
        let running = |peer: &mut Peer| {
            peer.execution = "running";
            peer.source = "active-thread";
        };
        if let Some(dir) = &peer_dir {
            // Identity files that point at another master or goal come
            // before everything: nothing may then promote this peer.
            let foreign = identity_not_foreign(dir, view).err();
            let lifetime = trusted_lifetime(dir, &slug, view);
            if dir.join("closed").exists() {
                // Closed is shown whatever the phase says; the identity is
                // kept only from a lifetime that is trusted and ours.
                peer.execution = "closed";
                peer.source = "closed-marker";
                if let (Ok(rec), None) = (&lifetime, foreign) {
                    peer.task_id = Some(rec.task_id.clone());
                    peer.generation = Some(rec.generation.clone());
                    peer.turn_id = rec.turn_id.clone();
                    peer.master = Some(rec.master.clone());
                }
            } else {
                match (lifetime, foreign) {
                    (Ok(rec), None) => {
                        peer.execution = match rec.phase {
                            "pending" => "queued",
                            other => other,
                        };
                        peer.source = "lifetime";
                        peer.task_id = Some(rec.task_id);
                        peer.generation = Some(rec.generation);
                        peer.turn_id = rec.turn_id;
                        peer.master = Some(rec.master);
                    }
                    (_, Some(why)) => peer.reason = Some(why.to_string()),
                    (Err(why), None) => {
                        // A lifetime of another session is evidence of
                        // that session; no thread makes it ours.
                        let other_session = why == "lifetime-master-not-current-session";
                        peer.reason = Some(why);
                        let wanted = match snap {
                            Some(snap) => snapshot_bound_here(snap, view).then(|| active(snap)),
                            None => own_thread.then_some(true),
                        };
                        if let (Some(is_active), false) = (wanted, other_session) {
                            match identity_proven(dir, view) {
                                Err(why) => peer.reason = Some(why.to_string()),
                                Ok(()) if is_active => running(&mut peer),
                                Ok(()) => {}
                            }
                        }
                    }
                }
            }
        } else if let Some(snap) = snap {
            if !snapshot_bound_here(snap, view) {
                peer.reason = Some("runtime-evidence-identity-incomplete".into());
            } else if active(snap) {
                running(&mut peer);
            } else {
                peer.reason = Some("no-active-thread".into());
            }
        }

        let native_dir = native_root.join(&slug);
        if native_dir.exists() {
            peer.last = latest_terminated(&native_dir, &slug);
        } else if let Some(dir) = &peer_dir {
            peer.last = latest_terminated(dir, &slug);
        } else {
            // No report of a finished turn: what the snapshot says about
            // its own outcome is kept as a note, not shown as one.
            let mut claims = Vec::new();
            let claim = |name: &str| snap.and_then(|s| text_of(s.get(name)));
            if let Some(outcome) = claim("outcome") {
                claims.push(format!("snapshot-outcome-untrusted:{outcome}"));
            }
            if let Some(source) = claim("outcome_source") {
                claims.push(format!("snapshot-outcome-source-untrusted:{source}"));
            }
            peer.last.notes = Some(claims);
        }
        peers.push((slug, peer));
    }
    (peers, notes)
}

// ---------------------------------------------------------------------------
// ACK lines on the board, by content
// ---------------------------------------------------------------------------

struct Ack {
    key: String,
    line: usize,
    sha256: String,
}

/// The board's ACK lines. A line's key is the word with a digit inside
/// `ACK(…)` (the task number), else its position among the ACK lines; a
/// second line with the same key gets `#1`, and so on. Changing `done` to
/// `blocked` keeps the key, so it is an edit and not a removal plus an
/// addition.
fn ack_lines(board: &Path) -> Vec<Ack> {
    let mut out: Vec<Ack> = Vec::new();
    let Some(raw) = read_regular(board) else {
        return out;
    };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (at, line) in lines(&raw).into_iter().enumerate() {
        let inside = line
            .split_once("ACK(")
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(inside, _)| inside);
        let Some(inside) = inside else {
            continue;
        };
        let numbered = words(inside).into_iter().find(|word| {
            word.chars()
                .any(|c| c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric()))
        });
        let base = match numbered {
            Some(word) => word.to_string(),
            None => format!("ack-{}", out.len()),
        };
        let seen = counts.entry(base.clone()).or_insert(0);
        let key = if *seen == 0 {
            base
        } else {
            format!("{base}#{seen}")
        };
        *seen += 1;
        out.push(Ack {
            key,
            line: at + 1,
            sha256: hex(line.as_bytes()),
        });
    }
    out
}

struct AckEvent {
    kind: &'static str,
    ack: String,
    line: Value,
    /// The line's hash, shortened; for an edit, the new one.
    sha256: String,
    old_sha256: Option<String>,
}

impl AckEvent {
    fn json(&self) -> Json {
        let mut pairs = vec![
            ("type", text(self.kind)),
            ("ack", text(&self.ack)),
            ("line", Json::raw(&self.line)),
        ];
        match &self.old_sha256 {
            Some(old) => {
                pairs.push(("old_sha256", text(old)));
                pairs.push(("new_sha256", text(&self.sha256)));
            }
            None => pairs.push(("sha256", text(&self.sha256))),
        }
        obj(pairs)
    }
}

fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn ack_events(current: &[Ack], baseline: &Map<String, Value>) -> Vec<AckEvent> {
    let before = |key: &str| baseline.get(key).filter(|v| v.is_object());
    let hash_of = |entry: &Value| {
        entry
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut events = Vec::new();
    for ack in current {
        let (kind, old) = match before(&ack.key).map(hash_of) {
            None => ("ack-added", None),
            Some(old) if old != ack.sha256 => ("ack-inplace-edit", Some(short(&old))),
            Some(_) => continue,
        };
        events.push(AckEvent {
            kind,
            ack: ack.key.clone(),
            line: Value::from(ack.line),
            sha256: short(&ack.sha256),
            old_sha256: old,
        });
    }
    // In the order the lines stood on the board when they were recorded.
    let mut gone: Vec<(&String, &Value)> = baseline
        .iter()
        .filter(|(key, entry)| entry.is_object() && !current.iter().any(|a| &a.key == *key))
        .collect();
    gone.sort_by_key(|(_, entry)| {
        entry
            .get("line")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    });
    for (key, entry) in gone {
        events.push(AckEvent {
            kind: "ack-removed",
            ack: key.clone(),
            line: entry.get("line").cloned().unwrap_or(Value::Null),
            sha256: short(&hash_of(entry)),
            old_sha256: None,
        });
    }
    events
}

/// The monitor's own cache and, when it could not be used, why.
fn read_monitor_state(review_dir: &Path) -> (Map<String, Value>, Option<&'static str>) {
    let cache = review_dir.join("monitor-state.json");
    if !cache.exists() {
        return (Map::new(), None);
    }
    let Some(raw) = read_regular(&cache) else {
        return (Map::new(), Some("monitor-state.json unreadable"));
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(map)) => (map, None),
        Ok(_) => (Map::new(), Some("monitor-state.json wrong shape")),
        Err(_) => (
            Map::new(),
            Some("monitor-state.json corrupt (keeping evidence, starting fresh)"),
        ),
    }
}

/// Replace `monitor-state.json` with the board's ACK lines as they are
/// now; whatever else the file held is kept.
fn write_monitor_state(
    review_dir: &Path,
    kept: &Map<String, Value>,
    current: &[Ack],
) -> Result<(), std::io::Error> {
    let acks = current
        .iter()
        .map(|ack| {
            (
                ack.key.clone(),
                obj(vec![
                    ("line", Json::Raw(Value::from(ack.line))),
                    ("sha256", text(&ack.sha256)),
                ]),
            )
        })
        .collect();
    let mut pairs = vec![
        ("protocol".to_string(), text(PROTOCOL)),
        ("acks".to_string(), Json::Obj(acks)),
    ];
    for (key, value) in kept {
        if key != "protocol" && key != "acks" {
            pairs.push((key.clone(), Json::raw(value)));
        }
    }
    let body = format!("{}\n", Json::Obj(pairs).pretty(1));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = review_dir.join(format!(".monitor-state-{}-{stamp}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&tmp)
        .and_then(|mut file| file.write_all(body.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, review_dir.join("monitor-state.json")));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

// ---------------------------------------------------------------------------
// blocked transitions and escalations — this runtime and profile only
// ---------------------------------------------------------------------------

struct Negative {
    kind: &'static str,
    goal_id: Value,
    /// Only an escalation names a peer.
    slug: Option<Value>,
    detail: String,
    runtime: String,
}

impl Negative {
    fn json(&self) -> Json {
        let mut pairs = vec![
            ("kind", text(self.kind)),
            ("goal_id", Json::raw(&self.goal_id)),
        ];
        if let Some(slug) = &self.slug {
            pairs.push(("slug", Json::raw(slug)));
        }
        pairs.push(("detail", text(&self.detail)));
        pairs.push(("runtime", text(&self.runtime)));
        obj(pairs)
    }
}

fn negative_events(view: &View) -> Vec<Negative> {
    let mut out = Vec::new();
    let (Some(runtime), Some(tag)) = (&view.runtime, &view.runtime_tag) else {
        return out;
    };
    // No profile, no events: another profile's are never read instead.
    if view.profile.is_empty() {
        return out;
    }
    let file = runtime
        .join("profiles")
        .join(&view.profile)
        .join("data/events.jsonl");
    let Some(raw) = read_regular(&file) else {
        return out;
    };
    for line in lines(&raw) {
        let Ok(Value::Object(event)) = serde_json::from_str::<Value>(strip(line)) else {
            continue;
        };
        // Another goal's event is not ours; one without a goal is kept.
        let event_goal = text_of(event.get("goal_id"));
        if let (Some(theirs), Some(ours)) = (event_goal, &view.goal) {
            if theirs != ours {
                continue;
            }
        }
        if text_of(event.get("session")).is_some_and(|s| view.session_differs(s)) {
            continue;
        }
        let detail = event.get("detail").map(shown).unwrap_or_default();
        let goal_id = event.get("goal_id").cloned().unwrap_or(Value::Null);
        match event.get("kind").and_then(Value::as_str) {
            Some("goal_transition")
                if detail.contains("blocked") || detail.contains("escalation") =>
            {
                out.push(Negative {
                    kind: "goal_transition",
                    goal_id,
                    slug: None,
                    detail,
                    runtime: tag.clone(),
                });
            }
            Some("escalation") => out.push(Negative {
                kind: "escalation",
                goal_id,
                slug: Some(event.get("slug").cloned().unwrap_or(Value::Null)),
                detail,
                runtime: tag.clone(),
            }),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the report
// ---------------------------------------------------------------------------

struct Report {
    identity: String,
    corrupt: bool,
    frozen: bool,
    challenge_accepted: bool,
    cross_count: usize,
    peers: Vec<(String, Peer)>,
    threads: Vec<Thread>,
    deliverables: Vec<String>,
    ack_events: Vec<AckEvent>,
    events: Vec<Negative>,
    diagnostics: Vec<String>,
}

struct Args {
    review_dir: PathBuf,
    runtime: Option<String>,
    board: Option<PathBuf>,
    profile: String,
    session: Option<String>,
}

/// `review-state.json` and whether it could not be used. Never written.
fn review_state(review_dir: &Path) -> (Map<String, Value>, bool) {
    let file = review_dir.join("review-state.json");
    if !file.exists() {
        return (Map::new(), false);
    }
    let state = read_regular(&file).and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    match state {
        Some(Value::Object(map)) => (map, false),
        _ => (Map::new(), true),
    }
}

fn render(args: &Args) -> Report {
    let (state, corrupt) = review_state(&args.review_dir);
    let stated = |name: &str| state.get(name).filter(|v| truthy(v));
    let session = match &args.session {
        Some(given) if !given.is_empty() => Some(Value::String(given.clone())),
        _ => stated("session").cloned(),
    };
    let view = View {
        runtime: args.runtime.as_ref().map(PathBuf::from),
        runtime_tag: args.runtime.clone(),
        profile: args.profile.clone(),
        session: session.clone().map(|raw| Session { raw }),
        goal: text_of(state.get("goal")).map(str::to_string),
    };

    let threads = ui_threads(&view);
    let (peers, peer_notes) = collect_peers(&args.review_dir, &view, &threads);

    // Listed, and never called accepted because a file is there.
    let deliverables = names_in(&args.review_dir)
        .into_iter()
        .filter(|name| {
            let path = args.review_dir.join(name);
            let kind = path.extension().and_then(|e| e.to_str());
            matches!(kind, Some("md" | "json"))
                && name != "monitor-state.json"
                && path.is_file()
                && !is_symlink(&path)
        })
        .collect();

    let mut diagnostics = Vec::new();
    if corrupt {
        diagnostics.push(
            "review-state.json corrupt — rendered with degraded state, file untouched".to_string(),
        );
    }
    let mut events = Vec::new();
    if let Some(board) = &args.board {
        let current = ack_lines(board);
        let (kept, problem) = read_monitor_state(&args.review_dir);
        let none = Map::new();
        let baseline = kept.get("acks").and_then(Value::as_object).unwrap_or(&none);
        events = ack_events(&current, baseline);
        let failed = write_monitor_state(&args.review_dir, &kept, &current)
            .err()
            .map(|e| format!("monitor-state.json write failed: {e}"));
        let problems: Vec<String> = problem
            .map(str::to_string)
            .into_iter()
            .chain(failed)
            .collect();
        if !problems.is_empty() {
            diagnostics.push(problems.join("; "));
        }
    }
    diagnostics.extend(peer_notes);

    // Who this is: the runtime with profile, session, master and goal —
    // never a bare goal id.
    let runtime = match &args.runtime {
        Some(tag) => tag.clone(),
        None => state.get("runtime").map(shown).unwrap_or_default(),
    };
    let mut identity = vec![if runtime.is_empty() {
        "runtime:unknown".to_string()
    } else {
        runtime
    }];
    if !args.profile.is_empty() {
        identity.push(format!("profile:{}", args.profile));
    }
    if let Some(session) = &session {
        identity.push(format!("session:{}", shown(session)));
    }
    if let Some(master) = stated("master").filter(|m| Some(*m) != session.as_ref()) {
        identity.push(format!("master:{}", shown(master)));
    }
    if let Some(goal) = stated("goal") {
        identity.push(format!("goal:{}", shown(goal)));
    }

    Report {
        identity: identity.join(" | "),
        corrupt,
        frozen: stated("frozen").is_some(),
        challenge_accepted: state
            .get("challenge")
            .and_then(|c| c.get("accepted"))
            .is_some_and(truthy),
        cross_count: state
            .get("cross")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        events: negative_events(&view),
        peers,
        threads,
        deliverables,
        ack_events: events,
        diagnostics,
    }
}

impl Report {
    fn json(&self) -> Json {
        let per_peer = |each: &dyn Fn(&Peer) -> Json| {
            Json::Obj(
                self.peers
                    .iter()
                    .map(|(slug, peer)| (slug.clone(), each(peer)))
                    .collect(),
            )
        };
        let by_lifetime = self.peers.iter().any(|(_, p)| p.source == "lifetime");
        let mut pairs = vec![
            ("protocol", text(PROTOCOL)),
            ("identity", text(&self.identity)),
            ("review_state_corrupt", flag(self.corrupt)),
            (
                "lifecycle",
                obj(vec![
                    ("frozen", flag(self.frozen)),
                    ("challenge_accepted", flag(self.challenge_accepted)),
                    ("cross_count", Json::Raw(Value::from(self.cross_count))),
                    ("corrupt", flag(self.corrupt)),
                ]),
            ),
            (
                "current-turn",
                obj(vec![
                    (
                        "lifetime",
                        obj(vec![
                            // Authoritative when at least one peer is
                            // known through a trusted lifetime.
                            (
                                "state",
                                text(if by_lifetime {
                                    "authoritative"
                                } else {
                                    "unknown"
                                }),
                            ),
                            (
                                "peers",
                                per_peer(&|p| {
                                    text(if p.source == "lifetime" {
                                        "lifetime"
                                    } else {
                                        "untrusted"
                                    })
                                }),
                            ),
                        ]),
                    ),
                    (
                        "peers",
                        per_peer(&|p| {
                            let mut pairs =
                                vec![("execution", text(p.execution)), ("source", text(p.source))];
                            if let Some(reason) = p.reason.as_ref().filter(|r| !r.is_empty()) {
                                pairs.push(("reason", text(reason)));
                            }
                            pairs.push(("task_id", optional(&p.task_id)));
                            pairs.push((
                                "generation",
                                p.generation.as_ref().map_or(Json::Null, Json::raw),
                            ));
                            pairs.push(("turn_id", optional(&p.turn_id)));
                            pairs.push(("master_session_id", optional(&p.master)));
                            obj(pairs)
                        }),
                    ),
                    (
                        "threads",
                        Json::Arr(self.threads.iter().map(Thread::json).collect()),
                    ),
                ]),
            ),
            (
                "last-outcome",
                obj(vec![
                    ("peers", per_peer(&|p| p.last.json())),
                    ("note", text("最近终止结果;与 current-turn 分层显示,不混写")),
                ]),
            ),
            (
                "deliverables",
                obj(vec![
                    ("files", list(&self.deliverables)),
                    ("accepted", flag(false)),
                    ("note", text("deliverables 不凭文件存在宣称 accepted")),
                ]),
            ),
            (
                "ack_events",
                Json::Arr(self.ack_events.iter().map(AckEvent::json).collect()),
            ),
            (
                "events",
                Json::Arr(self.events.iter().map(Negative::json).collect()),
            ),
        ];
        if !self.diagnostics.is_empty() {
            pairs.push(("diagnostics", list(&self.diagnostics)));
        }
        obj(pairs)
    }

    fn human(&self) -> String {
        let yes_no = |b: bool| if b { "True" } else { "False" };
        let mut out = vec![format!(
            "== OctoLoop review monitor [{PROTOCOL}] == {}",
            self.identity
        )];
        out.push(format!(
            "[lifecycle] frozen={} challenge={} cross={}{}",
            yes_no(self.frozen),
            yes_no(self.challenge_accepted),
            self.cross_count,
            if self.corrupt {
                " CORRUPT(read-only)"
            } else {
                ""
            }
        ));
        let by_lifetime = self.peers.iter().any(|(_, p)| p.source == "lifetime");
        out.push(format!(
            "[current-turn] lifetime={}",
            if by_lifetime {
                "authoritative"
            } else {
                "unknown"
            }
        ));
        for (slug, peer) in &self.peers {
            let reason = match peer.reason.as_ref().filter(|r| !r.is_empty()) {
                Some(reason) => format!(" ({reason})"),
                None => String::new(),
            };
            out.push(format!(
                "  peer {slug}: current-turn={} [{}]{reason}",
                peer.execution, peer.source
            ));
        }
        if self.peers.is_empty() {
            out.push("  (no peers)".to_string());
            out.push("[last-outcome] (no peers yet)".to_string());
        }
        for (slug, peer) in &self.peers {
            let notes = match peer.last.notes.as_ref().filter(|n| !n.is_empty()) {
                Some(notes) => format!(" notes={}", notes.join(";")),
                None => String::new(),
            };
            out.push(format!(
                "  peer {slug}: last-outcome={} (最近终止, source={}){notes}",
                peer.last.state, peer.last.source
            ));
        }
        out.push(format!(
            "[deliverables] {} (accepted=false)",
            if self.deliverables.is_empty() {
                "(none)".to_string()
            } else {
                self.deliverables.join(", ")
            }
        ));
        for event in &self.ack_events {
            out.push(format!(
                "[ack-event] {} ack={} line={} {}",
                event.kind,
                event.ack,
                shown(&event.line),
                event.sha256
            ));
        }
        for event in &self.events {
            let goal = match Some(&event.goal_id).filter(|g| truthy(g)) {
                Some(goal) => format!(" goal={}", shown(goal)),
                None => String::new(),
            };
            let slug = match event.slug.as_ref().filter(|s| truthy(s)) {
                Some(slug) => format!(" slug={}", shown(slug)),
                None => String::new(),
            };
            out.push(format!(
                "[event] {}{goal}{slug} {} (runtime={})",
                event.kind, event.detail, event.runtime
            ));
        }
        for diagnostic in &self.diagnostics {
            out.push(format!("[diag] {diagnostic}"));
        }
        out.join("\n")
    }
}

/// `argv` is what follows `olp review monitor`.
pub fn run_cli(argv: &[String]) -> i32 {
    let prog = "olp review monitor";
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    let (mut watch, mut rest) = (false, Vec::new());
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        match name {
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            "--watch" if inline.is_none() => watch = true,
            "--runtime-dir" | "--board" | "--profile" | "--session" | "--format" | "--interval" => {
                let Some(value) = inline.or_else(|| words.next().cloned()) else {
                    eprintln!("{prog}: {name} 缺少取值\n{USAGE}");
                    return 2;
                };
                values.insert(name, value);
            }
            other if other.starts_with('-') && other.len() > 1 => {
                eprintln!("{prog}: 不认识的参数: {other}\n{USAGE}");
                return 2;
            }
            _ => rest.push(word.clone()),
        }
    }
    let [review_dir] = rest.as_slice() else {
        eprintln!("{USAGE}");
        return 2;
    };
    let as_json = match values.get("--format").map(String::as_str) {
        None | Some("human") => false,
        Some("json") => true,
        Some(other) => {
            eprintln!("{prog}: --format 只能是 json 或 human: {other}\n{USAGE}");
            return 2;
        }
    };
    let interval = match values.get("--interval").map(|v| v.parse::<f64>()) {
        None => 2.0,
        Some(Ok(seconds)) if seconds.is_finite() && seconds >= 0.0 => seconds,
        Some(_) => {
            eprintln!("{prog}: --interval 要一个不小于 0 的秒数\n{USAGE}");
            return 2;
        }
    };
    let given = |name: &str| values.get(name).filter(|v| !v.is_empty());
    let args = Args {
        review_dir: PathBuf::from(path_text(review_dir)),
        runtime: given("--runtime-dir").map(|dir| path_text(dir)),
        board: given("--board").map(PathBuf::from),
        profile: values
            .get("--profile")
            .cloned()
            .unwrap_or_else(|| DEFAULT_PROFILE.to_string()),
        session: values.get("--session").cloned(),
    };
    let rendered = || {
        let report = render(&args);
        if as_json {
            report.json().pretty(1)
        } else {
            report.human()
        }
    };
    // A reader that went away ends the view; it is not an error to report.
    let emit = |body: &str| writeln!(std::io::stdout().lock(), "{body}").is_ok();
    if !watch {
        return if emit(&rendered()) { 0 } else { 1 };
    }
    let stop = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::signal::SIGINT, Arc::clone(&stop));
    let mut shown_last: Option<String> = None;
    while !stop.load(Ordering::Relaxed) {
        let body = rendered();
        if shown_last.as_ref() != Some(&body) {
            if !emit(&body) {
                return 1;
            }
            shown_last = Some(body);
        }
        let mut left = Duration::from_secs_f64(interval);
        while !left.is_zero() && !stop.load(Ordering::Relaxed) {
            let step = left.min(Duration::from_millis(100));
            std::thread::sleep(step);
            left -= step;
        }
        if interval == 0.0 {
            std::thread::yield_now();
        }
    }
    0
}
