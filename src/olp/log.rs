//! The event log: the default store for OctoLoop data. One directory per
//! project holds append-only JSON Lines segments (`events-000001.jsonl`,
//! …), one event per line. Everything else — card states, inboxes, unit
//! occupancy — is computed by replaying it.
//!
//! Every write reads the log, takes the directory's lock, reads what was
//! appended in between, decides, appends and syncs, so a decision ("has this message been claimed", "is this key
//! used", "what is the next card number") is made on the log as it stands.
//! Nothing here is edited by hand; people read a rendered view.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A segment is closed (and a new one started) once it reaches this size.
pub const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const LOCK_FILE: &str = "events.lock";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// 1-based, contiguous across segments.
    pub seq: u64,
    /// UTC, RFC 3339.
    pub ts: String,
    /// What happened: `dispatched`, `claimed`, `progress`, `ack`, …
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<u64>,
    /// Who wrote it, e.g. `外环(hub)` or `单元(m07)`.
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Idempotency key: a second append with the same key adds nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The event's own fields. A card's text lives here as a plain string,
    /// so nothing in it can be mistaken for a machine-read line.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
}

/// An event before the log gives it a sequence number and a time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub kind: String,
    pub card: Option<u64>,
    pub actor: String,
    pub unit: Option<String>,
    pub key: Option<String>,
    pub data: serde_json::Value,
}

#[derive(Debug)]
pub enum LogError {
    Io(std::io::Error),
    /// A complete line that is not a valid event. Never skipped silently.
    Corrupt {
        file: PathBuf,
        line: usize,
        why: String,
    },
    /// Another writer held the lock for the whole wait.
    Busy,
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogError::Io(e) => write!(f, "事件日志读写失败: {e}"),
            LogError::Corrupt { file, line, why } => {
                write!(f, "事件日志损坏: {} 第 {line} 行: {why}", file.display())
            }
            LogError::Busy => write!(f, "事件日志正被另一个写入者占用,等待超时"),
        }
    }
}

impl std::error::Error for LogError {}

impl From<std::io::Error> for LogError {
    fn from(e: std::io::Error) -> Self {
        LogError::Io(e)
    }
}

/// What the closure of [`Log::append_with`] can return instead of drafts.
#[derive(Debug)]
pub enum AppendError<E> {
    Log(LogError),
    /// The caller's rule refused; nothing was written.
    Refused(E),
}

/// How far a read got: the segment, the bytes of whole lines taken from
/// it, and how many lines that was.
#[derive(Default)]
struct Mark {
    segment: u32,
    offset: u64,
    lines: usize,
}

pub struct Log {
    dir: PathBuf,
    segment_bytes: u64,
    lock_wait: Duration,
}

fn segment_name(index: u32) -> String {
    format!("events-{index:06}.jsonl")
}

