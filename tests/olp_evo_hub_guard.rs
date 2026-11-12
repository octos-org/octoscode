#![cfg(unix)]

//! Contract tests for the evolution-loop hub guard (SDD spec:
//! `specs/task-olp-evo-hub-guard.spec.md`).
//!
//! On a multi-machine deployment `scripts/olp-evo-harvest.sh` and
//! `scripts/olp-evo-retro.sh` keep per-machine counters and cursors, so only
//! the hub may run them. These tests drive the real scripts with a hub name
//! configured through `OLP_HUB_HOST` or the hub-host file.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const NOT_THIS_HOST: &str = "olp-test-hub-that-is-not-this-machine";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script(name: &str) -> PathBuf {
    repo_root().join("scripts").join(name)
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn this_host() -> String {
    let out = Command::new("hostname").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Sandbox {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    state: PathBuf,
    /// A hub-host file path that does not exist unless a test writes it, so
    /// the developer's own `~/.octos/outer/hub-host` never leaks in.
    hub_file: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".octos")).unwrap();
        std::fs::write(repo.join(".octos/OUTER_LOOP_REVIEW.md"), "## Active\n").unwrap();
        std::fs::write(repo.join(".octos/EVOLUTION.md"), "").unwrap();
        Self {
            state: dir.path().join("state"),
            hub_file: dir.path().join("hub-host"),
            repo,
            _dir: dir,
        }
    }

    fn run(&self, name: &str, dry_run: bool, hub_env: Option<&str>) -> Output {
        let mut cmd = Command::new("bash");
        cmd.arg(script(name)).arg(&self.repo);
        if dry_run {
            cmd.arg("--dry-run");
        }
        cmd.env("OLP_EVO_STATE", &self.state)
            .env("OLP_HUB_HOST_FILE", &self.hub_file)
            .env_remove("OLP_HUB_HOST");
        if let Some(hub) = hub_env {
            cmd.env("OLP_HUB_HOST", hub);
        }
        cmd.output().unwrap()
    }

    fn evo_board_len(&self) -> u64 {
        std::fs::metadata(self.repo.join(".octos/EVOLUTION.md"))
            .unwrap()
            .len()
    }

    fn assert_untouched(&self) {
        assert!(!self.state.exists(), "state dir must not be created");
        assert_eq!(self.evo_board_len(), 0, "evolution board must be untouched");
    }
}

fn assert_refused(out: &Output) {
    assert_eq!(out.status.code(), Some(3), "stderr: {}", stderr(out));
    let err = stderr(out);
    assert!(err.contains(NOT_THIS_HOST), "hub must be named: {err}");
    assert!(err.contains("hub"), "{err}");
}

/// Scenario: 非 hub 上 harvest 在任何写入前被拒绝
#[test]
fn evo_harvest_refuses_off_hub_before_any_write() {
    let sb = Sandbox::new();
    let out = sb.run("olp-evo-harvest.sh", false, Some(NOT_THIS_HOST));
    assert_refused(&out);
    assert!(stderr(&out).contains("olp-evo-harvest"));
    sb.assert_untouched();
}

/// Scenario: 非 hub 上 retro 在任何写入前被拒绝
#[test]
fn evo_retro_refuses_off_hub_before_any_write() {
    let sb = Sandbox::new();
    let out = sb.run("olp-evo-retro.sh", false, Some(NOT_THIS_HOST));
    assert_refused(&out);
    assert!(stderr(&out).contains("olp-evo-retro"));
    sb.assert_untouched();
}

/// Scenario: 本机就是 hub 时放行(忽略大小写与域名后缀)
#[test]
fn evo_retro_runs_when_this_host_is_the_hub() {
    let host = this_host();
    let label = host.split('.').next().unwrap();
    for spelling in [
        host.clone(),
        label.to_uppercase(),
        format!("{label}.cluster.example.net"),
    ] {
        let sb = Sandbox::new();
        let out = sb.run("olp-evo-retro.sh", false, Some(&spelling));
        assert!(
            out.status.success(),
            "hub spelled {spelling:?} must match {host:?}: {}",
            stderr(&out)
        );
        assert!(sb.state.exists(), "the retro ran and created its state");
    }
}

/// Scenario: dry-run 在非 hub 上仍可用
#[test]
fn evo_retro_dry_run_is_allowed_off_hub() {
    let sb = Sandbox::new();
    let out = sb.run("olp-evo-retro.sh", true, Some(NOT_THIS_HOST));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    sb.assert_untouched();
}

/// Scenario: hub-host 文件生效,环境变量优先
#[test]
fn evo_hub_host_file_is_honoured_and_env_overrides_it() {
    let sb = Sandbox::new();
    // Comments, blank lines, surrounding whitespace and CRLF are tolerated.
    std::fs::write(
        &sb.hub_file,
        format!("# the cluster hub\r\n\r\n  {NOT_THIS_HOST}  \r\nignored-second-line\n"),
    )
    .unwrap();

    let out = sb.run("olp-evo-retro.sh", false, None);
    assert_refused(&out);
    sb.assert_untouched();

    let out = sb.run("olp-evo-retro.sh", false, Some(&this_host()));
    assert!(
        out.status.success(),
        "OLP_HUB_HOST overrides the file: {}",
        stderr(&out)
    );
}

/// Scenario: 未配置 hub 时行为不变
#[test]
fn evo_scripts_run_unguarded_when_no_hub_is_configured() {
    let sb = Sandbox::new();
    assert!(!Path::new(&sb.hub_file).exists());
    let out = sb.run("olp-evo-retro.sh", false, None);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(sb.state.exists());

    // An empty OLP_HUB_HOST and an empty file both mean "not configured".
    let sb = Sandbox::new();
    std::fs::write(&sb.hub_file, "# nothing configured\n\n").unwrap();
    let out = sb.run("olp-evo-retro.sh", false, Some(""));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

/// Scenario: 单独拷贝的脚本缺守卫库时,未配置放行、已配置拒绝
#[test]
fn evo_lone_script_copy_without_guard_library_fails_closed() {
    let sb = Sandbox::new();
    let lone = sb.repo.join("lone-scripts");
    std::fs::create_dir_all(&lone).unwrap();
    // Everything the retro needs except the guard library itself.
    for name in ["olp-evo-retro.sh", "olp-evo-lib.py", "olp-preflight.sh"] {
        std::fs::copy(script(name), lone.join(name)).unwrap();
    }
    let run = |hub: Option<&str>| {
        let mut cmd = Command::new("bash");
        cmd.arg(lone.join("olp-evo-retro.sh"))
            .arg(&sb.repo)
            .env("OLP_EVO_STATE", &sb.state)
            .env("OLP_HUB_HOST_FILE", &sb.hub_file)
            .env_remove("OLP_HUB_HOST");
        if let Some(hub) = hub {
            cmd.env("OLP_HUB_HOST", hub);
        }
        cmd.output().unwrap()
    };

    // Even naming this very host cannot be verified without the library.
    let out = run(Some(&this_host()));
    assert_eq!(out.status.code(), Some(3), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("olp-hub-guard.sh"),
        "{}",
        stderr(&out)
    );
    sb.assert_untouched();

    let out = run(None);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}
