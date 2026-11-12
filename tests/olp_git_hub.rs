#![cfg(unix)]

//! Contract tests for the OLP local git hub (SDD spec:
//! `specs/task-olp-local-git-hub.spec.md`): `scripts/olp-git-hub.py` guards
//! the unit → hub push, `scripts/olp-task-push.sh` is the unit-side push.
//!
//! Pushes go to a local-path remote, so the hub's hooks inherit the pushing
//! process's environment: setting `OLP_UNIT` there stands in for the ssh
//! forced command, which one test also exercises end to end through a fake
//! ssh.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn hub_tool() -> PathBuf {
    repo_root().join("scripts/olp-git-hub.py")
}

fn push_tool() -> PathBuf {
    repo_root().join("scripts/olp-task-push.sh")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn has_flock() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const BOARD: &str = "\
# 外环审查通道

### 1. 第一张卡(2026-11-12,外环(hub))
unit: m07

正文。

### 2. 第二张卡(2026-11-12,外环(hub))
unit: m08

### 3. 没有指定单元的卡

```
unit: m07
```

### 4. 改派过的卡
unit: m07

> 外环(hub)·改派:m07 失联
unit: m08
";

struct Cluster {
    dir: tempfile::TempDir,
}

impl Cluster {
    /// A hub repo with a seeded `main`, a board, and one unit work repo that
    /// has the hub as its `hub` remote.
    fn new() -> Self {
        let c = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(c.board(), BOARD).unwrap();
        let out = Command::new("python3")
            .arg("-B")
            .arg(hub_tool())
            .arg("init")
            .arg(c.hub())
            .arg("--board")
            .arg(c.board())
            .output()
            .unwrap();
        assert!(out.status.success(), "init: {}", stderr(&out));

        let work = c.work();
        std::fs::create_dir_all(&work).unwrap();
        c.git_ok(&["init", "-q", "-b", "main"]);
        c.git_ok(&["remote", "add", "hub", c.hub().to_str().unwrap()]);
        c.commit("seed.txt", "seed", "seed");
        // The hub operator (no unit) seeds the integration branch.
        c.git_ok(&["push", "-q", "hub", "main"]);
        c
    }

    fn hub(&self) -> PathBuf {
        self.dir.path().join("hub.git")
    }

    fn board(&self) -> PathBuf {
        self.dir.path().join("OUTER_LOOP_REVIEW.md")
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn base(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(self.work())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "unit")
            .env("GIT_AUTHOR_EMAIL", "unit@example.invalid")
            .env("GIT_COMMITTER_NAME", "unit")
            .env("GIT_COMMITTER_EMAIL", "unit@example.invalid")
            .env_remove("OLP_UNIT");
        cmd
    }

    fn git(&self, args: &[&str], unit: Option<&str>) -> Output {
        let mut cmd = self.base("git");
        cmd.args(args);
        if let Some(u) = unit {
            cmd.env("OLP_UNIT", u);
        }
        cmd.output().unwrap()
    }

    fn git_ok(&self, args: &[&str]) -> String {
        let out = self.git(args, None);
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
        stdout(&out).trim().to_string()
    }

    fn commit(&self, file: &str, content: &str, message: &str) -> String {
        std::fs::write(self.work().join(file), content).unwrap();
        self.git_ok(&["add", file]);
        self.git_ok(&["commit", "-q", "-m", message]);
        self.git_ok(&["rev-parse", "HEAD"])
    }

    /// Create `task/<n>` from main with one commit; returns its sha.
    fn task_branch(&self, n: u32) -> String {
        self.git_ok(&["checkout", "-q", "-b", &format!("task/{n}"), "main"]);
        self.commit(&format!("task{n}.txt"), "work", &format!("task {n}"))
    }

    /// The unit-side push: `olp-task-push.sh <n> --verified-by <signer>`.
    fn task_push(&self, n: &str, extra: &[&str], unit: Option<&str>) -> Output {
        let mut cmd = self.base("bash");
        cmd.arg(push_tool()).arg(n).args(extra);
        if let Some(u) = unit {
            cmd.env("OLP_UNIT", u);
        }
        cmd.output().unwrap()
    }

    fn hub_ref(&self, name: &str) -> Option<String> {
        let out = Command::new("git")
            .arg("--git-dir")
            .arg(self.hub())
            .args(["rev-parse", "--verify", "--quiet", name])
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| stdout(&out).trim().to_string())
    }

    fn board_text(&self) -> String {
        std::fs::read_to_string(self.board()).unwrap()
    }
}

