//! `octoscode olp-runtime-watch` — slice 1: validated identity snapshot.
//!
//! OBSERVATION ONLY: this command never writes boards, never accepts tasks,
//! never kills/restarts agents. It freezes a validated runtime observation
//! identity (agent/backend/monitor PIDs, repo, ledger, session, turn, task)
//! and emits `task_accepted:false` unconditionally. Acceptance authority
//! stays with the outer loop; there is deliberately no flag to set it true.

use std::io::{Read, Seek};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Parser;
use eyre::{Result, bail};
use serde_json::json;
use sha2::{Digest, Sha256};

/// `octoscode olp-runtime-watch` flags (runtime/process/ledger observer).
#[derive(Debug, Parser)]
#[command(
    name = "octoscode olp-runtime-watch",
    about = "Observe agent runtime, live ledger, and progress files; never accepts tasks",
    long_about = "Observe agent runtime, live ledger, and progress files; never accepts tasks.\n\n\
Works for any inner loop herdr knows (octoscode, Claude Code, Codex). The \
octos-specific inputs are optional: without --backend-pid there is no backend \
probe; without --ledger/--session/--turn-id (given together or not at all) \
there is no turn layer, and herdr's `working` status stands in for ledger \
activity in the stall advisory."
)]
pub struct OlpRuntimeWatchCli {
    /// Herdr agent name to observe.
    #[arg(long)]
    agent: String,
    /// Agent process id (> 0).
    #[arg(long)]
    agent_pid: u32,
    /// Backend process id (> 0); octos only. Omit for Claude Code / Codex.
    #[arg(long)]
    backend_pid: Option<u32>,
    /// Existing watcher/monitor pid; repeatable.
    #[arg(long = "monitor-pid")]
    monitor_pid: Vec<u32>,
    /// Repository root directory.
    #[arg(long)]
    repo: PathBuf,
    /// Live UI-protocol JSONL ledger (regular file); octos only.
    #[arg(long)]
    ledger: Option<PathBuf>,
    /// Wire session id (with --ledger).
    #[arg(long)]
    session: Option<String>,
    /// Target turn id (with --ledger).
    #[arg(long)]
    turn_id: Option<String>,
    /// Task identity words; repeatable.
    #[arg(long = "task")]
    task: Vec<String>,
    /// Progress file owned by the task; repeatable.
    #[arg(long = "progress-file")]
    progress_file: Vec<PathBuf>,
    /// No-progress stall threshold in seconds (>= 1).
    #[arg(long, default_value_t = 300)]
    stall_seconds: u64,
    /// Poll interval in milliseconds (>= 1).
    #[arg(long, default_value_t = 5000)]
    interval_ms: u64,
    /// Emit one snapshot then exit (default: watch; later slice).
    #[arg(long)]
    once: bool,
}

/// Frozen observation identity: what the guard observes, never judges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentitySnapshot {
    pub agent: String,
    pub agent_pid: u32,
    pub backend_pid: Option<u32>,
    pub monitor_pid: Vec<u32>,
    pub repo: PathBuf,
    pub ledger: Option<TurnLedger>,
    pub task: Vec<String>,
    pub progress_file: Vec<PathBuf>,
    pub stall_seconds: u64,
    pub interval_ms: u64,
    pub once: bool,
}

/// The octos turn the ledger layer follows. Inner loops without a ledger
/// (Claude Code, Codex) have none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnLedger {
    pub path: PathBuf,
    pub session: String,
    pub turn_id: String,
}

impl IdentitySnapshot {
    fn ledger_json(&self) -> (serde_json::Value, serde_json::Value, serde_json::Value) {
        match &self.ledger {
            Some(l) => (json!(l.path), json!(l.session), json!(l.turn_id)),
            None => (json!(null), json!(null), json!(null)),
        }
    }
}

/// Parsed, validated arguments ready to run.
#[derive(Debug)]
pub struct OlpRuntimeWatchArgs {
    pub snapshot: IdentitySnapshot,
    /// Always false; no code path may set it true.
    pub task_accepted: bool,
}

/// Read-only bounded `ps` probe result for one pid. A timeout/failure is
/// [`ProbeState::Unknown`] — never inferred as death.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeState {
    /// Alive, carrying a birth identity (hashed ps lstart) for reuse detection.
    Alive(String),
    Missing,
    Zombie,
    Unknown,
}

/// Hard deadline for one bounded probe child (ps/herdr) before it is killed
/// (probe child only — never a watched pid) and reported as `Unknown`.
/// 2s: fork/exec on a loaded macOS host was observed exceeding 1s under
/// parallel test load, misclassifying healthy probes; still firmly bounded.
const PROBE_DEADLINE: Duration = Duration::from_secs(2);
/// Poll cadence while waiting for the probe child.
const PROBE_POLL: Duration = Duration::from_millis(10);
/// Upper bound on accepted ps output; a listing this large is a parse failure.
const PROBE_MAX_OUTPUT: usize = 8 * 1024;

