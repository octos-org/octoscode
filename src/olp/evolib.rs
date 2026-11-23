//! What the evolution loop's tools share: reading the evolution board's
//! cards, grouping them into candidates, the hub guard, and JSON written
//! the way the state files have always been written.
//!
//! One place for parsing, grouping text, normalisation, anchors and layer
//! hints, so the retro and the metrics cannot drift apart.

use std::sync::LazyLock;

use fancy_regex::Regex;
use serde_json::Value;

/// Triggers whose symptom is a line from the review board.
const BOARD_TRIGGERS: [&str; 6] = [
    "ack_blocked",
    "ack_wontdo",
    "override",
    "r2_record",
    "returned",
    "unit_alert",
];
/// Triggers whose symptom is an event record.
const EVENTS_TRIGGERS: [&str; 6] = [
    "escalation",
    "turn_error",
    "goal_blocked",
    "goal_budget_limited",
    "fallback_switch",
    "malformed_exhausted",
];

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a pattern written in this file")
}

static SIGNED_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| re(r"^外环\([^)]*\)·(?:改判|R2 记档|打回)\([^)]*\)[:：]\s*"));
// `内环(<unit>)·#N ACK(blocked): …` and `单元(<unit>)·负哨(#N): …`: without
// the signature the same symptom from two units groups together.
static INNER_ACK_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^内环\([^)]*\)·(?:#\d+\s*)?ACK\((?:blocked|wontdo)\)(?:\([^)]*\))?[:：]\s*(?:#\d+(?:\([^)]*\))?[:：]?\s*)?",
    )
});
static UNIT_ALERT_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| re(r"^单元\([^)]*\)·负哨\([^)]*\)[:：]\s*"));
static ALERT_DETAIL: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:turn|seq|pid|age)=\S+\s*"));
static LANES: LazyLock<Regex> = LazyLock::new(|| re(r"(\S+) -> (\S+)"));
static LATENCY: LazyLock<Regex> = LazyLock::new(|| re(r",\s*\d+ms"));
static PATH: LazyLock<Regex> = LazyLock::new(|| re(r"(?<![\w])/[^\s]+"));
static HEX: LazyLock<Regex> = LazyLock::new(|| re(r"(?<![A-Za-z\d])[0-9a-f]{8,}(?![A-Za-z\d])"));
static NUM: LazyLock<Regex> = LazyLock::new(|| re(r"(?<![A-Za-z_\d])\d+(?![A-Za-z\d])"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));
static FAILOVER: LazyLock<Regex> = LazyLock::new(|| re(r"router failover: (\S+) -> (\S+)"));

/// One card on the evolution board. A field the card lacks is `None`; the
/// caller decides what that means.
#[derive(Clone)]
pub struct Card {
    pub id: String,
    pub num: u64,
    pub trigger: Option<String>,
    pub identity: Option<String>,
    pub symptom: Option<String>,
    pub source: String,
    pub envelope: String,
}

pub fn parse_cards(text: &str) -> Vec<Card> {
    let mut cards: Vec<Card> = Vec::new();
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix("### EVO-") {
            let num = rest.split('（').next().unwrap_or_default();
            cards.push(Card {
                id: format!("EVO-{num}"),
                num: if !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()) {
                    num.parse().unwrap_or(0)
                } else {
                    0
                },
                trigger: None,
                identity: None,
                symptom: None,
                source: "-".to_string(),
                envelope: "-".to_string(),
            });
            continue;
        }
        let Some(card) = cards.last_mut() else {
            continue;
        };
        let value = |field: &str| line.strip_prefix(field).map(|v| v.trim().to_string());
        if let Some(v) = value("trigger:") {
            card.trigger = Some(v);
        }
        if let Some(v) = value("identity:") {
            card.identity = Some(v);
        }
        if let Some(v) = value("symptom:") {
            card.symptom = Some(v);
        }
        if let Some(v) = value("source:") {
            card.source = v;
        }
        if let Some(v) = value("envelope:") {
            card.envelope = v;
        }
    }
    cards
}

