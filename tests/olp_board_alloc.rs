#![cfg(unix)]

//! Contract tests for number allocation on the OLP board and the shared
//! tool preflight (SDD spec: `specs/task-olp-board-alloc.spec.md`).
//!
//! Drives `scripts/olp-board-append.sh`, `scripts/olp-id-alloc.sh` and the
//! preflight wiring in `scripts/olp-evo-retro.sh` /
//! `scripts/olp-evo-spec-skeleton.sh` through real subprocesses in fresh
//! temp dirs. Scripts run under `/bin/bash` (Bash 3.2 on macOS) so the
//! tests do not depend on which bash the caller's PATH resolves.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const MINIMAL_PATH: &str = "/usr/bin:/bin";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn script(name: &str) -> PathBuf {
    repo_root().join("scripts").join(name)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Run `olp-board-append.sh` with `body` on stdin.
fn append(board: &Path, args: &[&str], body: &str, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("/bin/bash");
    cmd.arg(script("olp-board-append.sh"))
        .args(args)
        .arg(board)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    // The script may reject its arguments and exit before reading stdin; the
    // write then fails with EPIPE, which is not the test's concern.
    let _ = child.stdin.take().unwrap().write_all(body.as_bytes());
    child.wait_with_output().unwrap()
}

fn id_alloc(registry: &Path, prefix: &str, width: &str, key: &str) -> Output {
    Command::new("/bin/bash")
        .arg(script("olp-id-alloc.sh"))
        .arg(registry)
        .arg(prefix)
        .arg(width)
        .arg(key)
        .output()
        .unwrap()
}

/// The numbers of every `### <n>` heading on the board, in file order.
fn heading_numbers(board: &Path) -> Vec<u64> {
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

/// True when `flock` is reachable from the minimal PATH, directly or through
/// the Homebrew directories the scripts discover on macOS.
fn host_has_flock() -> bool {
    ["/usr/bin", "/bin", "/opt/homebrew/bin", "/usr/local/bin"]
        .iter()
        .any(|d| Path::new(d).join("flock").exists())
}

/// A PATH directory holding only `dirname` and `uname`: enough for a script
/// to locate its siblings, with every capability the preflight checks absent.
fn sealed_path(dir: &Path) -> PathBuf {
    let bin = dir.join("sealed-bin");
    std::fs::create_dir_all(&bin).unwrap();
    for tool in ["dirname", "uname"] {
        let src = ["/usr/bin", "/bin"]
            .iter()
            .map(|d| Path::new(d).join(tool))
            .find(|p| p.exists())
            .unwrap_or_else(|| panic!("host has no {tool}"));
        std::os::unix::fs::symlink(src, bin.join(tool)).unwrap();
    }
    bin
}

/// Scenario: 锁内分配下一个编号
#[test]
fn board_append_allocates_next_number_under_lock() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("OUTER_LOOP_REVIEW.md");
    // Out-of-order headings and a missing trailing newline: the next number
    // is max + 1, and the new entry must still start at a line head.
    std::fs::write(&board, "### 19. later\nbody\n### 18. earlier\nbody").unwrap();

    let out = append(
        &board,
        &[],
        "### {N}. 标题(2026-11-12,外环)\n分支 task/{N},见 #{N}\n",
        &[],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "20");

    let text = std::fs::read_to_string(&board).unwrap();
    assert!(
        text.ends_with("body\n\n### 20. 标题(2026-11-12,外环)\n分支 task/20,见 #20\n"),
        "{text}"
    );
    assert!(!text.contains("{N}"), "{text}");

    // A board that does not exist yet starts at 1.
    let fresh = dir.path().join("fresh.md");
    let out = append(&fresh, &[], "### {N}. first\n", &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "1");
    assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "### 1. first\n");
}

