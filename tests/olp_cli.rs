#![cfg(unix)]

//! Contract tests for `octoscode olp` (SDD spec: `specs/task-olp-cli.spec.md`,
//! command surface: `docs/OLP_CLI.md`). Phase one runs the bundled scripts:
//! the same arguments, output and exit codes, from any directory, with the
//! scripts' siblings (helpers, hub guard, onboarding cards) in place.

#[path = "support/olp_env.rs"]
mod olp_env;

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
        self.olp_env(args, stdin, &[])
    }

    fn olp_env(&self, args: &[&str], stdin: &str, envs: &[(String, String)]) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_octoscode"))
            .arg("olp")
            .args(args)
            .envs(envs.iter().map(|(k, v)| (k, v)))
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

/// Scenario: 命令目录给程序读;查帮助不执行命令
#[test]
fn olp_commands_json_is_the_catalog_and_help_json_does_not_run() {
    let sb = Sb::new();
    let out = sb.olp(&["commands", "--json"], "");
    assert!(out.status.success(), "{}", stderr(&out));
    let catalog: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(catalog["version"], env!("CARGO_PKG_VERSION"));
    let commands = catalog["commands"].as_array().unwrap();
    let find = |route: &str| {
        commands
            .iter()
            .find(|c| c["route"] == route)
            .unwrap_or_else(|| panic!("no {route} in the catalog"))
    };
    // Every line of the human listing is in the catalog, and so are the
    // commands the listing hides.
    let listing = stdout(&sb.olp(&[], ""));
    for line in listing.lines().filter(|l| l.starts_with("  ")) {
        let words: Vec<&str> = line.split_whitespace().take(2).collect();
        let route = match words[1] {
            "…" => format!("octoscode olp {}", words[0]),
            action => format!("octoscode olp {} {action}", words[0]),
        };
        find(&route);
    }
    let dispatch = find("octoscode olp card dispatch");
    assert_eq!(dispatch["runs_on"], "hub");
    assert_eq!(dispatch["writes"], true);
    assert_eq!(dispatch["needs_lease"], true);
    assert_eq!(dispatch["implementation"], "rust");
    assert_eq!(dispatch["help"], "octoscode olp card dispatch --help");
    let inbox = find("octoscode olp board inbox");
    assert_eq!(inbox["writes"], false);
    assert_eq!(find("octoscode olp guard watch")["implementation"], "rust");
    assert_eq!(find("octoscode olp repo hook")["runs_on"], "internal");
    // A group whose next word is the tool's own subcommand says so, and
    // does not claim to know what each subcommand does.
    let remote = find("octoscode olp remote");
    assert_eq!(remote["subcommands"], true);
    assert!(remote["writes"].is_null());
    assert_eq!(
        catalog["aliases"][0],
        serde_json::json!({"alias": "octoscode olp dispatch", "route": "octoscode olp card dispatch"})
    );

    // --help --json describes one command and runs nothing: the board
    // does not exist, and nothing complains about it.
    for args in [
        ["card", "dispatch", "--help", "--json"],
        ["card", "dispatch", "--json", "--help"],
    ] {
        let out = sb.olp(&args, "");
        assert!(out.status.success(), "{}", stderr(&out));
        let one: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        assert_eq!(&one, dispatch);
    }
    let out = sb.olp(&["dispatch", "-h", "--json"], "");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout(&out)).unwrap()["route"],
        "octoscode olp card dispatch"
    );
    // --json on its own still belongs to the command.
    let out = sb.olp(
        &[
            "inbox",
            "--json",
            "--board",
            sb.p("nope.md").to_str().unwrap(),
        ],
        "",
    );
    assert_ne!(out.status.code(), Some(0), "the command itself ran");

    assert_eq!(sb.olp(&["commands"], "").status.code(), Some(0));
    let out = sb.olp(&["commands", "--yaml"], "");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("commands [--json]"),
        "{}",
        stderr(&out)
    );
}

/// Scenario: 打包的工具与仓库里的逐字节相同,工具目录按内容复用
#[test]
fn olp_bundle_matches_the_repository_and_is_reused() {
    let sb = Sb::new();
    // `unit` is Rust now; the legacy switch still runs its script, and with
    // it unpacks the bundle.
    let legacy = [("OCTOSCODE_OLP_LEGACY".to_string(), "1".to_string())];
    let out = sb.olp_env(&["unit", "status"], "", &legacy);
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
    assert!(
        sb.olp_env(&["unit", "status"], "", &legacy)
            .status
            .success()
    );
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
    // Retrospective #05: every doc the skill body names is there too.
    let body = std::fs::read_to_string(current.join("skills/octoloop/AGENTS.md")).unwrap();
    let mut named: Vec<&str> = body
        .match_indices("docs/")
        .map(|(at, _)| {
            let tail = &body[at..];
            let end = tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || "/_.-".contains(c)))
                .unwrap_or(tail.len());
            tail[..end].trim_end_matches('.')
        })
        .filter(|name| name.ends_with(".md"))
        .collect();
    named.sort();
    named.dedup();
    assert!(named.len() >= 10, "the skill names its docs: {named:?}");
    let absent: Vec<&&str> = named
        .iter()
        .filter(|d| !current.join(d).is_file())
        .collect();
    assert!(
        absent.is_empty(),
        "the installed skill cannot read {absent:?}"
    );
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
        let mut report: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}{}", stdout(&out), stderr(&out)));
        // Running processes depend on the machine the test runs on.
        let kept: Vec<serde_json::Value> = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] != "运行中的旧版本")
            .cloned()
            .collect();
        let count = |s: &str| kept.iter().filter(|c| c["status"] == s).count();
        report["counts"] =
            serde_json::json!({"ok": count("ok"), "warn": count("warn"), "fail": count("fail")});
        report["checks"] = serde_json::Value::Array(kept);
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

