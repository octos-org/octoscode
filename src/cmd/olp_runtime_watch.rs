//! `octoscode olp-runtime-watch` — slice 1: validated identity snapshot.
//!
//! OBSERVATION ONLY: this command never writes boards, never accepts tasks,
//! never kills/restarts agents. It freezes a validated runtime observation
//! identity (agent/backend/monitor PIDs, repo, ledger, session, turn, task)
//! and emits `task_accepted:false` unconditionally. Acceptance authority
//! stays with the outer loop; there is deliberately no flag to set it true.

use std::path::PathBuf;

use clap::Parser;
use eyre::{Result, bail};
use serde_json::json;

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

impl OlpRuntimeWatchCli {
    /// Validate PIDs/intervals/threshold and file paths; freeze the snapshot.
    pub fn into_args(self) -> Result<OlpRuntimeWatchArgs> {
        for (name, pid) in [("agent-pid", self.agent_pid), ("backend-pid", self.backend_pid)] {
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
        if self.stall_seconds == 0 || self.interval_ms == 0 {
            bail!("olp-runtime-watch: --stall-seconds and --interval-ms must be >= 1");
        }
        let repo = std::fs::metadata(&self.repo)
            .map_err(|e| eyre::eyre!("--repo {}: {e}", self.repo.display()))?;
        if !repo.is_dir() {
            bail!("olp-runtime-watch: --repo {} is not a directory", self.repo.display());
        }
        let ledger = std::fs::metadata(&self.ledger)
            .map_err(|e| eyre::eyre!("--ledger {}: {e}", self.ledger.display()))?;
        if !ledger.is_file() {
            bail!("olp-runtime-watch: --ledger {} is not a regular file", self.ledger.display());
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

/// Checkpoint run: emit the initial identity JSON line, exit 0. Full watch,
/// ps probes, ledger filtering, and stall detection arrive in later slices.
pub fn run(args: OlpRuntimeWatchArgs) -> Result<i32> {
    let s = &args.snapshot;
    println!(
        "{}",
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
    );
    Ok(0)
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
            "olp-runtime-watch", "--agent", "inner", "--agent-pid", "10",
            "--backend-pid", "20", "--monitor-pid", "30", "--monitor-pid", "31",
            "--repo", dir.path().join("repo").to_str().unwrap(),
            "--ledger", dir.path().join("ledger.jsonl").to_str().unwrap(),
            "--session", "s1", "--turn-id", "t1", "--task", "guard",
            "--stall-seconds", "60", "--interval-ms", "500", "--once",
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
                "olp-runtime-watch", "--agent", "a", "--agent-pid", pid,
                "--backend-pid", "2", "--repo", dir.path().join("repo").to_str().unwrap(),
                "--ledger", dir.path().join("ledger.jsonl").to_str().unwrap(),
                "--session", "s", "--turn-id", "t",
            ])
        };
        assert!(mk("0").into_args().is_err());
        assert!(mk("1").into_args().is_ok());
        let bad = OlpRuntimeWatchCli::parse_from([
            "olp-runtime-watch", "--agent", "a", "--agent-pid", "1",
            "--backend-pid", "2", "--repo", "/definitely/missing/repo",
            "--ledger", dir.path().join("ledger.jsonl").to_str().unwrap(),
            "--session", "s", "--turn-id", "t",
        ]);
        assert!(bad.into_args().is_err());
    }
}
