#![cfg(unix)]

//! Contract tests for `octoscode olp` (SDD spec: `specs/task-olp-cli.spec.md`,
//! command surface: `docs/OLP_CLI.md`). Phase one runs the bundled scripts:
//! the same arguments, output and exit codes, from any directory, with the
//! scripts' siblings (helpers, hub guard, onboarding cards) in place.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn has_flock() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A sandbox: HOME, the bundle root and the unit registry all in a temp dir,
/// and the working directory somewhere that is not the repository.
struct Sb {
    dir: tempfile::TempDir,
}

impl Sb {
    fn new() -> Self {
        let sb = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(sb.p("home")).unwrap();
        std::fs::create_dir_all(sb.p("cwd")).unwrap();
        sb
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn olp(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_octoscode"))
            .arg("olp")
            .args(args)
            .current_dir(self.p("cwd"))
            .env("HOME", self.p("home"))
            .env("OCTOSCODE_OLP_BUNDLE", self.p("bundle"))
            .env("OLP_UNITS_DIR", self.p("units"))
            .env_remove("OLP_HUB")
            .env_remove("OLP_UNIT_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }

    /// The one bundle directory this binary wrote.
    fn bundle(&self) -> PathBuf {
        let dirs: Vec<PathBuf> = std::fs::read_dir(self.p("bundle"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir() && !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        assert_eq!(dirs.len(), 1, "{dirs:?}");
        dirs[0].clone()
    }
}

/// Scenario: 列出命令面;未知的组与动作以退出码 2 拒绝
#[test]
fn olp_lists_the_command_surface_and_rejects_unknown_words() {
    let sb = Sb::new();
    let out = sb.olp(&[], "");
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    for line in [
        "card dispatch",
        "board inbox",
        "board summary",
        "unit …",
        "remote …",
        "repo promote",
        "ask mcp",
        "guard watch",
        "keepalive …",
        "duty …",
        "evo harvest",
        "review …",
        "dispatch=card dispatch",
    ] {
        assert!(text.contains(line), "listing lacks {line:?}:\n{text}");
    }
    assert!(
        !text.contains("local-pack"),
        "git-only commands are not listed"
    );
    assert_eq!(sb.olp(&["--help"], "").status.code(), Some(0));

    let out = sb.olp(&["nope"], "");
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("没有这个组"), "{}", stderr(&out));
    let out = sb.olp(&["repo", "nope"], "");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("可选:init / serve"),
        "{}",
        stderr(&out)
    );
    let out = sb.olp(&["evo"], "");
    assert_eq!(out.status.code(), Some(2));
    assert!(stdout(&out).contains("harvest / retro"), "{}", stdout(&out));
}

/// Scenario: 打包的工具与仓库里的逐字节相同,工具目录按内容复用
#[test]
fn olp_bundle_matches_the_repository_and_is_reused() {
    let sb = Sb::new();
    let out = sb.olp(&["unit", "status"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "没有登记任何单元");
    let bundle = sb.bundle();
    for rel in [
        "scripts/olp-unit.py",
        "scripts/olp-board-append.sh",
        "scripts/olp-hub-guard.sh",
        "scripts/olp-evo-lib.py",
        "docs/OCTOLOOP_AGENTS.zh-CN.md",
        "skills/octoloop/AGENTS.md",
    ] {
        assert_eq!(
            std::fs::read(bundle.join(rel)).unwrap(),
            std::fs::read(repo_root().join(rel)).unwrap(),
            "{rel} differs from the repository"
        );
    }
    // A second run reuses the same directory.
    let before = std::fs::metadata(bundle.join("scripts/olp-unit.py"))
        .unwrap()
        .modified()
        .unwrap();
    assert!(sb.olp(&["unit", "status"], "").status.success());
    assert_eq!(sb.bundle(), bundle);
    assert_eq!(
        std::fs::metadata(bundle.join("scripts/olp-unit.py"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
}

/// Scenario: 参数、输出、退出码原样透传,别名与完整写法等价
#[test]
fn olp_passes_arguments_output_and_exit_codes_through() {
    let sb = Sb::new();
    // A usage error in the tool is the tool's own exit code and message.
    for args in [&["card", "dispatch"][..], &["dispatch"][..]] {
        let out = sb.olp(args, "");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(stderr(&out).contains("--board"), "{}", stderr(&out));
    }
    let out = sb.olp(&["repo", "push"], "");
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("olp-task-push"), "{}", stderr(&out));

    if !has_flock() {
        eprintln!("SKIP (explicit): flock not installed; stopping before board writes");
        return;
    }
    // board append allocates numbers exactly as the script does; the board
    // path is relative to the caller's working directory.
    let out = sb.olp(&["board", "append", "board.md"], "### {N}. 第一张\n");
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "1");
    let out = sb.olp(
        &["board", "append", "board.md"],
        "### {N}. 第二张\nunit: m01\n",
    );
    assert_eq!(stdout(&out).trim(), "2");
    let board = sb.p("cwd/board.md");
    let full = sb.olp(&["board", "inbox", board.to_str().unwrap(), "--json"], "");
    let alias = sb.olp(&["inbox", board.to_str().unwrap(), "--json"], "");
    assert!(full.status.success(), "{}", stderr(&full));
    assert_eq!(stdout(&full), stdout(&alias));
    let v: serde_json::Value = serde_json::from_str(&stdout(&full)).unwrap();
    assert_eq!(v["items"][1]["unit"], "m01");
}

/// Scenario: 工具依赖的同目录文件都在:上岗卡、写板助手、hub 守卫、本机 hub 的写板
#[test]
fn olp_tools_find_their_siblings_in_the_bundle() {
    let sb = Sb::new();
    // init lays the onboarding card from ../docs and installs the helpers.
    let project = sb.p("cwd/proj");
    std::fs::create_dir_all(&project).unwrap();
    assert!(
        Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(&project)
            .status()
            .unwrap()
            .success()
    );
    let out = sb.olp(&["init", "proj"], "");
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    let card = std::fs::read_to_string(project.join("AGENTS.md")).unwrap();
    assert_eq!(
        card,
        std::fs::read_to_string(repo_root().join("docs/OCTOLOOP_AGENTS.md")).unwrap()
    );
    for helper in ["board-append.sh", "watch-board.sh"] {
        assert!(
            sb.p("home/.octos/outer").join(helper).is_file(),
            "{helper} installed"
        );
    }

    // The evolution harvest sources the hub guard from its own directory: on
    // a machine that is not the configured hub it refuses (exit 3), instead
    // of failing to find the guard.
    std::fs::create_dir_all(sb.p("home/.octos/outer")).unwrap();
    std::fs::write(sb.p("home/.octos/outer/hub-host"), "some-other-host\n").unwrap();
    let out = sb.olp(&["evo", "harvest", project.to_str().unwrap()], "");
    let err = stderr(&out);
    if err.contains("缺") && out.status.code() == Some(2) {
        eprintln!("SKIP (explicit): harvest preflight needs GNU tools here: {err}");
    } else {
        assert_eq!(out.status.code(), Some(3), "{err}");
        assert!(err.contains("不是 hub"), "{err}");
        assert!(!err.contains("找不到 olp-hub-guard.sh"), "{err}");
    }

    // skill install records the bundle as the octoscode home: the body the
    // entry points at exists there.
    let out = sb.olp(&["skill", "install", "--home-only"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    let home = std::fs::read_to_string(sb.p("home/.octos/outer/octoscode-home")).unwrap();
    assert!(
        Path::new(home.trim())
            .join("skills/octoloop/AGENTS.md")
            .is_file(),
        "{home}"
    );

    if !has_flock() {
        eprintln!("SKIP (explicit): flock not installed; stopping before remote writes");
        return;
    }
    // remote with `hub = local` runs the hub side (serve), which calls the
    // board-append helper next to it.
    let board = project.join(".octos/OUTER_LOOP_REVIEW.md");
    std::fs::write(
        sb.p("home/.octos/outer/unit.conf"),
        format!("hub = local\nunit = m01\nboard = {}\n", board.display()),
    )
    .unwrap();
    let out = sb.olp(&["remote", "ack", "1", "done", "commit abc"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        std::fs::read_to_string(&board)
            .unwrap()
            .contains("> 内环(m01)·#1 ACK(done): commit abc"),
        "{}",
        std::fs::read_to_string(&board).unwrap()
    );
}

/// Scenario: Rust 实现的命令直接调用;保活按平台选工具
#[test]
fn olp_native_commands_and_keepalive_pick_the_right_implementation() {
    let sb = Sb::new();
    let out = sb.olp(&["guard", "watch", "--help"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("never accepts tasks"),
        "{}",
        stdout(&out)
    );

    let project = sb.p("cwd");
    let out = sb.olp(
        &["duty", "check", "--project", project.to_str().unwrap()],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "VACANT");

    // ask mcp answers an MCP initialize on stdio.
    let out = sb.olp(
        &["ask", "mcp"],
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"t\",\"version\":\"0\"}}}\n",
    );
    assert!(
        stdout(&out).contains("olp-mcp"),
        "{}{}",
        stdout(&out),
        stderr(&out)
    );

    let out = sb.olp(
        &[
            "keepalive",
            "render",
            "demo",
            "--interval",
            "60",
            "--",
            "/bin/true",
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    if cfg!(target_os = "macos") {
        assert!(
            stdout(&out).contains("<key>StartInterval</key>"),
            "{}",
            stdout(&out)
        );
    } else {
        assert!(
            stdout(&out).contains("OnUnitActiveSec=60"),
            "{}",
            stdout(&out)
        );
    }
}