/// Scenario: 外环会话结束后,带着它租约号的写板被拒(退出码 6);别的窗格、没有租约号的写入照常
#[test]
fn olp_board_append_refuses_a_dead_outer_lease() {
    if !has_flock() {
        eprintln!("SKIP (explicit): board append needs flock here");
        return;
    }
    let sb = Sb::new();
    let board = sb.p("board.md");
    std::fs::write(&board, "# board\n\n## Active\n").unwrap();
    std::fs::create_dir_all(sb.p("proj")).unwrap();
    let envfile = sb.p("agent.env");
    let sentinel = sb.p("run");
    std::fs::write(&sentinel, "").unwrap();
    // The outer: records the environment hold gave it, then waits.
    let agent = format!(
        "env | grep '^OLP_DUTY_' > {e}.tmp && mv {e}.tmp {e}; while [ -e {s} ]; do sleep 0.05; done",
        e = envfile.display(),
        s = sentinel.display()
    );
    let mut hold = Command::new(env!("CARGO_BIN_EXE_octoscode"))
        .args(["olp", "duty", "hold", "--project"])
        .arg(sb.p("proj"))
        .args([
            "--signature",
            "s",
            "--duties",
            "d",
            "--",
            "/bin/sh",
            "-c",
            &agent,
        ])
        .env("HOME", sb.p("home"))
        .env_remove("HERDR_PANE_ID")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut lease_env = Vec::new();
    for _ in 0..400 {
        if let Ok(text) = std::fs::read_to_string(&envfile) {
            lease_env = text
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let keys: Vec<&str> = lease_env.iter().map(|(k, _)| k.as_str()).collect();
    for key in [
        "OLP_DUTY_LEASE",
        "OLP_DUTY_PROJECT",
        "OLP_DUTY_BIN",
        "OLP_DUTY_PANE",
    ] {
        assert!(
            keys.contains(&key),
            "hold must hand {key} to the agent: {keys:?}"
        );
    }
    // `pane` is the writer's HERDR_PANE_ID ("" = the outer's own, outside herdr).
    let with = |pane: &str, line: &str| {
        let mut envs = lease_env.clone();
        envs.push(("HERDR_PANE_ID".into(), pane.to_string()));
        let out = sb.olp_env(&["board", "append", board.to_str().unwrap()], line, &envs);
        (out.status.code(), std::fs::read_to_string(&board).unwrap())
    };

    let (code, text) = with("", "while held\n");
    assert_eq!(code, Some(0));
    assert!(text.contains("while held"), "{text}");

    std::fs::remove_file(&sentinel).unwrap();
    assert!(hold.wait().unwrap().success());
    let (code, text) = with("", "after the outer is gone\n");
    assert_eq!(code, Some(6), "a dead outer's lease is refused");
    assert!(!text.contains("after the outer is gone"), "{text}");
    // The Python write tools refuse the same way (archive stands in for them).
    let mut envs = lease_env.clone();
    envs.push(("HERDR_PANE_ID".into(), String::new()));
    let out = sb.olp_env(&["board", "archive", board.to_str().unwrap()], "", &envs);
    assert_eq!(out.status.code(), Some(6), "{}", stderr(&out));
    assert!(stderr(&out).contains("主审权租约无效"), "{}", stderr(&out));

    // Another herdr pane: not the outer, only an inherited environment.
    let (code, text) = with("w9:p9", "from an inner pane\n");
    assert_eq!(code, Some(0));
    assert!(text.contains("from an inner pane"), "{text}");

    // No lease at all: an inner loop, a unit or a person.
    let out = sb.olp(&["board", "append", board.to_str().unwrap()], "no lease\n");
    assert!(out.status.success(), "{}", stderr(&out));
}

fn doctor_json(sb: &Sb, args: &[&str]) -> serde_json::Value {
    let mut full = vec!["doctor", "--json"];
    full.extend_from_slice(args);
    let out = sb.olp(&full, "");
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}{}", stdout(&out), stderr(&out)))
}

fn items_of<'a>(report: &'a serde_json::Value, kind: &str) -> Vec<&'a serde_json::Value> {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == kind)
        .collect()
}

