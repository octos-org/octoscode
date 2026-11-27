//! The event log: the default store for OctoLoop data. One directory per
//! project holds append-only JSON Lines segments (`events-000001.jsonl`,
//! …), one event per line. Everything else — card states, inboxes, unit
//! occupancy — is computed by replaying it.
//!
//! Every write reads the log, takes the directory's lock, reads what was
//! appended in between, decides, appends and syncs, so a decision ("has this message been claimed", "is this key
//! used", "what is the next card number") is made on the log as it stands.
//! Nothing here is edited by hand; people read a rendered view.
//!
//! So that the replay does not grow for ever, the log can be compacted
//! (`Log::rebase`, `olp log compact`): the events still needed are written
//! to `base.jsonl`, the segments they were read from move to `archive/`
//! byte for byte, and the log goes on in a new segment. A read is the base
//! plus what was written since; sequence numbers are never reused, so
//! after a compaction there are gaps. The archive and the segments in use
//! are together the whole history, numbered from 1 without a gap.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A segment is closed (and a new one started) once it reaches this size.
pub const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const LOCK_FILE: &str = "events.lock";
/// What the last compaction kept, and where the archive ends.
const BASE_FILE: &str = "base.jsonl";
/// Where compacted segments go, byte for byte.
const ARCHIVE_DIR: &str = "archive";

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

/// How far a read got: the base it stands on, the segment, the bytes of
/// whole lines taken from it, how many lines that was, and the number of
/// the last event written.
#[derive(Default)]
struct Mark {
    /// The base the events were read on top of (`None`: there is none).
    base: Option<String>,
    /// The first segment that is not in the archive.
    first_segment: u32,
    segment: u32,
    offset: u64,
    lines: usize,
    /// The number of the last event written, kept by the base or not.
    head: u64,
}

/// A card whose events a compaction moved to the archive. What is left of
/// it here is what a later write has to know: its number is taken, the
/// key it was opened with stands for it, and nothing more is written
/// about it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gone {
    pub card: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub title: String,
    /// The key the card was opened with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Its first and its last event.
    pub first: u64,
    pub last: u64,
    /// When its last event was written.
    pub ended: String,
}

/// The first line of `base.jsonl`: where the archive ends.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Floor {
    /// Every event up to this number is in the archive; the base keeps
    /// the ones that are still needed.
    pub upto: u64,
    /// The segment the log goes on in.
    pub segment: u32,
    /// When the base was written.
    pub made: String,
    /// The highest card number ever given out.
    pub max_card: u64,
    /// How many events the base leaves out, over all compactions.
    pub dropped: u64,
    /// The cards that are only in the archive.
    #[serde(skip)]
    pub gone: std::collections::BTreeMap<u64, Gone>,
}

impl Floor {
    fn id(&self) -> String {
        format!("{}#{}", self.made, self.upto)
    }

    /// The archived card `key` was opened with.
    pub fn card_of_key(&self, key: &str) -> Option<u64> {
        self.gone
            .values()
            .find(|gone| gone.key.as_deref() == Some(key))
            .map(|gone| gone.card)
    }
}