/// Bounded `ps -o pid=,lstart=,state=` probe. Birth identity hashes ONLY the
/// `pid + lstart` fields, so a later STAT-column change (state transitions
/// like R→S) is not misreported as PID reuse; the STAT column is parsed
/// separately for Zombie. The probe child is polled with `try_wait` under a
/// ~1s deadline and killed/reaped on timeout (probe child only — never a
/// watched pid). Raw ps text is never emitted.
/// Run `cmd` with stdout/stderr redirected to anonymous temp FILES (not
/// pipes: no reader thread, no hang if a grandchild inherits the fd), polling
/// `try_wait` under PROBE_DEADLINE. On timeout the probe child is killed and
/// reaped — never a watched pid — and `None` is returned. Output is size-
/// bounded via metadata before a regular-file read.
/// RAII: from spawn until Drop, any early return (including `?`) kills +
/// reaps OUR probe child. The watched pid is never touched.
struct ReapOnDrop(Option<std::process::Child>);

impl ReapOnDrop {
    fn reap(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        self.reap();
    }
}

fn bounded_run(mut cmd: Command) -> Option<(std::process::ExitStatus, Vec<u8>, u64)> {
    let out_file = tempfile::tempfile().ok()?;
    let err_file = tempfile::tempfile().ok()?;
    let out_fd = out_file.try_clone().ok()?;
    let err_fd = err_file.try_clone().ok()?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(out_fd))
        .stderr(Stdio::from(err_fd));
    let mut child = ReapOnDrop(Some(cmd.spawn().ok()?));
    let deadline = Instant::now() + PROBE_DEADLINE;
    let status = loop {
        match child.0.as_mut()?.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() >= deadline => {
                // Deadline: guard Drop reaps our probe child, then Unknown.
                return None;
            }
            Ok(None) => {
                // In-loop size bound: over-large output or an UNKNOWABLE size
                // (metadata failure — never assumed empty) => Unknown now.
                let over = |f: &std::fs::File| {
                    f.metadata()
                        .map(|m| m.len() > PROBE_MAX_OUTPUT as u64)
                        .map_err(|_| ())
                };
                if matches!(over(&out_file), Ok(true))
                    || matches!(over(&err_file), Ok(true))
                    || over(&out_file).is_err()
                    || over(&err_file).is_err()
                {
                    return None;
                }
                std::thread::sleep(PROBE_POLL);
            }
            // try_wait failure: Drop reaps before we bail, no orphan.
            Err(_) => return None,
        }
    };
    let status = status?;
    if out_file.metadata().ok()?.len() as usize > PROBE_MAX_OUTPUT {
        return None;
    }
    let mut out_file = out_file;
    out_file.seek(std::io::SeekFrom::Start(0)).ok()?;
    let mut raw = Vec::new();
    out_file
        .take(PROBE_MAX_OUTPUT as u64 + 1)
        .read_to_end(&mut raw)
        .ok()?;
    if raw.len() > PROBE_MAX_OUTPUT {
        return None;
    }
    // stderr size must be KNOWABLE, not assumed: metadata failure is a
    // failed observation (Unknown), never "empty stderr".
    let err_len = err_file.metadata().ok()?.len();
    if err_len > PROBE_MAX_OUTPUT as u64 {
        return None;
    }
    child.0 = None; // exited normally; nothing left to reap
    Some((status, raw, err_len))
}

/// Bounded, strictly-parsed `ps -o pid=,lstart=,state= -p <pid>` probe.
pub fn probe_pid(pid: u32) -> ProbeState {
    let mut cmd = Command::new("ps");
    cmd.args(["-o", "pid=,lstart=,state=", "-p", &pid.to_string()])
        .env("LC_ALL", "C");
    let Some((status, raw, err_len)) = bounded_run(cmd) else {
        return ProbeState::Unknown;
    };
    let out = String::from_utf8_lossy(&raw).trim().to_string();
    let exit_code = status.code();
    if out.is_empty() {
        // BSD ps for a valid-format but nonexistent pid: exit 0 or 1 with
        // BOTH stdout and stderr empty => "no such process" => Missing.
        // Exit 42 / signal death / any stderr => Unknown, never death.
        return if err_len == 0 && matches!(exit_code, Some(0) | Some(1)) {
            ProbeState::Missing
        } else {
            ProbeState::Unknown
        };
    }
    if !status.success() || err_len > 0 {
        return ProbeState::Unknown;
    }
    parse_ps_listing(&out, pid).unwrap_or(ProbeState::Unknown)
}