/// Scenario: 标题不贴着上一条的末行
#[test]
fn board_append_separates_a_heading_from_a_non_blank_last_line() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("OUTER_LOOP_REVIEW.md");
    // The template olp-init.sh lays down ends with a bare `ACK:` line.
    std::fs::write(&board, "### 1. 黑板启用\n\nACK:\n").unwrap();

    let out = append(&board, &[], "### {N}. 第二张\nunit: m01\n", &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "2");
    let text = std::fs::read_to_string(&board).unwrap();
    assert_eq!(
        text, "### 1. 黑板启用\n\nACK:\n\n### 2. 第二张\nunit: m01\n",
        "one blank line before the new heading"
    );

    // Lines that are not headings still attach directly: a verdict or an ACK
    // appended at the end belongs right under what is there.
    let out = append(&board, &[], "> 外环(hub)·采认(#2): 通过\n", &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = std::fs::read_to_string(&board).unwrap();
    assert!(
        text.ends_with("unit: m01\n> 外环(hub)·采认(#2): 通过\n"),
        "{text}"
    );

    // A board that already ends with a blank line gets no second one.
    std::fs::write(&board, "### 1. a\n\n").unwrap();
    let out = append(&board, &[], "### {N}. b\n", &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(&board).unwrap(),
        "### 1. a\n\n### 2. b\n"
    );
}

/// Scenario: 100 个并发写入方无重号、无交错
#[test]
fn board_append_concurrent_writers_get_unique_numbers_and_intact_entries() {
    const WRITERS: usize = 100;
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");

    let handles: Vec<_> = (0..WRITERS)
        .map(|i| {
            let board = board.clone();
            std::thread::spawn(move || {
                let body =
                    format!("### {{N}}. writer-{i}\nline-a writer-{i} #{{N}}\nline-b writer-{i}\n");
                let out = append(&board, &[], &body, &[]);
                assert!(out.status.success(), "stderr: {}", stderr(&out));
                stdout(&out).trim().parse::<u64>().unwrap()
            })
        })
        .collect();
    let returned: BTreeSet<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let expected: BTreeSet<u64> = (1..=WRITERS as u64).collect();
    assert_eq!(returned, expected, "every writer gets its own number");
    let on_board = heading_numbers(&board);
    assert_eq!(on_board.len(), WRITERS);
    assert_eq!(on_board.iter().copied().collect::<BTreeSet<_>>(), expected);

    // No interleaving: each entry is exactly its own three lines, in order,
    // and the number inside the body is the number in the heading.
    let text = std::fs::read_to_string(&board).unwrap();
    // Entries are separated by one blank line.
    assert_eq!(text.matches("\n\n").count(), WRITERS - 1, "{text}");
    assert!(!text.contains("\n\n\n"), "{text}");
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), WRITERS * 3, "{text}");
    for entry in lines.chunks(3) {
        let (n, who) = entry[0]
            .strip_prefix("### ")
            .and_then(|r| r.split_once(". "))
            .unwrap_or_else(|| panic!("bad heading: {}", entry[0]));
        assert_eq!(entry[1], format!("line-a {who} #{n}"));
        assert_eq!(entry[2], format!("line-b {who}"));
    }
}

/// Scenario: 同一自然键重发返回原编号
#[test]
fn board_append_same_key_returns_original_number_without_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    std::fs::write(&board, "### 4. existing\n").unwrap();

    let first = append(
        &board,
        &["--key", "m07:card-0042"],
        "### {N}. dispatched\nbody\n",
        &[],
    );
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert_eq!(stdout(&first).trim(), "5");
    let after_first = std::fs::read_to_string(&board).unwrap();
    assert!(
        after_first.ends_with("### 5. dispatched\nbody\n<!-- olp-key: m07:card-0042 n=5 -->\n"),
        "{after_first}"
    );

    // Resend (different body, same key) while other entries were added.
    assert!(
        append(&board, &[], "### {N}. other\n", &[])
            .status
            .success()
    );
    let resend = append(
        &board,
        &["--key", "m07:card-0042"],
        "### {N}. dispatched (resent)\n",
        &[],
    );
    assert!(resend.status.success(), "stderr: {}", stderr(&resend));
    assert_eq!(stdout(&resend).trim(), "5", "original number is returned");
    assert!(stderr(&resend).contains("m07:card-0042"));
    assert_eq!(heading_numbers(&board), vec![4, 5, 6]);

    // A key that is a prefix of an existing key is a different key.
    let prefix = append(&board, &["--key", "m07:card-004"], "### {N}. prefix\n", &[]);
    assert_eq!(stdout(&prefix).trim(), "7");

    // Concurrent resends of one new key: exactly one entry lands.
    let handles: Vec<_> = (0..20)
        .map(|_| {
            let board = board.clone();
            std::thread::spawn(move || {
                let out = append(&board, &["--key", "replay:1"], "### {N}. replayed\n", &[]);
                assert!(out.status.success(), "stderr: {}", stderr(&out));
                stdout(&out).trim().to_string()
            })
        })
        .collect();
    let numbers: BTreeSet<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(numbers, BTreeSet::from(["8".to_string()]));
    assert_eq!(heading_numbers(&board), vec![4, 5, 6, 7, 8]);
}