/// What a compaction keeps, decided by the closure of [`Log::rebase`].
pub struct Rebase {
    /// The events that stay readable, a subset of the ones shown, in order.
    pub keep: Vec<Event>,
    /// The cards all of whose events are left out.
    pub gone: Vec<Gone>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum BaseLine {
    Floor(Floor),
    Gone(Gone),
    Event(Event),
}

pub struct Log {
    dir: PathBuf,
    segment_bytes: u64,
    lock_wait: Duration,
}

fn segment_name(index: u32) -> String {
    format!("events-{index:06}.jsonl")
}

/// Segment indexes in `dir`, ascending.
fn segments_in(dir: &Path) -> std::io::Result<Vec<u32>> {
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(dir) {
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

/// Read a segment's whole lines from `offset` on: each must carry the
/// number after `head`. Returns the bytes and lines taken. A last line
/// without its newline is a write that did not finish: left out of the
/// `active` segment, an error in a closed one.
fn read_segment(
    path: &Path,
    offset: u64,
    lines_before: usize,
    active: bool,
    head: &mut u64,
    mut take: impl FnMut(Event),
) -> Result<(u64, usize), LogError> {
    let mut text = String::new();
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    file.read_to_string(&mut text)?;
    let (mut bytes, mut lines) = (0u64, 0usize);
    for raw in text.split_inclusive('\n') {
        let corrupt = |why: String| LogError::Corrupt {
            file: path.to_path_buf(),
            line: lines_before + lines + 1,
            why,
        };
        let Some(line) = raw.strip_suffix('\n') else {
            if active {
                break; // the torn tail of the active segment
            }
            return Err(corrupt("封存的分段以不完整的行结尾".to_string()));
        };
        let event: Event = serde_json::from_str(line).map_err(|e| corrupt(e.to_string()))?;
        let expected = *head + 1;
        if event.seq != expected {
            return Err(corrupt(format!("序号是 {},应为 {expected}", event.seq)));
        }
        *head = event.seq;
        take(event);
        bytes += raw.len() as u64;
        lines += 1;
    }
    Ok((bytes, lines))
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
        segments_in(&self.dir)
    }

    /// The base a compaction left: its floor and, when `events` is given,
    /// the events it kept. `None`: the log was never compacted.
    fn base(&self, mut events: Option<&mut Vec<Event>>) -> Result<Option<Floor>, LogError> {
        let path = self.dir.join(BASE_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut floor: Option<Floor> = None;
        let mut last = 0u64;
        for (at, line) in text.lines().enumerate() {
            let corrupt = |why: String| LogError::Corrupt {
                file: path.clone(),
                line: at + 1,
                why,
            };
            let parsed: BaseLine =
                serde_json::from_str(line).map_err(|e| corrupt(e.to_string()))?;
            let Some(known) = floor.as_mut() else {
                match parsed {
                    BaseLine::Floor(first) => floor = Some(first),
                    _ => return Err(corrupt("第一行应是基线的说明".to_string())),
                }
                continue;
            };
            match parsed {
                BaseLine::Floor(_) => return Err(corrupt("基线的说明只能有一行".to_string())),
                BaseLine::Gone(gone) => {
                    known.gone.insert(gone.card, gone);
                }
                BaseLine::Event(event) => {
                    // The cards that are gone come first: a caller that
                    // wants only those stops here.
                    let Some(events) = events.as_mut() else { break };
                    if event.seq <= last || event.seq > known.upto {
                        return Err(corrupt(format!("序号 {} 不在应有的位置", event.seq)));
                    }
                    last = event.seq;
                    events.push(event);
                }
            }
        }
        match floor {
            Some(floor) => Ok(Some(floor)),
            None => Err(LogError::Corrupt {
                file: path,
                line: 1,
                why: "基线是空的".to_string(),
            }),
        }
    }

    /// Which base is in place now, from its first line alone.
    fn base_id(&self) -> Result<Option<String>, LogError> {
        let path = self.dir.join(BASE_FILE);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(file), &mut line)?;
        match serde_json::from_str::<BaseLine>(line.trim_end()) {
            Ok(BaseLine::Floor(floor)) => Ok(Some(floor.id())),
            Ok(_) => Err(LogError::Corrupt {
                file: path,
                line: 1,
                why: "第一行应是基线的说明".to_string(),
            }),
            Err(e) => Err(LogError::Corrupt {
                file: path,
                line: 1,
                why: e.to_string(),
            }),
        }
    }

    /// What compaction left behind: where the archive ends, the highest
    /// card number given out and the cards that are only in the archive.
    /// All zero and empty for a log that was never compacted. Inside the
    /// closure of [`Log::append_with`] it is the floor the events shown
    /// stand on (a compaction takes the same lock).
    pub fn floor(&self) -> Result<Floor, LogError> {
        Ok(self.base(None)?.unwrap_or_default())
    }

    /// The events that are still needed, oldest first: what the last
    /// compaction kept, then everything written since. Numbers only go
    /// up; after a compaction there are gaps where archived events were.
    /// A last line without its newline is a write that did not finish and
    /// is left out; any other line that does not parse, or a sequence
    /// number out of step, is an error.
    pub fn read(&self) -> Result<Vec<Event>, LogError> {
        let mut events = Vec::new();
        self.read_on(&mut events, &mut Mark::default())?;
        Ok(events)
    }

    /// Read on from `mark`: push what the log holds after it onto `events`
    /// and move the mark. Segments are append-only, so what was read
    /// before is not read again — unless the log was compacted meanwhile:
    /// then `events` is read anew from the new base.
    fn read_on(&self, events: &mut Vec<Event>, mark: &mut Mark) -> Result<(), LogError> {
        // A compaction replaces the base and then moves segments away. A
        // read that one crossed starts again; under the lock none can.
        for _ in 0..16 {
            if self.read_once(events, mark)? {
                return Ok(());
            }
            events.clear();
            *mark = Mark::default();
        }
        Err(LogError::Busy)
    }

    /// `Ok(false)`: the log was compacted while it was read.
    fn read_once(&self, events: &mut Vec<Event>, mark: &mut Mark) -> Result<bool, LogError> {
        if self.base_id()? != mark.base {
            events.clear();
            *mark = Mark::default();
            if let Some(floor) = self.base(Some(events))? {
                mark.base = Some(floor.id());
                mark.first_segment = floor.segment;
                mark.head = floor.upto;
            }
        }
        // Segments below the base's are in the archive, or on their way.
        let segments: Vec<u32> = self
            .segments()?
            .into_iter()
            .filter(|index| *index >= mark.first_segment)
            .collect();
        for (position, index) in segments.iter().enumerate() {
            if *index < mark.segment {
                continue;
            }
            if *index > mark.segment {
                mark.segment = *index;
                mark.offset = 0;
                mark.lines = 0;
            }
            let path = self.dir.join(segment_name(*index));
            let active = position + 1 == segments.len();
            let taken = read_segment(
                &path,
                mark.offset,
                mark.lines,
                active,
                &mut mark.head,
                |event| events.push(event),
            );
            let (bytes, lines) = match taken {
                Ok(taken) => taken,
                Err(LogError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(false); // moved to the archive just now
                }
                Err(e) => return Err(e),
            };
            mark.offset += bytes;
            mark.lines += lines;
        }
        Ok(self.base_id()? == mark.base)
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

    /// Under the lock: show `decide` the log and append what it returns.
    /// The events it sees are exactly the ones its drafts follow.
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
        let written = self.write(&mark, drafts).map_err(AppendError::Log)?;
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

    /// The segment to write to, with a torn tail cut off: its index and
    /// whether it is full.
    fn open_segment(&self, first: u32) -> Result<(u32, bool), LogError> {
        let index = self
            .segments()?
            .into_iter()
            .rfind(|index| *index >= first)
            .unwrap_or(first.max(1));
        let path = self.dir.join(segment_name(index));
        let mut full = false;
        if let Ok(mut file) = OpenOptions::new().read(true).write(true).open(&path) {
            let len = file.metadata()?.len();
            let complete = whole_lines(&mut file, len)?;
            if complete != len {
                file.set_len(complete)?;
                file.sync_all()?;
            }
            full = complete >= self.segment_bytes;
        }
        Ok((index, full))
    }

    fn write(&self, mark: &Mark, drafts: Vec<Draft>) -> Result<Vec<Event>, LogError> {
        if drafts.is_empty() {
            return Ok(Vec::new());
        }
        // Cut off a torn tail before writing after it.
        let (mut index, full) = self.open_segment(mark.first_segment)?;
        if full {
            index += 1;
        }
        let path = self.dir.join(segment_name(index));
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut out = Vec::new();
        let mut buffer = Vec::new();
        for (seq, draft) in (mark.head + 1..).zip(drafts) {
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

    /// Compact the log. Under the lock: show `decide` the events and the
    /// floor they stand on; what it does not keep is from then on only in
    /// the archive. Nothing is rewritten there — the segments move to
    /// `archive/` byte for byte, and the log goes on in a new segment with
    /// the next number. `Ok(None)`: everything was kept, nothing changed.
    ///
    /// The new base is written beside the old one, synced, and renamed
    /// over it; only then do the segments move. A crash in between leaves
    /// segments the base already covers: readers pass them by, and the
    /// next compaction moves them.
    pub fn rebase<E>(
        &self,
        decide: impl FnOnce(&[Event], &Floor) -> Result<Rebase, E>,
    ) -> Result<Option<Floor>, AppendError<E>> {
        let lock = self.lock().map_err(AppendError::Log)?;
        let mut events = Vec::new();
        let mut mark = Mark::default();
        self.read_on(&mut events, &mut mark)
            .map_err(AppendError::Log)?;
        let mut floor = self.floor().map_err(AppendError::Log)?;
        let plan = decide(&events, &floor).map_err(AppendError::Refused)?;
        let done = self
            .rebase_to(&events, &mark, &mut floor, plan)
            .map_err(AppendError::Log)?;
        drop(lock);
        Ok(done.then_some(floor))
    }

    fn rebase_to(
        &self,
        events: &[Event],
        mark: &Mark,
        floor: &mut Floor,
        plan: Rebase,
    ) -> Result<bool, LogError> {
        let refuse = |why: &str| LogError::Io(std::io::Error::other(why.to_string()));
        // What is kept is some of what is there, as it is and in order.
        let mut rest = events.iter();
        if !plan.keep.iter().all(|kept| rest.any(|e| e == kept)) {
            return Err(refuse("要保留的事件不是日志里的事件"));
        }
        let dropped = events.len() - plan.keep.len();
        if dropped == 0 {
            return Ok(false);
        }
        // The archive holds whole lines only.
        let (last, _) = self.open_segment(mark.first_segment)?;
        floor.upto = mark.head;
        floor.segment = last + 1;
        floor.made = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        floor.dropped += dropped as u64;
        floor.max_card = events
            .iter()
            .filter(|e| e.kind == "dispatched")
            .filter_map(|e| e.card)
            .chain(plan.gone.iter().map(|gone| gone.card))
            .chain([floor.max_card])
            .max()
            .unwrap_or(0);
        for gone in plan.gone {
            floor.gone.insert(gone.card, gone);
        }
        let mut buffer = Vec::new();
        let mut line = |value: &BaseLine| -> Result<(), LogError> {
            serde_json::to_writer(&mut buffer, value)
                .map_err(|e| LogError::Io(std::io::Error::other(e)))?;
            buffer.push(b'\n');
            Ok(())
        };
        line(&BaseLine::Floor(floor.clone()))?;
        for gone in floor.gone.values() {
            line(&BaseLine::Gone(gone.clone()))?;
        }
        for event in plan.keep {
            line(&BaseLine::Event(event))?;
        }
        let fresh = self.dir.join(format!("{BASE_FILE}.new"));
        let mut file = File::create(&fresh)?;
        file.write_all(&buffer)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&fresh, self.dir.join(BASE_FILE))?;
        sync_dir(&self.dir);
        self.retire(floor.segment)?;
        Ok(true)
    }

    /// Move the segments below `first` to the archive.
    fn retire(&self, first: u32) -> Result<(), LogError> {
        let archive = self.dir.join(ARCHIVE_DIR);
        let old: Vec<u32> = self
            .segments()?
            .into_iter()
            .filter(|index| *index < first)
            .collect();
        if old.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&archive)?;
        for index in old {
            let name = segment_name(index);
            std::fs::rename(self.dir.join(&name), archive.join(&name))?;
        }
        sync_dir(&archive);
        sync_dir(&self.dir);
        Ok(())
    }

    /// Bytes in use: the base, and the segments that are not archived.
    pub fn bytes(&self) -> Result<(u64, u64), LogError> {
        let size = |path: PathBuf| std::fs::metadata(path).map_or(0, |meta| meta.len());
        let first = self.floor()?.segment;
        let segments = self
            .segments()?
            .into_iter()
            .filter(|index| *index >= first)
            .map(|index| size(self.dir.join(segment_name(index))))
            .sum();
        Ok((size(self.dir.join(BASE_FILE)), segments))
    }

    /// Every event ever written, oldest first, numbered without a gap:
    /// the archive, then the segments in use. For looking back; nothing
    /// that decides reads this.
    pub fn history(&self) -> Result<Vec<Event>, LogError> {
        // Under the lock, so that no segment moves while it is read.
        let lock = self.lock()?;
        let archive = self.dir.join(ARCHIVE_DIR);
        let mut paths: std::collections::BTreeMap<u32, PathBuf> = segments_in(&archive)?
            .into_iter()
            .map(|index| (index, archive.join(segment_name(index))))
            .collect();
        for index in self.segments()? {
            paths.insert(index, self.dir.join(segment_name(index)));
        }
        let mut events = Vec::new();
        let mut head = 0u64;
        let count = paths.len();
        for (position, path) in paths.values().enumerate() {
            let active = position + 1 == count;
            read_segment(path, 0, 0, active, &mut head, |event| events.push(event))?;
        }
        drop(lock);
        Ok(events)
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
        let mut mark = Mark::default();
        log.read_on(&mut Vec::new(), &mut mark).unwrap();
        log.write(&mark, vec![draft("b", None)]).unwrap();
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

    fn seqs(events: &[Event]) -> Vec<u64> {
        events.iter().map(|e| e.seq).collect()
    }

    /// Compact, keeping the events with these numbers.
    fn rebase_keeping(log: &Log, keep: &[u64]) -> Option<Floor> {
        log.rebase(|events, _| {
            Ok::<_, std::convert::Infallible>(Rebase {
                keep: events
                    .iter()
                    .filter(|e| keep.contains(&e.seq))
                    .cloned()
                    .collect(),
                gone: Vec::new(),
            })
        })
        .unwrap()
    }

    #[test]
    fn a_compacted_log_keeps_its_numbers_and_its_whole_history() {
        let dir = tempfile::tempdir().unwrap();
        // Small segments: several of them move to the archive.
        let log = Log::at(dir.path()).with_segment_bytes(200);
        for _ in 0..6 {
            log.append(draft("a", None)).unwrap();
        }
        let closed = log.segments().unwrap();
        assert!(closed.len() > 1);
        let before = log.read().unwrap();

        let floor = rebase_keeping(&log, &[2, 5]).unwrap();
        assert_eq!((floor.upto, floor.dropped), (6, 4));
        // What was kept is read as it was written, numbers and all.
        let kept = log.read().unwrap();
        assert_eq!(kept, [before[1].clone(), before[4].clone()]);
        // The segments moved, byte for byte; the log goes on after them.
        assert!(log.segments().unwrap().is_empty());
        assert_eq!(segments_in(&dir.path().join(ARCHIVE_DIR)).unwrap(), closed);
        let (event, _) = log.append(draft("b", None)).unwrap();
        assert_eq!(event.seq, 7);
        assert_eq!(log.segments().unwrap(), [closed.last().unwrap() + 1]);
        assert_eq!(seqs(&log.read().unwrap()), [2, 5, 7]);
        assert_eq!(seqs(&log.history().unwrap()), [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(log.history().unwrap()[..6], before[..]);

        // A compaction on top of a compaction.
        let floor = rebase_keeping(&log, &[5, 7]).unwrap();
        assert_eq!((floor.upto, floor.dropped), (7, 5));
        assert_eq!(log.append(draft("c", None)).unwrap().0.seq, 8);
        assert_eq!(seqs(&log.read().unwrap()), [5, 7, 8]);
        assert_eq!(seqs(&log.history().unwrap()), (1..=8).collect::<Vec<_>>());

        // Everything kept: nothing changes, nothing moves.
        assert!(rebase_keeping(&log, &[5, 7, 8]).is_none());
        assert_eq!(log.floor().unwrap().upto, 7);
        assert_eq!(log.segments().unwrap().len(), 1);
    }

    #[test]
    fn a_key_the_base_kept_is_still_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path());
        log.append(draft("a", Some("k1"))).unwrap();
        log.append(draft("a", Some("k2"))).unwrap();
        rebase_keeping(&log, &[2]).unwrap();
        let (event, fresh) = log.append(draft("a", Some("k2"))).unwrap();
        assert_eq!((event.seq, fresh), (2, false));
        // Nothing but the base is left to read, and the number goes on.
        let (event, fresh) = log.append(draft("a", Some("k3"))).unwrap();
        assert_eq!((event.seq, fresh), (3, true));
    }

    #[test]
    fn a_compaction_cut_short_after_the_new_base_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path()).with_segment_bytes(200);
        for _ in 0..6 {
            log.append(draft("a", None)).unwrap();
        }
        rebase_keeping(&log, &[3]).unwrap();
        // As after a crash between the rename and the move: the segments
        // the base covers are still where they were.
        let archive = dir.path().join(ARCHIVE_DIR);
        let moved = segments_in(&archive).unwrap();
        for index in &moved {
            let name = segment_name(*index);
            std::fs::rename(archive.join(&name), dir.path().join(&name)).unwrap();
        }
        assert_eq!(seqs(&log.read().unwrap()), [3]);
        assert_eq!(log.append(draft("b", None)).unwrap().0.seq, 7);
        assert_eq!(seqs(&log.read().unwrap()), [3, 7]);
        assert_eq!(seqs(&log.history().unwrap()), (1..=7).collect::<Vec<_>>());
        // The next compaction moves them.
        rebase_keeping(&log, &[7]).unwrap();
        assert!(log.segments().unwrap().is_empty());
        assert_eq!(segments_in(&archive).unwrap().len(), moved.len() + 1);
        assert_eq!(seqs(&log.history().unwrap()), (1..=7).collect::<Vec<_>>());
    }

    #[test]
    fn a_read_a_compaction_crossed_starts_again_on_the_new_base() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path());
        for _ in 0..3 {
            log.append(draft("a", None)).unwrap();
        }
        let mut events = Vec::new();
        let mut mark = Mark::default();
        log.read_on(&mut events, &mut mark).unwrap();
        assert_eq!(events.len(), 3);
        rebase_keeping(&log, &[2]).unwrap();
        log.append(draft("b", None)).unwrap();
        // The mark is from before the compaction.
        log.read_on(&mut events, &mut mark).unwrap();
        assert_eq!(seqs(&events), [2, 4]);
        assert_eq!(events, log.read().unwrap());
        // And from here it reads on as before.
        log.append(draft("c", None)).unwrap();
        log.read_on(&mut events, &mut mark).unwrap();
        assert_eq!(seqs(&events), [2, 4, 5]);
    }