/// Strict parse of one `ps -o pid=,lstart=,state=` LC_ALL=C listing line:
/// exactly one line, first field == requested pid, then the fixed five
/// lstart fields (Day Mon DD HH:MM:SS YYYY), then one STAT token from the
/// known BSD set. Birth identity hashes pid + lstart ONLY, so a STAT change
/// (R->S) is never misreported as PID reuse.
fn parse_ps_listing(line: &str, want: u32) -> Option<ProbeState> {
    let mut lines = line.lines().filter(|l| !l.trim().is_empty());
    let first = lines.next()?;
    if lines.next().is_some() {
        return None; // exactly one listing row
    }
    let fields: Vec<&str> = first.split_whitespace().collect();
    // pid + 5 lstart fields + stat = 7 tokens.
    if fields.len() != 7 {
        return None;
    }
    let ps_pid: u32 = fields[0].parse().ok()?;
    if ps_pid != want {
        return None;
    }
    let stat = fields[6];
    // STAT grammar: exactly one leading state char, then 0+ modifiers.
    // A bare modifier ('+' alone) or an unknown char rejects the row.
    let mut stat_chars = stat.chars();
    let first = stat_chars.next()?;
    if !matches!(first, 'R' | 'S' | 'U' | 'I' | 'Z' | 'T' | 'W' | 'X') {
        return None;
    }
    if !stat_chars.all(|c| matches!(c, '+' | 's' | 'l' | 'N' | 'L' | 'J')) {
        return None;
    }
    // lstart is five fixed-width C-locale fields; validate the shape of the
    // time and year tokens so arbitrary text is never accepted as Alive.
    let (day, mon, date, hms, year) = (fields[1], fields[2], fields[3], fields[4], fields[5]);
    if !matches!(day, "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun") {
        return None;
    }
    if !matches!(
        mon,
        "Jan"
            | "Feb"
            | "Mar"
            | "Apr"
            | "May"
            | "Jun"
            | "Jul"
            | "Aug"
            | "Sep"
            | "Oct"
            | "Nov"
            | "Dec"
    ) {
        return None;
    }
    // lstart is fixed-width; each of HH/MM/SS must be exactly 2 digits.
    // (chrono's %H:%M:%S tolerates single digits, so enforce width here.)
    let hms_parts: Vec<&str> = hms.split(':').collect();
    if hms_parts.len() != 3
        || hms_parts
            .iter()
            .any(|p| p.len() != 2 || !p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    // Build the instant with chrono (real calendar: rejects Feb 31, 99:99:99,
    // month/day/time out of range) — no hand-rolled year math.
    let date_padded = format!("{:0>2}", date);
    let dt = chrono::NaiveDateTime::parse_from_str(
        &format!("{year}-{mon}-{date_padded}T{hms}"),
        "%Y-%b-%dT%H:%M:%S",
    )
    .ok()?;
    // Cross-check ps's weekday token against the real calendar weekday.
    let real_dow = dt.format("%a").to_string();
    if real_dow != day {
        return None;
    }
    let lstart = format!("{day} {mon} {date} {hms} {year}");
    let birth = hex(&Sha256::digest(format!("{ps_pid} {lstart}").as_bytes()));
    Some(if stat.contains('Z') {
        ProbeState::Zombie
    } else {
        ProbeState::Alive(birth)
    })
}

/// Bounded `herdr agent get <name>` probe: decode the JSON reply, require an
/// EXACT agent-name match, and expose ONLY the status string — never stdout
/// payload or stderr. Unavailable/malformed/timeout => None (advisory
/// observation_unknown; no crash, no kill, never aborts other probes).
/// Known Herdr agent_status values (live-calibrated: working/done observed
/// on real agents; starting/idle are the documented remainder). Anything
/// else is an UNKNOWN observation — never echoed back (no string leak).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Working,
    Done,
    Idle,
    Starting,
}

/// Classified Herdr agent status for one observation cycle: a terminal
/// Done/Idle is completion-UNVERIFIED (awaiting outer review), never
/// acceptance; unknown values / probe failures are Unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentObservation {
    Working,
    /// Herdr reports a terminal state — task completion still unverified.
    CompletionUnverifiedAwaitingReview,
    Starting,
    Unknown,
}

pub fn herdr_agent_status(name: &str) -> AgentObservation {
    let herdr = std::env::var_os("OLP_HERDR").unwrap_or_else(|| "herdr".into());
    let mut cmd = Command::new(herdr);
    cmd.args(["agent", "get", name]);
    let Some((status, raw, err_len)) = bounded_run(cmd) else {
        return AgentObservation::Unknown;
    };
    if !status.success() || err_len > 0 {
        return AgentObservation::Unknown;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return AgentObservation::Unknown;
    };
    // Real envelope: {"id":...,"result":{"agent":{...,"agent_status":...}}};
    // errors come back as {"error":{...}} with a nonzero exit status.
    let Some(agent) = value.get("result").and_then(|r| r.get("agent")) else {
        return AgentObservation::Unknown;
    };
    // EXACT name match required; a mismatched reply is not this agent.
    if agent.get("name").and_then(|n| n.as_str()) != Some(name) {
        return AgentObservation::Unknown;
    }
    // Whitelist: only known values pass through; the raw string is never
    // emitted, so an unexpected value cannot leak into NDJSON.
    match agent.get("agent_status").and_then(|s| s.as_str()) {
        Some("working") => AgentObservation::Working,
        Some("done") | Some("idle") => AgentObservation::CompletionUnverifiedAwaitingReview,
        Some("starting") => AgentObservation::Starting,
        _ => AgentObservation::Unknown,
    }
}