/// Scenario: 派给本单元且带复验记录的推送被接收并登记到黑板
#[test]
fn hub_accepts_verified_push_from_assigned_unit_and_logs_board_line() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let c = Cluster::new();
    let sha = c.task_branch(1);
    let out = c.task_push("1", &["--verified-by", "m07-ver"], Some("m07"));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        c.hub_ref("refs/heads/task/1").as_deref(),
        Some(sha.as_str())
    );

    let expected = format!(
        "{BOARD}> 单元(m07)·本地复验通过(#1): task/1 @ {} 复验 m07-ver\n",
        &sha[..12]
    );
    assert_eq!(c.board_text(), expected);
    assert!(stderr(&out).contains("已在黑板登记"), "{}", stderr(&out));
}

/// Scenario: 违反推送规则的推送被拒收
#[test]
fn hub_rejects_pushes_that_break_the_rules() {
    let c = Cluster::new();
    let sha1 = c.task_branch(1);
    c.task_branch(2);
    c.task_branch(3);
    c.task_branch(4);
    c.task_branch(9);
    c.git_ok(&["tag", "v-test", "main"]);
    let verified = format!("olp-verified={sha1}:m07-ver");
    let other_sha = c.git_ok(&["rev-parse", "main"]);
    let stale = format!("olp-verified={other_sha}:m07-ver");
    let bad_signer = format!("olp-verified={sha1}:bad signer");

    // (what, git push arguments, expected fragment of the refusal)
    let cases: Vec<(&str, Vec<&str>, &str)> = vec![
        (
            "integration branch",
            vec!["-o", &verified, "hub", "task/1:main"],
            "task/<N>",
        ),
        (
            "other branch",
            vec!["-o", &verified, "hub", "task/1:feature/x"],
            "task/<N>",
        ),
        (
            "non-numeric task",
            vec!["-o", &verified, "hub", "task/1:task/abc"],
            "task/<N>",
        ),
        ("tag", vec!["hub", "v-test"], "task/<N>"),
        ("card not on board", vec!["hub", "task/9"], "没有条目 #9"),
        (
            "card of another unit",
            vec!["hub", "task/2"],
            "派给的是 m08",
        ),
        ("card without a unit", vec!["hub", "task/3"], "未指定单元"),
        ("reassigned card", vec!["hub", "task/4"], "派给的是 m08"),
        (
            "no verification record",
            vec!["hub", "task/1"],
            "缺少本地复验记录",
        ),
        (
            "record for another commit",
            vec!["-o", &stale, "hub", "task/1"],
            "缺少本地复验记录",
        ),
        (
            "malformed signer",
            vec!["-o", &bad_signer, "hub", "task/1"],
            "缺少本地复验记录",
        ),
    ];
    for (what, args, needle) in &cases {
        let mut full = vec!["push"];
        full.extend(args.iter().copied());
        let out = c.git(&full, Some("m07"));
        assert!(!out.status.success(), "{what}: push must fail");
        let err = stderr(&out);
        assert!(
            err.contains("拒收") && err.contains(needle),
            "{what}: {err}"
        );
    }
    assert_eq!(c.hub_ref("refs/heads/task/1"), None);
    assert_eq!(c.hub_ref("refs/heads/feature/x"), None);
    assert_eq!(c.hub_ref("refs/tags/v-test"), None);
    assert_eq!(c.hub_ref("refs/heads/main"), Some(other_sha));
    assert_eq!(c.board_text(), BOARD, "a refused push leaves no board line");

    // One bad ref refuses the whole push, including the good one next to it.
    let out = c.git(
        &["push", "-o", &verified, "hub", "task/1", "task/2"],
        Some("m07"),
    );
    assert!(!out.status.success());
    assert_eq!(c.hub_ref("refs/heads/task/1"), None);

    // A unit cannot delete a branch, not even its own.
    let out = c.task_push("1", &["--verified-by", "m07-ver"], Some("m07"));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let out = c.git(&["push", "hub", ":task/1"], Some("m07"));
    assert!(!out.status.success());
    assert!(stderr(&out).contains("不能删除分支"), "{}", stderr(&out));
    assert_eq!(c.hub_ref("refs/heads/task/1"), Some(sha1));
}

