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
use eyre::{bail, Result};
use serde_json::json;
use sha2::{Digest, Sha256};

/// `octoscode olp-runtime-watch` flags (runtime/process/ledger observer).
#[derive(Debug, Parser)]
#[command(
    name = "octoscode olp-runtime-watch",
    about = "Observe agent runtime, live ledger, and progress files; never accepts tasks"
)]
pub struct OlpRuntimeWatchCli {
    /// Herdr agent name to observe.
    #[arg(long)]
    agent: String,
    /// Agent process id (> 0).
    #[arg(long)]
    agent_pid: u32,
    /// Backend process id (> 0).
    #[arg(long)]
    backend_pid: u32,
    /// Existing watcher/monitor pid; repeatable.
    #[arg(long = "monitor-pid")]
    monitor_pid: Vec<u32>,
    /// Repository root directory.
    #[arg(long)]
    repo: PathBuf,
    /// Live UI-protocol JSONL ledger (regular file).
    #[arg(long)]
    ledger: PathBuf,
    /// Wire session id.
    #[arg(long)]
    session: String,
    /// Target turn id.
    #[arg(long)]
    turn_id: String,
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
    pub backend_pid: u32,
    pub monitor_pid: Vec<u32>,
    pub repo: PathBuf,
    pub ledger: PathBuf,
    pub session: String,
    pub turn_id: String,
    pub task: Vec<String>,
    pub progress_file: Vec<PathBuf>,
    pub stall_seconds: u64,
    pub interval_ms: u64,
    pub once: bool,
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

/// Hard deadline for one `ps` probe child before it is killed (probe child
/// only — never a watched pid) and reported as `Unknown`.
const PROBE_DEADLINE: Duration = Duration::from_secs(1);
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
fn bounded_run(mut cmd: Command) -> Option<(std::process::ExitStatus, Vec<u8>, u64)> {
    let mut out_file = tempfile::tempfile().ok()?;
    let err_file = tempfile::tempfile().ok()?;
    let out_fd = out_file.try_clone().ok()?;
    let err_fd = err_file.try_clone().ok()?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(out_fd))
        .stderr(Stdio::from(err_fd));
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + PROBE_DEADLINE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(PROBE_POLL),
            Err(_) => return None,
        }
    };
    let status = status?;
    if out_file.metadata().ok()?.len() as usize > PROBE_MAX_OUTPUT {
        return None;
    }
    out_file.seek(std::io::SeekFrom::Start(0)).ok()?;
    let mut raw = Vec::new();
    out_file
        .take(PROBE_MAX_OUTPUT as u64 + 1)
        .read_to_end(&mut raw)
        .ok()?;
    if raw.len() > PROBE_MAX_OUTPUT {
        return None;
    }
    let err_len = err_file.metadata().map(|m| m.len()).unwrap_or(0);
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
    if !status.success() || err_len > PROBE_MAX_OUTPUT as u64 {
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
    if !stat.chars().all(|c| {
        matches!(
            c,
            'R' | 'S' | 'U' | 'I' | 'Z' | '+' | 's' | 'l' | 'W' | 'X' | 'T' | 'N' | 'L' | 'J'
        )
    }) || stat.is_empty()
    {
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
    let date_num: u32 = if (1..=2).contains(&date.len()) && date.chars().all(|c| c.is_ascii_digit())
    {
        date.parse().ok()?
    } else {
        return None;
    };
    if !(1..=31).contains(&date_num) {
        return None;
    }
    let hms_parts: Vec<&str> = hms.split(':').collect();
    if hms_parts.len() != 3
        || !hms_parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) {
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
pub fn herdr_agent_status(name: &str) -> Option<String> {
    let mut cmd = Command::new("herdr");
    cmd.args(["agent", "get", name]);
    let (status, raw, err_len) = bounded_run(cmd)?;
    if !status.success() || err_len > 0 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let agent = value.get("agent")?;
    let got = agent.get("name")?.as_str()?;
    if got != name {
        return None;
    }
    Some(agent.get("status")?.as_str()?.to_string())
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
fn observation_record(
    s: &IdentitySnapshot,
    agent: &ProbeState,
    backend: &ProbeState,
    monitors: &[(u32, ProbeState)],
    seq: u64,
    iso_time: &str,
    agent_status: Option<&str>,
) -> serde_json::Value {
    json!({
        "seq": seq,
        "observation_time": iso_time,
        "identity": {
            "agent": s.agent,
            "agent_pid": s.agent_pid,
            "backend_pid": s.backend_pid,
            "repo": s.repo,
            "ledger": s.ledger,
            "session": s.session,
            "turn_id": s.turn_id,
            "task": s.task,
        },
        "task_accepted": false,
        "agent": probe_json(agent),
        "backend": probe_json(backend),
        "monitors": monitors
            .iter()
            .map(|(pid, p)| json!({"pid": pid, "state": monitor_probe_json(p)}))
            .collect::<Vec<_>>(),
        "observation_state": "watching",
        "agent_status": agent_status.unwrap_or("observation_unknown"),
        "agent_status_advisory": if agent_status.is_some() { "observed" } else { "herdr_unavailable_observation_unknown" },
    })
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
            ("backend-pid", self.backend_pid),
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
        if self.session.trim().is_empty() || self.turn_id.trim().is_empty() {
            bail!("olp-runtime-watch: --session and --turn-id must be non-empty");
        }
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
        let ledger = std::fs::metadata(&self.ledger)
            .map_err(|e| eyre::eyre!("--ledger {}: {e}", self.ledger.display()))?;
        if !ledger.is_file() {
            bail!(
                "olp-runtime-watch: --ledger {} is not a regular file",
                self.ledger.display()
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
                ledger: self.ledger,
                session: self.session,
                turn_id: self.turn_id,
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
    json!({
        "identity": {
            "agent": s.agent,
            "agent_pid": s.agent_pid,
            "backend_pid": s.backend_pid,
            "monitor_pid": s.monitor_pid,
            "repo": s.repo,
            "ledger": s.ledger,
            "session": s.session,
            "turn_id": s.turn_id,
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
    let mut last_birth: Option<String> = None;
    loop {
        seq += 1;
        let agent = probe_pid(s.agent_pid);
        let backend = probe_pid(s.backend_pid);
        let monitors: Vec<(u32, ProbeState)> =
            s.monitor_pid.iter().map(|p| (*p, probe_pid(*p))).collect();
        let agent_status = herdr_agent_status(&s.agent);
        if let ProbeState::Alive(birth) = &agent {
            if let Some(prev) = &last_birth {
                if prev != birth {
                    println!(
                        "{}",
                        json!({"seq": seq, "alert": "process_identity_changed", "pid": s.agent_pid, "task_accepted": false})
                    );
                }
            }
            last_birth = Some(birth.clone());
        }
        println!(
            "{}",
            observation_record(
                s,
                &agent,
                &backend,
                &monitors,
                seq,
                &iso_now(),
                agent_status.as_deref()
            )
        );
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