fn agent_status_json(o: &AgentObservation) -> serde_json::Value {
    match o {
        AgentObservation::Working => json!({"state": "working"}),
        AgentObservation::CompletionUnverifiedAwaitingReview => json!({
            "state": "completion_unverified",
            "disposition": "awaiting_review",
        }),
        AgentObservation::Starting => json!({"state": "starting"}),
        AgentObservation::Unknown => json!({"state": "observation_unknown"}),
    }
}

/// Lowercase hex of a digest (used for birth identities; never raw ps text).
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn probe_json(p: &ProbeState) -> serde_json::Value {
    match p {
        ProbeState::Alive(birth) => json!({"state": "alive", "birth": birth}),
        ProbeState::Missing => json!({"state": "process_missing"}),
        ProbeState::Zombie => json!({"state": "process_zombie"}),
        ProbeState::Unknown => json!({"state": "observation_unknown"}),
    }
}

/// One observation cycle's whitelisted NDJSON record.
/// Everything one observation cycle contributes to the snapshot record.
struct Cycle<'a> {
    agent: &'a ProbeState,
    backend: Option<&'a ProbeState>,
    monitors: &'a [(u32, ProbeState)],
    seq: u64,
    iso_time: &'a str,
    agent_status: &'a AgentObservation,
    identity_alerts: &'a [u32],
    ledger: &'a LedgerObservation,
    progress: &'a ProgressObservation,
}

fn observation_record(s: &IdentitySnapshot, c: &Cycle<'_>) -> serde_json::Value {
    let (ledger, session, turn_id) = s.ledger_json();
    json!({
        "seq": c.seq,
        "observation_time": c.iso_time,
        "identity": {
            "agent": s.agent,
            "agent_pid": s.agent_pid,
            "backend_pid": s.backend_pid,
            "repo": s.repo,
            "ledger": ledger,
            "session": session,
            "turn_id": turn_id,
            "task": s.task,
        },
        "task_accepted": false,
        "agent": probe_json(c.agent),
        "backend": c.backend.map(probe_json).unwrap_or_else(|| json!({"state": "not_observed"})),
        "monitors": c.monitors
            .iter()
            .map(|(pid, p)| json!({"pid": pid, "state": monitor_probe_json(p)}))
            .collect::<Vec<_>>(),
        // A finished turn — seen through Herdr or in the ledger — is only
        // ever "awaiting review": completion is not acceptance.
        "observation_state": match c.agent_status {
            AgentObservation::CompletionUnverifiedAwaitingReview => {
                "awaiting_review"
            }
            _ if c.ledger.turn_finished => "awaiting_review",
            _ => "watching",
        },
        "alerts": c.identity_alerts
            .iter()
            .map(|pid| {
                json!({
                    "kind": "process_identity_changed",
                    "pid": pid,
                })
            })
            .chain(c.ledger.alerts.iter().cloned())
            .chain(c.progress.alert.iter().cloned())
            .collect::<Vec<_>>(),
        "ledger": c.ledger.record,
        "progress": c.progress.record,
        "agent_status": agent_status_json(c.agent_status),
        "agent_status_advisory": match c.agent_status {
            AgentObservation::Unknown => "herdr_unavailable_or_unknown_value",
            _ => "observed",
        },
    })
}

/// What one cycle learned from the live ledger.
struct LedgerObservation {
    /// The whitelisted poll result (offsets, turn state, unknown code).
    record: serde_json::Value,
    /// Target-turn alerts, tagged `"source": "ledger"`.
    alerts: Vec<serde_json::Value>,
    /// The ledger shows the target turn completed, cancelled or aborted.
    turn_finished: bool,
    /// New bytes were consumed after the initial backlog: the agent is doing
    /// something, whether or not it is making progress.
    activity: bool,
}

/// Incremental reader of the target turn's records in the live ledger. The
/// parsing, scoping, rotation and privacy rules live in
/// `olp_runtime_watch_ledger`; this layer only folds a poll into the record.
#[cfg(unix)]
struct LedgerLayer {
    cursor: super::olp_runtime_watch_ledger::LedgerCursor,
    last_offset: Option<u64>,
}

#[cfg(unix)]
impl LedgerLayer {
    fn new(l: &TurnLedger) -> Self {
        Self {
            cursor: super::olp_runtime_watch_ledger::LedgerCursor::new(
                l.path.clone(),
                l.session.clone(),
                l.turn_id.clone(),
            ),
            last_offset: None,
        }
    }

