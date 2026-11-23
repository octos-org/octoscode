//! `octoscode olp review cargo --repo R --selector S --expect pass|fail …`
//! — run one test of a real Cargo repository and write a receipt of what
//! actually happened.
//!
//! Evidence is execution: `cargo test … -- --list` must find exactly one
//! test the selector names, that test is then run with `--exact`, and the
//! verdict is read from the test's own status line, the summary and
//! cargo's exit code together — a selector that matches nothing, an
//! ignored test, or an exit code that disagrees with the output is never a
//! pass. The receipt binds the repository's HEAD and working tree before
//! and after, the manifest (which must belong to the same git worktree),
//! the test's source file, and the full output.
//!
//! Tracked files must equal HEAD before and after the run; untracked
//! files (an injected probe, artifacts, the target directory) are allowed.
//!
//! Prints `{"ok": true, …}` and exits 0 when what was observed is what was
//! expected; else an error envelope `{"error": {code, message, detail}}`
//! and exit 1. Exit 2: usage.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use fancy_regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::evolib::Json;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a pattern written in this file")
}

static PASS_SUMMARY: LazyLock<Regex> = LazyLock::new(|| re(r"test result: ok\."));
static FAIL_SUMMARY: LazyLock<Regex> = LazyLock::new(|| re(r"test result: FAILED\."));
static PANIC: LazyLock<Regex> = LazyLock::new(|| re(r"panicked at [^\s:]+:\d+:\d+"));
static TEST_LINE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?m)^test (\S+) \.\.\. (ok|FAILED|ignored)"));
// A rustc error (`error[E0308]:` or `error: <msg>`), not libtest's note.
static COMPILE_ERR: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^error(?:\[[A-Z0-9]+\])?: .*$"));
static LIST_LINE: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^(\S+): (test|benchmark)\s*$"));
static RUST_IDENT: LazyLock<Regex> =
    LazyLock::new(|| re(r"^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$"));

/// The one environment key passed into the receipt by default.
const DEFAULT_ENV_KEYS: [&str; 1] = ["OLP_ADAPTER_FIXTURE"];

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha256_file(path: &Path) -> Json {
    match std::fs::read(path) {
        Ok(bytes) => Json::Str(hex(&bytes)),
        Err(_) => Json::Null,
    }
}

fn text(s: impl Into<String>) -> Json {
    Json::Str(s.into())
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn emit(value: &Json) {
    println!("{}", value.pretty(1));
}

fn fail(code: &str, message: String, detail: Option<Json>) -> i32 {
    let mut error = vec![("code", text(code)), ("message", text(message))];
    if let Some(detail) = detail {
        error.push(("detail", detail));
    }
    emit(&obj(vec![("error", obj(error))]));
    1
}

/// Absolute, with symlinks followed where the path exists.
fn resolve(path: &str) -> PathBuf {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path)))
}

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

enum RunError {
    Missing,
    Timeout,
}

fn run(mut cmd: Command, limit: Duration) -> Result<Ran, RunError> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| RunError::Missing)?;
    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout);
            }
        }
    };
    Ok(Ran {
        code: status.code(),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo);
    run(cmd, Duration::from_secs(30))
        .ok()
        .filter(|ran| ran.code == Some(0))
        .map(|ran| ran.stdout)
}