    #[test]
    fn what_a_compaction_keeps_must_be_in_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path());
        for _ in 0..3 {
            log.append(draft("a", None)).unwrap();
        }
        for wrong in ["changed", "reordered"] {
            let result = log.rebase(|events, _| {
                let mut keep = vec![events[0].clone(), events[1].clone()];
                if wrong == "changed" {
                    keep[0].actor = "别人".to_string();
                } else {
                    keep.reverse();
                }
                Ok::<_, std::convert::Infallible>(Rebase {
                    keep,
                    gone: Vec::new(),
                })
            });
            assert!(matches!(result, Err(AppendError::Log(_))), "{wrong}");
        }
        assert!(!dir.path().join(BASE_FILE).exists());
        assert_eq!(log.read().unwrap().len(), 3);
    }

    #[test]
    fn the_floor_remembers_the_cards_that_are_only_in_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::at(dir.path());
        assert_eq!(log.floor().unwrap(), Floor::default());
        for card in 1..=3 {
            log.append(Draft {
                kind: "dispatched".to_string(),
                card: Some(card),
                key: Some(format!("k{card}")),
                ..draft("", None)
            })
            .unwrap();
        }
        let gone = |card: u64| Gone {
            card,
            unit: Some("m07".to_string()),
            title: format!("卡 {card}"),
            key: Some(format!("k{card}")),
            first: card,
            last: card,
            ended: String::new(),
        };
        log.rebase(|events, _| {
            Ok::<_, std::convert::Infallible>(Rebase {
                keep: vec![events[1].clone()],
                gone: vec![gone(1), gone(3)],
            })
        })
        .unwrap();
        let floor = log.floor().unwrap();
        assert_eq!(floor.max_card, 3);
        assert_eq!(floor.gone.keys().copied().collect::<Vec<_>>(), [1, 3]);
        assert_eq!(floor.card_of_key("k3"), Some(3));
        assert_eq!(floor.card_of_key("k2"), None);
        // A later compaction adds to them.
        log.rebase(|_, floor| {
            assert_eq!(floor.gone.len(), 2);
            Ok::<_, std::convert::Infallible>(Rebase {
                keep: Vec::new(),
                gone: vec![gone(2)],
            })
        })
        .unwrap();
        let floor = log.floor().unwrap();
        assert_eq!((floor.max_card, floor.gone.len()), (3, 3));
        assert!(log.read().unwrap().is_empty());
        assert_eq!(log.history().unwrap().len(), 3);
    }

    #[test]
    fn writers_readers_and_a_compactor_never_lose_or_repeat_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let open = move || Log::at(path.clone()).with_segment_bytes(600);
        let writers: Vec<_> = (0..4)
            .map(|_| {
                let open = open.clone();
                std::thread::spawn(move || {
                    for _ in 0..40 {
                        open().append(draft("a", None)).unwrap();
                    }
                })
            })
            .collect();
        let compactor = {
            let open = open.clone();
            std::thread::spawn(move || {
                for _ in 0..25 {
                    // Odd numbers stay.
                    open()
                        .rebase(|events, _| {
                            Ok::<_, std::convert::Infallible>(Rebase {
                                keep: events.iter().filter(|e| e.seq % 2 == 1).cloned().collect(),
                                gone: Vec::new(),
                            })
                        })
                        .unwrap();
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        };
        let reader = {
            let open = open.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    // Whatever a compaction is doing, a read is a whole
                    // one: ascending, and never an even number that a
                    // compaction it started after had already dropped
                    // followed by a gap in what was written since.
                    let events = open().read().unwrap();
                    assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
                }
            })
        };
        for thread in writers {
            thread.join().unwrap();
        }
        compactor.join().unwrap();
        reader.join().unwrap();
        let log = open();
        assert_eq!(seqs(&log.history().unwrap()), (1..=160).collect::<Vec<_>>());
        // Everything written after the last compaction is read.
        let upto = log.floor().unwrap().upto;
        let read = seqs(&log.read().unwrap());
        let since: Vec<u64> = read.iter().copied().filter(|seq| *seq > upto).collect();
        assert_eq!(since, (upto + 1..=160).collect::<Vec<_>>());
        assert!(read.iter().all(|seq| *seq > upto || seq % 2 == 1));
    }
}