    fn observe(&mut self) -> LedgerObservation {
        use super::olp_runtime_watch_ledger::LedgerTurnState;
        let poll = self.cursor.poll();
        let alerts = poll
            .alerts
            .iter()
            .filter_map(|a| serde_json::to_value(a).ok())
            .map(|mut v| {
                if let Some(map) = v.as_object_mut() {
                    map.insert("source".into(), json!("ledger"));
                }
                v
            })
            .collect();
        let turn_finished = matches!(
            poll.turn_state.map(|t| t.state),
            Some(LedgerTurnState::CompletionUnverified | LedgerTurnState::AwaitingReview)
        );
        // Reading the backlog that already existed is not new activity.
        let activity =
            !poll.initial_scan && self.last_offset.is_some_and(|prev| poll.byte_offset > prev);
        self.last_offset = Some(poll.byte_offset);
        LedgerObservation {
            record: serde_json::to_value(&poll)
                .unwrap_or_else(|_| json!({"observation_unknown": "ledger_unserializable"})),
            alerts,
            turn_finished,
            activity,
        }
    }
}

/// The ledger reader identifies files by device and inode, so it is only
/// built on Unix. Elsewhere the layer says so instead of guessing.
#[cfg(not(unix))]
struct LedgerLayer;

#[cfg(not(unix))]
impl LedgerLayer {
    fn new(_l: &TurnLedger) -> Self {
        Self
    }

    fn observe(&mut self) -> LedgerObservation {
        LedgerObservation {
            record: json!({"observation_unknown": "ledger_unsupported_platform"}),
            alerts: Vec::new(),
            turn_finished: false,
            activity: false,
        }
    }
}

/// No ledger was given (an inner loop that is not octos): the layer reports
/// that it observes nothing, rather than guessing.
fn no_ledger_observation() -> LedgerObservation {
    LedgerObservation {
        record: json!({"not_observed": "no_ledger"}),
        alerts: Vec::new(),
        turn_finished: false,
        activity: false,
    }
}

/// What one cycle learned about real progress.
struct ProgressObservation {
    record: serde_json::Value,
    alert: Option<serde_json::Value>,
}

/// Largest progress file read in full; a bigger one is fingerprinted by its
/// length and modification time instead.
const PROGRESS_MAX_READ: u64 = 16 * 1024 * 1024;

/// Detects "busy but not moving": the bytes of the caller-named progress
/// files and the repository HEAD stay the same for `--stall-seconds`. It is
/// an advisory — a small read-only task legitimately changes nothing — and
/// it never restarts, kills or prompts anything.
struct ProgressTracker {
    fingerprint: Option<String>,
    last_change: Instant,
    /// Whether the ledger showed activity at any point since `last_change`.
    activity_since_change: bool,
}

impl ProgressTracker {
    fn new() -> Self {
        Self {
            fingerprint: None,
            last_change: Instant::now(),
            activity_since_change: false,
        }
    }

    fn git_head(repo: &std::path::Path) -> Option<String> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(repo).args(["rev-parse", "HEAD"]);
        let (status, out, err_len) = bounded_run(cmd)?;
        if !status.success() || err_len != 0 {
            return None;
        }
        let head = String::from_utf8(out).ok()?.trim().to_string();
        (head.len() >= 40 && head.bytes().all(|b| b.is_ascii_hexdigit())).then_some(head)
    }

    fn observe(&mut self, s: &IdentitySnapshot, ledger_activity: bool) -> ProgressObservation {
        if s.progress_file.is_empty() {
            // Without named files there is nothing to compare: commits alone
            // are too rare to tell a stall from normal work.
            return ProgressObservation {
                record: json!({"tracked_files": 0, "stall_detection": "disabled_no_progress_files"}),
                alert: None,
            };
        }
        let mut hasher = Sha256::new();
        let mut missing: Vec<&PathBuf> = Vec::new();
        for path in &s.progress_file {
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update([0u8]);
            match std::fs::metadata(path) {
                Ok(meta) if meta.is_file() && meta.len() <= PROGRESS_MAX_READ => {
                    match std::fs::read(path) {
                        Ok(bytes) => hasher.update(Sha256::digest(&bytes)),
                        Err(_) => hasher.update(b"unreadable"),
                    }
                }
                Ok(meta) if meta.is_file() => {
                    hasher.update(meta.len().to_le_bytes());
                    if let Ok(modified) = meta.modified() {
                        if let Ok(d) = modified.duration_since(std::time::UNIX_EPOCH) {
                            hasher.update(d.as_nanos().to_le_bytes());
                        }
                    }
                }
                _ => {
                    hasher.update(b"missing");
                    missing.push(path);
                }
            }
        }
        let head = Self::git_head(&s.repo);
        hasher.update(head.as_deref().unwrap_or("head_unknown").as_bytes());
        let fingerprint = hex(&hasher.finalize());

        let changed = self
            .fingerprint
            .as_ref()
            .is_some_and(|prev| *prev != fingerprint);
        if self.fingerprint.is_none() || changed {
            // First sight is the baseline, not progress; a real change resets.
            self.fingerprint = Some(fingerprint);
            self.last_change = Instant::now();
            self.activity_since_change = false;
        }
        self.activity_since_change |= ledger_activity;
        let age = self.last_change.elapsed().as_secs();
        let stalled = age >= s.stall_seconds;
        let alert = stalled.then(|| {
            json!({
                "kind": "stalled_no_progress",
                "advisory": true,
                "progress_age_seconds": age,
                // Events without byte changes is a read-only livelock; no
                // events at all is a silent stall. Neither is acted on here.
                "activity": if self.activity_since_change { "events_continuing" } else { "no_events" },
            })
        });
        ProgressObservation {
            record: json!({
                "tracked_files": s.progress_file.len(),
                "missing_files": missing,
                "git_head": match &head { Some(_) => "observed", None => "observation_unknown" },
                "progress_age_seconds": age,
                "changed_this_cycle": changed,
            }),
            alert,
        }
    }
}

