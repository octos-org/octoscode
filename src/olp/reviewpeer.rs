//! Is a review report what it says it is: written by this reviewer, in a
//! turn that really finished, by the model its lane names?
//!
//! A report's own `outcome: completed` proves nothing. It is believed
//! only with an outside authority: the peer's native directory (the
//! highest `result-N.md` is this peer's, completed, numbered as it says,
//! `turns.txt` agrees on that round, and the `originator` and `goal`
//! files bind it to this session and goal) or a runtime-evidence snapshot
//! of a terminated, completed peer of this session. The report's turn
//! must be exactly that latest round.
//!
//! Which model wrote a turn is read from the runtime's own ledger of the
//! peer's session, joined to the report by the runtime's turn id — never
//! by counting files.

use std::collections::BTreeMap;
use std::path::Path;

use super::reviewmonitor::{same_origin, trunk_and_cwd};
use super::reviewstate::{
    Fail, Frontmatter, J, LANES, Res, digits, fail, lines, obj, or_none, positive_turn, read_lossy,
    read_strict, repr_opt, resolve, sha256_bytes, show, strip, words,
};

/// The highest-numbered `result-N.md` in a peer's native directory.
fn latest_result(peer_dir: &Path) -> Option<(u64, String)> {
    let mut names: Vec<String> = std::fs::read_dir(peer_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut best: Option<(u64, String)> = None;
    for name in names {
        let round = name
            .strip_prefix("result-")
            .and_then(|rest| rest.strip_suffix(".md"))
            .and_then(digits);
        if let Some(round) = round {
            if best.as_ref().is_none_or(|(b, _)| round > *b) {
                best = Some((round, name));
            }
        }
    }
    best
}

/// `turns.txt` as round → outcome. None when a round is listed twice:
/// the index is then not to be trusted, whatever the two lines say.
fn turns_index(peer_dir: &Path) -> Option<BTreeMap<u64, String>> {
    let mut index = BTreeMap::new();
    let Ok(text) = read_lossy(&peer_dir.join("turns.txt")) else {
        return Some(index);
    };
    for line in lines(&text) {
        let parts = words(line);
        let Some(round) = parts.first().and_then(|p| digits(p)) else {
            continue;
        };
        let outcome = parts.get(1).copied().unwrap_or("").to_string();
        if index.insert(round, outcome).is_some() {
            return None;
        }
    }
    Some(index)
}

/// A peer's identity file names `expected`: the same text, or the same
/// originator once working-directory suffixes are accounted for.
fn identity_file_matches(peer_dir: &Path, name: &str, expected: &J) -> bool {
    let Ok(text) = read_lossy(&peer_dir.join(name)) else {
        return false;
    };
    expected
        .str()
        .is_some_and(|expected| strip(&text) == expected || same_origin(strip(&text), expected))
}

// ---------------------------------------------------------------------------
// the runtime's ledger: which turns a peer ran, and with which models
// ---------------------------------------------------------------------------

struct LedgerTurn {
    turn_no: u64,
    turn_id: String,
    models: Vec<String>,
    terminal: Option<String>,
    stream: String,
    sha256: String,
}

fn hex_name(name: &str) -> Option<String> {
    if name.len() % 2 != 0 || !name.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..name.len() / 2)
        .map(|i| u8::from_str_radix(&name[2 * i..2 * i + 2], 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// One peer's whole stream of turns. Empty when anything is missing or
/// unclear: a gap in the sequence, a malformed row, two streams for the
/// peer, a turn without its start.
fn ledger_turns(runtime_root: &Path, session: &str, slug: &str) -> Vec<LedgerTurn> {
    read_ledger(runtime_root, session, slug).unwrap_or_default()
}

fn read_ledger(runtime_root: &Path, session: &str, slug: &str) -> Option<Vec<LedgerTurn>> {
    if session.is_empty() || slug.is_empty() {
        return None;
    }
    let master = session.split_once('#').map_or(session, |(head, _)| head);
    let peer_session = format!("{master}#peer-{slug}");
    let mut streams = Vec::new();
    for entry in std::fs::read_dir(runtime_root.join("ui-protocol")).ok()? {
        let dir = entry.ok()?.path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir.file_name()?.to_string_lossy().into_owned();
        let Some(wire) = hex_name(&name) else {
            continue;
        };
        if trunk_and_cwd(&wire).0 != peer_session {
            continue;
        }
        let logs: Vec<_> = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
                name.is_some_and(|n| n.starts_with("ledger-") && n.ends_with(".log"))
            })
            .collect();
        if !logs.is_empty() {
            streams.push((wire, logs));
        }
    }
    let [(wire, logs)] = streams.as_slice() else {
        return None;
    };
    let mut rows: BTreeMap<i64, J> = BTreeMap::new();
    for log in logs {
        for line in lines(&read_strict(log).ok()?) {
            let row = J::parse(line).ok()?;
            let seq = row.at("seq").int().filter(|s| *s >= 1)?;
            let event = row.get("event").filter(|e| e.is_obj())?;
            if !row.is_obj() || rows.insert(seq, event.clone()).is_some() {
                return None;
            }
        }
    }
    if rows.is_empty() || !rows.keys().copied().eq(1..=rows.len() as i64) {
        return None;
    }
    struct Open {
        turn_id: String,
        models: Vec<String>,
        terminal: Option<String>,
        events: Vec<J>,
    }
    const ENDS: [&str; 3] = ["turn_completed", "turn_error", "turn_interrupted"];
    let mut turns: Vec<Open> = Vec::new();
    for (seq, ev) in &rows {
        let metadata = ev.at("metadata");
        let costs = metadata.is_obj() && metadata.at("kind").is("token_cost_update");
        let Some(turn_id) = ev.get("turn_id").filter(|t| !t.is_null()) else {
            // Envelope events may carry no turn; a turn's own events may not.
            let own = ev.at("kind").str();
            if own == Some("turn_started") || own.is_some_and(|k| ENDS.contains(&k)) || costs {
                return None;
            }
            continue;
        };
        let turn_id = turn_id.text()?;
        if !ev.at("session_id").is(&peer_session) {
            return None;
        }
        let kind = if ev.at("record_kind").is("notification") {
            ev.at("kind").str()
        } else {
            None
        };
        let started = kind == Some("turn_started");
        let known = turns.iter().position(|t| t.turn_id == turn_id);
        let at = match (known, started) {
            (None, true) => {
                turns.push(Open {
                    turn_id: turn_id.to_string(),
                    models: Vec::new(),
                    terminal: None,
                    events: Vec::new(),
                });
                turns.len() - 1
            }
            (Some(at), false) => at,
            // A turn that does not begin with its start, or starts twice.
            _ => return None,
        };
        let turn = &mut turns[at];
        let is_model = ev.at("record_kind").is("progress") && costs;
        let terminal = kind.filter(|k| ENDS.contains(k));
        if started || is_model || terminal.is_some() {
            if turn.terminal.is_some() {
                return None;
            }
            turn.events
                .push(obj(vec![("seq", J::from(*seq)), ("event", ev.clone())]));
        }
        if is_model {
            let model = metadata.at("token_cost").at("model").text()?.to_string();
            if !turn.models.contains(&model) {
                turn.models.push(model);
            }
        }
        if let Some(kind) = terminal {
            turn.terminal = Some(kind.to_string());
        }
    }
    Some(
        turns
            .into_iter()
            .enumerate()
            .map(|(at, turn)| {
                let evidence = obj(vec![
                    ("stream", J::from(wire.as_str())),
                    ("events", J::Arr(turn.events)),
                ]);
                LedgerTurn {
                    turn_no: at as u64 + 1,
                    turn_id: turn.turn_id,
                    models: turn.models,
                    terminal: turn.terminal,
                    stream: wire.clone(),
                    sha256: sha256_bytes(evidence.canonical().as_bytes()),
                }
            })
            .collect(),
    )
}

const REDO: &str = "请新建短 reviewer peer 并在新评审上下文重做初审与 cross";

fn unverified(message: String) -> Fail {
    fail("peer-model-unverified", message)
}

/// A model's name belongs to a lane: the lane's name, alone or followed
/// by `-` and a version.
fn model_of_lane(model: &str, lane: &str) -> bool {
    let Some(rest) = model.strip_prefix(lane) else {
        return false;
    };
    let Some(version) = rest.strip_prefix('-') else {
        return rest.is_empty();
    };
    version.starts_with(|c: char| c.is_ascii_alphanumeric())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// The ledger's evidence that turn `native_turn_id` of `slug` completed
/// and was written by a model of `lane`.
pub(super) fn model_from_ledger(
    runtime_root: &Path,
    session: &str,
    slug: &str,
    lane: &str,
    report_turn: u64,
    native_turn_id: &str,
) -> Res<J> {
    let turns = ledger_turns(runtime_root, session, slug);
    let mut matches = turns.iter().filter(|t| t.turn_id == native_turn_id);
    let (Some(turn), None) = (matches.next(), matches.next()) else {
        return Err(unverified(format!(
            "{slug} turn={report_turn} 无完整 runtime 轮次证据；若 ledger 前段已轮转丢失，请新建短 reviewer peer，在新评审上下文重做初审与 cross；不得将剩余尾段重新编号"
        )));
    };
    // result-N counts files that were written; only the runtime's turn id
    // joins a report to the ledger.
    if turn.terminal.as_deref() != Some("turn_completed") || turn.models.is_empty() {
        return Err(unverified(format!(
            "{slug} turn={report_turn} 未完成或缺实际模型"
        )));
    }
    let models = J::Arr(turn.models.iter().map(|m| J::from(m.as_str())).collect());
    if !LANES.contains(&lane) || turn.models.iter().any(|m| !model_of_lane(m, lane)) {
        return Err(fail(
            "peer-model-mismatch",
            format!("{slug} lane={lane},实际模型={}", models.repr()),
        ));
    }
    Ok(obj(vec![
        ("model", models),
        ("turn_id", J::from(turn.turn_id.as_str())),
        ("turn_no", J::from(report_turn)),
        ("ledger_turn_no", J::from(turn.turn_no)),
        ("stream", J::from(turn.stream.as_str())),
        ("sha256", J::from(turn.sha256.as_str())),
    ]))
}

/// Where a report's native `result-N.md` is and what it hashes to, taken
/// when the report is recorded. None without a native root (a snapshot
/// alone is an audit trail, not a binding) or when the file is not there.
pub fn capture_native_report(native_root: Option<&Path>, slug: &str, turn: &J) -> Res<J> {
    let Some(root) = native_root else {
        return Ok(J::Null);
    };
    let Some(number) = positive_turn(turn) else {
        return Err(unverified("原生报告编号无效".to_string()));
    };
    let path = resolve(&root.join(slug).join(format!("result-{number}.md")));
    match std::fs::read(&path) {
        Ok(bytes) => Ok(obj(vec![
            ("path", J::from(show(&path))),
            ("sha256", J::from(sha256_bytes(&bytes))),
        ])),
        // Another lane may be the only one with native files. Nothing is
        // inferred for this one; model verification refuses it later.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(J::Null),
        Err(_) => Err(unverified(format!("{slug} 原生报告不可读取"))),
    }
}

/// The runtime's turn id in the native report a binding points at. The
/// report must be the file that was bound, unchanged.
fn native_turn_id(binding: &J, slug: &str, number: u64) -> Res<String> {
    let (Some(path), Some(sha256)) = (binding.at("path").text(), binding.at("sha256").text())
    else {
        return Err(unverified(format!("{slug} 缺冻结的原生报告绑定；{REDO}")));
    };
    let path = Path::new(path);
    let unreadable = || unverified(format!("{slug} 原生报告不可读取"));
    let raw = std::fs::read(path).map_err(|_| unreadable())?;
    let named = path.file_name().and_then(|n| n.to_str()) == Some(&format!("result-{number}.md"));
    if !named || sha256_bytes(&raw) != sha256 {
        return Err(unverified(format!("{slug} 原生报告已变更或编号不匹配")));
    }
    // The identity is read from the very bytes that were hashed.
    let text = String::from_utf8(raw).map_err(|_| unreadable())?;
    let fm = Frontmatter::of_text(&text, String::new());
    if fm.missing {
        return Err(unverified(format!("{slug} 原生报告缺 frontmatter")));
    }
    if fm.repeated {
        return Err(unverified(format!("{slug} 原生报告身份键重复")));
    }
    if fm.get("slug") != Some(slug)
        || fm.get("outcome") != Some("completed")
        || fm.get("turn") != Some(number.to_string().as_str())
    {
        return Err(unverified(format!("{slug} 原生报告身份/编号/终态不匹配")));
    }
    match fm.text("turn_id") {
        Some(id) => Ok(id.to_string()),
        None => Err(unverified(format!(
            "{slug} 原生报告缺 turn_id；请升级 octos，新建短 reviewer peer 并在新评审上下文重做初审与 cross；不得按文件编号补 ID"
        ))),
    }
}

fn whole_number(value: &J) -> Option<i64> {
    strip(&value.shown()).parse().ok()
}

/// The ledger evidence for a reviewer's first review and its later cross
/// report: both by the lane's model, the cross in a later turn.
pub fn model_pair(state: &J, slug: &J, cross_turn: &J, cross_native_report: &J) -> Res<J> {
    let reviews = state.at("reviews");
    let lanes: Vec<&str> = LANES
        .into_iter()
        .filter(|lane| {
            let review = reviews.at(lane);
            review.is_obj() && !slug.is_null() && review.at("peer").same(slug)
        })
        .collect();
    let (Some(runtime), Some(session)) = (state.at("runtime").text(), state.at("session").text())
    else {
        return Err(unverified("缺 runtime/session/reviewer 绑定".to_string()));
    };
    let ([lane], Some(slug)) = (lanes.as_slice(), slug.str()) else {
        return Err(unverified("缺 runtime/session/reviewer 绑定".to_string()));
    };
    let review = reviews.at(lane);
    let (Some(first), Some(later)) = (whole_number(review.at("turn")), whole_number(cross_turn))
    else {
        return Err(unverified("报告轮次无效".to_string()));
    };
    if first < 1 || later <= first {
        return Err(unverified("cross 必须属于初审后的新轮次".to_string()));
    }
    let (first, later) = (first as u64, later as u64);
    let runtime = Path::new(runtime);
    let initial_id = native_turn_id(review.at("native_report"), slug, first)?;
    let cross_id = native_turn_id(cross_native_report, slug, later)?;
    let initial = model_from_ledger(runtime, session, slug, lane, first, &initial_id)?;
    let cross = model_from_ledger(runtime, session, slug, lane, later, &cross_id)?;
    if cross.at("ledger_turn_no").int() <= initial.at("ledger_turn_no").int() {
        return Err(unverified(
            "cross 的 runtime turn_id 必须晚于初审，不得复用旧轮".to_string(),
        ));
    }
    Ok(obj(vec![("initial", initial), ("cross", cross)]))
}

/// Both reviewers' latest cross reports still carry model evidence that
/// the ledger confirms now.
pub fn models_verified(state: &J) -> bool {
    let reviews = state.at("reviews");
    let Some(pairs) = reviews.pairs() else {
        return false;
    };
    let lanes_only = pairs.len() == LANES.len() && LANES.iter().all(|lane| reviews.has(lane));
    let peers: Vec<&str> = pairs
        .iter()
        .filter_map(|(_, r)| r.at("peer").text())
        .collect();
    if !lanes_only || peers.len() != 2 || peers[0] == peers[1] {
        return false;
    }
    let empty = Vec::new();
    let cross = match state.at("cross") {
        J::Null => &empty,
        J::Arr(items) => items,
        _ => return false,
    };
    // The history of cross reports only grows: a later verified one
    // replaces an old one, and a later unverified one takes its place too.
    let mut latest: Vec<(&str, &J)> = Vec::new();
    for entry in cross {
        let Some(slug) = entry.at("slug").text().filter(|_| entry.is_obj()) else {
            return false;
        };
        match latest.iter_mut().find(|(s, _)| *s == slug) {
            Some(slot) => slot.1 = entry,
            None => latest.push((slug, entry)),
        }
    }
    if latest.len() != 2 || !latest.iter().all(|(slug, _)| peers.contains(slug)) {
        return false;
    }
    latest.iter().all(|(_, entry)| {
        model_pair(
            state,
            entry.at("slug"),
            entry.at("turn"),
            entry.at("native_report"),
        )
        .is_ok_and(|pair| entry.at("model_evidence").same(&pair))
    })
}

// ---------------------------------------------------------------------------
// a report's validity
// ---------------------------------------------------------------------------

/// What a report is checked against.
pub struct Authority<'a> {
    pub native_dir: Option<&'a Path>,
    pub runtime_evidence: Option<&'a J>,
    pub slug: &'a str,
    /// Refuse a report that no outside authority vouches for.
    pub require: bool,
    /// The report's turn must be later than this one (a cross report
    /// after the frozen first review).
    pub after_turn: u64,
    pub session: &'a J,
    pub goal: &'a J,
}

/// Refuse a report that is pending, errored, stale, from the future or
/// somebody else's. Returns whether an outside authority vouched for it.
pub fn check_peer_validity(fm: &Frontmatter, a: &Authority) -> Res<bool> {
    let slug = a.slug;
    let invalid = |message: String| fail("peer-outcome-invalid", message);
    if fm.get("outcome") != Some("completed") {
        return Err(invalid(format!(
            "{slug} 自报 outcome={}(非 completed),不得作为有效初审/交叉",
            or_none(fm.get("outcome"))
        )));
    }
    let Some(report_turn) = fm.get("turn").and_then(digits) else {
        return Err(invalid(format!(
            "{slug} 报告 turn={} 缺失/非数字,无法与权威轮次对账",
            repr_opt(fm.get("turn"))
        )));
    };
    let mut vouched = false;

    // A snapshot of the runtime: a terminated peer of this session.
    let snapshots = a.runtime_evidence.and_then(|e| e.at("peers").items());
    for peer in snapshots.into_iter().flatten() {
        if !peer.at("slug").is(slug) {
            continue;
        }
        if peer.at("active_thread").truthy() {
            return Err(invalid(format!(
                "{slug} runtime-evidence active_thread 非空(仍有活跃轮次),报告声明不可采信"
            )));
        }
        let ours = peer.at("status").is("terminated")
            && !a.session.is_null()
            && peer.at("session").same(a.session)
            && (peer.at("goal").is_null() || peer.at("goal").same(a.goal));
        if !ours {
            continue;
        }
        // As strict as the native path: completed, and of a round that
        // can be checked.
        if !peer.at("outcome").is("completed") {
            return Err(invalid(format!(
                "{slug} runtime-evidence 终止快照 outcome={}(非 completed),不构成终止权威",
                peer.at("outcome").shown()
            )));
        }
        let source = peer.at("source");
        let source_text = if source.truthy() {
            source.shown()
        } else {
            String::new()
        };
        let round = source_text
            .strip_prefix("result-")
            .and_then(|rest| rest.strip_suffix(".md"))
            .and_then(digits);
        let Some(round) = round else {
            return Err(invalid(format!(
                "{slug} runtime-evidence 终止快照缺可核验轮次来源(source={}),不得当终止权威",
                source.repr()
            )));
        };
        if report_turn < round {
            return Err(fail(
                "stale-turn",
                format!(
                    "{slug} 报告 turn={report_turn} 低于快照来源轮次 {round}(result-{round}.md)"
                ),
            ));
        }
        if report_turn > round {
            return Err(fail(
                "turn-mismatch",
                format!("{slug} 报告 turn={report_turn} 超过快照来源轮次 {round}(未来轮次)"),
            ));
        }
        vouched = true;
    }

    // The peer's native directory.
    let native = a.native_dir.filter(|dir| dir.exists());
    if let Some((dir, (n, name))) = native.and_then(|dir| Some((dir, latest_result(dir)?))) {
        let native_fm = Frontmatter::read(&dir.join(&name))?;
        if native_fm.get("slug") != Some(slug) {
            // A receipt without a slug is as foreign as one with another.
            return Err(fail(
                "peer-authority-mismatch",
                format!(
                    "{slug} native {name} slug={} 缺失/不匹配",
                    repr_opt(native_fm.get("slug"))
                ),
            ));
        }
        if native_fm.get("outcome") != Some("completed") {
            return Err(invalid(format!(
                "{slug} native 最高编号 {name} outcome={}(非 completed),不构成终止权威",
                or_none(native_fm.get("outcome"))
            )));
        }
        if native_fm.get("turn").and_then(digits) != Some(n) {
            return Err(invalid(format!(
                "{slug} native {name} turn={} 与编号 {n} 不一致",
                or_none(native_fm.get("turn"))
            )));
        }
        let Some(index) = turns_index(dir).filter(|index| !index.is_empty()) else {
            return Err(invalid(format!(
                "{slug} native turns.txt 缺失/为空/同轮重复,轮次索引不完整"
            )));
        };
        let last = index.keys().next_back().copied().unwrap_or(0);
        if n < last {
            return Err(invalid(format!(
                "{slug} native 最高 result-{n} 低于 turns.txt 已结束轮次 {last}(turns 索引不严格)"
            )));
        }
        let Some(outcome) = index.get(&n) else {
            return Err(invalid(format!(
                "{slug} turns.txt 缺最新轮 {n} 的记录(缺项不构成终止权威)"
            )));
        };
        if outcome != "completed" {
            return Err(invalid(format!(
                "{slug} turns.txt 第 {n} 轮 outcome={outcome} 与 {name} completed 冲突(同轮必须精确 completed)"
            )));
        }
        if report_turn < n {
            return Err(fail(
                "stale-turn",
                format!("{slug} 报告 turn={report_turn} 低于 native 最新轮次 {n}"),
            ));
        }
        if report_turn > n {
            return Err(fail(
                "turn-mismatch",
                format!("{slug} 报告 turn={report_turn} 超过 native 最新轮次 {n}(未来轮次)"),
            ));
        }
        // A native root from elsewhere cannot stand in for this session.
        if !a.session.is_null() && !identity_file_matches(dir, "originator", a.session) {
            return Err(fail(
                "peer-authority-mismatch",
                format!(
                    "{slug} native originator 与当前 session 视角不符/缺失(外来 native-root 不得冒充)"
                ),
            ));
        }
        if !a.goal.is_null() && !identity_file_matches(dir, "goal", a.goal) {
            return Err(fail(
                "peer-authority-mismatch",
                format!("{slug} native goal 与当前 goal 不符/缺失(跨 goal 收据不得作权威)"),
            ));
        }
        vouched = true;
    }
    if a.after_turn > 0 && report_turn <= a.after_turn {
        return Err(fail(
            "stale-turn",
            format!(
                "{slug} 报告 turn={report_turn} 不晚于冻结轮次 {}(旧初审不得冒充 cross)",
                a.after_turn
            ),
        ));
    }
    if a.require && !vouched {
        return Err(fail(
            "peer-authority-missing",
            format!(
                "{slug} 无外部终止权威(native result-N 或 runtime-evidence 终止快照), 仅自报 outcome 不足以采信"
            ),
        ));
    }
    Ok(vouched)
}