/// Scenario: doctor 发现单元的 hub 远程照抄了另一台机器的本地路径(复盘第 13 条)
#[test]
fn olp_doctor_flags_a_hub_remote_that_is_not_here() {
    let sb = Sb::new();
    let work = sb.p("work");
    std::fs::create_dir_all(&work).unwrap();
    assert!(git_in(&work, &["init", "-q"]).status.success());
    let w = work.to_str().unwrap();
    git_in(
        &work,
        &[
            "remote",
            "add",
            "hub",
            "/Users/team/team_workspace/git-hub/x.git",
        ],
    );
    let report = doctor_json(&sb, &["--work", w]);
    let hub = items_of(&report, "hub 远程");
    assert_eq!(hub.len(), 1, "{report}");
    assert_eq!(hub[0]["status"], "fail", "{report}");
    assert!(
        hub[0]["fix"]
            .as_str()
            .unwrap()
            .contains("git remote set-url"),
        "{report}"
    );

    let bare = sb.p("hub.git");
    assert!(
        git_in(
            &sb.p("cwd"),
            &["init", "-q", "--bare", bare.to_str().unwrap()]
        )
        .status
        .success()
    );
    git_in(&work, &["remote", "set-url", "hub", bare.to_str().unwrap()]);
    let report = doctor_json(&sb, &["--work", w]);
    assert_eq!(items_of(&report, "hub 远程")[0]["status"], "ok", "{report}");

    // A network URL is not checked offline.
    git_in(
        &work,
        &["remote", "set-url", "hub", "olp-hub-git:/srv/olp/hub.git"],
    );
    let report = doctor_json(&sb, &["--work", w]);
    assert!(items_of(&report, "hub 远程").is_empty(), "{report}");
}

/// Scenario: doctor 报告还在跑已被替换的二进制的进程(复盘第 06 条)
#[test]
fn olp_doctor_flags_a_process_running_a_replaced_binary() {
    let sb = Sb::new();
    let bin = sb.p("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // A long-running process named `octos`: a copy of this build in MCP
    // serve mode, which waits on its stdin (system binaries such as
    // /bin/sleep refuse to run from another path on macOS).
    let octos = bin.join("octos");
    let me = env!("CARGO_BIN_EXE_octoscode");
    std::fs::copy(me, &octos).unwrap();
    let mut child = Command::new(&octos)
        .arg("olp-mcp-serve")
        .env("HOME", sb.p("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // An upgrade replaces the file (new inode, newer mtime) under the
    // running process.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let staged = bin.join("octos.new");
    std::fs::copy(me, &staged).unwrap();
    // A copy may keep the source's old mtime; an upgrade writes a new file.
    std::fs::File::options()
        .write(true)
        .open(&staged)
        .unwrap()
        .set_modified(std::time::SystemTime::now())
        .unwrap();
    std::fs::rename(&staged, &octos).unwrap();

    let report = doctor_json(&sb, &[]);
    let pid = child.id();
    let _ = child.kill();
    let _ = child.wait();
    let canonical = std::fs::canonicalize(&octos).unwrap();
    let stale: Vec<_> = items_of(&report, "运行中的旧版本")
        .into_iter()
        .filter(|c| {
            let where_ = c["where"].as_str().unwrap();
            where_ == octos.to_str().unwrap() || where_ == canonical.to_str().unwrap()
        })
        .collect();
    assert_eq!(stale.len(), 1, "{report}");
    assert_eq!(stale[0]["status"], "warn");
    assert!(
        stale[0]["detail"]
            .as_str()
            .unwrap()
            .contains(&pid.to_string()),
        "{report}"
    );
}

/// Scenario: doctor 在 hub 上发现旧格式的单元心跳(复盘第 10 条)
#[test]
fn olp_doctor_flags_old_format_heartbeats() {
    let sb = Sb::new();
    let beats = sb.p("units/heartbeat");
    std::fs::create_dir_all(&beats).unwrap();
    std::fs::write(
        beats.join("m07.json"),
        r#"{"v":1,"unit":"m07","checks":[{"name":"disk","ok":true}]}"#,
    )
    .unwrap();
    std::fs::write(
        beats.join("m08.json"),
        r#"{"v":1,"unit":"m08","agents":[],"checks":[{"name":"agent","ok":false},{"name":"full","ok":false}]}"#,
    )
    .unwrap();
    let report = doctor_json(&sb, &[]);
    let beats = items_of(&report, "单元心跳");
    let of = |u: &str| {
        beats
            .iter()
            .find(|c| c["where"] == u)
            .unwrap_or_else(|| panic!("{report}"))
    };
    assert_eq!(of("m07")["status"], "warn", "{report}");
    assert!(
        of("m07")["fix"]
            .as_str()
            .unwrap()
            .contains("keepalive heartbeat --unit m07")
    );
    assert_eq!(of("m08")["status"], "ok", "{report}");
}