/// RFC3339 UTC timestamp of "now" with second precision (chrono, existing dep).
fn iso_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Monitors are watchers: a missing one is `watcher_missing`, distinct from
/// the agent's own `process_missing`.
fn monitor_probe_json(p: &ProbeState) -> serde_json::Value {
    match p {
        ProbeState::Alive(birth) => json!({"state": "alive", "birth": birth}),
        ProbeState::Missing => json!({"state": "watcher_missing"}),
        ProbeState::Zombie => json!({"state": "process_zombie"}),
        ProbeState::Unknown => json!({"state": "observation_unknown"}),
    }
}

impl OlpRuntimeWatchCli {
    /// Validate PIDs/intervals/threshold and file paths; freeze the snapshot.
    pub fn into_args(self) -> Result<OlpRuntimeWatchArgs> {
        for (name, pid) in [
            ("agent-pid", self.agent_pid),
            ("backend-pid", self.backend_pid.unwrap_or(1)),
        ] {
            if pid == 0 {
                bail!("olp-runtime-watch: --{name} must be a positive pid");
            }
        }
        if let Some(bad) = self.monitor_pid.iter().find(|p| **p == 0) {
            bail!("olp-runtime-watch: --monitor-pid {bad} must be positive");
        }
        if self.agent.trim().is_empty() {
            bail!("olp-runtime-watch: --agent must be non-empty");
        }
        let ledger = match (self.ledger, self.session, self.turn_id) {
            (None, None, None) => None,
            (Some(path), Some(session), Some(turn_id)) => {
                if session.trim().is_empty() || turn_id.trim().is_empty() {
                    bail!("olp-runtime-watch: --session and --turn-id must be non-empty");
                }
                let meta = std::fs::metadata(&path)
                    .map_err(|e| eyre::eyre!("--ledger {}: {e}", path.display()))?;
                if !meta.is_file() {
                    bail!(
                        "olp-runtime-watch: --ledger {} is not a regular file",
                        path.display()
                    );
                }
                Some(TurnLedger {
                    path,
                    session,
                    turn_id,
                })
            }
            _ => {
                return Err(eyre::eyre!(
                    "olp-runtime-watch: --ledger, --session and --turn-id go together (all three, or none for an inner loop without an octos ledger)"
                ));
            }
        };
        if self.task.iter().all(|t| t.trim().is_empty()) {
            bail!("olp-runtime-watch: --task requires at least one non-empty entry");
        }
        if self.stall_seconds == 0 || self.interval_ms == 0 {
            bail!("olp-runtime-watch: --stall-seconds and --interval-ms must be >= 1");
        }
        let repo = std::fs::metadata(&self.repo)
            .map_err(|e| eyre::eyre!("--repo {}: {e}", self.repo.display()))?;
        if !repo.is_dir() {
            bail!(
                "olp-runtime-watch: --repo {} is not a directory",
                self.repo.display()
            );
        }
        for f in &self.progress_file {
            std::fs::metadata(f)
                .map_err(|e| eyre::eyre!("--progress-file {}: {e}", f.display()))?;
        }
        Ok(OlpRuntimeWatchArgs {
            snapshot: IdentitySnapshot {
                agent: self.agent,
                agent_pid: self.agent_pid,
                backend_pid: self.backend_pid,
                monitor_pid: self.monitor_pid,
                repo: self.repo,
                ledger,
                task: self.task,
                progress_file: self.progress_file,
                stall_seconds: self.stall_seconds,
                interval_ms: self.interval_ms,
                once: self.once,
            },
            task_accepted: false,
        })
    }
}

