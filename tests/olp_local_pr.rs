#![cfg(unix)]

//! Contract tests for `scripts/olp-local-pr.py` (SDD spec:
//! `specs/task-olp-local-pr.spec.md`): the state of every task branch in the
//! hub repository, and the gate in front of the push to GitHub.
//!
//! The "GitHub" remote is a second local bare repository.

use std::path::PathBuf;
use std::process::{Command, Output};

fn tool() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-local-pr.py")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Hub {
    dir: tempfile::TempDir,
}

impl Hub {
    /// A hub bare repo with a `github` remote (another bare repo), a work
    /// clone to author commits in, and an empty board.
    fn new() -> Self {
        let h = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for bare in [h.repo(), h.github()] {
            let out = h
                .git_in(None)
                .args(["init", "-q", "--bare", "-b", "main"])
                .arg(&bare)
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", stderr(&out));
        }
        h.hub_git(&["remote", "add", "github", h.github().to_str().unwrap()]);
        std::fs::create_dir_all(h.work()).unwrap();
        h.work_git(&["init", "-q", "-b", "main"]);
        h.work_git(&["remote", "add", "hub", h.repo().to_str().unwrap()]);
        std::fs::write(h.work().join("seed.txt"), "seed").unwrap();
        h.work_git(&["add", "seed.txt"]);
        h.work_git(&["commit", "-q", "-m", "seed"]);
        h.work_git(&["push", "-q", "hub", "main"]);
        std::fs::write(h.board(), "# 板\n\n").unwrap();
        h
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("hub.git")
    }
    fn github(&self) -> PathBuf {
        self.dir.path().join("github.git")
    }
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }
    fn board(&self) -> PathBuf {
        self.dir.path().join("OUTER_LOOP_REVIEW.md")
    }

    fn git_in(&self, cwd: Option<PathBuf>) -> Command {
        let mut cmd = Command::new("git");
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid");
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        cmd
    }

    fn work_git(&self, args: &[&str]) -> String {
        let out = self.git_in(Some(self.work())).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
        stdout(&out).trim().to_string()
    }

    fn hub_git(&self, args: &[&str]) -> String {
        let out = self
            .git_in(None)
            .arg("--git-dir")
            .arg(self.repo())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
        stdout(&out).trim().to_string()
    }

    /// Commit on `task/<n>` (created from main if needed), push it to the
    /// hub, and log the push on the board the way the hub's hook does.
    fn unit_push(&self, n: u32, content: &str) -> String {
        let branch = format!("task/{n}");
        let exists = self
            .git_in(Some(self.work()))
            .args(["rev-parse", "--verify", "--quiet", &branch])
            .output()
            .unwrap()
            .status
            .success();
        if exists {
            self.work_git(&["checkout", "-q", &branch]);
        } else {
            self.work_git(&["checkout", "-q", "-b", &branch, "main"]);
        }
        std::fs::write(self.work().join(format!("task{n}.txt")), content).unwrap();
        self.work_git(&["add", "."]);
        self.work_git(&["commit", "-q", "-m", &format!("task {n}: {content}")]);
        self.work_git(&["push", "-q", "hub", &branch]);
        let sha = self.work_git(&["rev-parse", "HEAD"]);
        self.board_line(&format!(
            "> 单元(m0{n})·本地复验通过(#{n}): task/{n} @ {} 复验 m0{n}-ver",
            &sha[..12]
        ));
        sha
    }

    fn board_line(&self, line: &str) {
        let mut text = std::fs::read_to_string(self.board()).unwrap();
        text.push_str(line);
        text.push('\n');
        std::fs::write(self.board(), text).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        let (cmd, rest) = args.split_first().unwrap();
        Command::new("python3")
            .arg("-B")
            .arg(tool())
            .arg(cmd)
            .args(rest)
            .arg("--repo")
            .arg(self.repo())
            .arg("--board")
            .arg(self.board())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
    }

    fn status(&self) -> serde_json::Value {
        let out = self.run(&["status", "--json"]);
        assert!(out.status.success(), "status: {}", stderr(&out));
        serde_json::from_str(&stdout(&out)).unwrap()
    }

    fn state(&self, n: u64) -> String {
        self.status()["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["number"] == n)
            .unwrap_or_else(|| panic!("task/{n} missing"))["state"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn github_ref(&self, name: &str) -> Option<String> {
        let out = Command::new("git")
            .arg("--git-dir")
            .arg(self.github())
            .args(["rev-parse", "--verify", "--quiet", name])
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| stdout(&out).trim().to_string())
    }
}

/// Scenario: 每个任务分支的状态由黑板上最后一行决定
#[test]
fn local_pr_status_follows_the_latest_board_line() {
    let h = Hub::new();
    h.unit_push(1, "v1");
    h.unit_push(2, "v1");
    h.unit_push(3, "v1");
    let sha4 = h.unit_push(4, "v1");
    h.board_line("> 外环(hub)·打回(#2): 缺测试");
    h.board_line("> 外环(hub)·采认(#3): 复验通过");
    h.board_line("> 外环(hub)·采认(#4): 复验通过");
    // #4 was already promoted: the tracking ref equals the head.
    h.hub_git(&["update-ref", "refs/remotes/github/task/4", &sha4]);

    let report = h.status();
    let rows: Vec<(u64, String, String, String)> = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["number"].as_u64().unwrap(),
                i["state"].as_str().unwrap().to_string(),
                i["waiting_on"].as_str().unwrap().to_string(),
                i["partition"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let s = |a: &str| a.to_string();
    assert_eq!(
        rows,
        vec![
            (1, s("pending"), s("outer"), s("inbox")),
            (2, s("returned"), s("unit"), s("working")),
            (3, s("accepted"), s("outer"), s("inbox")),
            (4, s("submitted"), s("human"), s("inbox")),
        ]
    );
    assert_eq!(report["counts"]["pending"], 1);
    assert_eq!(report["items"][0]["unit"], "m01");
    assert_eq!(report["warnings"].as_array().unwrap().len(), 0, "{report}");
    // `main` is not a local PR.
    assert_eq!(report["items"].as_array().unwrap().len(), 4);

    // Rework: a new push after the return brings #2 back for review, and a
    // push after an acceptance means the accepted commit is no longer the head.
    h.unit_push(2, "v2");
    h.unit_push(3, "v2");
    assert_eq!(h.state(2), "pending");
    assert_eq!(h.state(3), "pending");
    let again = h.status();
    assert_eq!(again["items"][1]["pushes"], 2);

    // The text view groups by state and says who it waits on.
    let out = h.run(&["status"]);
    let text = stdout(&out);
    assert!(text.contains("== 待审核(3)· 等大外环"), "{text}");
    assert!(
        text.contains("== 已提交 GitHub,待人合并(1)· 等人"),
        "{text}"
    );
}

/// Scenario: 推 GitHub 的闸门只放行已采认的分支头
#[test]
fn local_pr_promote_requires_acceptance_of_the_current_head() {
    let h = Hub::new();
    let sha1 = h.unit_push(1, "v1");
    h.unit_push(2, "v1");
    h.board_line("> 外环(hub)·打回(#2): 缺测试");

    // Not reviewed yet.
    let out = h.run(&["promote", "1"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("还没有采认"), "{}", stderr(&out));
    // Returned.
    let out = h.run(&["promote", "2"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("打回"), "{}", stderr(&out));
    // No such branch.
    assert_eq!(h.run(&["promote", "9"]).status.code(), Some(1));
    assert_eq!(h.github_ref("refs/heads/task/1"), None);
    assert_eq!(h.github_ref("refs/heads/task/2"), None);

    // Accepted: a dry run pushes nothing, the real run pushes the head.
    h.board_line("> 外环(hub)·采认(#1): 复验通过");
    let out = h.run(&["promote", "1", "--dry-run"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(h.github_ref("refs/heads/task/1"), None);
    assert_eq!(h.state(1), "accepted");

    let out = h.run(&["promote", "1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(h.github_ref("refs/heads/task/1"), Some(sha1));
    assert_eq!(h.state(1), "submitted");
    // Promoting again is a no-op.
    let out = h.run(&["promote", "1"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("无需再推"), "{}", stdout(&out));

    // The unit pushes again after the acceptance: the gate closes until the
    // new head is accepted, and GitHub keeps the commit that was accepted.
    let sha1b = h.unit_push(1, "v2");
    let out = h.run(&["promote", "1"]);
    assert_eq!(out.status.code(), Some(1));
    assert_ne!(h.github_ref("refs/heads/task/1"), Some(sha1b.clone()));
    h.board_line("> 外环(hub)·采认(#1): 第二版复验通过");
    let report = h.status();
    assert_eq!(report["items"][0]["state"], "accepted");
    assert!(
        report["warnings"].to_string().contains("GitHub 上是旧提交"),
        "{report}"
    );
    assert!(h.run(&["promote", "1"]).status.success());
    assert_eq!(h.github_ref("refs/heads/task/1"), Some(sha1b));
}

/// Scenario: 对不上的地方作为警告报告
#[test]
fn local_pr_reports_mismatches_as_warnings() {
    let h = Hub::new();
    h.unit_push(1, "v1");
    // Someone moves the branch on the hub without a logged push.
    let main = h.hub_git(&["rev-parse", "refs/heads/main"]);
    h.hub_git(&["update-ref", "refs/heads/task/1", &main]);
    // A branch with no record on the board at all.
    h.hub_git(&["update-ref", "refs/heads/task/7", &main]);
    // Forms inside a code fence or in running text do not count.
    h.board_line("```\n> 外环(hub)·采认(#1): 示例\n```");
    h.board_line("正文里提到 外环(hub)·采认(#7) 不算");

    let report = h.status();
    let warnings = report["warnings"].to_string();
    assert!(
        warnings.contains("#1") && warnings.contains("不符"),
        "{warnings}"
    );
    assert!(
        warnings.contains("#7") && warnings.contains("没有"),
        "{warnings}"
    );
    assert_eq!(h.state(1), "pending");
    assert_eq!(h.state(7), "pending");
}

/// Scenario: 用法错误
#[test]
fn local_pr_rejects_bad_usage() {
    let h = Hub::new();
    let run = |args: &[&str]| {
        Command::new("python3")
            .arg("-B")
            .arg(tool())
            .args(args)
            .output()
            .unwrap()
    };
    let repo = h.repo();
    let repo = repo.to_str().unwrap();
    let board = h.board();
    let board = board.to_str().unwrap();
    for args in [
        vec!["status", "--repo", "/nonexistent/hub.git", "--board", board],
        vec!["status", "--repo", repo, "--board", "/nonexistent/board.md"],
        vec![
            "status",
            "--repo",
            repo,
            "--board",
            board,
            "--remote",
            "bad remote",
        ],
        vec![
            "promote", "1", "--repo", repo, "--board", board, "--remote", "nowhere",
        ],
        vec!["promote", "x", "--repo", repo, "--board", board],
        vec!["status"],
        vec![],
    ] {
        let out = run(&args);
        let code = out.status.code();
        // `promote 1` with an unknown remote fails on the missing branch first.
        assert!(
            code == Some(2) || (args.first() == Some(&"promote") && code == Some(1)),
            "{args:?}: {code:?} {}",
            stderr(&out)
        );
    }
}