/// Scenario: 不带编号的条目按键去重
#[test]
fn board_append_key_dedups_entries_without_a_number() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    std::fs::write(&board, "### 1. task\n").unwrap();
    let verdict = "> 外环(hub)·R2 记档(#1): 采认 rid=c07:652b evidence=sha256:9f3a\n";

    for round in 0..2 {
        let out = append(&board, &["--key", "verdict:1:c07"], verdict, &[]);
        assert!(out.status.success(), "stderr: {}", stderr(&out));
        assert_eq!(stdout(&out), "", "round {round}: no number to report");
    }
    assert_eq!(
        std::fs::read_to_string(&board).unwrap(),
        format!("### 1. task\n{verdict}<!-- olp-key: verdict:1:c07 -->\n")
    );
}

/// Scenario: 旧写法原样追加
#[test]
fn board_append_legacy_body_is_appended_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    std::fs::write(&board, "### 1. old\n").unwrap();

    let out = append(&board, &[], "### 2. hand-numbered\nbody\n", &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "", "legacy mode prints nothing");
    assert_eq!(
        std::fs::read_to_string(&board).unwrap(),
        "### 1. old\n### 2. hand-numbered\nbody\n"
    );

    // `{N}` anywhere but at the start of the first line is ordinary text —
    // e.g. a harvest card quoting a board line. It is not substituted and
    // does not make the append fail.
    let quoted = "### EVO-0007（t，harvest）\nsymptom: 条目写了 ### {N}. 但没换号\n";
    let out = append(&board, &[], quoted, &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(std::fs::read_to_string(&board).unwrap().ends_with(quoted));
    std::fs::write(&board, "### 1. old\n### 2. hand-numbered\nbody\n").unwrap();

    // An empty body is a no-op, as before.
    let out = append(&board, &[], "", &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(&board).unwrap(),
        "### 1. old\n### 2. hand-numbered\nbody\n"
    );
}

/// Scenario: 非法输入在触碰黑板前被拒绝
#[test]
fn board_append_rejects_invalid_input_before_touching_board() {
    let dir = tempfile::tempdir().unwrap();
    let cases: [(&[&str], &str, &str); 4] = [
        (&["--key", "has space"], "### {N}. x\n", "--key"),
        (&["--key", "a-->b"], "### {N}. x\n", "--key"),
        (&["--key", ""], "### {N}. x\n", "--key"),
        (&["--key", "k1"], "", "正文为空"),
    ];
    for (i, (args, body, needle)) in cases.iter().enumerate() {
        let board = dir.path().join(format!("board-{i}.md"));
        let out = append(&board, args, body, &[]);
        assert_eq!(out.status.code(), Some(2), "case {i}: {}", stderr(&out));
        assert!(stderr(&out).contains(needle), "case {i}: {}", stderr(&out));
        assert_eq!(stdout(&out), "", "case {i}");
        assert!(!board.exists(), "case {i}: board must not be created");
        assert!(
            !dir.path().join(format!("board-{i}.md.lock")).exists(),
            "case {i}: lock file must not be created"
        );
    }
}

