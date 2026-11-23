#![cfg(unix)]

//! Contract tests for `octoscode olp board append` and `olp id alloc` in
//! Rust (SDD spec: `specs/task-olp-board-alloc.spec.md`): the same rules
//! as the scripts they replace, without needing a `flock` program, and
//! excluding those scripts through the same lock.

#[path = "support/olp_env.rs"]
mod olp_env;

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_octoscode");

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn feed(mut cmd: Command, body: &str) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(body.as_bytes());
    child.wait_with_output().unwrap()
}

fn append(board: &Path, args: &[&str], body: &str) -> Output {
    let mut cmd = Command::new(BIN);
    // No flock program on this PATH: the Rust command must not need one.
    cmd.args(["olp", "board", "append"])
        .args(args)
        .arg(board)
        .env("PATH", "/nonexistent");
    feed(cmd, body)
}

fn id_alloc(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(["olp", "id", "alloc"])
        .args(args)
        .env("PATH", "/nonexistent")
        .output()
        .unwrap()
}

fn headings(board: &Path) -> Vec<u64> {
    std::fs::read_to_string(board)
        .unwrap()
        .lines()
        .filter_map(|l| l.strip_prefix("### "))
        .filter_map(|rest| {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        })
        .collect()
}

/// Scenario: Rust 版在锁内分配编号、按键去重、旧写法原样追加
#[test]
fn native_append_allocates_dedups_and_keeps_legacy_entries() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    std::fs::write(&board, "# 板\n\n### 41. 旧条目\nACK: 末行不是空行").unwrap();

    let out = append(&board, &[], "### {N}. 新条目\n见 #{N} 的分支 task/{N}\n");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "42");
    let text = std::fs::read_to_string(&board).unwrap();
    assert!(
        text.contains("ACK: 末行不是空行\n\n### 42. 新条目\n见 #42 的分支 task/42\n"),
        "{text}"
    );

    // A key: the resend adds nothing and returns the original number.
    let keyed = |body: &str| append(&board, &["--key", "dispatch:m07:abc"], body);
    let first = keyed("### {N}. 带键的条目\n正文");
    assert_eq!(stdout(&first).trim(), "43");
    let again = keyed("### {N}. 带键的条目\n正文");
    assert_eq!(again.status.code(), Some(0));
    assert_eq!(stdout(&again).trim(), "43");
    assert!(
        stderr(&again).contains("已在板上(#43)"),
        "{}",
        stderr(&again)
    );
    let text = std::fs::read_to_string(&board).unwrap();
    assert_eq!(text.matches("带键的条目").count(), 1);
    assert!(
        text.contains("正文\n<!-- olp-key: dispatch:m07:abc n=43 -->\n"),
        "{text}"
    );

    // A keyed entry without a number, then again.
    let out = append(&board, &["--key", "note:1"], "> 外环(hub)·一行意见\n");
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), ""));
    let out = append(&board, &["--key", "note:1"], "> 外环(hub)·一行意见\n");
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), ""));
    let text = std::fs::read_to_string(&board).unwrap();
    assert_eq!(text.matches("一行意见").count(), 1);

    // The old way: appended as it is, `{N}` elsewhere is not a placeholder.
    let before = std::fs::read_to_string(&board).unwrap();
    let out = append(&board, &[], "> 采集哨:原文里有 {N} 三个字符\n");
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), ""));
    assert_eq!(
        std::fs::read_to_string(&board).unwrap(),
        format!("{before}> 采集哨:原文里有 {{N}} 三个字符\n")
    );
    assert_eq!(headings(&board), vec![41, 42, 43]);

    // Refused before anything is touched.
    let fresh = dir.path().join("fresh.md");
    for (args, body) in [
        (vec!["--key", "有 空格"], "### {N}. x\n"),
        (vec!["--key", ""], "### {N}. x\n"),
        (vec!["--key", "k"], ""),
        (vec!["--bogus"], "x\n"),
    ] {
        let out = append(&fresh, &args, body);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    }
    assert!(!fresh.exists());
    assert!(!dir.path().join("fresh.md.lock").exists());
    // An empty entry without a key is nothing to do.
    assert_eq!(append(&fresh, &[], "").status.code(), Some(0));
    assert!(!fresh.exists());
}

