#![cfg(unix)]

//! Contract tests for `scripts/olp-inbox.py` (SDD spec:
//! `specs/task-olp-board-inbox.spec.md`): the board's entries sorted into
//! inbox / working / snoozed / settled, derived from line-start forms only.

use std::path::PathBuf;
use std::process::{Command, Output};

fn tool() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-inbox.py")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Board {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn board(text: &str) -> Board {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("OUTER_LOOP_REVIEW.md");
    std::fs::write(&path, text).unwrap();
    Board { _dir: dir, path }
}

impl Board {
    fn run(&self, args: &[&str]) -> Output {
        Command::new("python3")
            .arg("-B")
            .arg(tool())
            .arg(&self.path)
            .args(args)
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut full = vec!["--json", "--all"];
        full.extend_from_slice(args);
        let out = self.run(&full);
        assert!(out.status.success(), "stderr: {}", stderr(&out));
        serde_json::from_str(&stdout(&out)).unwrap()
    }

    /// (partition, reason) of entry `number`.
    fn state(&self, number: u64, args: &[&str]) -> (String, String) {
        let report = self.json(args);
        let item = report["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["number"] == number)
            .unwrap_or_else(|| panic!("#{number} missing from {report}"));
        (
            item["partition"].as_str().unwrap().to_string(),
            item["reason"].as_str().unwrap().to_string(),
        )
    }
}

const TODAY: [&str; 2] = ["--today", "2026-11-12"];

/// Scenario: 每个条目按最后发生的事归入分区
#[test]
fn inbox_sorts_entries_by_their_latest_event() {
    let b = board(
        "\
# 外环审查通道

### 1. 已采认的卡(2026-11-01,外环(hub))
unit: m01
ACK(done): commit a1

### 2. 等复验的卡
unit: m02
ACK(done): commit b2

### 3. 还在做的卡
unit: m01

### 4. 阻塞的卡
ACK(blocked): 等上游

### 5. 内环拒做的卡
ACK(wontdo): 与 #2 重复

### 6. 被打回的卡
ACK(done): commit c3

### 7. 已推送的卡
unit: m03

### 8. 被作废的卡
ACK(done): commit d4

### 9. 改判作废 #8
> 外环(hub)·改判(作废 #8): 以本条为准

### 10. 已归档的卡
ACK(done): commit e5
<!-- olp-archived: OUTER_LOOP_REVIEW.archive.md -->

> 外环(hub)·采认(#1): 复验通过,证据 sha256:9f3a
> 外环(hub)·打回(#6): 测试没跑全,整改见 #11
> 单元(m03)·本地复验通过(#7): task/7 @ 3f2a9c1d4e5f 复验 m03-ver
",
    );
    let expect = [
        (1, "settled", "已采认"),
        (2, "inbox", "待复验"),
        (3, "working", "等内环 ACK"),
        (4, "inbox", "待解阻塞"),
        (5, "inbox", "待回应"),
        (6, "settled", "已打回"),
        (7, "inbox", "待审核"),
        (8, "settled", "改判作废"),
        (9, "working", "等内环 ACK"),
        (10, "settled", "已归档"),
    ];
    for (number, partition, needle) in expect {
        let (p, reason) = b.state(number, &TODAY);
        assert_eq!(p, partition, "#{number}: {reason}");
        assert!(reason.contains(needle), "#{number}: {reason}");
    }
    let report = b.json(&TODAY);
    assert_eq!(report["counts"]["inbox"], 4);
    assert_eq!(report["counts"]["working"], 2);
    assert_eq!(report["counts"]["snoozed"], 0);
    assert_eq!(report["counts"]["settled"], 4);
    assert_eq!(report["warnings"].as_array().unwrap().len(), 0, "{report}");

    // Without --all the settled entries are counted but not listed.
    let out = b.run(&["--json", "--today", "2026-11-12"]);
    let brief: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(brief["items"].as_array().unwrap().len(), 6);
    assert_eq!(brief["counts"]["settled"], 4);

    // --unit narrows to the cards dispatched to one unit.
    let only_m01 = b.json(&["--unit", "m01", "--today", "2026-11-12"]);
    let numbers: Vec<u64> = only_m01["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, vec![1, 3]);
}

/// Scenario: 判词之后的新动作让条目回到收件箱
#[test]
fn inbox_reopens_when_something_happens_after_a_verdict() {
    // #1: pushed, returned, then pushed again after rework.
    // #2: accepted, and nothing since.
    // #3: ACK(blocked) then ACK(done) inside the entry: the last ACK counts.
    let b = board(
        "\
### 1. 返工后重新推送
unit: m01

### 2. 采认之后没有新动作
ACK(done): commit a

### 3. 先阻塞后完成
ACK(blocked): 等依赖
ACK(done): 依赖到位

> 单元(m01)·本地复验通过(#1): task/1 @ aaaaaaaaaaaa 复验 m01-ver
> 外环(hub)·打回(#1): 缺测试
> 外环(hub)·采认(#2): 通过
> 单元(m01)·本地复验通过(#1): task/1 @ bbbbbbbbbbbb 复验 m01-ver
",
    );
    let (p, reason) = b.state(1, &TODAY);
    assert_eq!(p, "inbox", "{reason}");
    assert!(reason.contains("待审核"), "{reason}");
    assert_eq!(b.state(2, &TODAY).0, "settled");
    let (p, reason) = b.state(3, &TODAY);
    assert_eq!(p, "inbox");
    assert!(reason.contains("待复验"), "{reason}");
}

/// Scenario: 搁置到期前不进收件箱,到期或有新动作后回来
#[test]
fn inbox_snooze_hides_until_the_date_or_new_activity() {
    let b = board(
        "\
### 1. 等上游修复
ACK(blocked): 等 octos 修复

### 2. 搁置之后内环又 ACK 了
ACK(blocked): 等依赖
ACK(done): 依赖到位了

### 3. 还没开始就搁置
unit: m01

### 4. 搁置行写坏了
ACK(blocked): 等

> 外环(hub)·搁置(#1) 至 2026-12-01: 等上游发版
> 外环(hub)·搁置(#3) 至 2026-11-20: 等设备到货
> 外环(hub)·搁置(#4) 至 下周: 日期不合规
",
    );
    // The snooze for #2 sits BEFORE its last ACK: write it into the entry.
    let text = std::fs::read_to_string(&b.path).unwrap().replace(
        "ACK(blocked): 等依赖\nACK(done): 依赖到位了",
        "ACK(blocked): 等依赖\n> 外环(hub)·搁置(#2) 至 2026-12-31: 等依赖\nACK(done): 依赖到位了",
    );
    std::fs::write(&b.path, text).unwrap();

    let (p, reason) = b.state(1, &TODAY);
    assert_eq!(p, "snoozed");
    assert!(reason.contains("2026-12-01"), "{reason}");
    assert_eq!(b.state(3, &TODAY).0, "snoozed");

    // New activity after the snooze line cancels it.
    let (p, reason) = b.state(2, &TODAY);
    assert_eq!(p, "inbox", "{reason}");
    assert!(reason.contains("待复验"), "{reason}");

    // An unparseable date is ignored and reported.
    assert_eq!(b.state(4, &TODAY).0, "inbox");
    let report = b.json(&TODAY);
    let warnings = report["warnings"].to_string();
    assert!(
        warnings.contains("#4") && warnings.contains("搁置"),
        "{warnings}"
    );

    // On the wake date the entries come back, saying why.
    let (p, reason) = b.state(1, &["--today", "2026-12-01"]);
    assert_eq!(p, "inbox");
    assert!(
        reason.contains("待解阻塞") && reason.contains("到期"),
        "{reason}"
    );
    let (p, reason) = b.state(3, &["--today", "2026-11-20"]);
    assert_eq!(p, "inbox");
    assert!(reason.contains("搁置到期"), "{reason}");
    assert_eq!(b.state(1, &["--today", "2026-11-30"]).0, "snoozed");
}

/// Scenario: 只认行首定式,代码围栏与正文提及不算
#[test]
fn inbox_only_honours_line_start_forms_outside_code_fences() {
    let b = board(
        "\
### 1. 文档条目
unit: m01

示例(不是真的判词):

```
> 外环(hub)·采认(#1): 示例
ACK(done): 示例
### 99. 示例条目
```

正文里提到 外环(hub)·采认(#1) 不算,行中间的 ACK(done): 也不算。

### 2. 带引用前缀和加粗的判词
ACK(done): commit a

> > **外环(hub)·采认(#2): 多一层引用和加粗也认**
",
    );
    let (p, reason) = b.state(1, &TODAY);
    assert_eq!(p, "working", "{reason}");
    assert_eq!(b.state(2, &TODAY).0, "settled");
    let report = b.json(&TODAY);
    assert_eq!(report["items"].as_array().unwrap().len(), 2, "no #99 entry");
}

/// Scenario: 内环的署名写法也算 ACK
#[test]
fn inbox_accepts_signed_inner_ack_lines() {
    // The shapes found on this repository's live board.
    let b = board(
        "\
### 4. 带条目号的署名 ACK 落在别的条目下面

### 6. 没有 ACK 的条目

> 内环(octoscode)·#4 ACK(done)(2026-09-25): commit 1796302
> 内环(octoscode)·#4 checkpoint(2026-09-25): 这不是 ACK

### 7. 署名 ACK 在自己的条目里
> 内环(octoscode)·ACK(done): #7(2026-09-25): commit 1a6c142

### 8. 署名 ACK 不带条目号
> 内环(octoscode)·ACK(blocked): 等上游
",
    );
    let (p, reason) = b.state(4, &TODAY);
    assert_eq!(p, "inbox", "{reason}");
    assert!(reason.contains("待复验"), "{reason}");
    assert_eq!(
        b.state(6, &TODAY).0,
        "working",
        "the ACK for #4 must not land on #6"
    );
    assert_eq!(b.state(7, &TODAY).0, "inbox");
    let (p, reason) = b.state(8, &TODAY);
    assert_eq!(p, "inbox");
    assert!(reason.contains("待解阻塞"), "{reason}");
}

/// Scenario: 报告黑板的结构问题
#[test]
fn inbox_reports_board_problems_as_warnings() {
    let b = board(
        "\
### 1. 第一条
ACK(done): a

### 2. 第二条

### 1. 撞号的条目
ACK(blocked): 这条不算

> 外环(hub)·采认(#7): 板上没有 #7
",
    );
    let report = b.json(&TODAY);
    let warnings = report["warnings"].to_string();
    assert!(warnings.contains("#1 出现两次"), "{warnings}");
    assert!(
        warnings.contains("#7") && warnings.contains("没有这个条目"),
        "{warnings}"
    );
    // The first occurrence wins; the duplicate's ACK does not leak into it.
    let (p, reason) = b.state(1, &TODAY);
    assert_eq!(p, "inbox");
    assert!(reason.contains("待复验"), "{reason}");

    // The text view prints the same partitions and sends warnings to stderr.
    let out = b.run(&["--today", "2026-11-12"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(
        text.contains("== 收件箱(1)") && text.contains("== 进行中(1)"),
        "{text}"
    );
    assert!(text.contains("== 已了结(0),--all 查看"), "{text}");
    assert!(stderr(&out).contains("warning:"), "{}", stderr(&out));
}

/// Scenario: 用法错误
#[test]
fn inbox_rejects_bad_usage() {
    let b = board("### 1. x\n");
    assert_eq!(b.run(&["--today", "not-a-date"]).status.code(), Some(2));
    assert_eq!(b.run(&["--bogus"]).status.code(), Some(2));
    let missing = Command::new("python3")
        .arg("-B")
        .arg(tool())
        .arg(b.path.with_file_name("absent.md"))
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    // Reading the board never modifies it.
    assert_eq!(std::fs::read_to_string(&b.path).unwrap(), "### 1. x\n");
}
