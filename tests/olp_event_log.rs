#![cfg(unix)]

//! Contract tests for the event log as a board (SDD spec:
//! `specs/task-olp-event-log.spec.md`): a Markdown board imported into a
//! JSON Lines log is classified exactly as the board was.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn olp(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_octoscode"))
        .arg("olp")
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Every form the inbox reads: units and roles, bare and signed ACKs, the
/// three verdicts, a void, a push, an archive stub, a fenced example that
/// must not count, questions asked and closed, a closed history section.
const BOARD: &str = "\
# 外环审查通道

## Historical record

### 90. 只供审计的旧卡
ACK(done): 旧

## Active

### 1. 已采认的卡(2026-11-10,外环(hub))
unit: m07
impl: octoscode
review: codex, claude

正文一。

```
### 99. 示例里的标题
ACK(done): 示例
> 外环(hub)·采认(#2): 示例
```

### 2. 等复验的卡
unit: m08
ACK(done): commit abc

### 3. 被打回的单元卡
unit: m07

### 4. 搁置的卡
ACK(blocked): 等依赖

### 5. 作废的卡

### 6. 旧归档桩
unit: m09
ACK(done): 做完
<!-- olp-archived: board.archive.md -->

### 7. 还没动的卡
unit: m08

> 内环(m07)·#1 ACK(done): 推了
> 单元(m07)·本地复验通过(#1): task/1 @ 3f2a9c1d4e5f 复验 m07-codex+claude
> 外环(hub)·采认(#1): 复验记录齐
> 外环(hub)·打回(#3): 缺测试
> 外环(hub)·搁置(#4) 至 2026-12-01:等上游
> 外环(hub)·改判(作废 #5)
> 外环(hub)·采认(#77): 板上没有这张卡
> 单元(m08)·- 10:00 MCP(ask_outer) ask: id=q1 question=用哪个分支?
> 单元(m08)·- 10:01 MCP(ask_outer) ask: id=q2 question=要不要改接口?
> 单元(m08)·- 10:02 MCP(ask_outer) answer: id=q1
";

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new(board: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("board.md"), board).unwrap();
        Sandbox { dir }
    }

    fn board(&self) -> String {
        self.dir
            .path()
            .join("board.md")
            .to_string_lossy()
            .into_owned()
    }

    fn log(&self) -> String {
        self.dir.path().join("log").to_string_lossy().into_owned()
    }

    fn import(&self) -> Output {
        olp(&["board", "import", &self.board(), "--to", &self.log()])
    }

    /// The inbox report of `source`, without what legitimately differs
    /// between the two stores: warnings and line numbers.
    fn report(&self, source: &str, args: &[&str]) -> serde_json::Value {
        let mut full = vec!["board", "inbox", source, "--json"];
        full.extend_from_slice(args);
        let out = olp(&full);
        assert!(out.status.success(), "{}", stderr(&out));
        let mut report: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        let fields = report.as_object_mut().unwrap();
        fields.remove("warnings");
        if let Some(questions) = fields.get_mut("questions").and_then(|q| q.as_array_mut()) {
            for q in questions {
                q.as_object_mut().unwrap().remove("line");
            }
        }
        report
    }
}

fn events(log: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(Path::new(log).join("events-000001.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Scenario: 导入后的事件日志与原黑板得出同样的收件箱(critical)
#[test]
fn imported_log_classifies_every_card_as_the_board_did() {
    let sb = Sandbox::new(BOARD);
    let out = sb.import();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("已导入 7 张卡"), "{}", stdout(&out));
    // What the log has no place for is said, not dropped silently.
    assert!(
        stderr(&out).contains("未导入") && stderr(&out).contains("#77"),
        "{}",
        stderr(&out)
    );

    for args in [
        vec!["--all"],
        vec!["--all", "--today", "2026-11-20"],
        vec!["--all", "--today", "2026-12-05"],
        vec!["--all", "--unit", "m07"],
        vec!["--units"],
        vec![],
    ] {
        assert_eq!(
            sb.report(&sb.log(), &args),
            sb.report(&sb.board(), &args),
            "{args:?}"
        );
    }
    // The picture itself, so both being wrong together would show.
    let report = sb.report(&sb.log(), &["--all", "--today", "2026-11-20"]);
    let state = |n: u64| {
        let item = report["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["number"] == n)
            .unwrap();
        item["partition"].as_str().unwrap().to_string()
    };
    assert_eq!(
        [1, 2, 3, 4, 5, 6, 7].map(state),
        [
            "settled", "inbox", "working", "snoozed", "settled", "inbox", "working"
        ]
    );
    assert_eq!(report["questions"].as_array().unwrap().len(), 1);
    assert_eq!(report["questions"][0]["id"], "q2");
    assert_eq!(report["questions"][0]["unit"], "m08");
}

/// Scenario: 事件按来源行的顺序编号,卡的正文原样保存为一个字符串
#[test]
fn imported_events_keep_line_order_and_card_text() {
    let sb = Sandbox::new(BOARD);
    assert!(sb.import().status.success());
    let events = events(&sb.log());
    let seqs: Vec<u64> = events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, (1..=events.len() as u64).collect::<Vec<_>>());
    let lines: Vec<u64> = events
        .iter()
        .map(|e| e["data"]["line"].as_u64().unwrap())
        .collect();
    assert!(lines.windows(2).all(|w| w[0] <= w[1]), "{lines:?}");

    let first = &events[0];
    assert_eq!(first["kind"], "dispatched");
    assert_eq!(first["card"], 1);
    assert_eq!(first["unit"], "m07");
    assert_eq!(first["data"]["impl"], "octoscode");
    assert_eq!(
        first["data"]["review"],
        serde_json::json!(["codex", "claude"])
    );
    // The fenced example travels inside the text and is no event.
    let body = first["data"]["body"].as_str().unwrap();
    assert!(
        body.contains("### 99. 示例里的标题") && body.contains("正文一。"),
        "{body}"
    );
    assert!(events.iter().all(|e| e["card"] != 99 && e["card"] != 90));
    assert!(
        !events
            .iter()
            .any(|e| e["kind"] == "accepted" && e["card"] == 2),
        "a verdict inside a code fence is not a verdict"
    );
}

/// Scenario: 导入只做一次;dry-run 不写;黑板不被改动
#[test]
fn import_runs_once_and_never_touches_the_board() {
    let sb = Sandbox::new(BOARD);
    let out = olp(&[
        "board",
        "import",
        &sb.board(),
        "--to",
        &sb.log(),
        "--dry-run",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("dry-run"), "{}", stdout(&out));
    assert!(!Path::new(&sb.log()).exists(), "dry-run writes nothing");

    assert!(sb.import().status.success());
    let before = events(&sb.log()).len();
    let again = sb.import();
    assert_eq!(again.status.code(), Some(1));
    assert!(stderr(&again).contains("不重复导入"), "{}", stderr(&again));
    assert_eq!(events(&sb.log()).len(), before);
    assert_eq!(std::fs::read_to_string(sb.board()).unwrap(), BOARD);

    assert_eq!(
        olp(&["board", "import", &sb.board()]).status.code(),
        Some(2)
    );
    let missing = sb.dir.path().join("absent.md");
    let out = olp(&[
        "board",
        "import",
        missing.to_str().unwrap(),
        "--to",
        &sb.log(),
    ]);
    assert_eq!(out.status.code(), Some(2));
}

/// Scenario: 损坏的日志不被悄悄读成较短的日志
#[test]
fn inbox_refuses_a_corrupt_log() {
    let sb = Sandbox::new(BOARD);
    assert!(sb.import().status.success());
    let segment = PathBuf::from(sb.log()).join("events-000001.jsonl");
    let mut text = std::fs::read_to_string(&segment).unwrap();
    text.push_str("not json\n");
    std::fs::write(&segment, text).unwrap();
    let out = olp(&["board", "inbox", &sb.log(), "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("事件日志损坏"), "{}", stderr(&out));
    assert!(stdout(&out).is_empty());
}

/// Scenario: 仓库里冻结的真实黑板导入后结论不变
#[test]
fn the_frozen_campaign_board_survives_the_import() {
    let frozen = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/OUTER_LOOP_REVIEW.md");
    let sb = Sandbox::new(&std::fs::read_to_string(frozen).unwrap());
    assert!(sb.import().status.success());
    let from_log = sb.report(&sb.log(), &["--all"]);
    assert_eq!(from_log, sb.report(&sb.board(), &["--all"]));
    assert!(from_log["items"].as_array().unwrap().len() >= 20);
}
