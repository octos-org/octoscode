//! Reading the blackboard: which lines count, and the line-start forms the
//! tools act on. One implementation for every command (the scripts each
//! carried their own, and they drifted).

use std::collections::BTreeMap;

pub const ARCHIVED_MARK: &str = "<!-- olp-archived: ";

/// The board's lines with their 1-based numbers, without line endings.
pub fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_inclusive('\n')
        .enumerate()
        .map(|(i, raw)| (i + 1, raw.trim_end_matches(['\r', '\n'])))
}

/// Lines outside code fences. A fence closes only on a line made of its own
/// character, at least as long as the opening run: a `~~~` inside a ```
/// example is example text.
pub fn markdown_lines<'a>(
    numbered: impl Iterator<Item = (usize, &'a str)>,
) -> Vec<(usize, &'a str)> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    for (no, line) in numbered {
        let stripped = line.trim_start();
        if let Some(open) = fence {
            let mark = open.chars().next().unwrap_or('`');
            if stripped.starts_with(open) && stripped.trim_end().chars().all(|c| c == mark) {
                fence = None;
            }
            continue;
        }
        let run = |mark: char| stripped.len() - stripped.trim_start_matches(mark).len();
        let opening = [run('`'), run('~')].into_iter().find(|n| *n >= 3);
        if let Some(n) = opening {
            fence = Some(&stripped[..n]);
            continue;
        }
        out.push((no, line));
    }
    out
}

/// Lines that are live: a `## Historical record` section that is closed by
/// a later `## Active` is audit-only and skipped. One that runs to the end
/// of the file is kept — older writers appended live cards after it.
pub fn active_lines(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut history: Option<Vec<(usize, &str)>> = None;
    for (no, line) in markdown_lines(lines(text)) {
        let heading = line.trim_end().to_lowercase();
        if let Some(held) = history.as_mut() {
            if heading != "## active" {
                held.push((no, line));
                continue;
            }
            history = None;
        } else if heading == "## historical record" {
            history = Some(vec![(no, line)]);
            continue;
        }
        out.push((no, line));
    }
    if let Some(held) = history {
        out.extend(held);
    }
    out
}

/// Drop leading `> `, `**` and whitespace: signed lines are usually quoted.
pub fn strip_quote(line: &str) -> &str {
    let mut s = line.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("> ") {
            s = rest;
        } else if let Some(rest) = s.strip_prefix('>') {
            s = rest;
        } else if let Some(rest) = s.strip_prefix("**") {
            s = rest;
        } else {
            break;
        }
        s = s.trim_start();
    }
    s
}

/// Leading ASCII digits as a number, and what follows them.
fn number(s: &str) -> Option<(u64, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let value = s[..end].parse().ok()?;
    Some((value, &s[end..]))
}

/// `<who>(<name>)·` at the start: the name and what follows.
fn signature<'a>(s: &'a str, who: &str) -> Option<(&'a str, &'a str)> {
    let rest = s.strip_prefix(who)?.strip_prefix('(')?;
    let end = rest.find(')')?;
    if end == 0 {
        return None;
    }
    Some((&rest[..end], rest[end + 1..].strip_prefix('·')?))
}

