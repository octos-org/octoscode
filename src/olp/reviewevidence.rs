//! `octoscode olp review init|freeze|challenge|cross|status|classify` —
//! settle what two reviewers disagree on by running the test.
//!
//! ```text
//! init ──▶ freeze ──▶ challenge ──▶ cross (×2) ──▶ status
//!          two first    a test       each claim     the verdicts
//!          reviews,     really run   accepted,
//!          hashed                    refuted, pending
//! ```
//!
//! - `freeze` records two independent first reviews and their hashes; a
//!   review changed or deleted afterwards is refused everywhere. Each
//!   review must be vouched for from outside (see [`super::reviewpeer`]),
//!   and freezing fixes the set of claims: none is added later.
//! - `challenge --live-cargo` is the only way to bring evidence: this
//!   command itself runs `octoscode olp review cargo` (this same binary,
//!   never a program the caller names) and records the receipt it wrote.
//!   A receipt file handed in, a log that looks like cargo's, `--exec-argv`
//!   and `--imported` evidence are not proof that anything ran.
//! - `cross` records a reviewer's second report, which must cover every
//!   frozen claim in a structured block. A reproduced failure is refuted
//!   only by a later passing run recorded here, or by an operator's
//!   decision given on the command line.
//! - `status` says whether the review is closed: every claim backed by
//!   something that ran, both reviewers' cross reports in, and both
//!   written by the model their lane names.
//!
//! Every answer is JSON (`--format human` for people); a refusal is
//! `{"error": {"code", "message"}}` and exit 1. Exit 2: usage.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use super::reviewpeer::{
    Authority, capture_native_report, check_peer_validity, model_pair, models_verified,
};
use super::reviewstate::{
    Frontmatter, J, LANES, PROTOCOL, Res, STATE_FILENAME, broken, digits, fail, is_sha, load_state,
    now_unix, obj, parse_claims, parse_cross_claims, path_of, path_text, positive_turn, read_lossy,
    read_strict, repr_str, resolve, resolve_head, review_lock, save_state, sha256_file, show,
    strings, strip, words,
};

const USAGE: &str = "用法: octoscode olp review [--format json|human] <子命令> …

  init <目录> --base B --runtime R --session S --goal G [--repo .] [--head H]
  freeze <目录> --glm-review F --k3-review F --head H [--glm-slug S] [--k3-slug S]
         [--native-root D] [--runtime-evidence F]
  challenge <目录> --claim ID --live-cargo --selector 测试名 --expect pass|fail
         [--exec-repo D] [--lib] [--test-target T] [--manifest F] [--cargo-target-dir D] [--timeout 秒]
  challenge <目录> --claim ID --evidence F --imported      (外部证据:一律 not-replayed)
  cross <目录> --cross-report F --cross-slug S [--native-root D] [--runtime-evidence F]
         [--expect-claims a,b] [--allow-operator-refute] [--require-model-evidence]
  status <目录>
  classify --manifest F --replay-summary F [--review-dir D]… [--slot F --slot-log-dir D]
  cargo …      执行适配(由 challenge --live-cargo 调用)
  monitor …    只读进度视图

两份初审有分歧时,用实际执行的测试裁决。行为证据只认 challenge --live-cargo 由本命令实时执行的结果。
退出码:0;1 拒绝(输出 {\"error\": {code, message}});2 用法";

fn pattern(text: &str) -> fancy_regex::Regex {
    fancy_regex::Regex::new(text).expect("a pattern written here")
}

// What a log of a real test run contains. Necessary, never sufficient.
static PANIC: LazyLock<fancy_regex::Regex> =
    LazyLock::new(|| pattern(r"panicked at [^\s:]+:\d+:\d+"));
static FAIL_SUMMARY: LazyLock<fancy_regex::Regex> =
    LazyLock::new(|| pattern(r"test result: FAILED\..*\d+ failed"));
static PASS_SUMMARY: LazyLock<fancy_regex::Regex> = LazyLock::new(|| pattern(r"test result: ok\."));

fn found(pattern: &fancy_regex::Regex, text: &str) -> bool {
    pattern.is_match(text).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// arguments
// ---------------------------------------------------------------------------

/// A subcommand's arguments.
pub(super) struct Opts {
    values: BTreeMap<String, Vec<String>>,
    flags: BTreeSet<String>,
    pub(super) review_dir: PathBuf,
    /// As it was given, for messages.
    review_dir_text: String,
}

impl Opts {
    pub(super) fn value(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .and_then(|v| v.last())
            .map(String::as_str)
    }

    /// A value with something in it.
    fn given(&self, name: &str) -> Option<&str> {
        self.value(name).filter(|v| !v.is_empty())
    }

    pub(super) fn all(&self, name: &str) -> &[String] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
}

struct Shape {
    positional: bool,
    values: &'static [&'static str],
    flags: &'static [&'static str],
    required: &'static [&'static str],
}

fn shape(command: &str) -> Option<Shape> {
    Some(match command {
        "init" => Shape {
            positional: true,
            values: &[
                "--repo",
                "--head",
                "--base",
                "--runtime",
                "--session",
                "--goal",
            ],
            flags: &[],
            required: &["--base", "--runtime", "--session", "--goal"],
        },
        "freeze" => Shape {
            positional: true,
            values: &[
                "--glm-review",
                "--k3-review",
                "--glm-slug",
                "--k3-slug",
                "--head",
                "--native-root",
                "--runtime-evidence",
            ],
            flags: &[],
            required: &["--glm-review", "--k3-review"],
        },
        "challenge" => Shape {
            positional: true,
            values: &[
                "--glm-review",
                "--k3-review",
                "--evidence",
                "--claim",
                "--kind",
                "--test-name",
                "--harness-root",
                "--exec-argv",
                "--run-dir",
                "--timeout",
                "--selector",
                "--expect",
                "--exec-repo",
                "--test-target",
                "--manifest",
                "--cargo-target-dir",
            ],
            flags: &["--imported", "--live-cargo", "--lib"],
            required: &["--claim"],
        },
        "cross" => Shape {
            positional: true,
            values: &[
                "--glm-review",
                "--k3-review",
                "--cross-report",
                "--cross-slug",
                "--native-root",
                "--runtime-evidence",
                "--expect-claims",
            ],
            flags: &["--allow-operator-refute", "--require-model-evidence"],
            required: &["--cross-report"],
        },
        "status" => Shape {
            positional: true,
            values: &["--glm-review", "--k3-review"],
            flags: &[],
            required: &[],
        },
        "classify" => Shape {
            positional: false,
            values: &[
                "--manifest",
                "--replay-summary",
                "--review-dir",
                "--slot",
                "--slot-log-dir",
            ],
            flags: &[],
            required: &["--manifest", "--replay-summary"],
        },
        _ => return None,
    })
}

fn parse(command: &str, argv: &[String]) -> Result<Opts, String> {
    let shape = shape(command).ok_or_else(|| format!("不认识的子命令: {command}"))?;
    let mut opts = Opts {
        values: BTreeMap::new(),
        flags: BTreeSet::new(),
        review_dir: PathBuf::new(),
        review_dir_text: String::new(),
    };
    let mut positional = Vec::new();
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (word.as_str(), None),
        };
        if shape.flags.contains(&name) && inline.is_none() {
            opts.flags.insert(name.to_string());
        } else if shape.values.contains(&name) {
            let value = inline
                .or_else(|| words.next().cloned())
                .ok_or_else(|| format!("{name} 缺少取值"))?;
            opts.values.entry(name.to_string()).or_default().push(value);
        } else if name.starts_with('-') && name.len() > 1 {
            return Err(format!("不认识的参数: {name}"));
        } else {
            positional.push(word.clone());
        }
    }
    match (shape.positional, positional.as_slice()) {
        (true, [dir]) => {
            opts.review_dir_text = path_text(dir);
            opts.review_dir = path_of(dir);
        }
        (false, []) => {}
        _ => return Err(format!("{command}: 位置参数不对")),
    }
    if let Some(missing) = shape
        .required
        .iter()
        .find(|name| opts.value(name).is_none())
    {
        return Err(format!("{command}: 缺少 {missing}"));
    }
    if let Some(other) = opts
        .value("--expect")
        .filter(|e| !["pass", "fail"].contains(e))
    {
        return Err(format!("--expect 只能是 pass 或 fail: {other}"));
    }
    if let Some(other) = opts
        .value("--timeout")
        .filter(|t| t.parse::<i64>().is_err())
    {
        return Err(format!("--timeout 要一个整数: {other}"));
    }
    Ok(opts)
}

// ---------------------------------------------------------------------------
// the state's shape, and that nothing recorded was changed afterwards
// ---------------------------------------------------------------------------

fn shape_invalid(message: String) -> super::reviewstate::Fail {
    fail("state-shape-invalid", message)
}

/// A challenge record (`latest` or one of `history`) has the shape the
/// rest of the code reads.
fn record_shape_ok(rec: &J, claim: &str, place: &str, dir: &str) -> Res<()> {
    if !rec.is_obj() {
        return Err(shape_invalid(format!(
            "state.challenges[{claim}] {place} 非对象: {dir}"
        )));
    }
    if rec
        .get("accepted")
        .is_some_and(|a| !matches!(a, J::Bool(_)))
    {
        return Err(shape_invalid(format!(
            "state.challenges[{claim}] {place}.accepted 非布尔: {dir}"
        )));
    }
    let executed = rec.at("executed");
    if !executed.is_null() && !executed.is_obj() {
        return Err(shape_invalid(format!(
            "state.challenges[{claim}] {place}.executed 非对象: {dir}"
        )));
    }
    let artifacts = executed.at("artifacts");
    if !artifacts.is_null() && !artifacts.is_obj() {
        return Err(shape_invalid(format!(
            "state.challenges[{claim}] {place}.executed.artifacts 非对象: {dir}"
        )));
    }
    Ok(())
}

