#![cfg(unix)]

//! Contract tests for `scripts/olp-launchd.py` (SDD spec:
//! `specs/task-olp-unit-keepalive.spec.md`): launchd jobs that keep a unit's
//! resident tasks running.
//!
//! Every test passes `--no-load` and a temp `--dir`, so nothing touches the
//! real launchd domain and the tests also run on Linux CI.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tool() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-launchd.py")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Run the tool with HOME pointed at `home`, so logs and the default agents
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

/// The text between `<key>KEY</key>` and the next `<key>` in a plist.
fn value_after<'a>(plist: &'a str, key: &str) -> &'a str {
    let start = plist
        .find(&format!("<key>{key}</key>"))
        .unwrap_or_else(|| panic!("plist has no {key}: {plist}"));
    let rest = &plist[start + key.len() + 11..];
    rest.split("<key>").next().unwrap()
}

/// Scenario: 定时任务与常驻任务的 plist
#[test]
fn launchd_render_builds_interval_and_keepalive_jobs() {
    let home = tempfile::tempdir().unwrap();
    let out = run(
        home.path(),
        &[
            "render",
            "forwarder",
            "--interval",
            "30",
            "--env",
            "OLP_HUB=hub-01",
            "--",
            "/usr/bin/env",
            "--flag",
            "a b",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let plist = stdout(&out);
    assert!(value_after(&plist, "Label").contains("org.octos.olp.forwarder"));
    assert!(value_after(&plist, "StartInterval").contains("<integer>30</integer>"));
    assert!(!plist.contains("<key>KeepAlive</key>"));
    // The job's own flags and spaces survive as separate arguments.
    let argv = value_after(&plist, "ProgramArguments");
    for arg in [
        "<string>/usr/bin/env</string>",
        "<string>--flag</string>",
        "<string>a b</string>",
    ] {
        assert!(argv.contains(arg), "{argv}");
    }
    // EnvironmentVariables is a nested dict: read up to its closing tag.
    let env_start = plist.find("<key>EnvironmentVariables</key>").unwrap();
    let env = plist[env_start..].split("</dict>").next().unwrap();
    assert!(env.contains("<string>hub-01</string>"), "{env}");
    assert!(
        env.contains("/opt/homebrew/bin"),
        "default PATH covers Homebrew: {env}"
    );
    let log = format!("{}/.octos/outer/logs/forwarder.log", home.path().display());
    assert!(value_after(&plist, "StandardErrorPath").contains(&log));
    assert!(value_after(&plist, "RunAtLoad").contains("<true/>"));

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
    assert!(out.status.success(), "{}", stderr(&out));
    let plist = stdout(&out);
    assert!(value_after(&plist, "KeepAlive").contains("<true/>"));
    assert!(!plist.contains("<key>StartInterval</key>"));
    // render writes nothing.
    assert!(std::fs::read_dir(home.path()).unwrap().next().is_none());
}

/// Scenario: 安装、列出、卸载
#[test]
fn launchd_install_list_uninstall_roundtrip() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("agents");
    let dir_s = dir.to_str().unwrap();

    let out = run(
        home.path(),
        &[
            "install",
            "forwarder",
            "--interval",
            "45",
            "--dir",
            dir_s,
            "--no-load",
            "--",
            "/bin/echo",
            "hi",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let path = dir.join("org.octos.olp.forwarder.plist");
    let first = std::fs::read_to_string(&path).unwrap();
    assert!(first.contains("<integer>45</integer>"));
    assert!(
        home.path().join(".octos/outer/logs").is_dir(),
        "log dir is created"
    );

    // Installing again replaces the job in place.
    let out = run(
        home.path(),
        &[
            "install",
            "forwarder",
            "--keepalive",
            "--dir",
            dir_s,
            "--no-load",
            "--",
            "/bin/sleep",
            "9",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("<key>KeepAlive</key>")
    );
    // A foreign plist in the same directory is not ours.
    std::fs::write(dir.join("com.example.other.plist"), "x").unwrap();

    let out = run(home.path(), &["list", "--dir", dir_s, "--no-load"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let listing = stdout(&out);
    assert!(
        listing.contains("forwarder\t常驻\t未查询\t/bin/sleep 9"),
        "{listing}"
    );
    assert!(!listing.contains("example"), "{listing}");

    let out = run(
        home.path(),
        &["uninstall", "forwarder", "--dir", dir_s, "--no-load"],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!path.exists());
    assert!(
        dir.join("com.example.other.plist").exists(),
        "foreign files are left alone"
    );
    // Uninstalling a job that is not there is not an error.
    assert!(
        run(
            home.path(),
            &["uninstall", "forwarder", "--dir", dir_s, "--no-load"]
        )
        .status
        .success()
    );
    let out = run(home.path(), &["list", "--dir", dir_s, "--no-load"]);
    assert!(stdout(&out).contains("没有本工具注册的任务"));
}

/// Scenario: 心跳任务把巡检结果经 ssh 送到 hub
#[test]
fn launchd_heartbeat_job_pipes_the_unit_heartbeat_to_the_hub() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("agents");
    let scripts = home.path().join("octo scripts");
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
            dir.to_str().unwrap(),
            "--no-load",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "", "the unit tool exists, so no reminder");
    let plist = std::fs::read_to_string(dir.join("org.octos.olp.heartbeat.plist")).unwrap();
    assert!(value_after(&plist, "StartInterval").contains("<integer>60</integer>"));
    let argv = value_after(&plist, "ProgramArguments");
    assert!(argv.contains("<string>/bin/sh</string>") && argv.contains("<string>-c</string>"));
    // The script path has a space and must be quoted inside the pipeline.
    let expected = format!(
        "python3 -B &apos;{}/olp-unit.py&apos; heartbeat --unit m07 | ssh -o BatchMode=yes -o ConnectTimeout=10 olp@hub-01 receive",
        scripts.display()
    );
    assert!(
        argv.contains(&expected) || argv.contains(&expected.replace("&apos;", "'")),
        "{argv}"
    );

    // A missing unit tool is a reminder, not a failure.
    let out = run(
        home.path(),
        &[
            "heartbeat",
            "--unit",
            "m07",
            "--hub",
            "hub-01",
            "--scripts",
            "/nonexistent",
            "--dir",
            dir.to_str().unwrap(),
            "--no-load",
            "--interval",
            "120",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("olp-unit.py"), "{}", stderr(&out));

    for bad in [
        vec!["heartbeat", "--unit", "M07", "--hub", "hub-01"],
        vec!["heartbeat", "--unit", "m07", "--hub", "hub; rm -rf /"],
        vec!["heartbeat", "--unit", "m07", "--hub", "-oProxyCommand=x"],
    ] {
        let mut args = bad.clone();
        args.extend(["--dir", dir.to_str().unwrap(), "--no-load"]);
        let out = run(home.path(), &args);
        assert_eq!(out.status.code(), Some(2), "{bad:?}: {}", stderr(&out));
    }
}

/// Scenario: 电源设置检查只报告不修改
#[test]
fn launchd_doctor_reports_power_settings() {
    let home = tempfile::tempdir().unwrap();
    let sample = home.path().join("pmset.txt");
    let good = "\
System-wide power settings:
Currently in use:
 standby              1
 autorestart          1
 sleep                0 (sleep prevented by caffeinate)
 displaysleep         5
Battery Power:
 sleep                10
 autorestart          0
";
    std::fs::write(&sample, good).unwrap();
    let out = run(
        home.path(),
        &["doctor", "--pmset-output", sample.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert_eq!(stdout(&out).matches("ok ").count(), 2, "{}", stdout(&out));

    // Sleeping machine, and no autorestart line at all (a laptop).
    std::fs::write(&sample, "Currently in use:\n sleep                10\n").unwrap();
    let out = run(
        home.path(),
        &["doctor", "--pmset-output", sample.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(
        text.contains("FAIL") && text.contains("sudo pmset -a sleep 0"),
        "{text}"
    );
    assert!(
        text.contains("??") && text.contains("sudo pmset -a autorestart 1"),
        "{text}"
    );
    assert!(text.contains("自动登录"), "{text}");
}

/// Scenario: 用法错误不写任何文件
#[test]
fn launchd_rejects_bad_usage_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("agents");
    let d = dir.to_str().unwrap();
    let cases: Vec<Vec<&str>> = vec![
        vec![
            "install",
            "Bad_Name",
            "--keepalive",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec!["install", "job", "--dir", d, "--no-load", "--", "/bin/true"],
        vec![
            "install",
            "job",
            "--keepalive",
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
            "job",
            "--interval",
            "5",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec!["install", "job", "--keepalive", "--dir", d, "--no-load"],
        vec![
            "install",
            "job",
            "--keepalive",
            "--env",
            "NOEQUALS",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec![
            "uninstall",
            "job",
            "--dir",
            d,
            "--no-load",
            "--",
            "/bin/true",
        ],
        vec!["bogus"],
        vec![],
    ];
    for args in cases {
        let out = run(home.path(), &args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    assert!(!dir.exists(), "nothing may be written");
}