/// A JSON value as text the way the old tools read it: a string is itself.
fn plain(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Is the value one a `x or y` would skip?
fn empty(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => s.is_empty(),
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        Value::Bool(true) => false,
    }
}

/// `data.detail`, else `detail`, of an event record. `Err` when the symptom
/// is not such a record.
fn event_detail(symptom: &str) -> Result<Option<Value>, ()> {
    let record: Value = serde_json::from_str(symptom).map_err(|_| ())?;
    let Value::Object(map) = &record else {
        return Ok(None);
    };
    let nested = match map.get("data") {
        None => None,
        Some(Value::Object(data)) => data.get("detail").filter(|d| !empty(d)),
        Some(_) => return Err(()),
    };
    Ok(nested
        .or(map.get("detail"))
        .filter(|d| !d.is_null())
        .cloned())
}

/// The part of a card's symptom that says what went wrong, without who
/// said it or about which card.
pub fn group_text(card: &Card) -> String {
    let trigger = card.trigger.as_deref().unwrap_or("");
    let symptom = card.symptom.as_deref().unwrap_or("");
    if BOARD_TRIGGERS.contains(&trigger) {
        let mut t = symptom;
        loop {
            if let Some(rest) = t.strip_prefix("> ") {
                t = rest;
            } else if let Some(rest) = t.strip_prefix("**") {
                t = rest;
            } else {
                break;
            }
        }
        let mut t = t.trim_start();
        for prefix in ["ACK(blocked):", "ACK(wontdo):"] {
            if let Some(rest) = t.strip_prefix(prefix) {
                t = rest;
                break;
            }
        }
        for prefix in [&*SIGNED_PREFIX, &*INNER_ACK_PREFIX, &*UNIT_ALERT_PREFIX] {
            if let Ok(Some(found)) = prefix.find(t) {
                t = &t[found.end()..];
                break;
            }
        }
        if trigger == "unit_alert" {
            // The alert kind and its state are the symptom; which turn, pid
            // or sequence number, and how long it had lasted, are not.
            return ALERT_DETAIL.replace_all(t, "").trim().to_string();
        }
        return t.to_string();
    }
    if EVENTS_TRIGGERS.contains(&trigger) {
        if let Ok(Some(detail)) = event_detail(symptom) {
            let mut detail = plain(&detail);
            if trigger == "fallback_switch" {
                // The same session over different lanes groups together.
                detail = LANES.replace_all(&detail, "<lane> -> <lane>").into_owned();
                detail = LATENCY.replace_all(&detail, "").into_owned();
            }
            return detail;
        }
        return symptom.to_string();
    }
    // report_blocked / ask_outer_timeout
    match symptom.find("reason=") {
        Some(at) => symptom[at + "reason=".len()..].to_string(),
        None => symptom.to_string(),
    }
}

pub fn normalize(text: &str) -> String {
    let t = text.to_lowercase();
    let t = PATH.replace_all(&t, "<path>");
    let t = HEX.replace_all(&t, "<hex>");
    let t = NUM.replace_all(&t, "<num>");
    let t = SPACES.replace_all(&t, " ");
    t.trim().chars().take(80).collect()
}

/// The session an events card is about: the record's own `session`, else
/// the segment of the identity between its third and its last `#`.
fn events_session(identity: &str, card: &Card) -> Option<String> {
    let record = serde_json::from_str::<Value>(card.symptom.as_deref().unwrap_or("")).ok();
    if let Some(session) = record
        .as_ref()
        .and_then(|r| r.as_object())
        .and_then(|r| r.get("session"))
        .filter(|s| !empty(s))
    {
        return Some(plain(session));
    }
    let body = identity.strip_prefix("events:").unwrap_or(identity);
    let third = body.match_indices('#').nth(2).map(|(at, _)| at)?;
    let last = body.rfind('#')?;
    (last > third + 1)
        .then(|| &body[third + 1..last])
        .filter(|segment| *segment != "-")
        .map(str::to_string)
}

