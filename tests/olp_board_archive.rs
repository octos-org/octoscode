#![cfg(unix)]

//! Contract tests for `scripts/olp-board-archive.py` (SDD spec:
//! `specs/task-olp-board-archive.spec.md`): closed board entries move to an
//! archive file and leave a numbered stub behind.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tool() -> PathBuf {
    repo_root().join("scripts/olp-board-archive.py")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn archive(board: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("python3");
    cmd.arg("-B").arg(tool()).arg(board).args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn heading_numbers(text: &str) -> Vec<u64> {
    text.lines()
        .filter_map(|l| l.strip_prefix("### "))
        .filter_map(|rest| {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        })
        .collect()
}

const BOARD: &str = "\
# 外环审查通道

> 前言:外环只追加带日期的条目。

---

### 1. 第一条(2026-09-18,外环(claude))

背景与修法。

ACK(done): commit abc123,测试通过

> 外环(claude)·R2 记档(#1): 复验通过

### 2. 第二条(2026-09-19,外环(claude))

正文二。

ACK(blocked): 等上游

### 3. 第三条(2026-09-20,外环(gpt))

正文三。

ACK(wontdo): 与 #2 重复

## 小节标题

小节正文。

### 4. 第四条(2026-09-21,外环(gpt))

没有 ACK。

### 5. 第五条(2026-09-22,外环(gpt))

正文五。

ACK(done): commit def456
";

fn board_in(dir: &Path) -> PathBuf {
    let board = dir.join("OUTER_LOOP_REVIEW.md");
    std::fs::write(&board, BOARD).unwrap();
    board
}

/// Scenario: 已闭环条目移入归档并留编号桩
#[test]
fn board_archive_moves_closed_entries_and_leaves_numbered_stubs() {
    let dir = tempfile::tempdir().unwrap();
    let board = board_in(dir.path());
    let out = archive(&board, &["--keep", "0"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let listed = stdout(&out);
    for n in ["#1 ", "#3 ", "#5 "] {
        assert!(listed.contains(&format!("archived {n}")), "{listed}");
    }

    let text = read(&board);
    let expected = "\
# 外环审查通道

> 前言:外环只追加带日期的条目。

---

### 1. 第一条(2026-09-18,外环(claude))
ACK(done): commit abc123,测试通过
<!-- olp-archived: OUTER_LOOP_REVIEW.archive.md -->

### 2. 第二条(2026-09-19,外环(claude))

正文二。

ACK(blocked): 等上游

### 3. 第三条(2026-09-20,外环(gpt))
ACK(wontdo): 与 #2 重复
<!-- olp-archived: OUTER_LOOP_REVIEW.archive.md -->

## 小节标题

小节正文。

### 4. 第四条(2026-09-21,外环(gpt))

没有 ACK。

### 5. 第五条(2026-09-22,外环(gpt))
ACK(done): commit def456
<!-- olp-archived: OUTER_LOOP_REVIEW.archive.md -->

";
    assert_eq!(text, expected);
    // Numbers stay on the board, so the next number is still max + 1.
    assert_eq!(heading_numbers(&text), vec![1, 2, 3, 4, 5]);

    let archived = read(&dir.path().join("OUTER_LOOP_REVIEW.archive.md"));
    for needle in [
        "<!-- olp-archived-entry: 1 at ",
        "背景与修法。",
        "> 外环(claude)·R2 记档(#1): 复验通过",
        "<!-- olp-archived-entry: 3 at ",
        "正文三。",
        "<!-- olp-archived-entry: 5 at ",
        "正文五。",
    ] {
        assert!(
            archived.contains(needle),
            "archive lacks {needle:?}: {archived}"
        );
    }
    assert!(
        !archived.contains("正文二。"),
        "open entries stay on the board"
    );
    assert!(!archived.contains("没有 ACK。"));
}

/// Scenario: 最近的条目原样保留
#[test]
fn board_archive_keeps_the_most_recent_entries_intact() {
    let dir = tempfile::tempdir().unwrap();
    let board = board_in(dir.path());
    // The two most recent entries are #4 and #5; #5 is closed but protected.
    let out = archive(&board, &["--keep", "2"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = read(&board);
    assert!(text.contains("正文五。"), "{text}");
    assert!(!text.contains("背景与修法。"), "{text}");
    assert!(!text.contains("正文三。"), "{text}");

    // The default keeps ten: nothing on this five-entry board is archived.
    let dir = tempfile::tempdir().unwrap();
    let board = board_in(dir.path());
    let out = archive(&board, &[], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(read(&board), BOARD);
    assert!(!dir.path().join("OUTER_LOOP_REVIEW.archive.md").exists());
}

/// Scenario: 重跑幂等,中断后不重复归档
#[test]
fn board_archive_rerun_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let board = board_in(dir.path());
    let archive_path = dir.path().join("OUTER_LOOP_REVIEW.archive.md");
    // An interrupted earlier run: #1 reached the archive, the board was not
    // replaced.
    std::fs::write(
        &archive_path,
        "# 黑板归档\n\n<!-- olp-archived-entry: 1 at 2026-11-01T00:00:00Z -->\n### 1. 第一条\nearlier copy\n\n",
    )
    .unwrap();

    assert!(archive(&board, &["--keep", "0"], &[]).status.success());
    let first_board = read(&board);
    let first_archive = read(&archive_path);
    assert_eq!(
        first_archive.matches("<!-- olp-archived-entry: 1 ").count(),
        1,
        "{first_archive}"
    );
    assert!(first_archive.contains("earlier copy"));
    assert!(first_archive.contains("<!-- olp-archived-entry: 3 "));

    let out = archive(&board, &["--keep", "0"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("没有可归档的条目"),
        "{}",
        stdout(&out)
    );
    assert_eq!(read(&board), first_board);
    assert_eq!(read(&archive_path), first_archive);
}

/// Scenario: dry-run 不写任何文件
#[test]
fn board_archive_dry_run_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let board = board_in(dir.path());
    let out = archive(&board, &["--keep", "0", "--dry-run"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let listed = stdout(&out);
    assert!(listed.contains("would archive #1 "), "{listed}");
    assert!(listed.contains("would archive #3 "), "{listed}");
    assert!(!listed.contains("#2 "), "{listed}");
    assert_eq!(read(&board), BOARD);
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["OUTER_LOOP_REVIEW.md"], "no lock, no archive");
}

/// Scenario: 闭环判定只看围栏外的最后一条 ACK
#[test]
fn board_archive_closure_uses_last_ack_outside_code_fences() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    let text = "\
### 1. 围栏里的标题和 ACK 不算

```
### 99. 示例条目
ACK(done): 这是示例
```

### 2. 先 done 后 blocked

ACK(done): 初版
ACK(blocked): 复验打回后卡住

### 3. 旧格式

ACK: 旧格式不算闭环

### 4. 先 blocked 后 done

ACK(blocked): 等依赖
正文。
ACK(done): 依赖到位,commit 123
<!-- olp-key: m07:card-4 n=4 -->
";
    std::fs::write(&board, text).unwrap();
    let out = archive(&board, &["--keep", "0"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        stdout(&out).matches("archived #").count(),
        1,
        "{}",
        stdout(&out)
    );

    let after = read(&board);
    assert!(after.starts_with(
        "### 1. 围栏里的标题和 ACK 不算\n\n```\n### 99. 示例条目\nACK(done): 这是示例\n```\n\n### 2. 先 done 后 blocked\n"
    ));
    assert!(after.contains("ACK: 旧格式不算闭环"));
    // The stub keeps every ACK line of the entry, in order, and the
    // idempotency marker a keyed append left behind.
    assert!(
        after.ends_with(
            "### 4. 先 blocked 后 done\nACK(blocked): 等依赖\nACK(done): 依赖到位,commit 123\n<!-- olp-key: m07:card-4 n=4 -->\n<!-- olp-archived: board.archive.md -->\n\n"
        ),
        "{after}"
    );
}

/// Scenario: 多机部署的卡按外环的结论闭环,给机器读的行留在板上
#[test]
fn board_archive_follows_outer_verdicts_and_keeps_machine_lines() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    // Cluster-style cards have no ACK under the entry: the unit's push
    // record and the outer loop's verdict are appended at the end of the
    // board, physically under the last entry.
    let verdicts = "\
> 单元(m07)·本地复验通过(#1): task/1 @ aaaaaaaaaaaa 复验 m07-ver
> 外环(hub)·采认(#1): 通过
> 单元(m08)·本地复验通过(#2): task/2 @ bbbbbbbbbbbb 复验 m08-ver
> 外环(hub)·采认(#2): 通过
> 单元(m08)·本地复验通过(#2): task/2 @ cccccccccccc 复验 m08-ver
> 内环(m07)·#5 ACK(done): commit x
> 外环(hub)·打回(#5): 缺测试
";
    let text = format!(
        "\
### 1. 已采认
unit: m07

正文一。
<!-- olp-key: hub:card-1 n=1 -->

### 2. 采认之后单元又推送
unit: m08

正文二。

### 3. 被作废

正文三。

### 4. 改判作废 #3
> 外环(hub)·改判(作废 #3): 以本条为准

正文四。

### 5. 被打回
unit: m07

正文五。

{verdicts}"
    );
    std::fs::write(&board, &text).unwrap();
    let out = archive(&board, &["--keep", "0"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let listed = stdout(&out);
    for n in ["#1 ", "#3 ", "#5 "] {
        assert!(listed.contains(&format!("archived {n}")), "{listed}");
    }
    assert_eq!(listed.matches("archived #").count(), 3, "{listed}");

    let after = read(&board);
    let expected = format!(
        "\
### 1. 已采认
unit: m07
<!-- olp-key: hub:card-1 n=1 -->
<!-- olp-archived: board.archive.md -->

### 2. 采认之后单元又推送
unit: m08

正文二。

### 3. 被作废
<!-- olp-archived: board.archive.md -->

### 4. 改判作废 #3
> 外环(hub)·改判(作废 #3): 以本条为准

正文四。

### 5. 被打回
unit: m07
{verdicts}<!-- olp-archived: board.archive.md -->

"
    );
    assert_eq!(after, expected);
    let archived = read(&dir.path().join("board.archive.md"));
    for body in ["正文一。", "正文三。", "正文五。"] {
        assert!(archived.contains(body), "{archived}");
    }
}

/// Scenario: 归档期间板被改动则不替换
#[test]
fn board_archive_aborts_when_board_changes_underneath() {
    let dir = tempfile::tempdir().unwrap();
    let board = board_in(dir.path());
    let late_ack = "ACK(done): 内环在归档期间原地写入\n";
    let out = archive(
        &board,
        &["--keep", "0"],
        &[("OLP_BOARD_ARCHIVE_TEST_CONCURRENT_EDIT", late_ack)],
    );
    assert_eq!(out.status.code(), Some(4), "stderr: {}", stderr(&out));
    // The concurrent write survives and nothing was stubbed.
    assert_eq!(read(&board), format!("{BOARD}{late_ack}"));
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".olp-archive-"))
        .collect();
    assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");

    // The rerun completes without duplicating what already reached the archive.
    let out = archive(&board, &["--keep", "0"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let archived = read(&dir.path().join("OUTER_LOOP_REVIEW.archive.md"));
    assert_eq!(archived.matches("<!-- olp-archived-entry: 1 ").count(), 1);
    assert!(read(&board).contains(late_ack));
}

/// Scenario: 与写板助手共用一把锁
#[test]
fn board_archive_serialises_with_board_append() {
    let has_flock = Command::new("sh")
        .arg("-c")
        .arg("command -v flock >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !has_flock {
        eprintln!("SKIP (explicit): host has no flock on PATH; olp-board-append.sh needs it");
        return;
    }
    const WRITERS: usize = 30;
    let dir = tempfile::tempdir().unwrap();
    let board = board_in(dir.path());

    let appenders: Vec<_> = (0..WRITERS)
        .map(|i| {
            let board = board.clone();
            std::thread::spawn(move || {
                let mut child = Command::new("bash")
                    .arg(repo_root().join("scripts/olp-board-append.sh"))
                    .arg(&board)
                    .stdin(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(format!("### {}. concurrent-{i}\n", 100 + i).as_bytes())
                    .unwrap();
                assert!(child.wait().unwrap().success());
            })
        })
        .collect();
    // The appends are new open entries: a bare line would belong to the
    // last entry on the board and rightly move to the archive with it.
    // Archive repeatedly while the appends land. An archive pass may lose the
    // race to an append that slipped in before it took the lock — that is
    // exit 0 either way; exit 4 would mean an append landed INSIDE the pass.
    for _ in 0..5 {
        let out = archive(&board, &["--keep", "0"], &[]);
        assert!(out.status.success(), "stderr: {}", stderr(&out));
    }
    for h in appenders {
        h.join().unwrap();
    }

    let text = read(&board);
    for i in 0..WRITERS {
        assert_eq!(
            text.matches(&format!("concurrent-{i}\n")).count(),
            1,
            "append {i} must survive exactly once"
        );
    }
    assert!(text.contains("<!-- olp-archived: OUTER_LOOP_REVIEW.archive.md -->"));
}

/// Scenario: 用法错误不触碰任何文件
#[test]
fn board_archive_rejects_bad_usage() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("absent.md");
    let out = archive(&missing, &[], &[]);
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));

    let board = board_in(dir.path());
    for args in [
        vec!["--keep", "-1"],
        vec!["--archive", board.to_str().unwrap(), "--keep", "0"],
        vec!["--no-such-flag"],
    ] {
        let out = archive(&board, &args, &[]);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    assert_eq!(read(&board), BOARD);
    assert!(!dir.path().join("OUTER_LOOP_REVIEW.md.lock").exists());
    assert!(!dir.path().join("OUTER_LOOP_REVIEW.archive.md").exists());
}