/// Scenario: 找不到 flock 时在任何写入前失败
#[test]
fn board_append_fails_before_write_when_flock_missing() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    let sealed = sealed_path(dir.path());
    let out = append(
        &board,
        &[],
        "### {N}. x\n",
        &[
            ("PATH", &sealed.to_string_lossy()),
            ("OLP_PREFLIGHT_DISCOVER", "0"),
        ],
    );
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("flock"), "{}", stderr(&out));
    assert!(!board.exists());
    assert!(!dir.path().join("board.md.lock").exists());
}

/// Scenario: 最小 PATH 下可写板
#[test]
fn board_append_works_with_minimal_path() {
    if !host_has_flock() {
        eprintln!("SKIP (explicit): host has no flock in any location the script looks at");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.md");
    // What a non-interactive ssh session gets: no Homebrew directories.
    let out = append(
        &board,
        &[],
        "### {N}. over ssh\n",
        &[("PATH", MINIMAL_PATH)],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "1");
    assert_eq!(
        std::fs::read_to_string(&board).unwrap(),
        "### 1. over ssh\n"
    );
}

/// Scenario: 同键同号,各前缀独立计数
#[test]
fn id_alloc_same_key_returns_same_id_and_series_are_independent() {
    if !host_has_flock() {
        eprintln!("SKIP (explicit): host has no flock in any location the script looks at");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let reg = dir.path().join("ids.tsv");
    let alloc = |prefix: &str, width: &str, key: &str| {
        let out = id_alloc(&reg, prefix, width, key);
        assert!(out.status.success(), "stderr: {}", stderr(&out));
        stdout(&out).trim().to_string()
    };

    assert_eq!(alloc("API-SS-", "5", "ohos.ability.start"), "API-SS-00001");
    assert_eq!(alloc("API-SS-", "5", "ohos.ability.stop"), "API-SS-00002");
    assert_eq!(alloc("API-SS-", "5", "ohos.ability.start"), "API-SS-00001");
    // Keys are compared byte for byte: backslashes and spaces are literal.
    assert_eq!(
        alloc("API-SS-", "5", r"ohos\tability start"),
        "API-SS-00003"
    );
    assert_eq!(
        alloc("API-SS-", "5", r"ohos\tability start"),
        "API-SS-00003"
    );
    // Another series starts its own counter, even for the same key.
    assert_eq!(alloc("W", "5", "ohos.ability.start"), "W00001");
    assert_eq!(alloc("APP-", "5", "com.example.app"), "APP-00001");

    let rows: Vec<Vec<String>> = std::fs::read_to_string(&reg)
        .unwrap()
        .lines()
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect();
    assert_eq!(rows.len(), 5, "one row per distinct (series, key)");
    assert!(rows.iter().all(|r| r.len() == 4), "{rows:?}");
}

/// Scenario: 并发请求无重号且按键去重
#[test]
fn id_alloc_concurrent_requests_are_unique_and_deduplicated() {
    if !host_has_flock() {
        eprintln!("SKIP (explicit): host has no flock in any location the script looks at");
        return;
    }
    const KEYS: usize = 40;
    let dir = tempfile::tempdir().unwrap();
    let reg = dir.path().join("ids.tsv");

    // Every key is requested twice, all at once.
    let handles: Vec<_> = (0..KEYS * 2)
        .map(|i| {
            let reg = reg.clone();
            std::thread::spawn(move || {
                let key = format!("wall-signature-{}", i % KEYS);
                let out = id_alloc(&reg, "W", "5", &key);
                assert!(out.status.success(), "stderr: {}", stderr(&out));
                (key, stdout(&out).trim().to_string())
            })
        })
        .collect();
    let mut by_key: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for h in handles {
        let (key, id) = h.join().unwrap();
        by_key.entry(key).or_default().insert(id);
    }

    assert_eq!(by_key.len(), KEYS);
    assert!(
        by_key.values().all(|ids| ids.len() == 1),
        "both requests for a key get the same id: {by_key:?}"
    );
    let ids: BTreeSet<String> = by_key.values().flatten().cloned().collect();
    let expected: BTreeSet<String> = (1..=KEYS).map(|n| format!("W{n:05}")).collect();
    assert_eq!(ids, expected, "ids are dense and unique");
    assert_eq!(std::fs::read_to_string(&reg).unwrap().lines().count(), KEYS);
}

/// Scenario: 非法请求在触碰登记文件前被拒绝
#[test]
fn id_alloc_rejects_invalid_input_before_touching_registry() {
    let dir = tempfile::tempdir().unwrap();
    let reg = dir.path().join("ids.tsv");
    let cases = [
        ("9API-", "5", "k"),
        ("API_SS", "5", "k"),
        ("", "5", "k"),
        ("W", "0", "k"),
        ("W", "13", "k"),
        ("W", "x", "k"),
        ("W", "5", ""),
        ("W", "5", "has\ttab"),
        ("W", "5", "has\nnewline"),
    ];
    for (prefix, width, key) in cases {
        let out = id_alloc(&reg, prefix, width, key);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{prefix:?} {width:?} {key:?}: {}",
            stderr(&out)
        );
        assert_eq!(stdout(&out), "");
    }
    assert!(!reg.exists());
    assert!(!dir.path().join("ids.tsv.lock").exists());
}

/// Scenario: retro 缺工具时在任何写入前失败
#[test]
fn evo_retro_fails_before_write_when_tools_missing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".octos")).unwrap();
    std::fs::write(repo.join(".octos/EVOLUTION.md"), "").unwrap();
    let state = dir.path().join("state");
    let sealed = sealed_path(dir.path());

    let out = Command::new("/bin/bash")
        .arg(script("olp-evo-retro.sh"))
        .arg(&repo)
        .env("PATH", &sealed)
        .env("OLP_PREFLIGHT_DISCOVER", "0")
        .env("OLP_EVO_STATE", &state)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    let err = stderr(&out);
    for cap in ["sha256sum", "python3", "flock"] {
        assert!(err.contains(cap), "missing {cap} must be named: {err}");
    }
    assert!(!state.exists(), "state dir must not be created");
}