/// The state is valid JSON; is it also a state? Refuse one whose shape
/// the commands could not read, instead of failing somewhere inside them.
pub(super) fn validated_state(state: J, review_dir: &str) -> Res<J> {
    let rd = review_dir;
    if !state.is_obj() {
        return Err(shape_invalid(format!("state 顶层非对象: {rd}")));
    }
    if state
        .get("frozen")
        .is_some_and(|f| !matches!(f, J::Bool(_)))
    {
        return Err(shape_invalid(format!("state.frozen 非布尔: {rd}")));
    }
    if state.at("frozen").is_true() {
        // Freezing always writes these; a frozen state without them is
        // damaged.
        let Some(reviews) = state.get("reviews") else {
            return Err(shape_invalid(format!(
                "state.frozen=true 但缺 reviews(结构损坏): {rd}"
            )));
        };
        if let Some(lane) = LANES
            .iter()
            .find(|lane| reviews.is_obj() && !reviews.has(lane))
        {
            return Err(shape_invalid(format!(
                "state.frozen=true 但 reviews 缺 {lane}(双初审不完整): {rd}"
            )));
        }
        if state.at("head").text().is_none() {
            return Err(shape_invalid(format!(
                "state.frozen=true 但缺合法 head(结构损坏): {rd}"
            )));
        }
        for key in ["verdicts", "challenges"] {
            if !state.at(key).is_obj() {
                return Err(shape_invalid(format!(
                    "state.frozen=true 但 {key} 缺失/非对象(结构损坏): {rd}"
                )));
            }
        }
        if state.at("repo").str().is_none_or(|r| strip(r).is_empty()) {
            return Err(shape_invalid(format!(
                "state.frozen=true 但 repo 缺失/非非空字符串(结构损坏): {rd}"
            )));
        }
    }
    if let Some(reviews) = state.get("reviews") {
        let Some(pairs) = reviews.pairs() else {
            return Err(shape_invalid(format!("state.reviews 非对象: {rd}")));
        };
        for (label, rec) in pairs {
            if !rec.is_obj() {
                return Err(shape_invalid(format!(
                    "state.reviews[{}] 形状非法(须对象): {rd}",
                    repr_str(label)
                )));
            }
            for field in ["path", "sha256"] {
                if rec.at(field).text().is_none() {
                    return Err(shape_invalid(format!(
                        "state.reviews[{label}].{field} 缺失/非字符串: {rd}"
                    )));
                }
            }
        }
    }
    if let Some(challenges) = state.get("challenges") {
        let Some(pairs) = challenges.pairs() else {
            return Err(shape_invalid(format!("state.challenges 非对象: {rd}")));
        };
        for (claim, entry) in pairs {
            if claim.is_empty() || !entry.is_obj() {
                return Err(shape_invalid(format!(
                    "state.challenges[{}] 形状非法(须对象): {rd}",
                    repr_str(claim)
                )));
            }
            let history = entry.at("history");
            if !history.is_null() && history.items().is_none() {
                return Err(shape_invalid(format!(
                    "state.challenges[{claim}].history 非数组: {rd}"
                )));
            }
            if let Some(latest) = entry.get("latest").filter(|l| !l.is_null()) {
                record_shape_ok(latest, claim, "latest", rd)?;
            }
            for rec in history.items().into_iter().flatten() {
                record_shape_ok(rec, claim, "history[]", rd)?;
            }
        }
    }
    if let Some(verdicts) = state.get("verdicts") {
        let Some(pairs) = verdicts.pairs() else {
            return Err(shape_invalid(format!("state.verdicts 非对象: {rd}")));
        };
        if let Some((claim, _)) = pairs.iter().find(|(c, v)| c.is_empty() || !v.is_obj()) {
            return Err(shape_invalid(format!(
                "state.verdicts[{}] 形状非法(须对象): {rd}",
                repr_str(claim)
            )));
        }
    }
    if let Some(cross) = state.get("cross") {
        let Some(items) = cross.items() else {
            return Err(shape_invalid(format!("state.cross 非数组: {rd}")));
        };
        for entry in items {
            if !entry.is_obj() {
                return Err(shape_invalid(format!("state.cross 元素非对象: {rd}")));
            }
            for field in ["slug", "sha256"] {
                let bad = entry
                    .get(field)
                    .is_some_and(|v| v.str().is_none_or(|s| strip(s).is_empty()));
                if bad {
                    return Err(shape_invalid(format!(
                        "state.cross.{field} 非非空字符串: {rd}"
                    )));
                }
            }
            if entry
                .get("turn")
                .is_some_and(|t| positive_turn(t).is_none())
            {
                return Err(shape_invalid(format!("state.cross.turn 非正轮次: {rd}")));
            }
        }
    }
    Ok(state)
}

fn tampered(message: String) -> super::reviewstate::Fail {
    fail("live-receipt-tampered", message)
}

/// One accepted challenge record still matches the receipt and the output
/// files it was made from. The receipt, checked against its recorded
/// hash, is the source of truth; what the record's own snapshot lacks is
/// read from it, and where both speak they must agree.
fn check_accepted_record(claim: &str, rec: &J, place: &str) -> Res<()> {
    if !rec.at("accepted").is_true() {
        return Ok(());
    }
    let Some(receipt) = rec.at("receipt").text() else {
        return Err(tampered(format!(
            "{claim} {place} accepted 记录缺 receipt 路径(fail-closed)"
        )));
    };
    let Some(sha256) = rec.at("receipt_sha256").text() else {
        return Err(tampered(format!(
            "{claim} {place} accepted 记录缺 receipt_sha256(fail-closed)"
        )));
    };
    let path = Path::new(receipt);
    if !path.is_file() {
        return Err(tampered(format!(
            "{claim} {place} accepted receipt 被删除(文件缺失): {receipt}"
        )));
    }
    if sha256_file(path).ok().as_deref() != Some(sha256) {
        return Err(tampered(format!(
            "{claim} {place} accepted receipt 被改写(SHA256 不匹配): {receipt}"
        )));
    }
    let Some(rc) = read_strict(path).ok().and_then(|text| J::parse(&text).ok()) else {
        return Err(tampered(format!(
            "{claim} {place} accepted receipt 不可解析: {receipt}"
        )));
    };
    if !rc.is_obj() {
        return Err(tampered(format!(
            "{claim} {place} accepted receipt 顶层非对象: {receipt}"
        )));
    }
    if !rc.at("receipt_kind").is("cargo-test-execution") {
        return Err(tampered(format!(
            "{claim} {place} receipt_kind 非 cargo-test-execution: {receipt}"
        )));
    }
    let executed = rec.at("executed");
    for field in [
        "observed",
        "exit_code",
        "selector",
        "selector_qualified",
        "test_target_sha256",
        "stdout_sha256",
        "stderr_sha256",
        "head_before",
        "head_after",
    ] {
        let (theirs, ours) = (rc.at(field), executed.at(field));
        if !theirs.is_null() && !ours.is_null() {
            // `false` is not `0`: the kinds must agree, then the values.
            let same_kind = matches!((theirs, ours), (J::Num(_), J::Num(_)))
                || std::mem::discriminant(theirs) == std::mem::discriminant(ours);
            if !same_kind || !theirs.same(ours) {
                return Err(tampered(format!(
                    "{claim} {place} 核心字段 {field} 快照与 receipt 不一致(类型/值不匹配: {} vs {})",
                    ours.repr(),
                    theirs.repr()
                )));
            }
        }
        let needed = ["observed", "exit_code", "head_before", "head_after"].contains(&field);
        if theirs.is_null() && ours.is_null() && needed {
            return Err(tampered(format!(
                "{claim} {place} 核心字段 {field} 双侧缺失(fail-closed)"
            )));
        }
    }
    // The output files: a real receipt always names them.
    for key in ["stdout", "stderr"] {
        let theirs = rc.at("artifacts").at(key).text();
        let ours = executed.at("artifacts").at(key).text();
        if let (Some(a), Some(b)) = (theirs, ours) {
            if a != b {
                return Err(tampered(format!(
                    "{claim} {place} 执行工件 {key} 快照与 receipt 不一致"
                )));
            }
        }
        let Some(artifact) = theirs.or(ours) else {
            return Err(tampered(format!(
                "{claim} {place} receipt 缺执行工件 {key}(fail-closed)"
            )));
        };
        let field = format!("{key}_sha256");
        let declared = [rc.at(&field), executed.at(&field)]
            .into_iter()
            .find(|v| v.truthy())
            .and_then(J::text);
        let Some(declared) = declared else {
            return Err(tampered(format!(
                "{claim} {place} 缺执行工件 {key} hash(fail-closed)"
            )));
        };
        let file = Path::new(artifact);
        if !file.is_file() {
            return Err(tampered(format!(
                "{claim} {place} 执行工件 {key} 被删除(文件缺失): {artifact}"
            )));
        }
        if sha256_file(file).ok().as_deref() != Some(declared) {
            return Err(tampered(format!(
                "{claim} {place} 执行工件 {key} 被改写(SHA256 不匹配): {artifact}"
            )));
        }
    }
    Ok(())
}