/// What distinguishes one occurrence from another of the same symptom.
pub fn anchor(card: &Card) -> String {
    let trigger = card.trigger.as_deref().unwrap_or("");
    let identity = card.identity.as_deref().unwrap_or("");
    let fallback = if card.id.is_empty() {
        "EVO-0000".to_string()
    } else {
        card.id.clone()
    };
    if trigger == "fallback_switch" {
        // The lanes come from the raw detail: grouping normalises them, but
        // the anchor tells a->b from b->c.
        let symptom = card.symptom.as_deref().unwrap_or("");
        let record = serde_json::from_str::<Value>(symptom).ok();
        let detail = record
            .as_ref()
            .and_then(|r| r.as_object())
            .and_then(|r| match r.get("data") {
                None => r.get("detail").filter(|d| !empty(d)),
                Some(Value::Object(data)) => data
                    .get("detail")
                    .filter(|d| !empty(d))
                    .or(r.get("detail").filter(|d| !empty(d))),
                Some(_) => None,
            })
            .map(plain)
            .unwrap_or_default();
        let detail = if detail.is_empty() {
            symptom.to_string()
        } else {
            detail
        };
        let Some(session) = events_session(identity, card) else {
            return fallback;
        };
        return match FAILOVER.captures(&detail) {
            Ok(Some(lanes)) => format!("{session}|{}->{}", &lanes[1], &lanes[2]),
            _ => session,
        };
    }
    let (kind, body) = ["board:", "events:", "mcp:"]
        .iter()
        .find_map(|prefix| identity.strip_prefix(prefix).map(|rest| (*prefix, rest)))
        .unwrap_or(("", identity));
    let parts: Vec<&str> = body.split('#').collect();
    let back = if kind == "board:" { 3 } else { 2 };
    let segment = if parts.len() >= back {
        parts[parts.len() - back]
    } else {
        "-"
    };
    // "-1" is the harvest's mark for "no heading".
    if matches!(segment, "-" | "" | "-1") {
        fallback
    } else {
        segment.to_string()
    }
}

/// Which layer a trigger points at.
pub fn layer(trigger: &str) -> &'static str {
    match trigger {
        "ack_blocked" | "ack_wontdo" | "goal_blocked" | "goal_budget_limited" | "escalation" => {
            "Lifecycle"
        }
        "report_blocked" | "ask_outer_timeout" | "malformed_exhausted" => "Tooling",
        "turn_error" | "fallback_switch" | "unit_alert" => "Execution",
        "override" => "Governance",
        "r2_record" | "returned" => "Verification",
        _ => "Observability",
    }
}

/// Cards that are one candidate: the same trigger and normalised symptom.
pub struct Candidate {
    pub key: String,
    pub trigger: String,
    pub layer: &'static str,
    /// Distinct anchors, in the order they were first seen.
    pub anchors: Vec<String>,
    pub cards: Vec<Card>,
}

impl Candidate {
    /// How many distinct anchors: a hint, not a count across goals.
    pub fn recurrence_hint(&self) -> usize {
        self.anchors.len()
    }
}

pub fn group(cards: &[Card]) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for card in cards {
        let trigger = card.trigger.clone().unwrap_or_default();
        let key = format!("{trigger}|{}", normalize(&group_text(card)));
        let at = match out.iter().position(|c| c.key == key) {
            Some(at) => at,
            None => {
                out.push(Candidate {
                    key,
                    layer: layer(&trigger),
                    trigger,
                    anchors: Vec::new(),
                    cards: Vec::new(),
                });
                out.len() - 1
            }
        };
        let candidate = &mut out[at];
        let anchor = anchor(card);
        if !candidate.anchors.contains(&anchor) {
            candidate.anchors.push(anchor);
        }
        candidate.cards.push(card.clone());
    }
    out
}