/// Scenario: 返工后同一单元可以强推自己的任务分支
#[test]
fn hub_allows_rework_force_push_by_owner() {
    if !has_flock() {
        eprintln!("SKIP (explicit): host has no flock on PATH; the board append needs it");
        return;
    }
    let c = Cluster::new();
    let first = c.task_branch(1);
    assert!(
        c.task_push("1", &["--verified-by", "m07-ver"], Some("m07"))
            .status
            .success()
    );

    // Rework: the commit is rewritten, so the push is not a fast-forward.
    std::fs::write(c.work().join("task1.txt"), "reworked").unwrap();
    c.git_ok(&["commit", "-q", "-a", "--amend", "-m", "task 1 (reworked)"]);
    let second = c.git_ok(&["rev-parse", "HEAD"]);
    assert_ne!(first, second);

    let out = c.task_push("1", &["--verified-by", "m07-ver"], Some("m07"));
    assert!(
        !out.status.success(),
        "git refuses a non-fast-forward without --force"
    );
    assert_eq!(c.hub_ref("refs/heads/task/1"), Some(first.clone()));

    let out = c.task_push("1", &["--verified-by", "m07-ver2", "--force"], Some("m07"));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(c.hub_ref("refs/heads/task/1"), Some(second.clone()));

    let board = c.board_text();
    assert!(
        board.contains(&format!("task/1 @ {} 复验 m07-ver\n", &first[..12])),
        "{board}"
    );
    assert!(
        board.contains(&format!("task/1 @ {} 复验 m07-ver2\n", &second[..12])),
        "{board}"
    );
}

/// Scenario: hub 本机不带单元号的推送不受限制
#[test]
fn hub_local_pushes_without_unit_are_unrestricted() {
    let c = Cluster::new();
    c.git_ok(&["checkout", "-q", "main"]);
    let head = c.commit("integration.txt", "merged", "integration");
    c.git_ok(&["tag", "seed-1"]);
    c.git_ok(&[
        "push",
        "-q",
        "hub",
        "main",
        "seed-1",
        "main:refs/heads/integration",
    ]);
    assert_eq!(c.hub_ref("refs/heads/main"), Some(head.clone()));
    assert_eq!(c.hub_ref("refs/heads/integration"), Some(head));
    assert!(c.hub_ref("refs/tags/seed-1").is_some());
    assert_eq!(
        c.board_text(),
        BOARD,
        "operator pushes are not logged as local PRs"
    );
}