/// Scenario: Rust 版与脚本并发写同一块板,编号不重复、条目不交错
#[test]
fn native_and_script_writers_share_one_lock() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    std::fs::write(&board, "# 板\n").unwrap();
    let flock = ["/usr/bin", "/bin", "/opt/homebrew/bin", "/usr/local/bin"]
        .iter()
        .any(|d| Path::new(d).join("flock").exists());
    if !flock {
        eprintln!("NOTE: flock not installed; only the Rust writers run");
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/olp-board-append.sh");
    let workers: Vec<_> = (0..16)
        .map(|i| {
            let (board, script) = (board.clone(), script.clone());
            std::thread::spawn(move || {
                let body = format!("### {{N}}. 写入方 {i}\n第一行 {i}\n第二行 {i}\n");
                let out = if flock && i % 2 == 1 {
                    let mut cmd = Command::new("/bin/bash");
                    cmd.arg(script).arg(&board);
                    feed(cmd, &body)
                } else {
                    append(&board, &[], &body)
                };
                assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
                stdout(&out).trim().parse::<u64>().unwrap()
            })
        })
        .collect();
    let numbers: BTreeSet<u64> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(numbers, (1..=16).collect::<BTreeSet<u64>>());
    let mut seen = headings(&board);
    seen.sort_unstable();
    assert_eq!(seen, (1..=16).collect::<Vec<u64>>());
    // Every entry is whole: its two lines follow its heading.
    let text = std::fs::read_to_string(&board).unwrap();
    for i in 0..16 {
        assert!(
            text.contains(&format!("写入方 {i}\n第一行 {i}\n第二行 {i}\n")),
            "entry {i} was interleaved:\n{text}"
        );
    }
}

/// Scenario: Rust 版按自然键分配编号,并发不重复
#[test]
fn native_id_alloc_is_stable_per_key_and_unique_under_load() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("ids.tsv");
    let reg = registry.to_str().unwrap();
    let id = |prefix: &str, width: &str, key: &str| {
        let out = id_alloc(&[reg, prefix, width, key]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        stdout(&out).trim().to_string()
    };
    assert_eq!(
        id("API-SS-", "5", "ohos.ability.startAbility"),
        "API-SS-00001"
    );
    assert_eq!(id("API-SS-", "5", "ohos.ability.stop"), "API-SS-00002");
    assert_eq!(
        id("API-SS-", "5", "ohos.ability.startAbility"),
        "API-SS-00001"
    );
    assert_eq!(id("W", "3", "ohos.ability.startAbility"), "W001");
    assert_eq!(id("API-SS-", "5", r"带\t反斜杠的键"), "API-SS-00003");
    let rows = std::fs::read_to_string(&registry).unwrap();
    assert_eq!(rows.lines().count(), 4, "{rows}");
    assert!(rows.lines().all(|l| l.split('\t').count() == 4), "{rows}");

    let workers: Vec<_> = (0..12)
        .map(|i| {
            let reg = reg.to_string();
            std::thread::spawn(move || {
                let key = format!("key-{}", i % 6);
                stdout(&id_alloc(&[&reg, "APP-", "4", &key]))
                    .trim()
                    .to_string()
            })
        })
        .collect();
    let ids: BTreeSet<String> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    let wanted: BTreeSet<String> = (1..=6).map(|n| format!("APP-{n:04}")).collect();
    assert_eq!(
        ids, wanted,
        "six keys, six ids, each key always the same one"
    );

    let fresh = dir.path().join("fresh.tsv");
    let f = fresh.to_str().unwrap();
    for args in [
        vec![f, "9X-", "5", "k"],
        vec![f, "A B", "5", "k"],
        vec![f, "A-", "0", "k"],
        vec![f, "A-", "13", "k"],
        vec![f, "A-", "x", "k"],
        vec![f, "A-", "5", ""],
        vec![f, "A-", "5", "有\t制表符"],
        vec![f, "A-", "5"],
    ] {
        assert_eq!(id_alloc(&args).status.code(), Some(2), "{args:?}");
    }
    assert!(!fresh.exists() && !dir.path().join("fresh.tsv.lock").exists());
}