/// Nothing recorded was changed since: the two first reviews still hash
/// to what was frozen, and every accepted challenge's receipt and output
/// files are as they were.
pub(super) fn verify_no_tamper(state: &J) -> Res<()> {
    for lane in LANES {
        let rec = state.at("reviews").at(lane);
        if !rec.truthy() {
            continue;
        }
        let path = Path::new(rec.at("path").str().unwrap_or_default());
        if !path.exists() {
            return Err(fail(
                "first-review-tampered",
                format!("{lane} 初审在冻结后被删除(文件缺失),不予采信"),
            ));
        }
        if sha256_file(path).ok().as_deref() != rec.at("sha256").str() {
            return Err(fail(
                "first-review-tampered",
                format!("{lane} 初审在冻结后被修改(SHA256 不匹配),不予采信"),
            ));
        }
    }
    for (claim, entry) in state.at("challenges").pairs().into_iter().flatten() {
        if let Some(latest) = entry.get("latest").filter(|l| !l.is_null()) {
            check_accepted_record(claim, latest, "latest")?;
        }
        for (at, rec) in entry
            .at("history")
            .items()
            .into_iter()
            .flatten()
            .enumerate()
        {
            check_accepted_record(claim, rec, &format!("history[{at}]"))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// init, freeze
// ---------------------------------------------------------------------------

fn cmd_init(opts: &Opts) -> Res<J> {
    let _lock = review_lock(&opts.review_dir, false)?;
    let state = validated_state(load_state(&opts.review_dir)?, &opts.review_dir_text)?;
    if state.truthy() {
        return Err(fail(
            "already-initialized",
            format!("{} 已初始化", opts.review_dir_text),
        ));
    }
    let repo = path_of(opts.value("--repo").unwrap_or("."));
    // The real HEAD; `--head` only cross-checks it.
    let head = resolve_head(&repo)?;
    if let Some(given) = opts.given("--head").filter(|given| *given != head) {
        return Err(fail(
            "head-mismatch",
            format!("--head={given} 与 repo 真实 HEAD={head} 不一致"),
        ));
    }
    let mut state = obj(vec![
        ("protocol", J::from(PROTOCOL)),
        ("head", J::from(head.as_str())),
        ("base", J::from(opts.value("--base"))),
        ("runtime", J::from(opts.value("--runtime"))),
        ("session", J::from(opts.value("--session"))),
        ("goal", J::from(opts.value("--goal"))),
        ("repo", J::from(show(&resolve(&repo)))),
        ("reviews", obj(vec![])),
        ("frozen", J::from(false)),
        ("challenge", J::Null),
        ("challenges", obj(vec![])),
        ("cross", J::Arr(vec![])),
        ("verdicts", obj(vec![])),
    ]);
    save_state(&opts.review_dir, &mut state)?;
    Ok(obj(vec![
        ("ok", J::from(true)),
        ("state", J::from("initialized")),
        ("head", J::from(head)),
    ]))
}

/// A first review says which session and goal it belongs to, and they are
/// this review's; its runtime and repository, when it names them, too.
fn check_report_identity(fm: &Frontmatter, label: &str, state: &J) -> Res<()> {
    for key in ["session", "goal"] {
        let Some(theirs) = fm.text(key) else {
            return Err(fail(
                "report-identity-missing",
                format!("{label} 初审缺 {key} frontmatter(身份缺失不得绕过校验)"),
            ));
        };
        let ours = state.at(key);
        if ours.truthy() && !ours.is(theirs) {
            return Err(fail(
                "report-identity-mismatch",
                format!("{label} 初审 {key}={theirs} != 评审视角 {}", ours.shown()),
            ));
        }
    }
    for key in ["runtime", "repo"] {
        if let (Some(theirs), Some(ours)) = (fm.text(key), state.at(key).text()) {
            if resolve(&path_of(theirs)) != resolve(&path_of(ours)) {
                return Err(fail(
                    "report-identity-mismatch",
                    format!("{label} 初审 {key}={theirs} != 评审视角 {ours}"),
                ));
            }
        }
    }
    Ok(())
}

fn read_runtime_evidence(opts: &Opts) -> Res<Option<J>> {
    let Some(path) = opts.given("--runtime-evidence").map(path_of) else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let text = read_lossy(&path).map_err(broken)?;
    J::parse(&text)
        .map(Some)
        .map_err(|e| broken(format!("{} 不是合法 JSON: {e}", path.display())))
}

fn cmd_freeze(opts: &Opts) -> Res<J> {
    let _lock = review_lock(&opts.review_dir, false)?;
    let mut state = validated_state(load_state(&opts.review_dir)?, &opts.review_dir_text)?;
    if !state.truthy() {
        return Err(fail("not-initialized", "先 init"));
    }
    if state.at("frozen").truthy() {
        return Err(fail("already-frozen", "初审已冻结"));
    }
    // The baseline is named outright and is the one init recorded.
    let Some(head) = opts.given("--head") else {
        return Err(fail(
            "head-anchor-missing",
            "freeze 需显式 --head 锚定评审基线",
        ));
    };
    if state.at("head").truthy() && !state.at("head").is(head) {
        return Err(fail(
            "head-mismatch",
            format!("--head={head} != 评审 HEAD={}", state.at("head").shown()),
        ));
    }
    let slug_of = |lane: &str| {
        opts.given(&format!("--{lane}-slug"))
            .unwrap_or(lane)
            .to_string()
    };
    let lanes: Vec<(&str, PathBuf, String)> = LANES
        .into_iter()
        .map(|lane| {
            let path = path_of(opts.value(&format!("--{lane}-review")).unwrap_or_default());
            (lane, path, slug_of(lane))
        })
        .collect();
    for (lane, path, _) in &lanes {
        if !path.exists() {
            return Err(fail(
                "first-reviews-incomplete",
                format!("{lane} 初审文件缺失: {}", path.display()),
            ));
        }
    }
    if lanes[0].2 == lanes[1].2 || resolve(&lanes[0].1) == resolve(&lanes[1].1) {
        return Err(fail(
            "duplicate-reviewer",
            "两份初审必须来自不同评审方(同 peer/同文件不得计为两独立报告)",
        ));
    }
    let reports = [
        Frontmatter::read(&lanes[0].1)?,
        Frontmatter::read(&lanes[1].1)?,
    ];
    let native_root = opts.given("--native-root").map(path_of);
    let runtime_evidence = read_runtime_evidence(opts)?;

    let mut claims_by_lane = Vec::new();
    for ((lane, path, slug), fm) in lanes.iter().zip(&reports) {
        let native_dir = native_root.as_ref().map(|root| root.join(slug));
        check_peer_validity(
            fm,
            &Authority {
                native_dir: native_dir.as_deref(),
                runtime_evidence: runtime_evidence.as_ref(),
                slug,
                require: true,
                after_turn: 0,
                session: state.at("session"),
                goal: state.at("goal"),
            },
        )?;
        let Some(theirs) = fm.text("HEAD").or(fm.text("head")) else {
            return Err(fail(
                "report-head-missing",
                format!("{lane} 初审缺 HEAD frontmatter(缺失不得绕过校验)"),
            ));
        };
        if theirs != head {
            return Err(fail(
                "head-mismatch",
                format!("{lane} 初审 HEAD={theirs} != --head={head}"),
            ));
        }
        check_report_identity(fm, lane, &state)?;
        let claims = parse_claims(path)?;
        if claims.is_empty() {
            return Err(fail(
                "claims-block-missing",
                format!("{lane} 初审缺显式 JSON claims 块(冻结即确立全部初始 claim ID)"),
            ));
        }
        claims_by_lane.push((*lane, claims));
    }

    // Freezing fixes the set of claims: both lanes' together, each with
    // the lanes that made it.
    let mut verdicts = obj(vec![]);
    for (_, claims) in &claims_by_lane {
        for claim in claims {
            let id = claim.at("id").str().unwrap_or_default();
            if verdicts.has(id) {
                continue;
            }
            let made_by: Vec<J> = claims_by_lane
                .iter()
                .flat_map(|(lane, theirs)| {
                    theirs
                        .iter()
                        .filter(move |c| c.at("id").is(id))
                        .map(move |_| J::from(*lane))
                })
                .collect();
            let or_default = |key: &str, default: J| claim.get(key).cloned().unwrap_or(default);
            verdicts.set(
                id,
                obj(vec![
                    ("state", or_default("verdict", J::from("approve"))),
                    ("evidence", or_default("evidence", J::Arr(vec![]))),
                    ("lanes", J::Arr(made_by)),
                ]),
            );
        }
    }
    let mut reviews = obj(vec![]);
    for ((lane, path, slug), fm) in lanes.iter().zip(&reports) {
        let turn = J::from(fm.get("turn"));
        reviews.set(
            lane,
            obj(vec![
                ("path", J::from(show(path))),
                ("sha256", J::from(fm.sha256.as_str())),
                ("peer", J::from(slug.as_str())),
                ("turn", turn.clone()),
                ("outcome", J::from(fm.get("outcome"))),
                (
                    "native_report",
                    capture_native_report(native_root.as_deref(), slug, &turn)?,
                ),
            ]),
        );
    }
    let mut claims: Vec<String> = verdicts
        .pairs()
        .into_iter()
        .flatten()
        .map(|(id, _)| id.clone())
        .collect();
    claims.sort();
    // The reviews, the claims and the flag go to disk in one write.
    state.set("reviews", reviews.clone());
    state.set("verdicts", verdicts);
    state.set("frozen", J::from(true));
    state.set("frozen_at_unix", J::from(now_unix()));
    save_state(&opts.review_dir, &mut state)?;
    Ok(obj(vec![
        ("ok", J::from(true)),
        ("state", J::from("frozen")),
        ("reviews", reviews),
        ("claims", strings(&claims)),
    ]))
}

// ---------------------------------------------------------------------------
// challenge
// ---------------------------------------------------------------------------

/// Record a challenge under its claim: the newest as `latest`, all of
/// them in `history`.
fn record_challenge(state: &mut J, claim: &str, rec: J) {
    let accepted = rec.at("accepted").is_true();
    let challenges = state.child("challenges");
    if !challenges.at(claim).is_obj() {
        challenges.set(claim, obj(vec![("history", J::Arr(vec![]))]));
    }
    let slot = challenges.child(claim);
    slot.set("latest", rec.clone());
    match slot.get_mut("history") {
        Some(J::Arr(history)) => history.push(rec.clone()),
        _ => slot.set("history", J::Arr(vec![rec.clone()])),
    }
    if accepted {
        // The most recent accepted challenge, where older readers look.
        state.set("challenge", rec);
    }
}

/// What ran, for [`apply_live`].
struct Live {
    receipt_path: PathBuf,
    receipt: J,
    observed: String,
    expectation_violated: bool,
}

/// Read back the receipt this command's own run just wrote and check what
/// it binds. A receipt from anywhere else never gets here.
fn load_execution_receipt(path: &Path, state_head: &str) -> Res<(J, String, String)> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = read_strict(path).map_err(broken)?;
    let Ok(rc) = J::parse(&text) else {
        return Err(fail(
            "receipt-invalid",
            format!("{name} 非合法 JSON,不是执行 receipt"),
        ));
    };
    if !rc.at("receipt_kind").is("cargo-test-execution") {
        return Err(fail(
            "receipt-invalid",
            format!("{name} 缺 receipt_kind=cargo-test-execution(非生产 cargo adapter 执行回执)"),
        ));
    }
    let observed = rc.at("observed");
    let Some(seen) = observed.str().filter(|o| ["pass", "fail"].contains(o)) else {
        return Err(fail(
            "receipt-not-conclusive",
            format!(
                "receipt observed={} 非终态(pass/fail),不得作行为证据",
                observed.shown()
            ),
        ));
    };
    let head_before = rc.at("head_before");
    if is_sha(state_head) && !head_before.is(state_head) {
        return Err(fail(
            "receipt-head-mismatch",
            format!(
                "receipt head_before={} != 评审 HEAD={state_head}",
                head_before.shown()
            ),
        ));
    }
    if !rc.at("head_after").same(head_before) {
        return Err(fail(
            "receipt-head-mismatch",
            "执行期间 HEAD 发生变化(head_before != head_after),树不一致",
        ));
    }
    if !rc.at("stdout_sha256").truthy() || rc.at("exit_code").is_null() {
        return Err(fail("receipt-invalid", "receipt 缺输出摘要/退出码绑定"));
    }
    let selector = rc.at("selector").str().unwrap_or_default().to_string();
    let seen = seen.to_string();
    Ok((rc, seen, selector))
}

/// Run a command and wait at most `limit`; past it the process is killed.
/// Returns its exit code and what it printed, or None when it ran over.
fn run_limited(mut command: Command, limit: Duration) -> std::io::Result<Option<(i32, String)>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut drains = Vec::new();
    let out = child.stdout.take();
    let err = child.stderr.take();
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    };
    if let Some(pipe) = out {
        drains.push(drain(Box::new(pipe)));
    }
    if let Some(pipe) = err {
        drains.push(drain(Box::new(pipe)));
    }
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut outputs = drains.into_iter().map(|d| d.join().unwrap_or_default());
    let stdout = outputs.next().unwrap_or_default();
    Ok(status.map(|s| {
        (
            s.code().unwrap_or(-1),
            String::from_utf8_lossy(&stdout).into_owned(),
        )
    }))
}