fn git_head(repo: &Path) -> Option<String> {
    git(repo, &["rev-parse", "HEAD"])
        .map(|out| out.trim().to_string())
        .filter(|head| {
            head.len() == 40 && head.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}

fn git_status(repo: &Path) -> Option<String> {
    git(repo, &["status", "--porcelain"])
}

/// Tracked changes in a `git status --porcelain` listing; untracked files
/// (`??`) do not count.
fn tracked_modified(porcelain: &str) -> Vec<&str> {
    porcelain
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with("??"))
        .collect()
}

/// The gate before and after the run: tracked files must equal HEAD, and a
/// status that cannot be read is not "clean". `true` when an error was
/// printed.
fn tracked_source_modified(repo: &Path, phase: &str) -> bool {
    let Some(porcelain) = git_status(repo) else {
        fail(
            "tracked-source-modified",
            format!("{phase}: git status 查询失败(不得当作 clean)"),
            None,
        );
        return true;
    };
    let lines = tracked_modified(&porcelain);
    if lines.is_empty() {
        return false;
    }
    let shown: Vec<String> = lines
        .iter()
        .take(8)
        .map(|l| format!("'{}'", l.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect();
    fail(
        "tracked-source-modified",
        format!(
            "{phase}: repo 存在 tracked 修改(untracked 探针/工件允许,tracked 生产源须等于 HEAD): [{}]",
            shown.join(", ")
        ),
        None,
    );
    true
}

fn rust_files(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if recursive {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The first of `files` that defines `fn <name>(`.
fn defining_file(files: Vec<PathBuf>, name: &str) -> Option<PathBuf> {
    let needle = re(&format!(r"fn\s+{}\s*\(", fancy_regex::escape(name)));
    files.into_iter().find(|file| {
        std::fs::read(file).is_ok_and(|bytes| {
            needle
                .is_match(&String::from_utf8_lossy(&bytes))
                .unwrap_or(false)
        })
    })
}

const USAGE: &str = "usage: octoscode olp review cargo --repo REPO --selector SELECTOR --expect {pass,fail} [--manifest MANIFEST] [--test-target TARGET] [--lib] [--artifact-dir DIR] [--receipt FILE] [--timeout SECONDS] [--env-key KEY]... [--jobs N] [--target-dir DIR]";

/// `argv` is what follows `olp review cargo`.
pub fn run_cli(argv: &[String]) -> i32 {
    let mut values: std::collections::BTreeMap<&str, String> = Default::default();
    let (mut lib, mut env_keys) = (false, Vec::new());
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        let known = [
            "--repo",
            "--manifest",
            "--test-target",
            "--selector",
            "--expect",
            "--artifact-dir",
            "--receipt",
            "--timeout",
            "--jobs",
            "--target-dir",
        ];
        match name {
            "-h" | "--help" => {
                println!(
                    "{USAGE}\n\n在真实 Cargo 仓库上执行一个测试并写下执行回执:先用 --list 唯一定位,再 --exact 执行,\n按状态行、汇总与退出码三者一致判定。成功输出 {{\"ok\": true}};否则输出错误信封,退出码 1。"
                );
                return 0;
            }
            "--lib" => lib = true,
            "--env-key" => match inline.or_else(|| words.next().cloned()) {
                Some(key) => env_keys.push(key),
                None => {
                    eprintln!("{USAGE}\nerror: --env-key 需要一个值");
                    return 2;
                }
            },
            other => match known.iter().find(|k| **k == other) {
                Some(key) => match inline.or_else(|| words.next().cloned()) {
                    Some(value) => {
                        values.insert(key, value);
                    }
                    None => {
                        eprintln!("{USAGE}\nerror: {other} 需要一个值");
                        return 2;
                    }
                },
                None => {
                    eprintln!("{USAGE}\nerror: unrecognized arguments: {word}");
                    return 2;
                }
            },
        }
    }
    let get = |key: &str| values.get(key).cloned();
    let (Some(repo_arg), Some(selector), Some(expect)) =
        (get("--repo"), get("--selector"), get("--expect"))
    else {
        eprintln!(
            "{USAGE}\nerror: the following arguments are required: --repo, --selector, --expect"
        );
        return 2;
    };
    if !matches!(expect.as_str(), "pass" | "fail") {
        eprintln!(
            "{USAGE}\nerror: argument --expect: invalid choice: '{expect}' (choose from 'pass', 'fail')"
        );
        return 2;
    }
    let timeout = match get("--timeout").map(|t| t.parse::<u64>()) {
        None => 600,
        Some(Ok(seconds)) => seconds,
        Some(Err(_)) => {
            eprintln!("{USAGE}\nerror: argument --timeout: invalid int value");
            return 2;
        }
    };
    let receipt_path = get("--receipt");
    let env_var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let jobs = get("--jobs")
        .or_else(|| env_var("CARGO_BUILD_JOBS"))
        .unwrap_or_else(|| "4".to_string());
    let target_dir = get("--target-dir").or_else(|| env_var("CARGO_TARGET_DIR"));

    let repo = resolve(&repo_arg);
    let manifest = match get("--manifest") {
        Some(path) => resolve(&path),
        None => repo.join("Cargo.toml"),
    };
    if !repo.is_dir() {
        return fail(
            "repo-missing",
            format!("repo 不存在: {}", repo.display()),
            None,
        );
    }
    if !manifest.is_file() {
        return fail(
            "manifest-missing",
            format!("Cargo manifest 不存在: {}", manifest.display()),
            None,
        );
    }
    if !RUST_IDENT.is_match(&selector).unwrap_or(false) {
        return fail(
            "selector-invalid",
            format!("selector 非合法 Rust 路径: {selector}"),
            None,
        );
    }
    // The manifest must belong to the git worktree that is actually run:
    // `--repo A --manifest B` across repositories is refused.
    let worktree = git(&repo, &["rev-parse", "--show-toplevel"])
        .map(|out| out.trim().to_string())
        .filter(|top| !top.is_empty())
        .map(|top| resolve(&top));
    let Some(worktree) = worktree else {
        return fail(
            "head-unresolvable",
            format!("repo 非 git 工作树: {}", repo.display()),
            None,
        );
    };
    if !manifest.starts_with(&worktree) {
        return fail(
            "foreign-manifest",
            format!(
                "manifest {} 不属于真实执行 repo 的 git worktree {}(跨仓库绑定拒绝)",
                manifest.display(),
                worktree.display()
            ),
            None,
        );
    }
    let Some(head_before) = git_head(&repo) else {
        return fail(
            "head-unresolvable",
            format!("无法解析 repo 真实 HEAD: {}", repo.display()),
            None,
        );
    };
    if tracked_source_modified(&repo, "执行前") {
        return 1;
    }
    let status_before = git_status(&repo);
    let (target, source) = if lib {
        let last = selector.rsplit("::").next().unwrap_or(&selector);
        match defining_file(rust_files(&repo.join("src"), true), last) {
            Some(file) => ("--lib".to_string(), file),
            None => {
                return fail(
                    "no-tests-matched",
                    format!(
                        "selector {selector} 在 {}/src/**/*.rs 无定义",
                        repo.display()
                    ),
                    None,
                );
            }
        }
    } else {
        let named = get("--test-target").or_else(|| {
            defining_file(rust_files(&repo.join("tests"), false), &selector)
                .and_then(|file| file.file_stem().map(|s| s.to_string_lossy().into_owned()))
        });
        match named {
            Some(target) => {
                let file = repo.join("tests").join(format!("{target}.rs"));
                (target, file)
            }
            None => {
                return fail(
                    "no-tests-matched",
                    format!(
                        "selector {selector} 在 {}/tests/*.rs 无定义",
                        repo.display()
                    ),
                    None,
                );
            }
        }
    };
    // Only whitelisted keys are recorded in the receipt.
    let env_receipt: Vec<(String, Json)> = DEFAULT_ENV_KEYS
        .iter()
        .map(|k| k.to_string())
        .chain(env_keys)
        .filter_map(|key| std::env::var(&key).ok().map(|value| (key, text(value))))
        .collect();
    let manifest_text = manifest.to_string_lossy().into_owned();
    let mut base: Vec<String> = vec![
        "cargo".into(),
        "test".into(),
        "--manifest-path".into(),
        manifest_text.clone(),
    ];
    if lib {
        base.push("--lib".into());
    } else {
        base.extend(["--test".to_string(), target.clone()]);
    }
    let command = |words: &[String]| {
        let mut cmd = Command::new(&words[0]);
        cmd.args(&words[1..])
            .current_dir(&repo)
            .env("CARGO_BUILD_JOBS", &jobs);
        if let Some(dir) = &target_dir {
            cmd.env("CARGO_TARGET_DIR", dir);
        }
        cmd
    };
    // The gate before execution: `--list` must name exactly one test that
    // is, or ends with `::`, the selector.
    let mut list = base.clone();
    list.extend([
        selector.clone(),
        "--".into(),
        "--list".into(),
        "--format".into(),
        "terse".into(),
    ]);
    let listed = run(command(&list), Duration::from_secs(timeout.min(600)))
        .ok()
        .filter(|ran| ran.code == Some(0))
        .map(|ran| ran.stdout)
        .unwrap_or_default();
    let candidates: Vec<String> = LIST_LINE
        .captures_iter(&listed)
        .flatten()
        .map(|found| found[1].to_string())
        .filter(|name| *name == selector || name.ends_with(&format!("::{selector}")))
        .collect();
    let [qualified] = candidates.as_slice() else {
        return fail(
            "no-tests-matched",
            format!(
                "selector {selector} 经 cargo --list 无法唯一定位(编译失败或零/多重匹配),不得执行"
            ),
            None,
        );
    };
    // The qualified name with --exact: no substring can match by accident.
    let mut exec = base.clone();
    exec.extend([
        qualified.clone(),
        "--".into(),
        "--exact".into(),
        "--test-threads=1".into(),
        "--nocapture".into(),
    ]);
    let started = Instant::now();
    let ran = match run(command(&exec), Duration::from_secs(timeout)) {
        Ok(ran) => ran,
        Err(RunError::Missing) => {
            return fail("cargo-missing", "cargo 不在 PATH".to_string(), None);
        }
        Err(RunError::Timeout) => {
            return fail(
                "execution-timeout",
                format!("cargo 执行超时(>{timeout}s)"),
                Some(obj(vec![("selector", text(&selector))])),
            );
        }
    };
    let elapsed = (started.elapsed().as_secs_f64() * 1000.0).round() / 1000.0;
    let head_after = git_head(&repo);
    let status_after = git_status(&repo);
    if tracked_source_modified(&repo, "执行后") {
        return 1;
    }
    let exit_code = ran.code.unwrap_or(-1);
    let combined = format!("{}\n{}", ran.stdout, ran.stderr);
    let matched: Vec<(String, String)> = TEST_LINE
        .captures_iter(&ran.stdout)
        .flatten()
        .map(|found| (found[1].to_string(), found[2].to_string()))
        .collect();
    let status_is = |want: &str| {
        matched
            .iter()
            .any(|(name, status)| name == qualified && status == want)
    };
    let (ok, failed) = (status_is("ok"), status_is("FAILED"));
    let ignored = status_is("ignored") && !(ok || failed);
    let found = |pattern: &Regex, text: &str| pattern.is_match(text).unwrap_or(false);
    // Trusted verdicts need the status line, the summary and cargo's exit
    // code to agree; everything else is an untrusted shape.
    let observed = if !(ok || failed || status_is("ignored")) {
        "no-tests-matched"
    } else if ignored {
        "ignored-not-run"
    } else if ok && !failed && found(&PASS_SUMMARY, &ran.stdout) && exit_code == 0 {
        "pass"
    } else if failed
        && !ok
        && found(&FAIL_SUMMARY, &ran.stdout)
        && exit_code != 0
        && found(&PANIC, &combined)
    {
        "fail"
    } else if failed && found(&FAIL_SUMMARY, &ran.stdout) && exit_code != 0 {
        "no-assertion-failure"
    } else if found(&COMPILE_ERR, &ran.stderr) {
        "compile-error"
    } else {
        "inconsistent-exit"
    };
    let trusted = matches!(observed, "pass" | "fail");

    // The full output is kept, not only its tail. Each run gets its own
    // files: a rerun never overwrites what an earlier receipt points at.
    let artifact_dir = get("--artifact-dir").map(|d| resolve(&d)).or_else(|| {
        receipt_path
            .as_ref()
            .and_then(|r| resolve(r).parent().map(Path::to_path_buf))
    });
    let mut artifacts = Vec::new();
    if let Some(dir) = &artifact_dir {
        let stem = qualified.replace("::", "_");
        let keep = |kind: &str, body: &str| -> std::io::Result<String> {
            std::fs::create_dir_all(dir)?;
            let file = tempfile::Builder::new()
                .prefix(&format!("{stem}.{kind}."))
                .suffix(".log")
                .tempfile_in(dir)?;
            std::fs::write(file.path(), body)?;
            let (_, path) = file.keep().map_err(|e| e.error)?;
            Ok(path.to_string_lossy().into_owned())
        };
        match (keep("stdout", &ran.stdout), keep("stderr", &ran.stderr)) {
            (Ok(out), Ok(err)) => {
                artifacts = vec![
                    ("stdout".to_string(), text(out)),
                    ("stderr".to_string(), text(err)),
                ];
            }
            (Err(e), _) | (_, Err(e)) => {
                return fail(
                    "artifact-write-failed",
                    format!("工件目录写不了 {}: {e}", dir.display()),
                    None,
                );
            }
        }
    }
    let tail = |body: &str, chars: usize| -> String {
        let all: Vec<char> = body.chars().collect();
        all[all.len().saturating_sub(chars)..].iter().collect()
    };
    let optional = |value: Option<String>| value.map(text).unwrap_or(Json::Null);
    let receipt_json = receipt_path.as_ref().map(text).unwrap_or(Json::Null);
    let receipt = obj(vec![
        ("receipt_kind", text("cargo-test-execution")),
        ("adapter", text("scripts/olp-review-evidence-cargo.py")),
        ("argv", Json::Arr(exec.iter().map(text).collect())),
        ("cwd", text(repo.to_string_lossy())),
        ("repo", text(repo.to_string_lossy())),
        ("worktree_root", text(worktree.to_string_lossy())),
        ("manifest", text(&manifest_text)),
        ("manifest_sha256", sha256_file(&manifest)),
        ("test_target", text(&target)),
        ("test_target_sha256", sha256_file(&source)),
        ("test_source", text(source.to_string_lossy())),
        ("selector", text(&selector)),
        ("selector_qualified", text(qualified)),
        ("expect", text(&expect)),
        ("env", Json::Obj(env_receipt)),
        ("exit_code", Json::Int(exit_code as i64)),
        ("observed", text(observed)),
        (
            "matched_tests",
            Json::Arr(matched.iter().map(|(name, _)| text(name)).collect()),
        ),
        ("head_before", text(&head_before)),
        ("head_after", optional(head_after)),
        ("status_before", optional(status_before)),
        ("status_after", optional(status_after)),
        ("artifacts", Json::Obj(artifacts)),
        ("elapsed_seconds", Json::Raw(json!(elapsed))),
        ("stdout_sha256", text(hex(ran.stdout.as_bytes()))),
        ("stderr_sha256", text(hex(ran.stderr.as_bytes()))),
        ("stdout_tail", text(tail(&ran.stdout, 4000))),
        ("stderr_tail", text(tail(&ran.stderr, 2000))),
        (
            "executed_unix",
            Json::Int(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            ),
        ),
    ]);
    if let Some(path) = &receipt_path {
        if let Err(e) = std::fs::write(path, format!("{}\n", receipt.pretty(1))) {
            return fail(
                "receipt-write-failed",
                format!("回执写不了 {path}: {e}"),
                None,
            );
        }
    }
    if !trusted {
        return fail(
            observed,
            format!("selector {selector} 真实执行形态={observed},非可信终态"),
            Some(obj(vec![
                ("observed", text(observed)),
                ("exit_code", Json::Int(exit_code as i64)),
                ("receipt", receipt_json),
            ])),
        );
    }
    if observed != expect {
        return fail(
            "expectation-violated",
            format!("期望 {expect} 但真实执行为 {observed}"),
            Some(obj(vec![
                ("observed", text(observed)),
                ("expect", text(&expect)),
                ("exit_code", Json::Int(exit_code as i64)),
                ("receipt", receipt_json),
            ])),
        );
    }
    emit(&obj(vec![
        ("ok", Json::Raw(Value::Bool(true))),
        ("observed", text(observed)),
        ("receipt", receipt_json),
    ]));
    0
}