/// Scenario: skeleton 缺工具时在任何写入前失败
#[test]
fn evo_skeleton_fails_before_write_when_tools_missing() {
    let dir = tempfile::tempdir().unwrap();
    let sealed = sealed_path(dir.path());
    let target = dir.path().join("out/skeleton.spec.md");

    let out = Command::new("/bin/bash")
        .arg(script("olp-evo-spec-skeleton.sh"))
        .arg(repo_root().join("fixtures/evolution/skeleton/FLAW-sample.md"))
        .arg("--out")
        .arg(&target)
        .env("PATH", &sealed)
        .env("OLP_PREFLIGHT_DISCOVER", "0")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    let err = stderr(&out);
    for cap in ["python3", "realpath-m"] {
        assert!(err.contains(cap), "missing {cap} must be named: {err}");
    }
    assert!(!target.parent().unwrap().exists(), "nothing may be created");
}

/// Scenario: 最小 PATH 下 skeleton 自行找到 GNU 工具
#[test]
fn evo_skeleton_out_works_with_minimal_path() {
    let gnu_realpath = Command::new("/bin/sh")
        .arg("-c")
        .arg("realpath -m /nonexistent/x >/dev/null 2>&1 && python3 -c 1 >/dev/null 2>&1")
        .env("PATH", MINIMAL_PATH)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let gnubin = ["/opt/homebrew", "/usr/local"]
        .iter()
        .any(|p| Path::new(p).join("opt/coreutils/libexec/gnubin").is_dir());
    let python = ["/usr/bin", "/opt/homebrew/bin", "/usr/local/bin"]
        .iter()
        .any(|d| Path::new(d).join("python3").exists());
    if !(gnu_realpath || (gnubin && python)) {
        eprintln!("SKIP (explicit): host has neither GNU realpath nor Homebrew coreutils");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out/skeleton.spec.md");
    let out = Command::new("/bin/bash")
        .arg(script("olp-evo-spec-skeleton.sh"))
        .arg(repo_root().join("fixtures/evolution/skeleton/FLAW-sample.md"))
        .arg("--out")
        .arg(&target)
        .env("PATH", MINIMAL_PATH)
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        std::fs::read_to_string(&target)
            .unwrap()
            .contains("satisfies: [REQ-OLP-EVO]")
    );
}