/// Checkpoint-era initial snapshot line.
pub fn initial_snapshot_json(s: &IdentitySnapshot) -> serde_json::Value {
    let (ledger, session, turn_id) = s.ledger_json();
    json!({
        "identity": {
            "agent": s.agent,
            "agent_pid": s.agent_pid,
            "backend_pid": s.backend_pid,
            "monitor_pid": s.monitor_pid,
            "repo": s.repo,
            "ledger": ledger,
            "session": session,
            "turn_id": turn_id,
            "task": s.task,
        },
        "progress_file": s.progress_file,
        "stall_seconds": s.stall_seconds,
        "interval_ms": s.interval_ms,
        "once": s.once,
        "task_accepted": false,
        "observation_state": "initializing",
    })
}

/// Watch run: bounded ps probes each interval, NDJSON per cycle (heartbeat),
/// `task_accepted:false` in every record. `--once` emits one cycle and exits.
pub fn run(args: OlpRuntimeWatchArgs) -> Result<i32> {
    let s = &args.snapshot;
    let mut seq = 0u64;
    // First-seen birth baselines for EVERY watched pid (agent, backend,
    // monitors). A later different birth => process_identity_changed alert;
    // a missing pid never overwrites its baseline.
    let mut births: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut ledger_layer = s.ledger.as_ref().map(LedgerLayer::new);
    let mut progress_tracker = ProgressTracker::new();
    loop {
        seq += 1;
        let agent = probe_pid(s.agent_pid);
        let backend = s.backend_pid.map(probe_pid);
        let monitors: Vec<(u32, ProbeState)> =
            s.monitor_pid.iter().map(|p| (*p, probe_pid(*p))).collect();
        let agent_status = herdr_agent_status(&s.agent);
        // First-seen baseline per pid: entry() inserts ONLY on first sight;
        // later cycles compare without overwriting, and a missing pid never
        // touches its baseline.
        let mut identity_alerts: Vec<u32> = Vec::new();
        for (pid, state) in std::iter::once((s.agent_pid, &agent))
            .chain(s.backend_pid.zip(backend.as_ref()))
            .chain(monitors.iter().map(|(p, s)| (*p, s)))
        {
            if let ProbeState::Alive(birth) = state {
                match births.entry(pid) {
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(birth.clone());
                    }
                    std::collections::hash_map::Entry::Occupied(slot) => {
                        if slot.get() != birth {
                            identity_alerts.push(pid);
                            // Deliberately do NOT overwrite the baseline: the
                            // alert repeats until the outer loop intervenes.
                        }
                    }
                }
            }
        }
        let ledger = match ledger_layer.as_mut() {
            Some(layer) => layer.observe(),
            None => no_ledger_observation(),
        };
        // Without a ledger, herdr's `working` status is the only sign that
        // the agent is doing something between file changes.
        let activity = match ledger_layer {
            Some(_) => ledger.activity,
            None => matches!(agent_status, AgentObservation::Working),
        };
        let progress = progress_tracker.observe(s, activity);
        let iso_time = iso_now();
        let cycle = Cycle {
            agent: &agent,
            backend: backend.as_ref(),
            monitors: &monitors,
            seq,
            iso_time: &iso_time,
            agent_status: &agent_status,
            identity_alerts: &identity_alerts,
            ledger: &ledger,
            progress: &progress,
        };
        println!("{}", observation_record(s, &cycle));
        if s.once {
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(s.interval_ms.min(60_000)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(dir: &std::path::Path) -> Vec<&'static str> {
        let b = dir.join("repo");
        let l = dir.join("ledger.jsonl");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(&l, "{\"turn\":\"t1\"}\n").unwrap();
        let mut v: Vec<&'static str> = vec![];
        let _ = (&b, &l, &mut v);
        v
    }

    #[test]
    fn herdr_envelope_parses_agent_status_exact_name() {
        let ok = br#"{"id":"cli:agent:get","result":{"agent":{"agent":"octoscode","agent_status":"working","name":"inner_x"}},"type":"agent_info"}"#;
        let v: serde_json::Value = serde_json::from_slice(ok).unwrap();
        let agent = v.get("result").unwrap().get("agent").unwrap();
        assert_eq!(agent.get("name").unwrap().as_str(), Some("inner_x"));
        assert_eq!(agent.get("agent_status").unwrap().as_str(), Some("working"));
        // Old shape (top-level agent / status) must NOT satisfy the read.
        let old = br#"{"agent":{"name":"inner_x","status":"working"}}"#;
        let v2: serde_json::Value = serde_json::from_slice(old).unwrap();
        assert!(v2.get("result").is_none());
        // Error envelope has no result.agent and must fail the read.
        let err = br#"{"error":{"code":"agent_not_found"},"id":"cli:agent:get"}"#;
        let v3: serde_json::Value = serde_json::from_slice(err).unwrap();
        assert!(v3.get("result").and_then(|r| r.get("agent")).is_none());
    }

    #[test]
    fn parse_ps_listing_accepts_real_shape_and_rejects_garbage() {
        let self_pid = std::process::id();
        let real = format!("{self_pid} Fri Sep 25 21:08:03 2026     S   \n");
        match parse_ps_listing(&real, self_pid) {
            Some(ProbeState::Alive(birth)) => assert_eq!(birth.len(), 64),
            other => panic!("expected Alive, got {other:?}"),
        }
        // pid mismatch
        assert!(parse_ps_listing(&real, self_pid + 1).is_none());
        // STAT-only change must keep the same birth identity (no reuse flag).
        let running = real.replace("     S   ", "     R   ");
        let a = parse_ps_listing(&real, self_pid).unwrap();
        let b = parse_ps_listing(&running, self_pid).unwrap();
        assert_eq!(a, b);
        // zombie flag
        let zomb = format!("{self_pid} Fri Sep 25 21:08:03 2026     Z   \n");
        assert!(matches!(
            parse_ps_listing(&zomb, self_pid),
            Some(ProbeState::Zombie)
        ));
        // garbage: two rows, wrong field count, bad month, bad hms
        assert!(parse_ps_listing(&format!("{real}{real}"), self_pid).is_none());
        assert!(parse_ps_listing("123 X", 123).is_none());
        assert!(parse_ps_listing("123 Fri Xop 25 21:08:03 2026 S", 123).is_none());
        assert!(parse_ps_listing("123 Fri Sep 25 21:8:03 2026 S", 123).is_none());
        // single-digit day (space-padded lstart collapses) accepted, with
        // the REAL weekday (2026-09-05 was a Saturday).
        assert!(matches!(
            parse_ps_listing("123 Sat Sep  5 01:02:03 2026 S", 123),
            Some(ProbeState::Alive(_))
        ));
        // weekday/calendar mismatch (Sep 5 2026 is Sat, not Wed) rejected
        assert!(parse_ps_listing("123 Wed Sep  5 01:02:03 2026 S", 123).is_none());
        // impossible calendar date rejected via chrono (Feb 31 does not exist)
        assert!(parse_ps_listing("123 Wed Feb 31 01:02:03 2026 S", 123).is_none());
        // out-of-range time rejected (99:99:99)
        assert!(parse_ps_listing("123 Fri Sep 25 99:99:99 2026 S", 123).is_none());
        // bare modifier without a leading state char rejected
        assert!(parse_ps_listing("123 Fri Sep 25 21:08:03 2026 +", 123).is_none());
        // modifier combinations still valid states
        assert!(matches!(
            parse_ps_listing("123 Fri Sep 25 21:08:03 2026 Ss+", 123),
            Some(ProbeState::Alive(_))
        ));
    }

    #[test]
    fn validates_and_freezes_snapshot_unaccepted() {
        let dir = tempfile::tempdir().unwrap();
        base(dir.path());
        let cli = OlpRuntimeWatchCli::parse_from([
            "olp-runtime-watch",
            "--agent",
            "inner",
            "--agent-pid",
            "10",
            "--backend-pid",
            "20",
            "--monitor-pid",
            "30",
            "--monitor-pid",
            "31",
            "--repo",
            dir.path().join("repo").to_str().unwrap(),
            "--ledger",
            dir.path().join("ledger.jsonl").to_str().unwrap(),
            "--session",
            "s1",
            "--turn-id",
            "t1",
            "--task",
            "guard",
            "--stall-seconds",
            "60",
            "--interval-ms",
            "500",
            "--once",
        ]);
        let args = cli.into_args().unwrap();
        assert!(!args.task_accepted);
        assert_eq!(args.snapshot.monitor_pid, vec![30, 31]);
        assert!(args.snapshot.once);
    }

    #[test]
    fn rejects_zero_pid_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo")).unwrap();
        std::fs::write(dir.path().join("ledger.jsonl"), "x\n").unwrap();
        let mk = |pid: &str| {
            OlpRuntimeWatchCli::parse_from([
                "olp-runtime-watch",
                "--agent",
                "a",
                "--agent-pid",
                pid,
                "--backend-pid",
                "2",
                "--repo",
                dir.path().join("repo").to_str().unwrap(),
                "--ledger",
                dir.path().join("ledger.jsonl").to_str().unwrap(),
                "--session",
                "s",
                "--turn-id",
                "t",
                "--task",
                "guard",
            ])
        };
        assert!(mk("0").into_args().is_err());
        assert!(mk("1").into_args().is_ok());
        let bad = OlpRuntimeWatchCli::parse_from([
            "olp-runtime-watch",
            "--agent",
            "a",
            "--agent-pid",
            "1",
            "--backend-pid",
            "2",
            "--repo",
            "/definitely/missing/repo",
            "--ledger",
            dir.path().join("ledger.jsonl").to_str().unwrap(),
            "--session",
            "s",
            "--turn-id",
            "t",
        ]);
        assert!(bad.into_args().is_err());
    }
}
