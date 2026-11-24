#![cfg(unix)]

//! Contract tests for `olp board stall` (SDD spec:
//! `specs/task-olp-stall.spec.md`): the cluster is stalled when cards are
//! open, nobody works on them and the log is silent; each silence is
//! reported with its card, cause and event, and not over and over.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::io::Write;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_octoscode");

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

struct Hub {
    dir: tempfile::TempDir,
}

impl Hub {
    fn new() -> Self {
        Hub {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn log(&self) -> String {
        self.dir.path().join("log").to_string_lossy().into_owned()
    }

    fn olp(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(BIN)
            .arg("olp")
            .args(args)
            .env("OLP_UNITS_DIR", self.dir.path().join("units"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
        child.wait_with_output().unwrap()
    }

    fn ok(&self, args: &[&str], stdin: &str) -> String {
        let out = self.olp(args, stdin);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        stdout(&out)
    }

    /// Register `unit` and give it a heartbeat whose one inner loop is in `status`.
    fn beat(&self, unit: &str, status: &str) {
        let host = format!("host-{unit}");
        let mac = format!("aa:bb:cc:dd:ee:{:02x}", unit.as_bytes()[unit.len() - 1]);
        self.ok(
            &["unit", "register", unit, "--hostname", &host, "--mac", &mac],
            "",
        );
        let beat = serde_json::json!({
            "v": 1, "unit": unit, "hostname": host, "macs": [mac], "checks": [],
            "agents": [{"kind": "octoscode", "status": status, "pane": "w1:p1"}],
        });
        self.ok(&["unit", "receive"], &beat.to_string());
    }

    fn card(&self, unit: &str, title: &str) {
        let log = self.log();
        self.ok(
            &[
                "card",
                "dispatch",
                unit,
                "--log",
                &log,
                "--title",
                title,
                "--wake",
                "w1:p1",
                "--allow-busy",
            ],
            "正文\n",
        );
    }

    fn stall(&self, extra: &[&str]) -> serde_json::Value {
        let log = self.log();
        let mut args = vec!["board", "stall", &log, "--json"];
        args.extend_from_slice(extra);
        serde_json::from_str(&self.ok(&args, "")).unwrap()
    }
}

/// Scenario: 有人在工作、或日志刚有动静,都不算静默
#[test]
fn working_units_and_a_fresh_log_are_not_a_stall() {
    let hub = Hub::new();
    hub.beat("m07", "working");
    hub.beat("m08", "idle");
    std::fs::create_dir_all(hub.log()).unwrap();
    // No open card at all.
    let seen = hub.stall(&["--quiet", "0"]);
    assert_eq!(
        (seen["stalled"].as_bool(), seen["open"].as_u64()),
        (Some(false), Some(0))
    );

    hub.card("m07", "长时间构建");
    hub.card("m08", "等着的卡");
    // One holder is working: a long build is normal.
    let seen = hub.stall(&["--quiet", "0"]);
    assert_eq!(seen["stalled"], false, "{seen}");
    assert_eq!(seen["working_units"], serde_json::json!(["m07"]));
    assert!(seen["alerts"].as_array().unwrap().is_empty());

    // Nobody works now, but the log moved a moment ago.
    hub.beat("m07", "idle");
    let seen = hub.stall(&[]);
    assert_eq!(seen["stalled"], false, "{seen}");
    let text = hub.ok(&["board", "stall", &hub.log()], "");
    assert!(text.contains("还没到静默的时间"), "{text}");
}

/// Scenario: 整体静默时逐张卡说明原因、起点和谁能推动
#[test]
fn a_stall_names_each_card_its_cause_and_who_can_move_it() {
    let hub = Hub::new();
    hub.beat("m07", "idle");
    hub.beat("m08", "blocked");
    hub.card("m07", "没领取的卡");
    hub.card("m08", "领了没回执的卡");
    hub.card("m08", "等外环复验的卡");
    let log = hub.log();
    let conf = hub.dir.path().join("unit.conf");
    std::fs::write(
        &conf,
        format!("hub = local\nunit = m08\nstore = log\nlog = {log}\n"),
    )
    .unwrap();
    let unit = |args: &[&str]| {
        let out = Command::new(BIN)
            .args(["olp", "remote"])
            .args(args)
            .env("OLP_UNIT_CONF", &conf)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    };
    unit(&["progress", "2", "claimed"]);
    unit(&["progress", "3", "claimed"]);
    unit(&["ack", "3", "done", "commit abc1234,测试通过"]);

    let seen = hub.stall(&["--quiet", "0"]);
    assert_eq!(seen["stalled"], true, "{seen}");
    let alerts = seen["alerts"].as_array().unwrap();
    assert_eq!(alerts.len(), 3, "{seen}");
    let of = |n: u64| alerts.iter().find(|a| a["number"] == n).unwrap();
    assert_eq!(of(1)["waits_on"], "unit");
    assert!(of(1)["cause"].as_str().unwrap().contains("没有领取"));
    assert_eq!(
        (of(1)["unit"].as_str(), of(1)["seq"].as_u64()),
        (Some("m07"), Some(1))
    );
    assert!(of(2)["cause"].as_str().unwrap().contains("领取后没有回执"));
    assert_eq!(of(3)["waits_on"], "outer");
    assert!(
        of(3)["cause"].as_str().unwrap().contains("待复验"),
        "{seen}"
    );
    assert!(!of(3)["since"].as_str().unwrap().is_empty());

    // A unit the hub cannot see is said to be unseen, not assumed busy.
    hub.ok(
        &[
            "card",
            "dispatch",
            "m09",
            "--log",
            &log,
            "--title",
            "未登记单元的卡",
            "--skip-health",
        ],
        "正文\n",
    );
    let seen = hub.stall(&["--quiet", "0", "--state", "/dev/null"]);
    assert_eq!(seen["unknown_units"], serde_json::json!(["m09"]));
    assert_eq!(seen["stalled"], true);
}

/// Scenario: 同一段沉默不连着报,变了才再报,最多三次
#[test]
fn the_same_silence_is_not_reported_over_and_over() {
    let hub = Hub::new();
    hub.beat("m07", "idle");
    hub.card("m07", "卡一");
    hub.card("m07", "卡二");
    let count = |seen: &serde_json::Value| seen["alerts"].as_array().unwrap().len();
    let run = || hub.stall(&["--quiet", "0"]);
    assert_eq!(count(&run()), 2); // run 1: both, first report
    let again = run(); // run 2
    assert_eq!((count(&again), again["held_back"].as_u64()), (0, Some(2)));
    assert_eq!(again["stalled"], true);
    assert_eq!(count(&run()), 0); // run 3

    // Something happened to card 2: it is reported afresh, card 1 is not.
    let log = hub.log();
    hub.ok(
        &[
            "card",
            "snooze",
            "--log",
            &log,
            "2",
            "--until",
            "2020-01-01",
            "等过了",
        ],
        "",
    );
    let seen = run(); // run 4: card 1 is due again as well (3 runs later)
    let numbers: Vec<u64> = seen["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, vec![1, 2], "{seen}");
    assert_eq!(seen["alerts"][0]["reports"], 2);
    assert_eq!(seen["alerts"][1]["reports"], 1);

    // Card 1 gets its third report three runs later, and then no more.
    let mut reports_of_1 = 2;
    for _ in 0..12 {
        let seen = run();
        if let Some(a) = seen["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["number"] == 1)
        {
            reports_of_1 = a["reports"].as_u64().unwrap();
        }
    }
    assert_eq!(reports_of_1, 3);
    assert_eq!(count(&run()), 0);

    // Usage.
    let out = hub.olp(&["board", "stall"], "");
    assert_eq!(out.status.code(), Some(2));
    let out = hub.olp(&["board", "stall", "/no/such/dir"], "");
    assert_eq!(out.status.code(), Some(2));
}

/// A policy: a shell script that ignores its input and prints `answer`.
#[cfg(test)]
fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

/// Scenario: 判定可以交给外部策略,它看到事实并决定每张卡的原因
#[test]
fn an_external_policy_decides_and_sees_the_facts() {
    let hub = Hub::new();
    hub.beat("m08", "blocked");
    hub.card("m08", "卡在审批框的卡");
    let seen_facts = hub.dir.path().join("facts.json");
    let policy = script(
        hub.dir.path(),
        "policy.sh",
        &format!(
            "cat > {}\necho 'a line that is not the answer'\n\
             echo '{{\"stalled\":true,\"alerts\":[{{\"number\":1,\"waits_on\":\"unit\",\"cause\":\"内环停在审批框上\"}}]}}'",
            seen_facts.display()
        ),
    );
    // The built-in rule cannot tell a blocked inner loop from an absent one.
    let builtin = hub.stall(&["--quiet", "0", "--state", "/dev/null"]);
    assert!(
        builtin["alerts"][0]["cause"]
            .as_str()
            .unwrap()
            .contains("没有领取")
    );

    let seen = hub.stall(&["--quiet", "0", "--state", "/dev/null", "--policy", &policy]);
    assert_eq!(seen["stalled"], true, "{seen}");
    let alert = &seen["alerts"][0];
    assert_eq!(alert["cause"], "内环停在审批框上");
    // What the tool knows stays the tool's: unit, event and time are not the policy's to say.
    assert_eq!(
        (alert["unit"].as_str(), alert["seq"].as_u64()),
        (Some("m08"), Some(1))
    );
    let facts: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&seen_facts).unwrap()).unwrap();
    let card = &facts["cards"][0];
    assert_eq!(
        (
            card["number"].as_u64(),
            card["unit"].as_str(),
            card["partition"].as_str(),
            card["claimed"].as_bool()
        ),
        (Some(1), Some("m08"), Some("working"), Some(false)),
        "{facts}"
    );
    assert_eq!(card["agents"], serde_json::json!(["blocked"]));
    assert_eq!(facts["quiet"], 0);

    // A policy may also say that this is no stall. The environment variable names one too.
    let calm = script(hub.dir.path(), "calm.sh", "echo '{\"stalled\":false}'");
    let log = hub.log();
    let out = Command::new(BIN)
        .args(["olp", "board", "stall", &log, "--quiet", "0", "--json"])
        .args(["--state", "/dev/null"])
        .env("OLP_UNITS_DIR", hub.dir.path().join("units"))
        .env("OLP_STALL_POLICY", &calm)
        .output()
        .unwrap();
    let seen: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(seen["stalled"], false, "{seen}");
}

/// Scenario: 策略失败或越界时退回内置规则并说明原因
#[test]
fn a_policy_that_fails_or_oversteps_falls_back_to_the_builtin_rule() {
    let hub = Hub::new();
    hub.beat("m07", "idle");
    hub.card("m07", "没领取的卡");
    let log = hub.log();
    let alert = |cause: &str, waits_on: &str, number: u64| {
        format!(
            "echo '{{\"stalled\":true,\"alerts\":[{{\"number\":{number},\"waits_on\":\"{waits_on}\",\"cause\":\"{cause}\"}}]}}'"
        )
    };
    let long = "长".repeat(201);
    let cases = [
        ("exit 3", "退出码 3"),
        ("echo not json", "输出里没有 JSON"),
        ("echo '{\"alerts\":[]}'", "输出不是约定的 JSON"),
        (&alert("x", "unit", 99)[..], "#99 不是未了结的卡"),
        (
            &alert("x", "operator", 1)[..],
            "waits_on 只能是 unit 或 outer",
        ),
        (&alert("", "unit", 1)[..], "cause 要是 1 到 200 字的一行"),
        (&alert(&long, "unit", 1)[..], "cause 要是 1 到 200 字的一行"),
        (
            "echo '{\"stalled\":true,\"alerts\":[{\"number\":1,\"waits_on\":\"unit\",\"cause\":\"a\"},{\"number\":1,\"waits_on\":\"unit\",\"cause\":\"b\"}]}'",
            "#1 出现了两次",
        ),
    ];
    for (index, (body, why)) in cases.iter().enumerate() {
        let policy = script(hub.dir.path(), &format!("p{index}.sh"), body);
        let out = hub.olp(
            &[
                "board",
                "stall",
                &log,
                "--quiet",
                "0",
                "--json",
                "--state",
                "/dev/null",
                "--policy",
                &policy,
            ],
            "",
        );
        assert!(out.status.success(), "{body}: {}", stderr(&out));
        let err = stderr(&out);
        assert!(
            err.contains(why) && err.contains("改用内置规则"),
            "{body}: {err}"
        );
        let seen: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        assert!(
            seen["alerts"][0]["cause"]
                .as_str()
                .unwrap()
                .contains("没有领取"),
            "{body}: {seen}"
        );
    }
    // A command that cannot start is a failed policy too, not a crash.
    let out = hub.olp(
        &[
            "board",
            "stall",
            &log,
            "--quiet",
            "0",
            "--state",
            "/dev/null",
            "--policy",
            "/nonexistent/policy",
        ],
        "",
    );
    assert!(
        out.status.success() && stderr(&out).contains("起不来"),
        "{}",
        stderr(&out)
    );
    assert!(stdout(&out).contains("没有领取"));
    // --policy without its value is a usage error.
    assert_eq!(
        hub.olp(&["board", "stall", &log, "--policy"], "")
            .status
            .code(),
        Some(2)
    );
}

/// Scenario: 没有带 relax feature 的构建拒绝 --policy-crate
#[cfg(not(feature = "relax"))]
#[test]
fn a_build_without_relax_refuses_a_policy_crate() {
    let hub = Hub::new();
    std::fs::create_dir_all(hub.log()).unwrap();
    let out = hub.olp(
        &[
            "board",
            "stall",
            &hub.log(),
            "--policy-crate",
            "/tmp/whatever",
        ],
        "",
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("--features relax"),
        "{}",
        stderr(&out)
    );
}

/// Scenario: 带 relax feature 时,策略包由 relax 在包目录里从源码运行
#[cfg(feature = "relax")]
#[test]
fn a_policy_crate_is_run_by_relax_in_its_own_folder() {
    let hub = Hub::new();
    hub.beat("m07", "idle");
    hub.card("m07", "没领取的卡");
    let log = hub.log();
    let package = hub.dir.path().join("stall-policy");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("Cargo.toml"),
        "[package]\nname = \"stall-policy\"\nversion = \"0.1.0\"\n[dependencies]\nname = \"x\"\n",
    )
    .unwrap();
    let calls = hub.dir.path().join("relax-calls");
    // A stand-in for relax: records how it was called and answers.
    let relax = script(
        hub.dir.path(),
        "relax",
        &format!(
            "echo \"$PWD|$*\" >> {}\ncat > /dev/null\n\
             echo '{{\"stalled\":true,\"alerts\":[{{\"number\":1,\"waits_on\":\"outer\",\"cause\":\"策略包说的\"}}]}}'",
            calls.display()
        ),
    );
    let run = |dir: &str| {
        Command::new(BIN)
            .args(["olp", "board", "stall", &log, "--quiet", "0", "--json"])
            .args(["--state", "/dev/null", "--policy-crate", dir])
            .env("OLP_UNITS_DIR", hub.dir.path().join("units"))
            .env("OLP_RELAX", &relax)
            .output()
            .unwrap()
    };
    let out = run(&package.to_string_lossy());
    assert!(out.status.success(), "{}", stderr(&out));
    let seen: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(seen["alerts"][0]["cause"], "策略包说的", "{seen}");
    assert_eq!(seen["alerts"][0]["waits_on"], "outer");
    let call = std::fs::read_to_string(&calls).unwrap();
    let real = std::fs::canonicalize(&package).unwrap();
    let (cwd, args) = call.trim().split_once('|').unwrap();
    assert_eq!(std::fs::canonicalize(cwd).unwrap(), real);
    assert_eq!(args, "run --unattended --quiet stall-policy");

    // A folder that is not a package is a usage error, before anything runs.
    let out = run(&hub.dir.path().to_string_lossy());
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("Cargo.toml"), "{}", stderr(&out));
}