impl Log {
    /// The log in `dir` (created on first write).
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Log {
            dir: dir.into(),
            segment_bytes: SEGMENT_BYTES,
            lock_wait: Duration::from_secs(15),
        }
    }

    pub fn with_segment_bytes(mut self, bytes: u64) -> Self {
        self.segment_bytes = bytes;
        self
    }

    pub fn with_lock_wait(mut self, wait: Duration) -> Self {
        self.lock_wait = wait;
        self
    }

    /// Segment indexes present, ascending.
    fn segments(&self) -> std::io::Result<Vec<u32>> {
        let mut found = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(found),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let name = entry?.file_name();
            let Some(name) = name.to_str() else { continue };
            let index = name
                .strip_prefix("events-")
                .and_then(|rest| rest.strip_suffix(".jsonl"))
                .filter(|digits| digits.len() == 6)
                .and_then(|digits| digits.parse::<u32>().ok());
            if let Some(index) = index {
                found.push(index);
            }
        }
        found.sort_unstable();
        Ok(found)
    }

    /// Every event, oldest first. A last line without its newline is a
    /// write that did not finish and is left out; any other line that does
    /// not parse, or a sequence number out of step, is an error.
    pub fn read(&self) -> Result<Vec<Event>, LogError> {
        let mut events = Vec::new();
        self.read_on(&mut events, &mut Mark::default())?;
        Ok(events)
    }

    /// Read on from `mark`: push what the log holds after it onto `events`
    /// and move the mark. Segments are append-only, so what was read
    /// before is not read again.
    fn read_on(&self, events: &mut Vec<Event>, mark: &mut Mark) -> Result<(), LogError> {
        let segments = self.segments()?;
        for (position, index) in segments.iter().enumerate() {
            if *index < mark.segment {
                continue;
            }
            if *index > mark.segment {
                *mark = Mark {
                    segment: *index,
                    offset: 0,
                    lines: 0,
                };
            }
            let path = self.dir.join(segment_name(*index));
            let mut text = String::new();
            let mut file = File::open(&path)?;
            file.seek(SeekFrom::Start(mark.offset))?;
            file.read_to_string(&mut text)?;
            let last_segment = position + 1 == segments.len();
            for raw in text.split_inclusive('\n') {
                let corrupt = |why: String| LogError::Corrupt {
                    file: path.clone(),
                    line: mark.lines + 1,
                    why,
                };
                let Some(line) = raw.strip_suffix('\n') else {
                    if last_segment {
                        break; // the torn tail of the active segment
                    }
                    return Err(corrupt("封存的分段以不完整的行结尾".to_string()));
                };
                let event: Event =
                    serde_json::from_str(line).map_err(|e| corrupt(e.to_string()))?;
                let expected = events.last().map_or(1, |last| last.seq + 1);
                if event.seq != expected {
                    return Err(corrupt(format!("序号是 {},应为 {expected}", event.seq)));
                }
                events.push(event);
                mark.offset += raw.len() as u64;
                mark.lines += 1;
            }
        }
        Ok(())
    }

    fn lock(&self) -> Result<File, LogError> {
        std::fs::create_dir_all(&self.dir)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(LOCK_FILE))?;
        let deadline = Instant::now() + self.lock_wait;
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                    if Instant::now() >= deadline {
                        return Err(LogError::Busy);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Under the lock: show `decide` the whole log and append what it
    /// returns. The events it sees are exactly the ones its drafts follow.
    /// Slow work (ssh, notifications) belongs outside this call.
    pub fn append_with<E>(
        &self,
        decide: impl FnOnce(&[Event]) -> Result<Vec<Draft>, E>,
    ) -> Result<Vec<Event>, AppendError<E>> {
        // The log is read before the lock is taken; under the lock only
        // what was appended meanwhile is read. However long the log, the
        // lock is held for the new lines, the decision and the write.
        let mut existing = Vec::new();
        let mut mark = Mark::default();
        self.read_on(&mut existing, &mut mark)
            .map_err(AppendError::Log)?;
        let lock = self.lock().map_err(AppendError::Log)?;
        self.read_on(&mut existing, &mut mark)
            .map_err(AppendError::Log)?;
        let drafts = decide(&existing).map_err(AppendError::Refused)?;
        let written = self.write(&existing, drafts).map_err(AppendError::Log)?;
        drop(lock);
        Ok(written)
    }

    /// Append one event. With a key the log already holds, nothing is
    /// written and the earlier event is returned (`false`).
    pub fn append(&self, draft: Draft) -> Result<(Event, bool), LogError> {
        let mut earlier = None;
        let result = self.append_with(|events| {
            earlier = draft
                .key
                .as_ref()
                .and_then(|key| events.iter().find(|e| e.key.as_ref() == Some(key)))
                .cloned();
            Ok::<_, std::convert::Infallible>(if earlier.is_some() {
                Vec::new()
            } else {
                vec![draft]
            })
        });
        let mut written = match result {
            Ok(written) => written,
            Err(AppendError::Log(e)) => return Err(e),
            Err(AppendError::Refused(never)) => match never {},
        };
        match (earlier, written.pop()) {
            (Some(event), _) => Ok((event, false)),
            (None, Some(event)) => Ok((event, true)),
            (None, None) => Err(LogError::Io(std::io::Error::other("nothing was appended"))),
        }
    }

    fn write(&self, existing: &[Event], drafts: Vec<Draft>) -> Result<Vec<Event>, LogError> {
        if drafts.is_empty() {
            return Ok(Vec::new());
        }
        let segments = self.segments()?;
        let mut index = segments.last().copied().unwrap_or(1);
        let mut path = self.dir.join(segment_name(index));
        // Cut off a torn tail before writing after it.
        if let Ok(mut file) = OpenOptions::new().read(true).write(true).open(&path) {
            let len = file.metadata()?.len();
            let complete = whole_lines(&mut file, len)?;
            if complete != len {
                file.set_len(complete)?;
                file.sync_all()?;
            }
            if complete >= self.segment_bytes {
                index += 1;
                path = self.dir.join(segment_name(index));
            }
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let next = existing.last().map_or(1, |last| last.seq + 1);
        let mut out = Vec::new();
        let mut buffer = Vec::new();
        for (seq, draft) in (next..).zip(drafts) {
            let event = Event {
                seq,
                ts: now.clone(),
                kind: draft.kind,
                card: draft.card,
                actor: draft.actor,
                unit: draft.unit,
                key: draft.key,
                data: draft.data,
            };
            serde_json::to_writer(&mut buffer, &event)
                .map_err(|e| LogError::Io(std::io::Error::other(e)))?;
            buffer.push(b'\n');
            out.push(event);
        }
        let created = !path.exists();
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.seek(SeekFrom::End(0))?;
        // One write for the batch: all of it lands, or a torn tail does.
        file.write_all(&buffer)?;
        file.sync_all()?;
        if created {
            sync_dir(&self.dir);
        }
        Ok(out)
    }
}

/// How many of the file's `len` bytes are whole lines: the end of its last
/// newline. Read from the end, so a full segment costs one small read.
fn whole_lines(file: &mut File, len: u64) -> std::io::Result<u64> {
    const CHUNK: u64 = 64 * 1024;
    let mut end = len;
    let mut chunk = vec![0u8; CHUNK as usize];
    while end > 0 {
        let start = end.saturating_sub(CHUNK);
        let part = &mut chunk[..(end - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(part)?;
        if let Some(at) = part.iter().rposition(|b| *b == b'\n') {
            return Ok(start + at as u64 + 1);
        }
        end = start;
    }
    Ok(0)
}

/// Make a new segment's directory entry durable. Best effort off Unix.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(kind: &str, key: Option<&str>) -> Draft {
        Draft {
            kind: kind.to_string(),
            actor: "外环(hub)".to_string(),
            key: key.map(str::to_string),
            ..Draft::default()
        }
    }

    #[test]
    fn appends_number_events_and_read_returns_them() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path().join("olp"));
        assert!(log.read().unwrap().is_empty(), "no directory yet: empty");
        let (first, new) = log
            .append(Draft {
                card: Some(1),
                unit: Some("m07".to_string()),
                data: serde_json::json!({"title": "修复启动", "body": "```\nreview:\n```\n### 9. 不是标题"}),
                ..draft("dispatched", None)
            })
            .unwrap();
        assert!(new);
        assert_eq!(first.seq, 1);
        let (second, _) = log.append(draft("claimed", None)).unwrap();
        assert_eq!(second.seq, 2);
        let events = log.read().unwrap();
        assert_eq!(events, [first, second]);
        // One line per event; the card text is one JSON string.
        let text = std::fs::read_to_string(dir.path().join("olp/events-000001.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with('\n'));
        assert!(!text.contains("\n### 9"), "the body is escaped: {text}");
    }

    #[test]
    fn the_same_key_is_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path());
        let (first, new) = log
            .append(draft("dispatched", Some("dispatch:m07:abc")))
            .unwrap();
        assert!(new);
        let (again, new) = log
            .append(draft("dispatched", Some("dispatch:m07:abc")))
            .unwrap();
        assert!(!new);
        assert_eq!(again, first);
        assert_eq!(log.read().unwrap().len(), 1);
    }

    #[test]
    fn a_decision_sees_the_log_it_appends_to() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path());
        log.append(draft("delivered", Some("m1"))).unwrap();
        let claim = |who: &str| {
            let who = who.to_string();
            log.append_with(move |events| {
                if events.iter().any(|e| e.kind == "claimed") {
                    return Err("消息已由另一次请求领取");
                }
                Ok(vec![Draft {
                    data: serde_json::json!({"request": who}),
                    ..draft("claimed", None)
                }])
            })
        };
        assert_eq!(claim("r1").unwrap().len(), 1);
        match claim("r2") {
            Err(AppendError::Refused(why)) => assert_eq!(why, "消息已由另一次请求领取"),
            other => panic!("the second claim was not refused: {other:?}"),
        }
        assert_eq!(log.read().unwrap().len(), 2);
    }

    #[test]
    fn a_torn_tail_is_ignored_then_cut_off_and_a_bad_line_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path());
        log.append(draft("a", None)).unwrap();
        let path = dir.path().join("events-000001.jsonl");
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"seq\":2,\"ts\":\"x\",\"ki").unwrap();
        drop(file);
        assert_eq!(
            log.read().unwrap().len(),
            1,
            "the unfinished line is not an event"
        );
        let (next, _) = log.append(draft("b", None)).unwrap();
        assert_eq!(next.seq, 2);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(!text.contains("\"ki\n"), "{text}");

        // A complete line that is not an event is never skipped.
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"not json\n").unwrap();
        drop(file);
        match log.read() {
            Err(LogError::Corrupt { line, .. }) => assert_eq!(line, 3),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            log.append(draft("c", None)),
            Err(LogError::Corrupt { .. })
        ));
    }

    #[test]
    fn the_end_of_the_last_whole_line_is_found_from_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        let long = "x".repeat(200_000);
        for (text, whole) in [
            (String::new(), 0),
            ("no newline".to_string(), 0),
            ("a\nb\n".to_string(), 4),
            ("a\nb\ntorn".to_string(), 4),
            // A torn tail longer than one read.
            (format!("a\n{long}"), 2),
            (format!("{long}\n{long}\n"), 400_002),
        ] {
            std::fs::write(&path, &text).unwrap();
            let mut file = File::open(&path).unwrap();
            assert_eq!(whole_lines(&mut file, text.len() as u64).unwrap(), whole);
        }
    }

    #[test]
    fn a_full_segment_is_closed_and_numbering_continues() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path()).with_segment_bytes(200);
        for i in 0..12 {
            log.append(draft(&format!("event-{i}"), None)).unwrap();
        }
        let segments = log.segments().unwrap();
        assert!(segments.len() > 1, "{segments:?}");
        let events = log.read().unwrap();
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            (1..=12).collect::<Vec<_>>()
        );
        // A gap between segments is corruption, not a shorter log.
        std::fs::remove_file(dir.path().join(segment_name(segments[0]))).unwrap();
        assert!(matches!(log.read(), Err(LogError::Corrupt { .. })));
    }

    #[test]
    fn concurrent_writers_never_share_or_skip_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let writers: Vec<_> = (0..8)
            .map(|w| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let log = Log::at(path);
                    for i in 0..25 {
                        // Half of the appends repeat a key another writer uses.
                        let key = (i % 2 == 0).then(|| format!("shared-{i}"));
                        log.append(draft(&format!("w{w}"), key.as_deref())).unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let events = Log::at(&path).read().unwrap();
        // 8 writers x 12 unkeyed appends, plus 13 shared keys written once.
        assert_eq!(events.len(), 8 * 12 + 13);
        let keys: Vec<&String> = events.iter().filter_map(|e| e.key.as_ref()).collect();
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(keys.len(), unique.len());
    }

    #[test]
    fn a_read_goes_on_from_where_the_last_one_stopped() {
        let dir = tempfile::tempdir().unwrap();
        // Small segments: the log rolls over while it is being followed.
        let log = Log::at(dir.path()).with_segment_bytes(200);
        for _ in 0..3 {
            log.append(draft("a", None)).unwrap();
        }
        let mut events = Vec::new();
        let mut mark = Mark::default();
        log.read_on(&mut events, &mut mark).unwrap();
        assert_eq!(events.len(), 3);
        let at = (mark.segment, mark.offset);
        // Nothing new: nothing is read again and the mark stays.
        log.read_on(&mut events, &mut mark).unwrap();
        assert_eq!((events.len(), (mark.segment, mark.offset)), (3, at));

        for _ in 0..4 {
            log.append(draft("b", None)).unwrap();
        }
        assert!(log.segments().unwrap().len() > 1, "the log rolled over");
        // A write that did not finish sits at the end.
        let last = *log.segments().unwrap().last().unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .open(dir.path().join(segment_name(last)))
            .unwrap();
        file.write_all(b"{\"seq\":8,\"ts\"").unwrap();
        log.read_on(&mut events, &mut mark).unwrap();
        assert_eq!(events, log.read().unwrap());
        assert_eq!(events.len(), 7);
        assert_eq!(events.last().unwrap().seq, 7);

        // The torn tail is cut off by the next write, which the read then
        // picks up from the same mark.
        log.append(draft("c", None)).unwrap();
        log.read_on(&mut events, &mut mark).unwrap();
        assert_eq!(events.len(), 8);
        assert_eq!(events, log.read().unwrap());
    }

    #[test]
    fn a_decision_sees_what_was_appended_while_it_waited_for_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path());
        log.append(draft("a", Some("k1"))).unwrap();
        // Hold the lock, start an append that has to wait, and write an
        // event behind its back (as the lock's holder would).
        let held = log.lock().unwrap();
        let path = dir.path().to_path_buf();
        let waiting = std::thread::spawn(move || {
            Log::at(path).append_with(|events| {
                let kinds: Vec<String> = events.iter().map(|e| e.kind.clone()).collect();
                Ok::<_, String>(vec![Draft {
                    data: serde_json::json!({ "saw": kinds }),
                    ..draft("c", None)
                }])
            })
        });
        std::thread::sleep(Duration::from_millis(300));
        let existing = log.read().unwrap();
        log.write(&existing, vec![draft("b", None)]).unwrap();
        drop(held);
        let written = waiting.join().unwrap().unwrap();
        assert_eq!(written[0].seq, 3);
        assert_eq!(written[0].data["saw"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn a_held_lock_times_out_instead_of_waiting_forever() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path()).with_lock_wait(Duration::from_millis(100));
        let held = log.lock().unwrap();
        assert!(matches!(log.append(draft("a", None)), Err(LogError::Busy)));
        drop(held);
        assert!(log.append(draft("a", None)).is_ok());
    }
}