/// Scenario: ssh 强制命令只放行对 hub 仓库的 git 操作并固定单元号
#[test]
fn hub_serve_pins_the_unit_and_only_allows_git_on_the_hub_repo() {
    let c = Cluster::new();
    let serve = |original: &str| {
        Command::new("python3")
            .arg("-B")
            .arg(hub_tool())
            .args(["serve", "--unit", "m07", "--repo"])
            .arg(c.hub())
            .env("SSH_ORIGINAL_COMMAND", original)
            .output()
            .unwrap()
    };
    let hub = c.hub();
    let hub_str = hub.to_str().unwrap();
    for original in [
        String::new(),
        "ls -la".to_string(),
        "bash".to_string(),
        format!("git-receive-pack '{hub_str}'; rm -rf /"),
        format!("git-receive-pack '{}'", c.work().display()),
        format!("git-upload-archive '{hub_str}'"),
    ] {
        let out = serve(&original);
        assert_eq!(out.status.code(), Some(3), "{original:?}: {}", stderr(&out));
    }

    // End to end through a fake ssh that runs the forced command, the way
    // sshd would. The client claims to be m08; the forced command wins.
    let fake_ssh = c.dir.path().join("fake-ssh");
    std::fs::write(
        &fake_ssh,
        format!(
            "#!/bin/sh\nshift\nSSH_ORIGINAL_COMMAND=\"$*\" exec python3 -B '{}' serve --unit m07 --repo '{}'\n",
            hub_tool().display(),
            hub_str
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake_ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let url = format!("ssh://hub.invalid{hub_str}");
    c.git_ok(&["remote", "add", "hub-ssh", &url]);
    let sha2 = c.task_branch(2);
    let sha1 = c.task_branch(1);

    let push = |n: &str, sha: &str| {
        let mut cmd = c.base("git");
        cmd.args([
            "push",
            "-o",
            &format!("olp-verified={sha}:m07-ver"),
            "hub-ssh",
            &format!("task/{n}"),
        ])
        .env("GIT_SSH", &fake_ssh)
        .env("GIT_SSH_VARIANT", "simple")
        .env("OLP_UNIT", "m08");
        cmd.output().unwrap()
    };
    // Card 2 belongs to m08, but this key is m07's.
    let out = push("2", &sha2);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("不是 m07"), "{}", stderr(&out));
    assert_eq!(c.hub_ref("refs/heads/task/2"), None);

    let out = push("1", &sha1);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(c.hub_ref("refs/heads/task/1"), Some(sha1));

    // Fetching through the forced command works too.
    let mut fetch = c.base("git");
    fetch
        .args(["fetch", "-q", "hub-ssh", "main"])
        .env("GIT_SSH", &fake_ssh)
        .env("GIT_SSH_VARIANT", "simple");
    let out = fetch.output().unwrap();
    assert!(out.status.success(), "fetch: {}", stderr(&out));
}

/// Scenario: 单元侧推送命令先做本地检查
#[test]
fn task_push_checks_inputs_before_pushing() {
    let c = Cluster::new();
    c.task_branch(1);
    let cases: [(&str, &[&str]); 6] = [
        ("1", &[]),
        ("abc", &["--verified-by", "m07-ver"]),
        ("7", &["--verified-by", "m07-ver"]),
        ("1", &["--verified-by", "bad signer"]),
        ("1", &["--verified-by", "m07-ver", "--remote", "nowhere"]),
        ("1", &["--verified-by", "m07-ver", "--bogus"]),
    ];
    for (n, extra) in cases {
        let out = c.task_push(n, extra, Some("m07"));
        assert_eq!(
            out.status.code(),
            Some(2),
            "{n} {extra:?}: {}",
            stderr(&out)
        );
    }
    assert_eq!(c.hub_ref("refs/heads/task/1"), None);
}

/// Scenario: init 可重跑且拒绝非裸仓库
#[test]
fn hub_init_is_rerunnable_and_refuses_a_non_bare_repo() {
    let c = Cluster::new();
    let init = |repo: &Path, board: &Path| {
        Command::new("python3")
            .arg("-B")
            .arg(hub_tool())
            .arg("init")
            .arg(repo)
            .arg("--board")
            .arg(board)
            .output()
            .unwrap()
    };
    let before = c.hub_ref("refs/heads/main");
    let out = init(&c.hub(), &c.board());
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        c.hub_ref("refs/heads/main"),
        before,
        "re-init keeps the refs"
    );
    for hook in ["pre-receive", "post-receive"] {
        let mode = std::fs::metadata(c.hub().join("hooks").join(hook))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "{hook} must be executable");
    }

    assert_eq!(init(&c.work(), &c.board()).status.code(), Some(2));
    let missing_board = c.dir.path().join("absent.md");
    assert_eq!(
        init(&c.dir.path().join("new.git"), &missing_board)
            .status
            .code(),
        Some(2)
    );
    assert!(!c.dir.path().join("new.git").exists());
}