/// `--live-cargo`: run the cargo adapter — `octoscode olp review cargo`
/// of this very binary, never a path the caller chooses — and check the
/// receipt it wrote. What the adapter guarantees (the repository, the
/// manifest, the exact test, HEAD, the output files, the exit code) is
/// its own business; this only passes the arguments on.
fn run_live_adapter(opts: &Opts, state: &J, claim: &str) -> Res<Live> {
    let Ok(adapter) = std::env::current_exe() else {
        return Err(fail(
            "adapter-missing",
            "生产 cargo adapter 缺失: 找不到本程序",
        ));
    };
    let Some(selector) = opts.given("--selector") else {
        return Err(fail(
            "live-args-missing",
            "--live-cargo 需 --selector 精确测试名",
        ));
    };
    let Some(expect) = opts.given("--expect") else {
        return Err(fail(
            "live-args-missing",
            "--live-cargo 需 --expect pass|fail(显式预期)",
        ));
    };
    let repo = match opts.given("--exec-repo") {
        Some(given) => resolve(&path_of(given)),
        None => path_of(state.at("repo").str().unwrap_or_default()),
    };
    if !repo.is_dir() {
        return Err(fail(
            "repo-missing",
            format!("live 执行 repo 不存在: {}", repo.display()),
        ));
    }
    let evidence_dir = opts.review_dir.join("live-evidence");
    std::fs::create_dir_all(&evidence_dir).map_err(broken)?;
    let receipt_path = evidence_dir.join(format!(
        "{claim}-{}-{}.receipt.json",
        now_unix(),
        std::process::id()
    ));
    let timeout: i64 = opts
        .value("--timeout")
        .and_then(|t| t.parse().ok())
        .unwrap_or(600);
    let mut command = Command::new(&adapter);
    command
        .args(["olp", "review", "cargo", "--repo"])
        .arg(&repo)
        .args(["--selector", selector, "--expect", expect, "--receipt"])
        .arg(&receipt_path)
        .arg("--artifact-dir")
        .arg(&evidence_dir)
        .args(["--timeout", &timeout.to_string()])
        // The adapter that runs is the one in this binary.
        .env_remove("OCTOSCODE_OLP_LEGACY");
    if opts.flag("--lib") {
        command.arg("--lib");
    }
    for (ours, theirs) in [
        ("--test-target", "--test-target"),
        ("--manifest", "--manifest"),
        ("--cargo-target-dir", "--target-dir"),
    ] {
        if let Some(value) = opts.given(ours) {
            command.args([theirs, value]);
        }
    }
    let limit = (timeout + 120).max(0) as u64;
    let ran = run_limited(command, Duration::from_secs(limit)).map_err(|_| {
        fail(
            "adapter-missing",
            format!("无法执行生产 adapter: {}", adapter.display()),
        )
    })?;
    let Some((code, stdout)) = ran else {
        return Err(fail(
            "evidence-not-executed",
            format!("adapter 执行超时(>{limit}s),已回收子进程"),
        ));
    };
    let mut expectation_violated = false;
    if code != 0 {
        let envelope = J::parse(&stdout).unwrap_or(J::Null);
        let error = envelope.at("error");
        let code = error.at("code").text().unwrap_or("adapter-failed");
        if code == "expectation-violated" && receipt_path.exists() {
            // It ran, and the result is the opposite of what was expected:
            // still evidence, marked as what it is.
            expectation_violated = true;
        } else {
            let message = error.get("message").map(J::shown).unwrap_or_default();
            return Err(fail(
                "evidence-not-executed",
                format!("生产 adapter 未产生可信终态 [{code}]: {message}"),
            ));
        }
    }
    if !receipt_path.exists() {
        return Err(fail(
            "evidence-not-executed",
            "adapter 未落 receipt,无执行证明",
        ));
    }
    let head = state.at("head").str().unwrap_or_default();
    let (receipt, observed, ran_selector) = load_execution_receipt(&receipt_path, head)?;
    if ran_selector != selector {
        return Err(fail(
            "receipt-invalid",
            format!("receipt selector={ran_selector} 与请求 {selector} 不一致"),
        ));
    }
    Ok(Live {
        receipt_path,
        receipt,
        observed,
        expectation_violated,
    })
}

/// Move the claim's verdict by what the run showed, and record the run.
fn apply_live(state: &mut J, claim: &str, live: &Live) -> Res<String> {
    let before = state
        .at("verdicts")
        .at(claim)
        .at("state")
        .str()
        .map(str::to_string);
    let failed = live.observed == "fail";
    let (new_state, reason) = if failed {
        ("blocked-on-evidence", "challenge-evidence-executed")
    } else if matches!(before.as_deref(), Some("blocked-on-evidence" | "flipped")) {
        // A failure was reproduced before and the test now passes.
        ("challenge-refuted", "executed-probe-passed")
    } else {
        ("approve", "executed-probe-passed")
    };
    let rc = &live.receipt;
    let from = |key: &str| rc.at(key).clone();
    let receipt = show(&live.receipt_path);
    let rec = obj(vec![
        ("accepted", J::from(true)),
        ("claim", J::from(claim)),
        ("observed", J::from(live.observed.as_str())),
        ("expectation_violated", J::from(live.expectation_violated)),
        ("receipt", J::from(receipt.as_str())),
        (
            "receipt_sha256",
            J::from(sha256_file(&live.receipt_path).map_err(broken)?),
        ),
        (
            "executed",
            obj(vec![
                ("adapter", from("adapter")),
                ("argv", from("argv")),
                ("observed", J::from(live.observed.as_str())),
                ("exit_code", from("exit_code")),
                ("stdout_sha256", from("stdout_sha256")),
                ("stderr_sha256", from("stderr_sha256")),
                ("head_before", from("head_before")),
                ("head_after", from("head_after")),
                ("run_dir", from("cwd")),
                ("selector", from("selector")),
                ("selector_qualified", from("selector_qualified")),
                ("test_target", from("test_target")),
                ("manifest_sha256", from("manifest_sha256")),
                ("test_target_sha256", from("test_target_sha256")),
                ("artifacts", from("artifacts")),
            ]),
        ),
        ("executed_unix", J::from(now_unix())),
        (
            "result",
            J::from(if failed {
                "probe-failed"
            } else {
                "probe-passed"
            }),
        ),
    ]);
    record_challenge(state, claim, rec);
    let mut verdict = obj(vec![
        ("state", J::from(new_state)),
        ("reason", J::from(reason)),
        ("evidence", J::from(receipt)),
    ]);
    if new_state == "challenge-refuted" {
        verdict.set("refuted_by", J::from("live-reexecution"));
    }
    state.child("verdicts").set(claim, verdict);
    Ok(new_state.to_string())
}