/// A JSON value with its keys in the order given.
pub enum Json {
    Null,
    Int(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
    /// A value read from a file, written back as it was.
    Raw(Value),
}

impl Json {
    fn from(value: &Value) -> Json {
        match value {
            Value::Null => Json::Null,
            Value::String(s) => Json::Str(s.clone()),
            Value::Number(n) if n.is_i64() => Json::Int(n.as_i64().unwrap_or(0)),
            Value::Array(items) => Json::Arr(items.iter().map(Json::from).collect()),
            Value::Object(map) => Json::Obj(
                map.iter()
                    .map(|(k, v)| (k.clone(), Json::from(v)))
                    .collect(),
            ),
            other => Json::Raw(other.clone()),
        }
    }

    fn string(text: &str, ascii: bool, out: &mut String) {
        out.push('"');
        for c in text.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c if ascii && !c.is_ascii() => {
                    let mut units = [0u16; 2];
                    for unit in c.encode_utf16(&mut units) {
                        out.push_str(&format!("\\u{unit:04x}"));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }

    fn write(&self, ascii: bool, indent: Option<usize>, depth: usize, out: &mut String) {
        let pad = |out: &mut String, depth: usize| {
            if let Some(width) = indent {
                out.push('\n');
                out.push_str(&" ".repeat(width * depth));
            }
        };
        let between = if indent.is_some() { "," } else { ", " };
        match self {
            Json::Null => out.push_str("null"),
            Json::Int(n) => out.push_str(&n.to_string()),
            Json::Str(s) => Json::string(s, ascii, out),
            Json::Raw(value) => out.push_str(&value.to_string()),
            Json::Arr(items) if items.is_empty() => out.push_str("[]"),
            Json::Obj(pairs) if pairs.is_empty() => out.push_str("{}"),
            Json::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(between);
                    }
                    pad(out, depth + 1);
                    item.write(ascii, indent, depth + 1, out);
                }
                pad(out, depth);
                out.push(']');
            }
            Json::Obj(pairs) => {
                out.push('{');
                for (i, (key, value)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(between);
                    }
                    pad(out, depth + 1);
                    Json::string(key, ascii, out);
                    out.push_str(": ");
                    value.write(ascii, indent, depth + 1, out);
                }
                pad(out, depth);
                out.push('}');
            }
        }
    }

    /// On one line, non-ASCII escaped: how the state files are written.
    pub fn line(&self) -> String {
        let mut out = String::new();
        self.write(true, None, 0, &mut out);
        out
    }

    /// Indented, non-ASCII kept: how reports are printed.
    pub fn pretty(&self, indent: usize) -> String {
        let mut out = String::new();
        self.write(false, Some(indent), 0, &mut out);
        out
    }

    pub fn raw(value: &Value) -> Json {
        Json::from(value)
    }
}

/// First label of a host name, lowercase: `Hub-01.local` is `hub-01`.
fn host_label(name: &str) -> String {
    name.split('.').next().unwrap_or_default().to_lowercase()
}

/// On a multi-machine deployment the evolution loop's writers run on the
/// hub only: their counters and cursors are per-machine state. With no hub
/// configured (a single machine) this lets everything through. `Err` is the
/// message for a machine that is not the hub.
pub fn hub_guard(who: &str) -> Result<(), String> {
    let configured = std::env::var("OLP_HUB_HOST")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            let file = std::env::var_os("OLP_HUB_HOST_FILE")
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                        .join(".octos/outer/hub-host")
                });
            std::fs::read_to_string(file).ok().and_then(|text| {
                text.lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty() && !l.starts_with('#'))
                    .map(str::to_string)
            })
        });
    let Some(hub) = configured else {
        return Ok(());
    };
    let said = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|name| !name.is_empty())
    };
    let here = said("hostname", &[]).or_else(|| said("uname", &["-n"]));
    if here.as_deref().map(host_label) == Some(host_label(&hub)) {
        return Ok(());
    }
    Err(format!(
        "error: {who}: 本机({})不是 hub({hub}),未写入任何内容。\n  进化环的采集与 retro 只在 hub 上运行;在 hub 上执行,或用 --dry-run 只读查看。",
        here.as_deref().unwrap_or("未知")
    ))
}
