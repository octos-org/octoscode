//! Tests never reach production OLP state (retrospective #15): every
//! integration test file includes `support/olp_env.rs`, which drops the
//! `OLP_*` variables a test process inherits before the harness starts.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::process::Command;

/// Marker for the child run below. Not `OLP_*`, so it survives the cleanup.
const CHILD: &str = "OCTOSCODE_TEST_ISOLATION_CHILD";

/// Scenario: 每个集成测试文件都引用了清除继承 OLP_* 变量的模块
#[test]
fn every_integration_test_file_drops_inherited_olp_variables() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            if !text.contains("#[path = \"support/olp_env.rs\"]") {
                missing.push(path.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
    }
    assert!(
        missing.is_empty(),
        "these test files must include `#[path = \"support/olp_env.rs\"] mod olp_env;`: {missing:?}"
    );
}

/// Runs only as the child of the test below: report the OLP_* it can see.
#[test]
fn child_reports_its_olp_variables() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let seen: Vec<String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("OLP_"))
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    println!("SEEN[{}]", seen.join(";"));
}

/// Scenario: 测试进程继承的 OLP_* 变量在测试开始前被清掉,测试开关保留
#[test]
fn inherited_olp_variables_are_gone_before_any_test_runs() {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_reports_its_olp_variables", "--nocapture"])
        .env(CHILD, "1")
        .env("OLP_UNIT_CONF", "/production/unit.conf")
        .env("OLP_HUB", "production-hub")
        .env("OLP_MCP_OUTER_ROOT", "/production/outer")
        .env("OLP_MCP_SERVER_BIN", "/kept/binary")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    let seen = stdout
        .split("SEEN[")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_else(|| panic!("the child did not report: {stdout}"));
    for name in ["OLP_UNIT_CONF", "OLP_HUB", "OLP_MCP_OUTER_ROOT"] {
        assert!(!seen.contains(name), "{name} reached the test: {seen}");
    }
    assert!(
        seen.contains("OLP_MCP_SERVER_BIN=/kept/binary"),
        "test knobs in {:?} stay: {seen}",
        olp_env::KEPT
    );
}
