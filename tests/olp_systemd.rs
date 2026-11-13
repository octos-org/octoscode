#![cfg(unix)]

//! Contract tests for `scripts/olp-systemd.py` (SDD spec:
//! `specs/task-olp-systemd-keepalive.spec.md`): systemd user units that keep
//! a Linux unit's resident tasks running.
//!
//! Every test passes `--no-load` and a temp `--dir`, so nothing touches a
//! real systemd user manager and the tests also run on macOS.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tool() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-systemd.py")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Run the tool with HOME pointed at `home`, so logs and the default units
/// directory never resolve to the developer's own.
fn run(home: &Path, args: &[&str]) -> Output {
    Command::new("python3")
        .arg("-B")
        .arg(tool())
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Scenario: 定时任务是 service 加 timer,常驻任务只有 service
#[test]
fn systemd_render_builds_interval_and_keepalive_units() {
    let home = tempfile::tempdir().unwrap();
    let out = run(
        home.path(),
        &[
            "render",
            "forwarder",
            "--interval",
            "30",
            "--env",
            "OLP_HUB=hub 01",
            "--",
            "/usr/bin/env",
            "--flag",
            "two words",
            r#"100% $HOME "q" back\slash"#,
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("# --- olp-forwarder.service\n"), "{text}");
    assert!(text.contains("# --- olp-forwarder.timer\n"), "{text}");
    assert!(text.contains("Type=oneshot\n"), "{text}");
    // Each argument is quoted on its own; `%` and `$` are doubled so systemd
    // neither expands a specifier nor a variable, `"` and `\` are escaped.
    assert!(
        text.contains(
            r#"ExecStart="/usr/bin/env" "--flag" "two words" "100%% $$HOME \"q\" back\\slash""#
        ),
        "{text}"
    );
    assert!(text.contains(r#"Environment="OLP_HUB=hub 01""#), "{text}");
    let path_line = text
        .lines()
        .find(|l| l.starts_with(r#"Environment="PATH="#))
        .expect("default PATH");
    assert!(path_line.contains("/.cargo/bin:"), "{path_line}");
    assert!(
        path_line.contains(":/usr/local/bin:/usr/bin:/bin"),
        "{path_line}"
    );
    let log = home.path().join(".octos/outer/logs/forwarder.log");
    assert!(
        text.contains(&format!("StandardOutput=append:{}", log.display())),
        "{text}"
    );
    assert!(text.contains("OnUnitActiveSec=30\n"), "{text}");
    assert!(text.contains("OnBootSec=10\n"), "{text}");
    assert!(text.contains("Unit=olp-forwarder.service\n"), "{text}");
    assert!(text.contains("WantedBy=timers.target\n"), "{text}");
    assert!(!text.contains("Restart=always"), "{text}");

    let out = run(
        home.path(),
        &[
            "render",
            "workspace",
            "--keepalive",
            "--",
            "/bin/sleep",
            "60",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Type=simple\n"), "{text}");
    assert!(text.contains("Restart=always\nRestartSec=10\n"), "{text}");
    assert!(text.contains("WantedBy=default.target\n"), "{text}");
    assert!(!text.contains(".timer"), "{text}");

    assert!(
        !home.path().join(".config").exists() && !home.path().join(".octos").exists(),
        "render writes nothing"
    );
}

/// Scenario: 安装、换形态、列出、卸载
#[test]
fn systemd_install_list_uninstall_roundtrip() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("units");
    let d = dir.to_str().unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    // Somebody else's unit with our prefix but without our marker, and one
    // without the prefix: neither is ours.
    std::fs::write(
        dir.join("olp-foreign.service"),
        "[Service]\nExecStart=/bin/true\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("other.service"),
        "[Service]\nExecStart=/bin/true\n",
    )
    .unwrap();

    let out = run(
        home.path(),
        &[
            "install",
            "demo",
            "--interval",
            "60",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/echo",
            "hi",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(dir.join("olp-demo.service").is_file());
    assert!(dir.join("olp-demo.timer").is_file());
    assert!(home.path().join(".octos/outer/logs").is_dir());

    let out = run(home.path(), &["list", "--dir", d, "--no-load"]);
    let text = stdout(&out);
    assert_eq!(text.lines().count(), 1, "only our job: {text}");
    assert!(
        text.starts_with("demo\t每 60 秒\t未查询\t/bin/echo hi"),
        "{text}"
    );

    // The same name as a keepalive job: the service is replaced in place and
    // the timer of the interval job must not stay behind.
    let out = run(
        home.path(),
        &[
            "install",
            "demo",
            "--keepalive",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/sleep",
            "60",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(!dir.join("olp-demo.timer").exists(), "stale timer removed");
    let service = std::fs::read_to_string(dir.join("olp-demo.service")).unwrap();
    assert!(service.contains("Restart=always"), "{service}");
    let out = run(home.path(), &["list", "--dir", d, "--no-load"]);
    assert!(
        stdout(&out).starts_with("demo\t常驻\t未查询\t/bin/sleep 60"),
        "{}",
        stdout(&out)
    );

    let out = run(home.path(), &["uninstall", "demo", "--dir", d, "--no-load"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        files_in(&dir),
        vec!["olp-foreign.service", "other.service"],
        "other people's units are untouched"
    );
    let out = run(home.path(), &["uninstall", "demo", "--dir", d, "--no-load"]);
    assert!(out.status.success(), "a second uninstall is not an error");
    assert!(stdout(&out).contains("不存在"), "{}", stdout(&out));
    let out = run(home.path(), &["list", "--dir", d, "--no-load"]);
    assert!(
        stdout(&out).contains("没有本工具注册的任务"),
        "{}",
        stdout(&out)
    );
}

/// Scenario: 心跳任务把巡检结果经 ssh 送到 hub
#[test]
fn systemd_heartbeat_job_pipes_the_unit_heartbeat_to_the_hub() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("units");
    let d = dir.to_str().unwrap();
    let scripts = home.path().join("my scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("olp-unit.py"), "").unwrap();

    let out = run(
        home.path(),
        &[
            "heartbeat",
            "--unit",
            "m07",
            "--hub",
            "olp@hub-01",
            "--scripts",
            scripts.to_str().unwrap(),
            "--dir",
            d,
            "--no-load",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
    let service = std::fs::read_to_string(dir.join("olp-heartbeat.service")).unwrap();
    let exec = service
        .lines()
        .find(|l| l.starts_with("ExecStart="))
        .expect("ExecStart");
    assert!(
        exec.starts_with(r#"ExecStart="/bin/sh" "-c" "python3 -B '"#),
        "{exec}"
    );
    assert!(
        exec.contains("my scripts/olp-unit.py' heartbeat --unit m07"),
        "{exec}"
    );
    assert!(
        exec.contains("| ssh -o BatchMode=yes -o ConnectTimeout=10 olp@hub-01 receive"),
        "{exec}"
    );
    let timer = std::fs::read_to_string(dir.join("olp-heartbeat.timer")).unwrap();
    assert!(timer.contains("OnUnitActiveSec=60\n"), "{timer}");

    // A scripts directory that is not there yet: the job is still written,
    // with a reminder.
    let out = run(
        home.path(),
        &[
            "heartbeat",
            "--unit",
            "m07",
            "--hub",
            "hub-01",
            "--scripts",
            "/nonexistent/scripts",
            "--dir",
            d,
            "--no-load",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("提醒"), "{}", stderr(&out));

    for bad in [
        vec!["heartbeat", "--unit", "M07", "--hub", "hub-01"],
        vec!["heartbeat", "--unit", "m07", "--hub", "hub;rm -rf /"],
        vec!["heartbeat", "--unit", "m07", "--hub", "-oProxyCommand=x"],
    ] {
        let mut args = bad.clone();
        args.extend(["--dir", d, "--no-load"]);
        let out = run(home.path(), &args);
        assert_eq!(out.status.code(), Some(2), "{bad:?}: {}", stderr(&out));
    }
}

/// Scenario: linger 检查只报告不修改
#[test]
fn systemd_doctor_reports_linger() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("linger.txt");

    std::fs::write(&file, "Linger=yes\n").unwrap();
    let out = run(
        home.path(),
        &["doctor", "--linger-output", file.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("ok "), "{}", stdout(&out));

    std::fs::write(&file, "Linger=no\n").unwrap();
    let out = run(
        home.path(),
        &["doctor", "--linger-output", file.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(text.starts_with("FAIL "), "{text}");
    assert!(text.contains("sudo loginctl enable-linger "), "{text}");
    assert!(text.contains("提醒"), "{text}");

    std::fs::write(&file, "something else\n").unwrap();
    let out = run(
        home.path(),
        &["doctor", "--linger-output", file.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout(&out).starts_with("?? "), "{}", stdout(&out));
}

/// Scenario: 用法错误不写任何文件
#[test]
fn systemd_rejects_bad_usage_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("units");
    let d = dir.to_str().unwrap();
    let cases: Vec<Vec<&str>> = vec![
        vec![
            "install",
            "Bad_Name",
            "--interval",
            "60",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec![
            "install",
            "demo",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec![
            "install",
            "demo",
            "--interval",
            "60",
            "--keepalive",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec![
            "install",
            "demo",
            "--interval",
            "5",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec![
            "install",
            "demo",
            "--interval",
            "60",
            "--dir",
            d,
            "--no-load",
        ],
        vec![
            "install",
            "demo",
            "--interval",
            "60",
            "--env",
            "NOEQUALS",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        // systemd does not search PATH for ExecStart.
        vec![
            "install",
            "demo",
            "--interval",
            "60",
            "--dir",
            d,
            "--no-load",
            "--",
            "echo",
            "hi",
        ],
        vec![
            "install",
            "demo",
            "--interval",
            "60",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/echo",
            "two\nlines",
        ],
        vec![
            "uninstall",
            "demo",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec!["frobnicate"],
        vec![],
    ];
    for args in cases {
        let out = run(home.path(), &args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    assert!(!dir.exists(), "nothing was written");
}