/// A test's name resolves to a function in the harness sources.
fn test_name_resolvable(test_name: &str, roots: &[PathBuf]) -> bool {
    let bare = test_name.rsplit("::").next().unwrap_or(test_name);
    let Ok(definition) =
        fancy_regex::Regex::new(&format!(r"fn\s+{}\s*\(", fancy_regex::escape(bare)))
    else {
        return false;
    };
    fn sources(root: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    roots.iter().filter(|root| root.exists()).any(|root| {
        let mut files = Vec::new();
        if root.is_dir() {
            sources(root, &mut files);
        } else {
            files.push(root.clone());
        }
        files
            .iter()
            .any(|file| read_lossy(file).is_ok_and(|text| found(&definition, &text)))
    })
}

fn cmd_challenge(opts: &Opts) -> Res<J> {
    let _lock = review_lock(&opts.review_dir, false)?;
    let mut state = validated_state(load_state(&opts.review_dir)?, &opts.review_dir_text)?;
    if !state.truthy() {
        return Err(fail("not-initialized", "先 init"));
    }
    if !state.at("frozen").truthy() {
        return Err(fail("not-frozen", "初审未冻结"));
    }
    verify_no_tamper(&state)?;

    let Some(claim) = opts.given("--claim") else {
        return Err(fail("claim-required", "challenge 需 --claim 判词 id"));
    };
    // Freezing fixed the claims; none is added by challenging it.
    if !state.at("verdicts").has(claim) {
        return Err(fail(
            "claim-not-frozen",
            format!("claim {claim} 不在冻结 claims 集合中(禁止新增未冻结 claim)"),
        ));
    }
    let (live, imported) = (opts.flag("--live-cargo"), opts.flag("--imported"));
    if live && imported {
        return Err(fail("args-conflict", "--live-cargo 与 --imported 互斥"));
    }
    let evidence = opts.given("--evidence").map(path_of);
    let missing = |path: &Path| {
        fail(
            "evidence-missing",
            format!("证据文件不存在: {}", path.display()),
        )
    };

    // Evidence from outside: recorded as not replayed, whatever it says.
    if imported {
        let Some(path) = evidence else {
            return Err(fail(
                "evidence-missing",
                "--imported 需 --evidence 导入文件",
            ));
        };
        if !path.exists() {
            return Err(missing(&path));
        }
        state.child("verdicts").set(
            claim,
            obj(vec![
                ("state", J::from("not-replayed")),
                ("reason", J::from("imported-evidence-not-replayed")),
                ("evidence", J::from(show(&path))),
            ]),
        );
        record_challenge(
            &mut state,
            claim,
            obj(vec![
                ("accepted", J::from(false)),
                ("imported", J::from(true)),
                ("claim", J::from(claim)),
                ("evidence", J::from(show(&path))),
                ("executed_unix", J::from(now_unix())),
            ]),
        );
        save_state(&opts.review_dir, &mut state)?;
        return Ok(obj(vec![
            ("ok", J::from(true)),
            ("state", J::from("not-replayed")),
            ("claim", J::from(claim)),
            (
                "note",
                J::from("imported 证据未经本入口独立执行,不自动 accepted"),
            ),
        ]));
    }

    if live {
        let ran = run_live_adapter(opts, &state, claim)?;
        let new_state = apply_live(&mut state, claim, &ran)?;
        save_state(&opts.review_dir, &mut state)?;
        return Ok(obj(vec![
            ("ok", J::from(true)),
            ("state", J::from(new_state)),
            ("claim", J::from(claim)),
            ("observed", J::from(ran.observed.as_str())),
            (
                "executed",
                obj(vec![
                    ("exit_code", ran.receipt.at("exit_code").clone()),
                    ("receipt", J::from(show(&ran.receipt_path))),
                ]),
            ),
        ]));
    }

    // Anything else can only be refused: a file that already exists is
    // not proof that something ran.
    let Some(path) = evidence else {
        return Err(fail(
            "evidence-missing",
            "challenge 需 --live-cargo 实时执行,或 --evidence 配合 --imported 分层",
        ));
    };
    if !path.exists() {
        return Err(missing(&path));
    }
    let body = read_lossy(&path).map_err(broken)?;
    if path.extension().is_some_and(|e| e == "json")
        && J::parse(&body).is_ok_and(|probe| probe.at("receipt_kind").truthy())
    {
        return Err(fail(
            "evidence-not-executed",
            "外部执行回执(receipt_kind)不作执行证明: 行为证据只认本入口 --live-cargo 实时执行;外层导入请用 --imported(判 not-replayed)",
        ));
    }
    let unverified = |state: &mut J| {
        state.child("verdicts").set(
            claim,
            obj(vec![
                ("state", J::from("unverified")),
                ("reason", J::from("evidence-not-behavioral")),
            ]),
        );
        save_state(&opts.review_dir, state)
    };
    let failure_form = found(&PANIC, &body) && found(&FAIL_SUMMARY, &body);
    let pass_form = found(&PASS_SUMMARY, &body);
    // Prose and string constants are never evidence, whatever kind is
    // claimed for them.
    if !found(&PANIC, &body) && !found(&FAIL_SUMMARY, &body) && !pass_form {
        unverified(&mut state)?;
        return Err(fail(
            "evidence-not-behavioral",
            "证据为文档性内容(字符串常量断言/纯 prose),无论 kind 声明如何均拒绝",
        ));
    }
    let test_name = opts.given("--test-name").unwrap_or_default();
    let mut lacking = Vec::new();
    if !test_name.is_empty() && !body.contains(test_name) {
        lacking.push(format!("测试名行缺失: {test_name}"));
    }
    if !failure_form && !pass_form {
        lacking.push(
            "行为锚缺失: 需失败形态(panicked at + test result: FAILED. N failed)或通过形态(test result: ok.)"
                .to_string(),
        );
    }
    if !lacking.is_empty() {
        unverified(&mut state)?;
        return Err(fail("evidence-not-behavioral", lacking.join("; ")));
    }
    let roots: Vec<PathBuf> = opts
        .all("--harness-root")
        .iter()
        .filter(|root| !root.is_empty())
        .map(|root| path_of(root))
        .collect();
    if !test_name.is_empty() && !roots.is_empty() && !test_name_resolvable(test_name, &roots) {
        return Err(fail(
            "evidence-not-behavioral",
            format!("测试名 {test_name} 无法在 harness 源解析到定义"),
        ));
    }
    // A program the caller names is never run.
    if let Some(argv) = opts.given("--exec-argv") {
        let executor = words(argv)
            .first()
            .and_then(|first| Path::new(first).file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| argv.to_string());
        return Err(fail(
            "executor-not-trusted",
            format!(
                "执行体 {executor} 不可信: --exec-argv 不作行为证据; 唯一 live 入口为 challenge --live-cargo(本入口实时执行固定的 生产 adapter scripts/olp-review-evidence-cargo.py)"
            ),
        ));
    }
    Err(fail(
        "evidence-not-executed",
        "结构锚齐全但不构成执行证明: 需 --live-cargo 由本入口实时执行生产 adapter",
    ))
}

// ---------------------------------------------------------------------------
// cross, status
// ---------------------------------------------------------------------------

fn any_challenge_accepted(state: &J) -> bool {
    state
        .at("challenges")
        .pairs()
        .into_iter()
        .flatten()
        .any(|(_, slot)| slot.at("latest").at("accepted").is_true())
}

fn reviewer_slugs(state: &J) -> BTreeSet<String> {
    state
        .at("reviews")
        .pairs()
        .into_iter()
        .flatten()
        .filter_map(|(_, review)| review.at("peer").text().map(str::to_string))
        .collect()
}

fn list_repr(items: &[String]) -> String {
    strings(items).repr()
}

fn cmd_cross(opts: &Opts) -> Res<J> {
    let _lock = review_lock(&opts.review_dir, false)?;
    let mut state = validated_state(load_state(&opts.review_dir)?, &opts.review_dir_text)?;
    if !state.truthy() {
        return Err(fail("not-initialized", "先 init"));
    }
    if !state.at("frozen").truthy() {
        return Err(fail("not-frozen", "初审未冻结"));
    }
    verify_no_tamper(&state)?;
    if !any_challenge_accepted(&state) {
        return Err(fail("challenge-pending", "挑战证据尚未接纳,不得收录 cross"));
    }
    let report = path_of(opts.value("--cross-report").unwrap_or_default());
    if !report.exists() {
        return Err(fail(
            "cross-missing",
            format!("cross 报告不存在: {}", report.display()),
        ));
    }
    let fm = Frontmatter::read(&report)?;
    let native_root = opts.given("--native-root").map(path_of);
    let slug = opts.given("--cross-slug").unwrap_or_default();
    let runtime_evidence = read_runtime_evidence(opts)?;

    // A cross report comes from one of the two frozen reviewers.
    let reviewers = reviewer_slugs(&state);
    if !reviewers.contains(slug) {
        let known: Vec<String> = reviewers.into_iter().collect();
        return Err(fail(
            "cross-reviewer-unknown",
            format!(
                "cross slug={} 不是冻结的原 reviewer {}",
                if slug.is_empty() { "(missing)" } else { slug },
                list_repr(&known)
            ),
        ));
    }
    let Some(head) = fm.text("HEAD").or(fm.text("head")) else {
        return Err(fail(
            "report-head-missing",
            "cross 报告缺 HEAD frontmatter(缺失不得绕过校验)",
        ));
    };
    if state.at("head").truthy() && !state.at("head").is(head) {
        return Err(fail(
            "head-mismatch",
            format!(
                "cross 报告 HEAD={head} != 评审 HEAD={}",
                state.at("head").shown()
            ),
        ));
    }
    // Later than both frozen first reviews: an old first review is not a
    // cross report.
    let after_turn = state
        .at("reviews")
        .pairs()
        .into_iter()
        .flatten()
        .filter_map(|(_, review)| match review.at("turn") {
            J::Str(turn) => digits(turn),
            J::Num(n) if !n.is_f64() => n.as_u64(),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let native_dir = native_root.as_ref().map(|root| root.join(slug));
    check_peer_validity(
        &fm,
        &Authority {
            native_dir: native_dir.as_deref(),
            runtime_evidence: runtime_evidence.as_ref(),
            slug,
            require: true,
            after_turn,
            session: state.at("session"),
            goal: state.at("goal"),
        },
    )?;

    // Every frozen claim must have its own entry in the structured block.
    let entries = parse_cross_claims(&report)?.unwrap_or_default();
    let mut required: BTreeSet<String> = state
        .at("verdicts")
        .pairs()
        .into_iter()
        .flatten()
        .map(|(id, _)| id.clone())
        .collect();
    let expected = opts.given("--expect-claims").unwrap_or_default();
    required.extend(
        expected
            .split(',')
            .filter(|c| !c.is_empty())
            .map(str::to_string),
    );
    let covered: BTreeSet<String> = entries
        .iter()
        .filter_map(|c| c.at("id").str().map(str::to_string))
        .collect();
    let missing: Vec<String> = required.difference(&covered).cloned().collect();
    if !missing.is_empty() {
        return Err(fail(
            "missing-claim-coverage",
            format!(
                "cross 报告未在结构化 cross_claims 块覆盖判词: {}",
                list_repr(&missing)
            ),
        ));
    }
    let extra: Vec<String> = covered.difference(&required).cloned().collect();
    if !extra.is_empty() {
        return Err(fail(
            "claim-not-frozen",
            format!(
                "cross 引入未冻结 claim: {}(冻结即确立全部 claim ID)",
                list_repr(&extra)
            ),
        ));
    }

    // A refutation stands only on a later passing run recorded here, or
    // on an operator's decision allowed on the command line.
    let mut refuted = Vec::new();
    for entry in &entries {
        let id = entry.at("id").str().unwrap_or_default();
        if !state.at("verdicts").has(id) || !entry.at("verdict").is("refute") {
            continue;
        }
        let current = state.at("verdicts").at(id).at("state");
        let reproduced = ["flipped", "blocked-on-evidence", "challenge-refuted"];
        if !current.str().is_some_and(|s| reproduced.contains(&s)) {
            return Err(fail(
                "refutation-without-challenge",
                format!("claim {id} 当前状态 {},无已复现失败可反驳", current.shown()),
            ));
        }
        let reference = match entry.at("reference") {
            reference if reference.truthy() => reference.clone(),
            _ => obj(vec![]),
        };
        let substantiated = match reference.at("kind").str() {
            Some("executed-evidence") => {
                let want = match reference.at("ref") {
                    wanted if wanted.truthy() => wanted.clone(),
                    _ => J::from(""),
                };
                let history = state.at("challenges").at(id).at("history");
                history.items().into_iter().flatten().any(|h| {
                    h.at("accepted").is_true()
                        && h.at("observed").is("pass")
                        && h.at("receipt_sha256").same(&want)
                })
            }
            Some("operator-decision") => {
                opts.flag("--allow-operator-refute")
                    && reference.at("operator").truthy()
                    && reference.at("note").truthy()
            }
            _ => false,
        };
        if !substantiated {
            return Err(fail(
                "refutation-unsubstantiated",
                format!(
                    "claim {id} refute 缺有效依据: 需已验证新行为证据(本入口 live PASS 执行记录 SHA)或显式 --allow-operator-refute 的 operator 决定(operator+note 齐全)"
                ),
            ));
        }
        let verdict = state.child("verdicts").child(id);
        verdict.set("state", J::from("challenge-refuted"));
        verdict.set("refuted_by", J::from(slug));
        verdict.set("refutation_reference", reference);
        refuted.push(id.to_string());
    }

    // Checking the model here only refuses early; closing the review
    // always checks both reviewers again.
    let turn = J::from(fm.get("turn"));
    let mut entry = obj(vec![
        ("slug", J::from(slug)),
        ("sha256", J::from(fm.sha256.as_str())),
        ("turn", turn.clone()),
        ("refuted", strings(&refuted)),
    ]);
    let mut verified = false;
    if opts.flag("--require-model-evidence") {
        let native_report = capture_native_report(native_root.as_deref(), slug, &turn)?;
        let evidence = model_pair(&state, &J::from(slug), &turn, &native_report)?;
        verified = true;
        entry.set("model_evidence", evidence);
        if native_report.truthy() {
            entry.set("native_report", native_report);
        }
    }
    match state.get_mut("cross") {
        Some(J::Arr(cross)) => cross.push(entry),
        _ => state.set("cross", J::Arr(vec![entry])),
    }
    save_state(&opts.review_dir, &mut state)?;
    let mut warnings = Vec::new();
    if !verified {
        let code = "model-evidence-not-verified";
        let message = "本次 cross 仅收录用于审计，未核验实际模型，最终 review_accepted=false；请带 --require-model-evidence 重新提交";
        eprintln!("warning[{code}]: {message}");
        warnings.push(obj(vec![
            ("code", J::from(code)),
            ("message", J::from(message)),
        ]));
    }
    Ok(obj(vec![
        ("ok", J::from(true)),
        ("state", J::from("cross-recorded")),
        ("refuted", strings(&refuted)),
        ("warnings", J::Arr(warnings)),
    ]))
}

/// The review may close as far as behaviour goes: frozen, a challenge
/// accepted, both reviewers' cross reports recorded, and every frozen
/// claim backed by a run or a valid refutation.
fn behavior_accepted(state: &J) -> bool {
    if !state.at("frozen").truthy() || !any_challenge_accepted(state) {
        return false;
    }
    let cross = state.at("cross").items();
    let slugs: Option<BTreeSet<&str>> = cross
        .into_iter()
        .flatten()
        .map(|c| c.at("slug").text().filter(|_| c.is_obj()))
        .collect();
    let recorded = match (cross, slugs) {
        (Some(_), Some(slugs)) => slugs,
        _ => BTreeSet::new(),
    };
    let reviewers = reviewer_slugs(state);
    if reviewers.len() < 2 || !reviewers.iter().all(|r| recorded.contains(r.as_str())) {
        return false;
    }
    state
        .at("verdicts")
        .pairs()
        .into_iter()
        .flatten()
        .all(|(_, verdict)| {
            verdict.at("state").is("challenge-refuted")
                || ["executed-probe-passed", "challenge-evidence-executed"]
                    .contains(&verdict.at("reason").str().unwrap_or_default())
        })
}

fn build_status(state: &J) -> J {
    // Two reviewers agreeing is not an experiment: an approval nothing
    // was run for is shown as still waiting for evidence.
    let mut verdicts = match state.at("verdicts") {
        J::Obj(pairs) => J::Obj(pairs.clone()),
        _ => obj(vec![]),
    };
    if let J::Obj(pairs) = &mut verdicts {
        for (_, verdict) in pairs {
            if verdict.at("state").is("approve")
                && !verdict.at("reason").is("executed-probe-passed")
            {
                verdict.set("state", J::from("pending-behavioral-evidence"));
            }
        }
    }
    let model_verified = models_verified(state);
    let behavior_verified = behavior_accepted(state);
    let or_default = |key: &str, default: J| state.get(key).cloned().unwrap_or(default);
    let identity = ["head", "base", "runtime", "session", "goal", "repo"]
        .into_iter()
        .map(|key| (key, state.at(key).clone()))
        .collect();
    obj(vec![
        ("protocol", J::from(PROTOCOL)),
        ("frozen", or_default("frozen", J::from(false))),
        ("challenge_accepted", J::from(any_challenge_accepted(state))),
        (
            "review_accepted",
            J::from(behavior_verified && model_verified),
        ),
        ("behavior_accepted", J::from(behavior_verified)),
        ("model_verified", J::from(model_verified)),
        ("verdicts", verdicts),
        ("cross", or_default("cross", J::Arr(vec![]))),
        ("identity", obj(identity)),
    ])
}

fn cmd_status(opts: &Opts) -> Res<J> {
    if !opts.review_dir.is_dir() || !opts.review_dir.join(STATE_FILENAME).exists() {
        return Err(fail("not-initialized", "先 init"));
    }
    // Read under the same lock the writers take, so that the check and
    // the answer are of one state.
    let _lock = review_lock(&opts.review_dir, true)?;
    let state = validated_state(load_state(&opts.review_dir)?, &opts.review_dir_text)?;
    if !state.truthy() {
        return Err(fail("not-initialized", "先 init"));
    }
    verify_no_tamper(&state)?;
    Ok(build_status(&state))
}

// ---------------------------------------------------------------------------
// output and entry
// ---------------------------------------------------------------------------

fn render_human(answer: &J) -> String {
    if let Some(error) = answer.get("error") {
        return format!(
            "错误 [{}]: {}",
            error.at("code").shown(),
            error.at("message").shown()
        );
    }
    let mut out = vec!["== olp-review-evidence 状态 ==".to_string()];
    let identity = answer.at("identity");
    if identity.truthy() {
        let of = |key: &str| identity.at(key).shown();
        out.push(format!(
            "identity: head={} base={} runtime={} session={} goal={}",
            of("head"),
            of("base"),
            of("runtime"),
            of("session"),
            of("goal")
        ));
    }
    let of = |key: &str| answer.at(key).shown();
    out.push(format!(
        "lifecycle: frozen={} challenge_accepted={} review_accepted={} model_verified={}",
        of("frozen"),
        of("challenge_accepted"),
        of("review_accepted"),
        of("model_verified")
    ));
    let verdicts = answer.at("verdicts");
    if verdicts.truthy() {
        out.push("verdicts:".to_string());
        let mut sorted: Vec<&(String, J)> = verdicts.pairs().into_iter().flatten().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (claim, verdict) in sorted {
            let reason = verdict.get("reason").map(J::shown).unwrap_or_default();
            out.push(format!(
                "  - {claim}: {} ({reason})",
                verdict.at("state").shown()
            ));
        }
    }
    let cross = answer.at("cross");
    if cross.truthy() {
        let slugs: Vec<String> = cross
            .items()
            .into_iter()
            .flatten()
            .map(|c| c.get("slug").map_or("?".to_string(), J::shown))
            .collect();
        out.push(format!("cross: {}", slugs.join(", ")));
    }
    if !identity.truthy() && !verdicts.truthy() && answer.at("state").truthy() {
        out.push(format!("state: {}", of("state")));
    }
    if answer.at("observed").truthy() {
        out.push(format!("observed: {}", of("observed")));
    }
    out.join("\n")
}

/// `argv` is what follows `olp review`.
pub fn run_cli(argv: &[String]) -> i32 {
    let mut rest = argv;
    let mut human = false;
    // `--format` belongs before the subcommand.
    loop {
        match rest {
            [flag, value, tail @ ..] if flag == "--format" => {
                match value.as_str() {
                    "human" => human = true,
                    "json" => human = false,
                    other => {
                        eprintln!("olp review: --format 只能是 json 或 human: {other}\n{USAGE}");
                        return 2;
                    }
                }
                rest = tail;
            }
            [flag, tail @ ..] if flag.starts_with("--format=") => {
                match &flag["--format=".len()..] {
                    "human" => human = true,
                    "json" => human = false,
                    other => {
                        eprintln!("olp review: --format 只能是 json 或 human: {other}\n{USAGE}");
                        return 2;
                    }
                }
                rest = tail;
            }
            _ => break,
        }
    }
    let Some((command, args)) = rest.split_first() else {
        eprintln!("{USAGE}");
        return 2;
    };
    if matches!(command.as_str(), "-h" | "--help")
        || args.iter().any(|a| a == "-h" || a == "--help")
    {
        println!("{USAGE}");
        return 0;
    }
    let opts = match parse(command, args) {
        Ok(opts) => opts,
        Err(problem) => {
            eprintln!("olp review: {problem}\n{USAGE}");
            return 2;
        }
    };
    let answer = match command.as_str() {
        "init" => cmd_init(&opts),
        "freeze" => cmd_freeze(&opts),
        "challenge" => cmd_challenge(&opts),
        "cross" => cmd_cross(&opts),
        "status" => cmd_status(&opts),
        _ => super::reviewclassify::classify(&opts),
    };
    let (code, answer) = match answer {
        Ok(answer) => (0, answer),
        Err(refusal) => (
            1,
            obj(vec![(
                "error",
                obj(vec![
                    ("code", J::from(refusal.code)),
                    ("message", J::from(refusal.message)),
                ]),
            )]),
        ),
    };
    let body = if human {
        render_human(&answer)
    } else {
        answer.pretty()
    };
    if writeln!(std::io::stdout().lock(), "{body}").is_err() {
        return 1;
    }
    code
}

/// The model gate on made-up ledgers (not review receipts): which turns
/// count, and that a report is joined to the ledger by the runtime's turn
/// id only.
#[cfg(test)]
mod model_gate {
    use super::super::reviewpeer::model_from_ledger;
    use super::*;
    use crate::olp::reviewstate::sha256_bytes;

    const SESSION: &str = "fixture:local:tui#master";

    fn hex(text: &str) -> String {
        text.bytes().map(|b| format!("{b:02x}")).collect()
    }

    fn write_rows(path: &Path, rows: &[J]) {
        let text: String = rows.iter().map(|r| r.canonical() + "\n").collect();
        std::fs::write(path, text).unwrap();
    }

    fn read_rows(path: &Path) -> Vec<J> {
        let text = std::fs::read_to_string(path).unwrap();
        text.lines().map(|l| J::parse(l).unwrap()).collect()
    }

    /// A peer's ledger: per model one turn of start, model, completed.
    fn ledger(root: &Path, slug: &str, models: &[&str]) -> PathBuf {
        let session = format!("fixture:local:tui#peer-{slug}");
        let wire = format!("{session}\0~cwd-123abc");
        let path = root
            .join("ui-protocol")
            .join(hex(&wire))
            .join("ledger-0001.log");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut rows = Vec::new();
        for (n, model) in models.iter().enumerate() {
            for kind in ["turn_started", "model", "turn_completed"] {
                let mut event = obj(vec![
                    ("record_kind", J::from("notification")),
                    ("kind", J::from(kind)),
                    ("session_id", J::from(session.as_str())),
                    ("turn_id", J::from(format!("{slug}-{}", n + 1))),
                ]);
                if kind == "model" {
                    event.set("record_kind", J::from("progress"));
                    event.set(
                        "metadata",
                        obj(vec![
                            ("kind", J::from("token_cost_update")),
                            ("token_cost", obj(vec![("model", J::from(*model))])),
                        ]),
                    );
                }
                rows.push(obj(vec![
                    ("seq", J::from(rows.len() as i64 + 1)),
                    ("event", event),
                ]));
            }
        }
        write_rows(&path, &rows);
        path
    }

    fn native_report(root: &Path, slug: &str, number: u64, turn_id: Option<&str>) -> J {
        let path = root
            .join("native")
            .join(slug)
            .join(format!("result-{number}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let turn_id = turn_id.map_or(format!("{slug}-{number}"), str::to_string);
        let body = format!(
            "---\nslug: {slug}\noutcome: completed\nturn: {number}\nturn_id: {turn_id}\n---\nreport\n"
        );
        std::fs::write(&path, &body).unwrap();
        obj(vec![
            ("path", J::from(show(&path))),
            ("sha256", J::from(sha256_bytes(body.as_bytes()))),
        ])
    }

    struct Fx {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        state: J,
    }

    impl Fx {
        fn new() -> Fx {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().to_path_buf();
            ledger(&root, "glm", &["glm-5.3", "glm-5.3"]);
            ledger(&root, "k3", &["k3-256k", "k3-256k"]);
            let mut reviews = obj(vec![]);
            for lane in LANES {
                reviews.set(
                    lane,
                    obj(vec![
                        ("peer", J::from(lane)),
                        ("turn", J::from("1")),
                        ("native_report", native_report(&root, lane, 1, None)),
                    ]),
                );
            }
            // Model acceptance alone: the behaviour side is made to hold.
            let state = obj(vec![
                ("runtime", J::from(show(&root))),
                ("session", J::from(SESSION)),
                ("frozen", J::from(true)),
                ("reviews", reviews),
                (
                    "challenges",
                    obj(vec![(
                        "X",
                        obj(vec![("latest", obj(vec![("accepted", J::from(true))]))]),
                    )]),
                ),
                (
                    "verdicts",
                    obj(vec![(
                        "X",
                        obj(vec![
                            ("state", J::from("approve")),
                            ("reason", J::from("executed-probe-passed")),
                        ]),
                    )]),
                ),
                ("cross", J::Arr(vec![])),
            ]);
            let mut fx = Fx {
                _tmp: tmp,
                root,
                state,
            };
            for lane in LANES {
                let binding = native_report(&fx.root, lane, 2, None);
                let evidence = fx.pair(lane, "2", &binding).unwrap();
                fx.cross().push(obj(vec![
                    ("slug", J::from(lane)),
                    ("turn", J::from("2")),
                    ("native_report", binding),
                    ("model_evidence", evidence),
                ]));
            }
            fx
        }

        fn cross(&mut self) -> &mut Vec<J> {
            match self.state.get_mut("cross") {
                Some(J::Arr(items)) => items,
                _ => panic!("cross is a list"),
            }
        }

        fn review(&mut self, lane: &str) -> &mut J {
            self.state.child("reviews").child(lane)
        }

        fn pair(&self, lane: &str, turn: &str, binding: &J) -> Res<J> {
            model_pair(&self.state, &J::from(lane), &J::from(turn), binding)
        }

        fn ledger_path(&self, slug: &str) -> PathBuf {
            let wire = format!("fixture:local:tui#peer-{slug}\0~cwd-123abc");
            self.root
                .join("ui-protocol")
                .join(hex(&wire))
                .join("ledger-0001.log")
        }

        fn accepted(&self) -> bool {
            build_status(&self.state).at("review_accepted").is_true()
        }

        #[track_caller]
        fn assert_blocked(&self) {
            let status = build_status(&self.state);
            assert!(status.at("behavior_accepted").is_true());
            assert!(!status.at("model_verified").is_true());
            assert!(!status.at("review_accepted").is_true());
        }
    }

    fn code_of(result: Res<J>) -> String {
        result.err().map(|e| e.code).unwrap_or_default()
    }

    #[test]
    fn completed_write_loss_cannot_borrow_an_earlier_model() {
        // A completed glm turn lost its result file: result-2 is really
        // the ledger's third turn, written by another model.
        let mut fx = Fx::new();
        ledger(&fx.root, "glm", &["glm-5.3", "glm-5.3", "k3-256k"]);
        let first = native_report(&fx.root, "glm", 1, Some("glm-1"));
        let second = native_report(&fx.root, "glm", 2, Some("glm-3"));
        fx.review("glm").set("native_report", first);
        fx.cross()[0].set("native_report", second);
        assert!(!models_verified(&fx.state));
    }

    #[test]
    fn malformed_cross_entries_fail_closed() {
        for entry in [
            "null",
            "\"bad\"",
            "[]",
            "[\"not\", \"a\", \"dict\"]",
            "{\"slug\": []}",
            "{\"slug\": {}}",
        ] {
            let mut fx = Fx::new();
            *fx.cross() = vec![J::parse(entry).unwrap()];
            assert!(!models_verified(&fx.state), "{entry}");
            assert!(!fx.accepted(), "{entry}");
        }
    }

    #[test]
    fn initial_state_turn_cannot_redirect_frozen_native_identity() {
        // The frozen report is untouched; only the copied turn number is
        // edited to skip a first turn by a foreign model.
        let mut fx = Fx::new();
        ledger(
            &fx.root,
            "glm",
            &["some-foreign-model", "glm-5.3", "glm-5.3"],
        );
        let third = native_report(&fx.root, "glm", 3, None);
        fx.review("glm").set("turn", J::from("2"));
        fx.cross()[0].set("turn", J::from("3"));
        fx.cross()[0].set("native_report", third.clone());
        let refusal = fx.pair("glm", "3", &third).unwrap_err();
        assert!(
            refusal.message.contains("编号不匹配"),
            "{}",
            refusal.message
        );
        fx.assert_blocked();
    }

    #[test]
    fn shifted_correct_model_uses_actual_turn_identity() {
        let mut fx = Fx::new();
        let path = ledger(&fx.root, "glm", &["glm-5.3", "glm-5.3", "glm-5.3"]);
        let binding = native_report(&fx.root, "glm", 2, Some("glm-3"));
        let evidence = fx.pair("glm", "2", &binding).unwrap();
        assert!(evidence.at("cross").at("turn_id").is("glm-3"));
        assert_eq!(evidence.at("cross").at("turn_no").int(), Some(2));
        assert_eq!(evidence.at("cross").at("ledger_turn_no").int(), Some(3));
        fx.cross()[0].set("native_report", binding.clone());
        fx.cross()[0].set("model_evidence", evidence);
        assert!(models_verified(&fx.state));
        // Counting files or terminals would join wrongly once an earlier
        // turn wrote no terminal; the ids still join rightly.
        let mut rows = read_rows(&path);
        for at in [1, 2] {
            rows[at].set(
                "event",
                obj(vec![
                    ("record_kind", J::from("notification")),
                    ("kind", J::from("envelope")),
                ]),
            );
        }
        write_rows(&path, &rows);
        let first = native_report(&fx.root, "glm", 1, Some("glm-2"));
        fx.review("glm").set("native_report", first);
        let evidence = fx.pair("glm", "2", &binding).unwrap();
        fx.cross()[0].set("model_evidence", evidence);
        assert!(models_verified(&fx.state));
    }

    #[test]
    fn native_report_identity_rejects_missing_unknown_reused_and_reversed_ids() {
        for (initial, later) in [
            ("", "glm-2"),
            ("glm-1", ""),
            ("glm-1", "unknown"),
            ("glm-1", "k3-2"),
            ("glm-1", "glm-1"),
            ("glm-2", "glm-1"),
        ] {
            let mut fx = Fx::new();
            let first = native_report(&fx.root, "glm", 1, Some(initial));
            fx.review("glm").set("native_report", first);
            let binding = native_report(&fx.root, "glm", 2, Some(later));
            assert_eq!(
                code_of(fx.pair("glm", "2", &binding)),
                "peer-model-unverified",
                "{initial} {later}"
            );
        }
    }

    #[test]
    fn legacy_and_changed_native_reports_cannot_verify() {
        let mut fx = Fx::new();
        let original = fx.state.clone();
        for initial in [true, false] {
            fx.state = original.clone();
            let rec = if initial {
                fx.review("glm")
            } else {
                &mut fx.cross()[0]
            };
            if let J::Obj(pairs) = rec {
                pairs.retain(|(key, _)| key != "native_report");
            }
            assert!(!models_verified(&fx.state));
        }
        fx.state = original;
        for number in [1, 2] {
            let path = fx.root.join(format!("native/glm/result-{number}.md"));
            let before = std::fs::read(&path).unwrap();
            let mut changed = before.clone();
            changed.extend_from_slice(b"changed report\n");
            std::fs::write(&path, changed).unwrap();
            assert!(!models_verified(&fx.state));
            std::fs::write(&path, &before).unwrap();
            assert!(models_verified(&fx.state));
            std::fs::remove_file(&path).unwrap();
            assert!(!models_verified(&fx.state));
            std::fs::write(&path, &before).unwrap();
        }
    }

    #[test]
    fn duplicate_native_identity_keys_rejected() {
        let fx = Fx::new();
        let path = fx.root.join("native/glm/result-2.md");
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("turn_id: glm-2", "turn_id: glm-1\nturn_id: glm-2");
        std::fs::write(&path, &text).unwrap();
        let binding = obj(vec![
            ("path", J::from(show(&path))),
            ("sha256", J::from(sha256_bytes(text.as_bytes()))),
        ]);
        assert!(fx.pair("glm", "2", &binding).is_err());
    }

    #[test]
    fn malformed_cross_fields_are_a_shape_error() {
        for entry in [
            "{\"slug\": []}",
            "{\"slug\": {}}",
            "{\"slug\": \"glm\", \"turn\": []}",
            "{\"slug\": \"glm\", \"turn\": \"²\"}",
            "{\"slug\": \"glm\", \"turn\": true}",
        ] {
            let state = obj(vec![("cross", J::Arr(vec![J::parse(entry).unwrap()]))]);
            let refusal = validated_state(state, "dir").unwrap_err();
            assert_eq!(refusal.code, "state-shape-invalid", "{entry}");
        }
    }

    #[test]
    fn dual_completed_and_initial_reviews_required() {
        let fx = Fx::new();
        assert!(fx.accepted());
        ledger(&fx.root, "k3", &["glm-5.3", "k3-256k"]);
        let binding = fx.state.at("cross").items().unwrap()[1]
            .at("native_report")
            .clone();
        assert_eq!(code_of(fx.pair("k3", "2", &binding)), "peer-model-mismatch");
        fx.assert_blocked();
    }

    #[test]
    fn missing_peer_or_deleted_anchor_blocks() {
        let mut fx = Fx::new();
        let original = fx.state.clone();
        fx.cross().pop();
        let status = build_status(&fx.state);
        assert!(!status.at("model_verified").is_true());
        assert!(!status.at("review_accepted").is_true());
        fx.state = original;
        if let J::Obj(pairs) = &mut fx.cross()[0] {
            pairs.retain(|(key, _)| key != "model_evidence");
        }
        fx.assert_blocked();
    }

    #[test]
    fn old_turn_cannot_replace_cross() {
        let mut fx = Fx::new();
        let initial = fx.cross()[0].at("model_evidence").at("initial").clone();
        fx.cross()[0].child("model_evidence").set("cross", initial);
        fx.assert_blocked();
        fx.cross()[0].set("turn", J::from("1"));
        fx.assert_blocked();
    }

    #[test]
    fn runtime_disappearance_and_raw_model_drift() {
        let fx = Fx::new();
        // Even a change within the family invalidates what was stored.
        ledger(&fx.root, "k3", &["k3-256k", "k3-new"]);
        fx.assert_blocked();
        std::fs::remove_file(fx.ledger_path("k3")).unwrap();
        fx.assert_blocked();
    }

    #[test]
    fn failed_interrupted_and_output_chunks_are_not_completion() {
        let fx = Fx::new();
        let path = fx.ledger_path("k3");
        let original = read_rows(&path);
        for kind in ["turn_error", "turn_interrupted", "stream_end", "response"] {
            let mut rows = original.clone();
            rows.last_mut()
                .unwrap()
                .child("event")
                .set("kind", J::from(kind));
            write_rows(&path, &rows);
            fx.assert_blocked();
        }
    }

    #[test]
    fn foreign_profile_rows_and_ambiguous_cwd_rejected() {
        let fx = Fx::new();
        let path = fx.ledger_path("k3");
        let mut rows = read_rows(&path);
        let at = rows.len() - 2;
        rows[at]
            .child("event")
            .set("session_id", J::from("foreign:local:tui#peer-k3"));
        write_rows(&path, &rows);
        fx.assert_blocked();
        ledger(&fx.root, "k3", &["k3-256k", "k3-256k"]);
        assert!(fx.accepted());
        let other = fx
            .root
            .join("ui-protocol")
            .join(hex("fixture:local:tui#peer-k3\0~cwd-ffffff"));
        std::fs::create_dir(&other).unwrap();
        std::fs::copy(&path, other.join("ledger-1.log")).unwrap();
        fx.assert_blocked();
    }

    #[test]
    fn malformed_and_missing_sequences_fail_closed() {
        let fx = Fx::new();
        let path = fx.ledger_path("k3");
        let original = std::fs::read_to_string(&path).unwrap();
        for value in ["null", "true", "0", "\"5\""] {
            let mut rows = read_rows(&path);
            let at = rows.len() - 2;
            rows[at].set("seq", J::parse(value).unwrap());
            write_rows(&path, &rows);
            fx.assert_blocked();
            std::fs::write(&path, &original).unwrap();
        }
        std::fs::write(&path, format!("{original}{{broken\n")).unwrap();
        fx.assert_blocked();
        let tail: Vec<&str> = original.lines().skip(1).collect();
        std::fs::write(&path, tail.join("\n") + "\n").unwrap();
        fx.assert_blocked();
    }

    #[test]
    fn missing_start_or_conflicting_terminal_rejected() {
        let fx = Fx::new();
        let path = fx.ledger_path("k3");
        let mut rows = read_rows(&path);
        let at = rows.len() - 3;
        rows[at].child("event").set("kind", J::from("envelope"));
        write_rows(&path, &rows);
        fx.assert_blocked();
        ledger(&fx.root, "k3", &["k3-256k", "k3-256k"]);
        let mut rows = read_rows(&path);
        let mut event = rows.last().unwrap().at("event").clone();
        event.set("kind", J::from("turn_error"));
        rows.push(obj(vec![("seq", J::from(7i64)), ("event", event)]));
        write_rows(&path, &rows);
        fx.assert_blocked();
    }

    #[test]
    fn rotated_complete_tail_is_not_renumbered() {
        let fx = Fx::new();
        let path = fx.ledger_path("k3");
        let rows = read_rows(&path);
        // Rotation is safe while every segment is still there.
        write_rows(&path, &rows[..3]);
        write_rows(&path.with_file_name("ledger-0002.log"), &rows[3..]);
        assert!(fx.accepted());
        // Retention deleted the first complete turn.
        std::fs::remove_file(&path).unwrap();
        fx.assert_blocked();
        let refusal = model_from_ledger(&fx.root, SESSION, "k3", "k3", 1, "k3-1").unwrap_err();
        assert_eq!(refusal.code, "peer-model-unverified");
        assert!(refusal.message.contains("新建短 reviewer peer"));
        assert!(refusal.message.contains("初审"));
    }

    #[test]
    fn model_name_is_not_a_freeform_prefix() {
        let fx = Fx::new();
        ledger(&fx.root, "k3", &["k3-256k", "k3pretending"]);
        let binding = fx.state.at("cross").items().unwrap()[1]
            .at("native_report")
            .clone();
        assert_eq!(code_of(fx.pair("k3", "2", &binding)), "peer-model-mismatch");
    }

    #[test]
    fn legacy_history_can_be_superseded_but_latest_unverified_blocks() {
        let mut fx = Fx::new();
        let legacy = obj(vec![("slug", J::from("glm")), ("turn", J::from("2"))]);
        fx.cross().insert(0, legacy.clone());
        assert!(fx.accepted());
        fx.cross().push(legacy);
        fx.assert_blocked();
    }

    #[test]
    fn lifecycle_and_model_rows_without_turn_id_rejected() {
        let fx = Fx::new();
        let path = fx.ledger_path("k3");
        let original = read_rows(&path);
        for at in 0..original.len() {
            let mut rows = original.clone();
            if let J::Obj(pairs) = rows[at].child("event") {
                pairs.retain(|(key, _)| key != "turn_id");
            }
            write_rows(&path, &rows);
            fx.assert_blocked();
        }
    }

    #[test]
    fn runtime_identity_shapes_fail_closed() {
        let fx = Fx::new();
        let binding = fx.state.at("cross").items().unwrap()[0]
            .at("native_report")
            .clone();
        for key in ["runtime", "session"] {
            for value in ["null", "\"\"", "42", "{\"unexpected\": true}", "true"] {
                let mut broken = Fx {
                    _tmp: tempfile::tempdir().unwrap(),
                    root: fx.root.clone(),
                    state: fx.state.clone(),
                };
                broken.state.set(key, J::parse(value).unwrap());
                broken.assert_blocked();
                assert!(broken.pair("glm", "2", &binding).is_err(), "{key}={value}");
            }
        }
    }

    #[test]
    fn behavior_failure_still_blocks_valid_models() {
        let mut fx = Fx::new();
        fx.state
            .child("verdicts")
            .set("X", obj(vec![("state", J::from("blocked-on-evidence"))]));
        let status = build_status(&fx.state);
        assert!(status.at("model_verified").is_true());
        assert!(!status.at("review_accepted").is_true());
    }
}
