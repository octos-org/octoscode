//! Cards from the event log. Replaying the events gives the same picture a
//! scan of the Markdown board gives — entries, the verdicts about them,
//! pending questions — with an event's sequence number where the board had
//! a line number, so every command judges both stores by the same rules.
//!
//! Event kinds read here (others, such as `progress`, are ignored):
//!
//! | kind | card | data |
//! |---|---|---|
//! | `dispatched` | N | `title`, `body`, `impl`, `review` (and `unit` on the event) |
//! | `assigned` | N | the event's `unit` is the card's unit from now on |
//! | `ack` | N | `result`: done / wontdo / blocked |
//! | `pushed` | N | the event's `unit` pushed its verified branch |
//! | `accepted`, `returned`, `voided` | N | |
//! | `snoozed` | N | `text`, starting `至 YYYY-MM-DD` |
//! | `archived` | N | a stub left by the Markdown archiver (imports only) |
//! | `asked` | – | `id`, `question`; `answered` / `timed_out`: `id` |

use super::board::{Ack, Entry, Event as BoardEvent, EventKind, Question, Scan};
use super::log::Event;

fn text<'a>(event: &'a Event, field: &str) -> &'a str {
    event.data.get(field).and_then(|v| v.as_str()).unwrap_or("")
}

/// The board picture of a log: `line` everywhere is the event's `seq`.
pub fn scan(events: &[Event]) -> Scan {
    let mut scan = Scan::default();
    for event in events {
        let at = event.seq as usize;
        let Some(card) = event.card else { continue };
        let mut verdict = |kind: EventKind| {
            scan.events
                .entry(card)
                .or_default()
                .push(BoardEvent { line: at, kind });
        };
        match event.kind.as_str() {
            "dispatched" => {
                if let Some(first) = scan.entries.get(&card) {
                    scan.warnings.push(format!(
                        "编号 #{card} 派出了两次(第 {} 条与第 {at} 条事件),以第一次为准",
                        first.line
                    ));
                    continue;
                }
                let review = event
                    .data
                    .get("review")
                    .and_then(|v| v.as_array())
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(|n| n.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let implementer = Some(text(event, "impl")).filter(|s| !s.is_empty());
                scan.entries.insert(
                    card,
                    Entry {
                        title: text(event, "title").to_string(),
                        line: at,
                        unit: event.unit.clone(),
                        implementer: implementer.map(str::to_string),
                        review,
                        ..Entry::default()
                    },
                );
            }
            "assigned" => {
                if let Some(entry) = scan.entries.get_mut(&card) {
                    entry.unit = event.unit.clone();
                }
            }
            "archived" => {
                if let Some(entry) = scan.entries.get_mut(&card) {
                    entry.archive_line = Some(at);
                }
            }
            "ack" => {
                let kind = match text(event, "result") {
                    "done" => Ack::Done,
                    "wontdo" => Ack::Wontdo,
                    "blocked" => Ack::Blocked,
                    _ => continue,
                };
                match scan.entries.get_mut(&card) {
                    Some(entry) => entry.acks.push((at, kind)),
                    None => verdict(EventKind::Dangling),
                }
            }
            "accepted" => verdict(EventKind::Accepted),
            "returned" => verdict(EventKind::Returned),
            "voided" => verdict(EventKind::Voided),
            "snoozed" => verdict(EventKind::Snooze(text(event, "text").to_string())),
            "pushed" => verdict(EventKind::Pushed(event.unit.clone().unwrap_or_default())),
            _ => {}
        }
    }
    for (card, events) in &scan.events {
        if !scan.entries.contains_key(card) {
            scan.warnings.push(format!(
                "第 {} 条事件的判词指向 #{card},但没有这张卡",
                events[0].line
            ));
        }
    }
    scan
}

/// The unit card `n` is with now: `None` when there is no such card,
/// `Some(None)` when it was opened without a unit.
pub fn unit_of(events: &[Event], n: u64) -> Option<Option<String>> {
    let mut found = None;
    let mut opened = false;
    for event in events.iter().filter(|e| e.card == Some(n)) {
        match event.kind.as_str() {
            "dispatched" if !opened => {
                opened = true;
                found = Some(event.unit.clone());
            }
            "assigned" if opened => found = Some(event.unit.clone()),
            _ => {}
        }
    }
    found
}

/// What a unit may see: every event about a card that is with it now, and
/// its own events that name no card (its questions). Nothing about any
/// other unit's cards.
pub fn visible_to<'a>(events: &'a [Event], unit: &str) -> Vec<&'a Event> {
    let mut owner: std::collections::BTreeMap<u64, Option<String>> = Default::default();
    for event in events {
        if let Some(n) = event.card {
            owner
                .entry(n)
                .or_insert_with(|| unit_of(events, n).flatten());
        }
    }
    events
        .iter()
        .filter(|event| match event.card {
            Some(n) => owner.get(&n).is_some_and(|o| o.as_deref() == Some(unit)),
            None => event.unit.as_deref() == Some(unit),
        })
        .collect()
}

/// A card dispatched to a unit's pane that the unit has not claimed yet.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Pending {
    pub number: u64,
    pub title: String,
    pub pane: String,
    #[serde(rename = "impl")]
    pub implementer: Option<String>,
    pub review: Vec<String>,
    /// The card's text as dispatched: the inbox is where the unit reads it.
    pub body: String,
}

/// The unit's delivery inbox: its cards that name a pane, are not voided
/// and have no `claimed` progress node yet, in number order. The log is the
/// delivery — a card is in the inbox from the moment it is written.
pub fn pending_for(events: &[Event], unit: &str) -> Vec<Pending> {
    let scan = scan(events);
    let mut out = Vec::new();
    for (number, entry) in &scan.entries {
        if entry.unit.as_deref() != Some(unit) {
            continue;
        }
        let about = || events.iter().filter(|e| e.card == Some(*number));
        let dispatched = about().find(|e| e.kind == "dispatched");
        let pane = dispatched
            .map(|e| text(e, "pane").to_string())
            .unwrap_or_default();
        let closed = about()
            .any(|e| e.kind == "voided" || (e.kind == "progress" && text(e, "node") == "claimed"));
        if pane.is_empty() || closed {
            continue;
        }
        out.push(Pending {
            number: *number,
            title: entry.title.clone(),
            pane,
            implementer: entry.implementer.clone(),
            review: entry.review.clone(),
            body: dispatched
                .map(|e| text(e, "body").trim().to_string())
                .unwrap_or_default(),
        });
    }
    out
}

/// Questions asked through `ask_outer` that were neither answered nor
/// timed out, in the order they were first asked.
pub fn pending_questions(events: &[Event]) -> Vec<Question> {
    let mut asked: Vec<Question> = Vec::new();
    let mut closed: Vec<&str> = Vec::new();
    for event in events {
        let id = text(event, "id");
        match event.kind.as_str() {
            "asked" if !id.is_empty() => {
                let record = Question {
                    id: id.to_string(),
                    unit: event.unit.clone(),
                    question: text(event, "question").trim().to_string(),
                    line: event.seq as usize,
                };
                match asked.iter_mut().find(|q| q.id == record.id) {
                    Some(existing) => *existing = record,
                    None => asked.push(record),
                }
            }
            "answered" | "timed_out" => closed.push(id),
            _ => {}
        }
    }
    asked.retain(|q| !closed.contains(&q.id.as_str()));
    asked
}
