#![cfg(unix)]

//! Contract tests for `scripts/olp-unit.py` (SDD spec:
//! `specs/task-olp-unit-registry.spec.md`): unit registration, heartbeats and
//! the health states the hub derives from them.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn tool() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-unit.py")
}

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
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn units_dir(&self) -> PathBuf {
        self.dir.path().join("units")
    }

    fn checks_dir(&self) -> PathBuf {
        self.dir.path().join("checks")
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::new("python3");
        cmd.arg("-B")
            .arg(tool())
            .env("OLP_UNITS_DIR", self.units_dir())
            .env("OLP_UNIT_CHECKS_DIR", self.checks_dir());
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    fn run_stdin(&self, args: &[&str], input: &[u8]) -> Output {
        let mut child = self
            .cmd()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The tool may reject and exit before reading everything.
        let _ = child.stdin.take().unwrap().write_all(input);
        child.wait_with_output().unwrap()
    }

    fn register(&self, unit: &str, hostname: &str, mac: &str) {
        let out = self.run(&["register", unit, "--hostname", hostname, "--mac", mac]);
        assert!(out.status.success(), "register {unit}: {}", stderr(&out));
    }

    fn receive(&self, beat: &serde_json::Value) -> Output {
        self.run_stdin(&["receive"], beat.to_string().as_bytes())
    }

    fn heartbeat_file(&self, unit: &str) -> PathBuf {
        self.units_dir()
            .join("heartbeat")
            .join(format!("{unit}.json"))
    }

    fn status(&self, extra: &[&str]) -> serde_json::Value {
        let mut args = vec!["status", "--json"];
        args.extend_from_slice(extra);
        let out = self.run(&args);
        assert!(out.status.success(), "status: {}", stderr(&out));
        serde_json::from_str(&stdout(&out)).unwrap()
    }

    fn state_of(&self, unit: &str, extra: &[&str]) -> (String, String) {
        let report = self.status(extra);
        let row = report["units"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["unit"] == unit)
            .unwrap_or_else(|| panic!("{unit} missing from {report}"))
            .clone();
        let reasons: Vec<String> = row["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect();
        (
            row["state"].as_str().unwrap().to_string(),
            reasons.join(";"),
        )
    }
}

fn beat(unit: &str, hostname: &str, macs: &[&str], checks: &[(&str, bool)]) -> serde_json::Value {
    serde_json::json!({
        "v": 1,
        "unit": unit,
        "hostname": hostname,
        "macs": macs,
        "sent_at": "2026-11-12T00:00:00Z",
        "checks": checks
            .iter()
            .map(|(name, ok)| serde_json::json!({"name": name, "ok": ok, "detail": format!("{name} detail")}))
            .collect::<Vec<_>>(),
    })
}

fn write_check(dir: &Path, name: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Scenario: 登记时单元号、主机名、网卡地址、主机密钥各自唯一
#[test]
fn unit_register_enforces_unique_identity() {
    let hub = Hub::new();
    let out = hub.run(&[
        "register",
        "m07",
        "--hostname",
        "M07.local",
        "--mac",
        "AA-BB-CC-DD-EE-07",
        "--hostkey",
        "SHA256:key07",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    // Normalised on the way in: first label, lower case, colon-separated.
    let registry = std::fs::read_to_string(hub.units_dir().join("registry.tsv")).unwrap();
    assert!(
        registry.contains("m07\tm07\taa:bb:cc:dd:ee:07\tSHA256:key07\t\tyes\t"),
        "{registry}"
    );

    let clashes: [(&[&str], &str); 3] = [
        (
            &["--hostname", "m07.lan", "--mac", "aa:bb:cc:dd:ee:08"],
            "hostname",
        ),
        (&["--hostname", "m08", "--mac", "aa:bb:cc:dd:ee:07"], "mac"),
        (
            &[
                "--hostname",
                "m08",
                "--mac",
                "aa:bb:cc:dd:ee:08",
                "--hostkey",
                "SHA256:key07",
            ],
            "hostkey",
        ),
    ];
    for (args, field) in clashes {
        let mut full = vec!["register", "m08"];
        full.extend_from_slice(args);
        let out = hub.run(&full);
        assert_eq!(out.status.code(), Some(2), "{field}: {}", stderr(&out));
        assert!(stderr(&out).contains("m07"), "{field}: {}", stderr(&out));
    }
    assert_eq!(
        std::fs::read_to_string(hub.units_dir().join("registry.tsv")).unwrap(),
        registry,
        "a rejected registration changes nothing"
    );

    for (unit, mac) in [("M07", "aa:bb:cc:dd:ee:09"), ("m09", "not-a-mac")] {
        let out = hub.run(&["register", unit, "--hostname", "h9", "--mac", mac]);
        assert_eq!(out.status.code(), Some(2), "{unit} {mac}: {}", stderr(&out));
    }

    // Re-registering updates the row and keeps a manual offline mark.
    assert!(hub.run(&["set-state", "m07", "offline"]).status.success());
    hub.register("m07", "m07", "aa:bb:cc:dd:ee:77");
    let registry = std::fs::read_to_string(hub.units_dir().join("registry.tsv")).unwrap();
    assert!(
        registry.contains("m07\tm07\taa:bb:cc:dd:ee:77\t\t\tno\t"),
        "{registry}"
    );
}

/// Scenario: 四种状态的判定与派卡前检查
#[test]
fn unit_status_classifies_healthy_degraded_stale_offline() {
    let hub = Hub::new();
    for n in 1..=4 {
        hub.register(
            &format!("m0{n}"),
            &format!("m0{n}"),
            &format!("aa:bb:cc:dd:ee:0{n}"),
        );
    }
    let ok = [("serve", true), ("disk", true)];
    let out = hub.receive(&beat(
        "m01",
        "m01",
        &["aa:bb:cc:dd:ee:01", "02:00:00:00:00:01"],
        &ok,
    ));
    assert!(out.status.success(), "{}", stderr(&out));
    let failing = [("serve", false), ("disk", true)];
    assert!(
        hub.receive(&beat("m02", "m02", &["aa:bb:cc:dd:ee:02"], &failing))
            .status
            .success()
    );
    // m03 never reports. m04 reports fine but is marked offline by hand.
    assert!(
        hub.receive(&beat("m04", "m04", &["aa:bb:cc:dd:ee:04"], &ok))
            .status
            .success()
    );
    assert!(hub.run(&["set-state", "m04", "offline"]).status.success());

    assert_eq!(hub.state_of("m01", &[]).0, "healthy");
    let (state, why) = hub.state_of("m02", &[]);
    assert_eq!(state, "degraded");
    assert!(why.contains("serve"), "{why}");
    assert_eq!(hub.state_of("m03", &[]).0, "stale");
    assert_eq!(hub.state_of("m04", &[]).0, "offline");

    let report = hub.status(&[]);
    let dispatchable: Vec<&str> = report["units"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["dispatchable"] == true)
        .map(|u| u["unit"].as_str().unwrap())
        .collect();
    assert_eq!(dispatchable, vec!["m01"], "only a healthy unit takes cards");

    // The dispatch gate: exit 0 only for a healthy unit.
    assert_eq!(hub.run(&["check", "m01"]).status.code(), Some(0));
    for unit in ["m02", "m03", "m04"] {
        let out = hub.run(&["check", unit]);
        assert_eq!(out.status.code(), Some(1), "{unit}: {}", stderr(&out));
        assert!(stderr(&out).contains(unit));
    }
    assert_eq!(hub.run(&["check", "m99"]).status.code(), Some(2));

    // Back online: the stored heartbeat counts again.
    assert!(hub.run(&["set-state", "m04", "online"]).status.success());
    assert_eq!(hub.state_of("m04", &[]).0, "healthy");
}

/// Scenario: 主机名或网卡地址与登记不符时降级
#[test]
fn unit_status_flags_identity_mismatch() {
    let hub = Hub::new();
    hub.register("m01", "m01", "aa:bb:cc:dd:ee:01");
    hub.register("m02", "m02", "aa:bb:cc:dd:ee:02");
    let ok = [("disk", true)];
    // Right MAC, wrong host name (domain suffix and case are ignored).
    assert!(
        hub.receive(&beat(
            "m01",
            "Other-Host.local",
            &["aa:bb:cc:dd:ee:01"],
            &ok
        ))
        .status
        .success()
    );
    // Right host name, but the registered MAC is not among the reported ones.
    assert!(
        hub.receive(&beat("m02", "M02.lan", &["02:00:00:00:00:99"], &ok))
            .status
            .success()
    );

    let (state, why) = hub.state_of("m01", &[]);
    assert_eq!(state, "degraded");
    assert!(why.contains("主机名不符"), "{why}");
    let (state, why) = hub.state_of("m02", &[]);
    assert_eq!(state, "degraded");
    assert!(why.contains("网卡地址不符"), "{why}");
    assert!(!why.contains("主机名不符"), "{why}");
}

/// Scenario: 未登记或与强制命令不符的心跳被拒收
#[test]
fn unit_receive_rejects_unregistered_and_mismatched_unit() {
    let hub = Hub::new();
    hub.register("m01", "m01", "aa:bb:cc:dd:ee:01");
    let ok = [("disk", true)];

    let out = hub.receive(&beat("m02", "m02", &["aa:bb:cc:dd:ee:02"], &ok));
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(!hub.heartbeat_file("m02").exists());

    // The connection is pinned to m02 (authorized_keys forced command), so a
    // heartbeat claiming to be m01 is refused even though m01 is registered.
    let claim = beat("m01", "m01", &["aa:bb:cc:dd:ee:01"], &ok);
    let out = hub.run_stdin(&["receive", "--unit", "m02"], claim.to_string().as_bytes());
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(!hub.heartbeat_file("m01").exists());

    let out = hub.run_stdin(&["receive", "--unit", "m01"], claim.to_string().as_bytes());
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(hub.heartbeat_file("m01").exists());
}

/// Scenario: 畸形心跳不落盘
#[test]
fn unit_receive_rejects_malformed_input() {
    let hub = Hub::new();
    hub.register("m01", "m01", "aa:bb:cc:dd:ee:01");
    let oversized = format!(
        "{{\"unit\":\"m01\",\"pad\":\"{}\"}}",
        "x".repeat(300 * 1024)
    );
    let cases: [&[u8]; 5] = [
        b"not json",
        b"[1,2,3]",
        b"{\"hostname\":\"m01\"}",
        b"{\"unit\":\"../../etc/passwd\"}",
        oversized.as_bytes(),
    ];
    for (i, input) in cases.iter().enumerate() {
        let out = hub.run_stdin(&["receive"], input);
        assert_eq!(out.status.code(), Some(2), "case {i}: {}", stderr(&out));
    }
    assert!(!hub.units_dir().join("heartbeat").exists());

    // Unexpected field types are tolerated and sanitised, not trusted.
    let odd = serde_json::json!({
        "unit": "m01", "hostname": 42, "macs": "aa:bb:cc:dd:ee:01",
        "checks": [{"name": "disk", "ok": "yes"}, "junk", {"ok": true}],
    });
    assert!(hub.receive(&odd).status.success());
    let (state, why) = hub.state_of("m01", &[]);
    assert_eq!(state, "degraded");
    assert!(
        why.contains("巡检未过 disk"),
        "a non-boolean ok is a failure: {why}"
    );
}

/// Scenario: 是否失联按 hub 收到心跳的时间算
#[test]
fn unit_stale_is_judged_by_hub_receive_time() {
    let hub = Hub::new();
    hub.register("m01", "m01", "aa:bb:cc:dd:ee:01");
    // The unit's own clock is far in the future; it must not matter.
    let mut b = beat("m01", "m01", &["aa:bb:cc:dd:ee:01"], &[("disk", true)]);
    b["sent_at"] = serde_json::json!("2099-01-01T00:00:00Z");
    assert!(hub.receive(&b).status.success());
    assert_eq!(hub.state_of("m01", &[]).0, "healthy");

    // Age the stored heartbeat by rewriting the hub-side receive time.
    let path = hub.heartbeat_file("m01");
    let mut stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let received = stored["received_at"].as_f64().unwrap();
    stored["received_at"] = serde_json::json!(received - 1000.0);
    std::fs::write(&path, stored.to_string()).unwrap();

    let (state, why) = hub.state_of("m01", &[]);
    assert_eq!(state, "stale", "{why}");
    assert_eq!(hub.state_of("m01", &["--stale-after", "2000"]).0, "healthy");
    assert_eq!(hub.run(&["check", "m01"]).status.code(), Some(1));
    assert_eq!(
        hub.run(&["check", "m01", "--stale-after", "2000"])
            .status
            .code(),
        Some(0)
    );
}

/// Scenario: 心跳带上内置巡检与扩展检查的结果
#[test]
fn unit_heartbeat_reports_builtin_and_plugin_checks() {
    let hub = Hub::new();
    write_check(&hub.checks_dir(), "10-lanes", "echo '2 lanes running'");
    write_check(
        &hub.checks_dir(),
        "20-device",
        "echo 'd03 not responding'; exit 1",
    );
    std::fs::write(hub.checks_dir().join("README"), "not executable").unwrap();

    let out = hub.run(&[
        "heartbeat",
        "--unit",
        "m01",
        "--require",
        "disk",
        "--disk-min-free",
        "0",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let b: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(b["unit"], "m01");
    assert!(!b["hostname"].as_str().unwrap().contains('.'));
    let names: Vec<(&str, bool)> = b["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["name"].as_str().unwrap(), c["ok"].as_bool().unwrap()))
        .collect();
    assert_eq!(
        names,
        vec![("disk", true), ("10-lanes", true), ("20-device", false)]
    );

    // Register this very machine from what it reported, then feed the
    // heartbeat to the hub side: the failing plugin makes the unit degraded.
    let Some(mac) = b["macs"]
        .as_array()
        .and_then(|m| m.first())
        .and_then(|m| m.as_str())
    else {
        eprintln!("SKIP (explicit): host reports no MAC address; stopping after the shape checks");
        return;
    };
    hub.register("m01", b["hostname"].as_str().unwrap(), mac);
    let out = hub.run_stdin(&["receive", "--unit", "m01"], stdout(&out).as_bytes());
    assert!(out.status.success(), "{}", stderr(&out));
    let (state, why) = hub.state_of("m01", &[]);
    assert_eq!(state, "degraded");
    assert!(
        why.contains("20-device") && why.contains("d03 not responding"),
        "{why}"
    );
    assert!(!why.contains("不符"), "identity matches: {why}");

    let out = hub.run(&["heartbeat", "--unit", "m01", "--require", "bogus"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

/// Scenario: 心跳报告内环后端的启动权限
#[test]
fn unit_heartbeat_reports_launch_permission() {
    let hub = Hub::new();
    let ps = hub.dir.path().join("ps.txt");
    let heartbeat = |listing: &str| -> (String, serde_json::Value) {
        std::fs::write(&ps, listing).unwrap();
        let out = hub.run(&[
            "heartbeat",
            "--unit",
            "m01",
            "--require",
            "",
            "--ps-output",
            ps.to_str().unwrap(),
        ]);
        assert!(out.status.success(), "{}", stderr(&out));
        let raw = stdout(&out);
        let v = serde_json::from_str(&raw).unwrap();
        (raw, v)
    };

    // The TUI wraps the backend and carries the same words in one argument;
    // only real `octos serve` processes count.
    let tui = "  4100 octoscode --stdio-command octos serve --stdio --solo --danger-full-access\n";
    let full = format!(
        "{tui}  4101 /opt/homebrew/bin/octos serve --stdio --solo --danger-full-access --api-key SECRET-TOKEN\n  4102 octos-jev serve --stdio --solo --danger-full-access\n  4103 /usr/bin/python3 other.py serve\n"
    );
    let (raw, hb) = heartbeat(&full);
    assert_eq!(hb["permission"], "full");
    assert_eq!(
        hb["backends"],
        serde_json::json!([
            {"pid": 4101, "full_access": true, "solo": true},
            {"pid": 4102, "full_access": true, "solo": true},
        ])
    );
    assert!(
        !raw.contains("SECRET-TOKEN"),
        "only two flags leave the unit: {raw}"
    );
    assert!(!raw.contains("api-key"), "{raw}");

    let (_, hb) = heartbeat("  4101 octos serve --stdio --solo\n");
    assert_eq!(hb["permission"], "sandboxed");
    let (_, hb) = heartbeat(
        "  4101 octos serve --stdio --solo\n  4102 octos serve --stdio --solo --danger-full-access\n",
    );
    assert_eq!(hb["permission"], "mixed");
    let (_, hb) = heartbeat(tui);
    assert_eq!(hb["permission"], "none");
    assert_eq!(hb["backends"], serde_json::json!([]));

    // Through the hub: the status shows what the unit reported, and the
    // permission does not change the health state.
    hub.register("m01", "m01", "aa:bb:cc:dd:ee:01");
    let mut reported = beat("m01", "m01", &["aa:bb:cc:dd:ee:01"], &[("disk", true)]);
    reported["permission"] = serde_json::json!("full");
    reported["backends"] = serde_json::json!([
        {"pid": 4101, "full_access": true, "solo": true, "argv": ["--api-key", "SECRET-TOKEN"]},
        {"pid": "x", "full_access": true},
        "junk",
    ]);
    assert!(hub.receive(&reported).status.success());
    let report = hub.status(&[]);
    let row = &report["units"][0];
    assert_eq!(row["state"], "healthy");
    assert_eq!(row["permission"], "full");
    assert_eq!(
        row["backends"],
        serde_json::json!([{"pid": 4101, "full_access": true, "solo": true}]),
        "unknown fields and malformed entries are dropped"
    );
    assert!(!report.to_string().contains("SECRET-TOKEN"));
    let text = stdout(&hub.run(&["status"]));
    assert!(text.contains("权限 完全访问"), "{text}");

    // A value outside the known set, or a heartbeat from an older tool
    // without the field, is shown as unknown rather than trusted or failed.
    for odd in [serde_json::json!("root"), serde_json::Value::Null] {
        let mut b = beat("m01", "m01", &["aa:bb:cc:dd:ee:01"], &[("disk", true)]);
        if !odd.is_null() {
            b["permission"] = odd;
        }
        assert!(hub.receive(&b).status.success());
        let row = hub.status(&[])["units"][0].clone();
        assert_eq!(row["permission"], "unknown");
        assert_eq!(row["state"], "healthy");
    }
}