/// `(#<N>)` at the start.
fn card_ref(s: &str) -> Option<(u64, &str)> {
    let (n, rest) = number(s.strip_prefix("(#")?)?;
    Some((n, rest.strip_prefix(')')?))
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Ack {
    Done,
    Wontdo,
    Blocked,
}

/// `ACK(done|wontdo|blocked)` at the start, and what follows.
fn ack(s: &str) -> Option<(Ack, &str)> {
    let rest = s.strip_prefix("ACK(")?;
    for (word, kind) in [
        ("done)", Ack::Done),
        ("wontdo)", Ack::Wontdo),
        ("blocked)", Ack::Blocked),
    ] {
        if let Some(rest) = rest.strip_prefix(word) {
            return Some((kind, rest));
        }
    }
    None
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EventKind {
    Accepted,
    Returned,
    /// The text after `搁置(#N)`.
    Snooze(String),
    Voided,
    /// The unit that pushed.
    Pushed(String),
    /// A signed ACK that names an entry the board does not have.
    Dangling,
    /// The unit took the card (`进度(#N): claimed`).
    Claimed,
    /// The unit's watcher could not hand the card to its pane
    /// (`进度(#N): undelivered`).
    Undelivered,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Event {
    pub line: usize,
    pub kind: EventKind,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Entry {
    pub title: String,
    pub line: usize,
    /// The last `unit:` line.
    pub unit: Option<String>,
    pub implementer: Option<String>,
    pub review: Vec<String>,
    /// The line of the archive stub's mark, when the entry is a stub.
    pub archive_line: Option<usize>,
    pub acks: Vec<(usize, Ack)>,
}

#[derive(Default, Debug)]
pub struct Scan {
    pub entries: BTreeMap<u64, Entry>,
    pub events: BTreeMap<u64, Vec<Event>>,
    pub warnings: Vec<String>,
}

/// `### <N>[.．] <title>`.
fn heading(line: &str) -> Option<(u64, &str)> {
    let (n, rest) = number(line.strip_prefix("### ")?)?;
    let rest = rest
        .strip_prefix('.')
        .or_else(|| rest.strip_prefix('．'))
        .unwrap_or(rest);
    Some((n, rest.trim()))
}

/// Whether a line (already through [`strip_quote`]) is one of the signed
/// line-start forms the tools read: an outer verdict, snooze or override, a
/// unit's push record, or a signed inner ACK.
pub fn is_signed_form(signed: &str) -> bool {
    if let Some((_, rest)) = signature(signed, "外环") {
        let verdict = ["采认", "打回", "搁置"]
            .iter()
            .any(|word| rest.strip_prefix(word).and_then(card_ref).is_some());
        let voided = rest
            .strip_prefix("改判(作废 #")
            .and_then(number)
            .is_some_and(|(_, rest)| rest.starts_with(')'));
        return verdict || voided;
    }
    if let Some((_, rest)) = signature(signed, "单元") {
        return rest
            .strip_prefix("本地复验通过")
            .and_then(card_ref)
            .is_some();
    }
    if let Some((_, rest)) = signature(signed, "内环") {
        let rest = match rest.strip_prefix('#').and_then(number) {
            Some((_, rest)) => rest.trim_start(),
            None => rest,
        };
        return ack(rest).is_some();
    }
    false
}

/// One pass over the board: numbered entries, the verdict lines about them
/// (found anywhere on the board), and what does not add up.
pub fn scan(text: &str) -> Scan {
    let mut scan = Scan::default();
    let mut current: Option<u64> = None;
    for (no, line) in active_lines(text) {
        if line.starts_with("### ") || line.starts_with("## ") {
            current = None;
            if let Some((n, title)) = heading(line) {
                if let Some(first) = scan.entries.get(&n) {
                    scan.warnings.push(format!(
                        "编号 #{n} 出现两次(第 {} 行与第 {no} 行),以第一次为准",
                        first.line
                    ));
                } else {
                    current = Some(n);
                    scan.entries.insert(
                        n,
                        Entry {
                            title: title.to_string(),
                            line: no,
                            ..Entry::default()
                        },
                    );
                }
            }
            continue;
        }
        if let Some(entry) = current.and_then(|n| scan.entries.get_mut(&n)) {
            if let Some((kind, _)) = ack(line) {
                entry.acks.push((no, kind));
                continue;
            }
            if let Some(value) = line.strip_prefix("unit:") {
                let value = value.trim();
                if !value.is_empty() && !value.contains(char::is_whitespace) {
                    entry.unit = Some(value.to_string());
                    continue;
                }
            }
            let role = [("impl:", true), ("review:", false)]
                .into_iter()
                .find_map(|(key, is_impl)| Some((line.strip_prefix(key)?.trim(), is_impl)));
            if let Some((value, is_impl)) = role.filter(|(value, _)| !value.is_empty()) {
                if is_impl {
                    entry.implementer = Some(value.to_string());
                } else {
                    entry.review = value
                        .split(',')
                        .map(str::trim)
                        .filter(|r| !r.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                continue;
            }
            if line.starts_with(ARCHIVED_MARK) {
                entry.archive_line = Some(no);
                continue;
            }
        }
        let signed = strip_quote(line);
        if let Some((_, rest)) = signature(signed, "内环") {
            // `·#4 ACK(done)…` or `·ACK(done)…: #7…`.
            let (before, rest) = match rest.strip_prefix('#').and_then(number) {
                Some((n, rest)) => (Some(n), rest.trim_start()),
                None => (None, rest),
            };
            if let Some((kind, rest)) = ack(rest) {
                // After the ACK: text up to the first colon, the colon,
                // optional spaces, then `#N`.
                let after = rest
                    .char_indices()
                    .find(|(_, c)| matches!(c, ':' | '：'))
                    .and_then(|(at, c)| rest[at + c.len_utf8()..].trim_start().strip_prefix('#'))
                    .and_then(number)
                    .map(|(n, _)| n);
                match before.or(after).or(current) {
                    Some(target) if scan.entries.contains_key(&target) => {
                        if let Some(entry) = scan.entries.get_mut(&target) {
                            entry.acks.push((no, kind));
                        }
                    }
                    Some(target) => scan.events.entry(target).or_default().push(Event {
                        line: no,
                        kind: EventKind::Dangling,
                    }),
                    None => {}
                }
                continue;
            }
        }
        if let Some((_, rest)) = signature(signed, "外环") {
            let verdict = [("采认", 0), ("打回", 1), ("搁置", 2)]
                .into_iter()
                .find_map(|(word, which)| Some((which, card_ref(rest.strip_prefix(word)?)?)));
            if let Some((which, (n, detail))) = verdict {
                let kind = match which {
                    0 => EventKind::Accepted,
                    1 => EventKind::Returned,
                    _ => EventKind::Snooze(detail.to_string()),
                };
                scan.events
                    .entry(n)
                    .or_default()
                    .push(Event { line: no, kind });
                continue;
            }
            let voided = rest
                .strip_prefix("改判(作废 #")
                .and_then(number)
                .filter(|(_, rest)| rest.starts_with(')'));
            if let Some((n, _)) = voided {
                scan.events.entry(n).or_default().push(Event {
                    line: no,
                    kind: EventKind::Voided,
                });
                continue;
            }
        }
        if let Some((unit, rest)) = signature(signed, "单元") {
            if let Some((n, _)) = rest.strip_prefix("本地复验通过").and_then(card_ref) {
                scan.events.entry(n).or_default().push(Event {
                    line: no,
                    kind: EventKind::Pushed(unit.to_string()),
                });
            }
            // A log rendered as Markdown: `进度(#N): claimed|undelivered …`.
            if let Some((n, detail)) = rest.strip_prefix("进度").and_then(card_ref) {
                let node = detail.trim_start_matches([':', '：', ' ']);
                let kind = match node.split_whitespace().next() {
                    Some("claimed") => Some(EventKind::Claimed),
                    Some("undelivered") => Some(EventKind::Undelivered),
                    _ => None,
                };
                if let Some(kind) = kind {
                    scan.events
                        .entry(n)
                        .or_default()
                        .push(Event { line: no, kind });
                }
            }
        }
    }
    for (n, events) in &scan.events {
        if !scan.entries.contains_key(n) {
            scan.warnings.push(format!(
                "第 {} 行的判词指向 #{n},但板上没有这个条目",
                events[0].line
            ));
        }
    }
    scan
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Question {
    pub id: String,
    pub unit: Option<String>,
    pub question: String,
    pub line: usize,
}

/// `MCP(ask_outer) <kind>: id=<id>[ question=<text>]`, anywhere in the line.
fn mcp_audit(s: &str) -> Option<(&str, &str, Option<&str>)> {
    const HEAD: &str = "MCP(ask_outer) ";
    let mut from = 0;
    while let Some(at) = s[from..].find(HEAD) {
        let rest = &s[from + at + HEAD.len()..];
        let parsed = ["ask", "answer", "timeout"].into_iter().find_map(|kind| {
            let rest = rest.strip_prefix(kind)?.strip_prefix(": id=")?;
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(rest.len());
            (end > 0).then(|| (kind, &rest[..end], rest[end..].strip_prefix(" question=")))
        });
        if parsed.is_some() {
            return parsed;
        }
        from += at + 1;
        while !s.is_char_boundary(from) {
            from += 1;
        }
    }
    None
}

/// One `ask_outer` audit line: a question (`closes` empty), or the answer
/// or timeout that closes the question `closes` names.
pub struct QuestionLine {
    pub question: Question,
    /// `answer` or `timeout` when this line closes a question.
    pub closes: Option<&'static str>,
}

/// Every `ask_outer` audit line on the live part of the board, in order.
pub fn question_lines(text: &str) -> Vec<QuestionLine> {
    let mut out = Vec::new();
    for (no, line) in active_lines(text) {
        let signed = strip_quote(line);
        let Some((kind, id, question)) = mcp_audit(signed) else {
            continue;
        };
        out.push(QuestionLine {
            question: Question {
                id: id.to_string(),
                unit: signature(signed, "单元").map(|(unit, _)| unit.to_string()),
                question: question.unwrap_or("").trim().to_string(),
                line: no,
            },
            closes: match kind {
                "answer" => Some("answer"),
                "timeout" => Some("timeout"),
                _ => None,
            },
        });
    }
    out
}

/// The text under each numbered entry's heading, up to the next heading,
/// as written (code fences included), keyed by the heading's line.
pub fn entry_bodies(text: &str) -> BTreeMap<usize, String> {
    let headings: Vec<usize> = active_lines(text)
        .into_iter()
        .filter(|(_, line)| line.starts_with("### ") || line.starts_with("## "))
        .map(|(no, _)| no)
        .collect();
    let all: Vec<&str> = text.split_inclusive('\n').collect();
    let mut bodies = BTreeMap::new();
    for (i, start) in headings.iter().enumerate() {
        let end = headings.get(i + 1).map_or(all.len(), |next| next - 1);
        bodies.insert(*start, all[*start..end.max(*start)].concat());
    }
    bodies
}

/// Inner-loop questions (`ask_outer`) that reached the board and were
/// neither answered nor timed out.
pub fn pending_questions(text: &str) -> Vec<Question> {
    let mut asked: Vec<Question> = Vec::new();
    let mut closed: Vec<String> = Vec::new();
    for line in question_lines(text) {
        let record = line.question;
        if line.closes.is_some() {
            closed.push(record.id);
            continue;
        }
        // A repeated id keeps its first position and takes the newer text.
        match asked.iter_mut().find(|q| q.id == record.id) {
            Some(existing) => *existing = record,
            None => asked.push(record),
        }
    }
    asked.retain(|q| !closed.contains(&q.id));
    asked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fence_closes_only_on_its_own_marker() {
        let text = "a\n```md\n~~~\nreview:\n```\nb\n````\n```\nc\n````\nd\n";
        let kept: Vec<&str> = markdown_lines(lines(text))
            .into_iter()
            .map(|(_, l)| l)
            .collect();
        assert_eq!(kept, ["a", "b", "d"]);
    }

    #[test]
    fn closed_history_is_skipped_and_open_history_is_kept() {
        let closed = "## Historical record\n### 1. old\n## Active\n### 2. new\n";
        assert_eq!(
            scan(closed).entries.keys().copied().collect::<Vec<_>>(),
            [2]
        );
        let open = "### 2. new\n## Historical record\n### 1. old\n";
        assert_eq!(
            scan(open).entries.keys().copied().collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn signed_inner_acks_name_their_entry_before_or_after() {
        let text = "### 4. a\n### 7. b\n\n> 内环(m07)·#4 ACK(done)(11-20): ok\n\
                    > 内环(x)·ACK(blocked): #7(11-20): 等依赖\n> 内环(x)·ACK(done): #9\n";
        let scan = scan(text);
        assert_eq!(scan.entries[&4].acks, [(4, Ack::Done)]);
        assert_eq!(scan.entries[&7].acks, [(5, Ack::Blocked)]);
        assert_eq!(scan.events[&9][0].kind, EventKind::Dangling);
    }
}
