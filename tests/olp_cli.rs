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
            // Whoever runs `octoscode olp` has octoscode on PATH; olp init's
            // dependency check looks for it there.
            .env("PATH", {
                let bin = Path::new(env!("CARGO_BIN_EXE_octoscode"))
                    .parent()
                    .unwrap()
                    .to_path_buf();
                let mut paths = vec![bin];
                paths.extend(std::env::split_paths(
                    &std::env::var_os("PATH").unwrap_or_default(),
                ));
                std::env::join_paths(paths).unwrap()
            })
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
            .filter(|p| {
                let name = p.file_name().unwrap().to_string_lossy();
                p.is_dir() && !name.starts_with('.') && name != "current"
            })
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

fn git_in(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
}

/// What the built binary writes as the command to call later.
fn bin_olp() -> String {
    format!("{} olp", env!("CARGO_BIN_EXE_octoscode"))
}

/// Scenario: 经 octoscode olp 写下的钩子、本机推送设置、心跳任务调用 octoscode olp,推送照常经钩子
#[test]
fn olp_wiring_calls_octoscode_not_the_bundle() {
    let sb = Sb::new();
    std::fs::write(
        sb.p("board.md"),
        "# board\n\n## Active\n\n### 1 task one\nunit: m01\n",
    )
    .unwrap();
    let hub = sb.p("hub.git");
    let out = sb.olp(
        &[
            "repo",
            "init",
            hub.to_str().unwrap(),
            "--board",
            sb.p("board.md").to_str().unwrap(),
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    for name in ["pre-receive", "post-receive"] {
        let hook = std::fs::read_to_string(hub.join("hooks").join(name)).unwrap();
        assert!(
            hook.contains(&format!("exec {} repo hook {name}", bin_olp())),
            "{hook}"
        );
        assert!(!hook.contains("olp-bundle"), "{hook}");
    }

    // Single machine: the unit's work repository pushes through local-pack.
    let work = sb.p("cwd");
    assert!(git_in(&work, &["init", "-q"]).status.success());
    assert!(
        git_in(&work, &["commit", "-q", "--allow-empty", "-m", "seed"])
            .status
            .success()
    );
    let out = sb.olp(
        &[
            "repo",
            "connect-local",
            "--unit",
            "m01",
            "--repo",
            hub.to_str().unwrap(),
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let pack = stdout(&git_in(&work, &["config", "remote.hub.receivepack"]));
    assert!(
        pack.starts_with(&format!("{} repo local-pack --unit m01", bin_olp())),
        "{pack}"
    );
    assert!(
        git_in(&work, &["checkout", "-q", "-b", "task/1"])
            .status
            .success()
    );
    assert!(
        git_in(&work, &["commit", "-q", "--allow-empty", "-m", "t1"])
            .status
            .success()
    );
    let sha = stdout(&git_in(&work, &["rev-parse", "HEAD"]));
    let push = git_in(
        &work,
        &[
            "push",
            "-o",
            &format!("olp-verified={}:m01-v", sha.trim()),
            "hub",
            "task/1",
        ],
    );
    assert!(push.status.success(), "{}", stderr(&push));
    if has_flock() {
        let board = std::fs::read_to_string(sb.p("board.md")).unwrap();
        assert!(board.contains("本地复验通过(#1)"), "{board}");
    }

    let jobs = sb.p("jobs");
    let out = sb.olp(
        &[
            "keepalive",
            "heartbeat",
            "--unit",
            "m01",
            "--hub",
            "local",
            "--no-load",
            "--dir",
            jobs.to_str().unwrap(),
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let job: String = std::fs::read_dir(&jobs)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert!(
        job.contains(&format!("{} unit heartbeat --unit m01", bin_olp()))
            && job.contains(&format!("{} unit receive --unit m01", bin_olp())),
        "{job}"
    );
    assert!(!job.contains("olp-unit.py"), "{job}");
}

/// Scenario: skill install 经 octoscode olp 运行时记录 current 链接,它指向本版本的工具目录
#[test]
fn olp_skill_install_records_the_current_link() {
    let sb = Sb::new();
    let out = sb.olp(&["skill", "install", "--home-only"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    let recorded = std::fs::read_to_string(sb.p("home/.octos/outer/octoscode-home")).unwrap();
    let current = sb.p("bundle").join("current");
    assert!(recorded.trim().ends_with("/bundle/current"), "{recorded}");
    assert_eq!(
        std::fs::canonicalize(recorded.trim()).unwrap(),
        std::fs::canonicalize(&current).unwrap()
    );
    assert_eq!(
        std::fs::canonicalize(&current).unwrap(),
        std::fs::canonicalize(sb.bundle()).unwrap()
    );
    assert!(current.join("skills/octoloop/AGENTS.md").is_file());
}

/// Scenario: doctor 把固定在某一版本工具目录的命令报 warn,路径消失后报 fail 并给出修复命令
#[test]
fn olp_doctor_flags_pinned_then_missing_wiring() {
    let sb = Sb::new();
    std::fs::write(sb.p("board.md"), "# board\n\n## Active\n").unwrap();
    // A hub repository whose hooks an older version wrote: they name a
    // script inside that version's tool directory.
    let old = sb.p("bundle/00000000deadbeef/scripts");
    std::fs::create_dir_all(&old).unwrap();
    let script = old.join("olp-git-hub.py");
    std::fs::write(&script, "").unwrap();
    let hub = sb.p("hub.git");
    assert!(
        git_in(
            &sb.p("cwd"),
            &["init", "-q", "--bare", hub.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(
        git_in(
            &hub,
            &["config", "olp.board", sb.p("board.md").to_str().unwrap()]
        )
        .status
        .success()
    );
    for name in ["pre-receive", "post-receive"] {
        let path = hub.join("hooks").join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nexec python3 -B {} hook {name}\n",
                script.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::create_dir_all(sb.p("home/.ssh")).unwrap();
    std::fs::write(
        sb.p("home/.ssh/authorized_keys"),
        format!(
            "command=\"{}/olp-unit.py receive --unit m02\",no-pty ssh-ed25519 AAAA k\n",
            old.display()
        ),
    )
    .unwrap();
    std::fs::write(old.join("olp-unit.py"), "").unwrap();

    let doctor = |sb: &Sb| {
        let out = sb.olp(&["doctor", "--repo", hub.to_str().unwrap(), "--json"], "");
        let report: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}{}", stdout(&out), stderr(&out)));
        (out.status.code(), report)
    };
    let (code, report) = doctor(&sb);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["counts"]["warn"], 3, "{report}");
    assert_eq!(report["counts"]["fail"], 0, "{report}");

    std::fs::remove_dir_all(sb.p("bundle/00000000deadbeef")).unwrap();
    let (code, report) = doctor(&sb);
    assert_eq!(code, Some(1), "{report}");
    assert_eq!(report["counts"]["fail"], 3, "{report}");
    let fixes: Vec<String> = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["fix"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        fixes
            .iter()
            .any(|f| f.starts_with("octoscode olp repo init ")),
        "{fixes:?}"
    );
    assert!(
        fixes.iter().any(|f| f.contains(&format!(
            "command=\"{} unit receive --unit m02\"",
            bin_olp()
        ))),
        "{fixes:?}"
    );

    // Fixed the suggested way: everything is fine again.
    let out = sb.olp(
        &[
            "repo",
            "init",
            hub.to_str().unwrap(),
            "--board",
            sb.p("board.md").to_str().unwrap(),
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    std::fs::write(
        sb.p("home/.ssh/authorized_keys"),
        format!(
            "command=\"{} unit receive --unit m02\",no-pty ssh-ed25519 AAAA k\n",
            bin_olp()
        ),
    )
    .unwrap();
    let (code, report) = doctor(&sb);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["counts"]["ok"], 3, "{report}");
}
