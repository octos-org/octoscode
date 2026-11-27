//! `octoscode olp review classify --manifest M --replay-summary S
//! [--review-dir D]… [--slot F --slot-log-dir D]` — what a replay of the
//! probes says about each pull request.
//!
//! The manifest lists the pull requests (head, base, the outer reviewer's
//! recommendation); the replay summary lists, per test, what a run
//! observed. Nothing in either is believed as written:
//!
//! - A result counts only when its receipt is on disk, agrees with the
//!   summary and the manifest's head, its output files hash to what it
//!   says, the output itself shows the test's line and the summary line —
//!   and the receipt is one that an accepted `challenge --live-cargo` of a
//!   review directory named with `--review-dir` recorded. A consistent
//!   set of files from anywhere else proves nothing. Anything short of
//!   that is a fault of the evidence, not a product failure.
//! - A failure is `existing` only with a slot: the same probe run on the
//!   base and on the head, both failing, both logs matching their hashes
//!   and both runs registered as above. Without it: `unassessed`.
//! - A pull request is `residual` when every failure is `existing` and no
//!   evidence is faulty, `clean` when everything passed and nothing is
//!   unassessed (which no supported evidence can show today), else
//!   `blocked`.
//!
//! Pull requests are never told apart by number: everything is derived
//! from the manifest's entries.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::reviewevidence::{Opts, validated_state, verify_no_tamper};
use super::reviewstate::{
    J, PROTOCOL, Res, ReviewLock, STATE_FILENAME, fail, obj, path_of, read_lossy, read_strict,
    resolve, resolve_head, review_lock, sha256_file, show, strings,
};

/// A run that an accepted live challenge of a trusted review recorded.
struct Trusted {
    /// The receipt's real path.
    receipt: String,
    /// The receipt's fields, with what the review's record adds.
    executed: J,
}

type Registry = BTreeMap<String, Trusted>;

/// `ok`, `FAILED` or `ignored` as the output shows them for one test.
/// libtest may put the status on a later line; then a panic and a bare
/// `FAILED` line count as failed.
fn test_statuses(text: &str, qualified: &str) -> Vec<&'static str> {
    let mut statuses = Vec::new();
    let mut anchored = false;
    for line in text.split('\n') {
        let Some(rest) = line.strip_prefix("test ") else {
            continue;
        };
        let name_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (name, tail) = rest.split_at(name_len);
        let Some(status) = tail.strip_prefix(" ... ") else {
            continue;
        };
        let status = ["ok", "FAILED", "ignored"]
            .into_iter()
            .find(|s| status.starts_with(s));
        if let (Some(status), true) = (status, name == qualified && !name.is_empty()) {
            statuses.push(status);
            anchored = true;
        }
    }
    if !anchored
        && text.contains(&format!("test {qualified} ..."))
        && text.contains("panicked at")
        && text.split('\n').any(|line| line == "FAILED")
    {
        statuses.push("FAILED");
    }
    statuses
}

fn has_line_starting(text: &str, start: &str) -> bool {
    text.split('\n').any(|line| line.starts_with(start))
}

fn load_object(path: &Path, code: &str) -> Res<J> {
    let value = read_strict(path)
        .map_err(|e| e.to_string())
        .and_then(|text| J::parse(&text).map_err(|e| e.to_string()))
        .map_err(|e| fail(code, format!("无法读取 {}: {e}", path.display())))?;
    if !value.is_obj() {
        return Err(fail(code, format!("{} 顶层不是对象", path.display())));
    }
    Ok(value)
}

/// A whole number that is not a boolean.
fn real_int(value: &J) -> bool {
    value.int().is_some()
}

fn zero(value: &J) -> bool {
    value.same(&J::from(0i64))
}

fn last_segment(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

// ---------------------------------------------------------------------------
// where a receipt comes from
// ---------------------------------------------------------------------------

fn context_invalid(message: String) -> super::reviewstate::Fail {
    fail("classify-context-invalid", message)
}

/// The accepted live runs of the review directories the caller trusts, by
/// the hash of their receipts. Each directory must be a frozen review
/// whose repository is at the review's head and in which nothing was
/// changed; each receipt must lie in that review's `live-evidence/` and
/// hash to what was recorded.
fn load_trusted(review_dirs: &[PathBuf]) -> Res<Registry> {
    let mut index = Registry::new();
    for dir in review_dirs {
        let rd = show(dir);
        if !dir.is_dir() {
            return Err(fail(
                "classify-context-missing",
                format!("受信上下文不存在: {rd}"),
            ));
        }
        let state_file = dir.join(STATE_FILENAME);
        let raw = state_file
            .exists()
            .then(|| read_strict(&state_file).ok())
            .flatten();
        let Some(raw) = raw else {
            return Err(fail(
                "classify-context-missing",
                format!("受信上下文缺 review-state.json: {rd}"),
            ));
        };
        let state = J::parse(&raw)
            .map_err(|e| context_invalid(format!("受信上下文 state 非合法 JSON: {e}: {rd}")))?;
        let state = validated_state(state, &rd)
            .map_err(|e| context_invalid(format!("受信上下文 state 形状非法: {}", e.message)))?;
        if !state.at("frozen").is_true() {
            return Err(fail(
                "classify-context-missing",
                format!("受信上下文未冻结(不可作信任来源): {rd}"),
            ));
        }
        if !state.at("reviews").truthy() {
            return Err(context_invalid(format!(
                "受信上下文 state.reviews 缺失/形状非法(须非空对象): {rd}"
            )));
        }
        // The repository really is at the head the review is about.
        let (Some(repo), Some(head)) = (state.at("repo").text(), state.at("head").text()) else {
            return Err(context_invalid(format!(
                "受信上下文 state.repo/head 缺失/非法: {rd}"
            )));
        };
        let real = resolve_head(Path::new(repo))
            .map_err(|_| context_invalid(format!("受信上下文 repo HEAD 不可解析: {rd}")))?;
        if real != head {
            return Err(context_invalid(format!(
                "受信上下文 repo 真实 HEAD 与 state.head 不符: {rd}"
            )));
        }
        verify_no_tamper(&state).map_err(|_| {
            fail(
                "classify-context-missing",
                format!("受信上下文 state 校验失败(tamper): {rd}"),
            )
        })?;
        let live_dir = resolve(&dir.join("live-evidence"));
        for (_, entry) in state.at("challenges").pairs().into_iter().flatten() {
            for rec in entry.at("history").items().into_iter().flatten() {
                if !rec.at("accepted").is_true() {
                    continue;
                }
                let executed = rec.at("executed");
                let (Some(receipt), Some(sha256), true) = (
                    rec.at("receipt").text(),
                    rec.at("receipt_sha256").text(),
                    executed.is_obj(),
                ) else {
                    return Err(context_invalid(format!("accepted live 记录形状非法: {rd}")));
                };
                let real = resolve(&path_of(receipt));
                if !real.starts_with(&live_dir) {
                    return Err(context_invalid(format!(
                        "accepted receipt 不在受信 live-evidence 内: {receipt}"
                    )));
                }
                if !real.is_file() {
                    return Err(context_invalid(format!(
                        "accepted receipt 文件缺失: {receipt}"
                    )));
                }
                if sha256_file(&real).ok().as_deref() != Some(sha256) {
                    return Err(context_invalid(format!(
                        "accepted receipt 与注册 sha256 不符(篡改): {receipt}"
                    )));
                }
                let rc = read_strict(&real)
                    .ok()
                    .and_then(|text| J::parse(&text).ok())
                    .filter(J::is_obj)
                    .unwrap_or_else(|| obj(vec![]));
                // A receipt of another head does not belong to this review.
                if !rc.at("head_before").is(head) || !rc.at("head_after").is(head) {
                    return Err(context_invalid(format!(
                        "accepted receipt head 与所属上下文 state.head 不符: {receipt}"
                    )));
                }
                // The receipt is the base; the record may add to it but
                // not contradict it.
                for field in [
                    "observed",
                    "exit_code",
                    "selector",
                    "selector_qualified",
                    "test_target_sha256",
                    "stdout_sha256",
                    "head_before",
                    "head_after",
                ] {
                    let (theirs, ours) = (rc.at(field), executed.at(field));
                    if !theirs.is_null() && !ours.is_null() && !theirs.same(ours) {
                        return Err(context_invalid(format!(
                            "accepted 记录与 receipt 核心字段 {field} 不一致: {receipt}"
                        )));
                    }
                }
                let mut merged = rc.clone();
                for (key, value) in executed.pairs().into_iter().flatten() {
                    if !value.is_null() {
                        merged.set(key, value.clone());
                    }
                }
                index.insert(
                    sha256.to_string(),
                    Trusted {
                        receipt: show(&real),
                        executed: merged,
                    },
                );
            }
        }
    }
    Ok(index)
}

/// The registered run a receipt path belongs to: the file's hash must be
/// registered, and the path must be the registered file itself — a copy
/// with the same content somewhere else is not it.
fn provenance<'a>(receipt: &J, trusted: &'a Registry) -> Result<&'a Trusted, &'static str> {
    let Some(receipt) = receipt.text() else {
        return Err("summary-receipt-path-missing");
    };
    let path = path_of(receipt);
    if !path.is_file() {
        return Err("receipt-file-missing");
    }
    let sha256 = sha256_file(&path).map_err(|_| "receipt-file-missing")?;
    let Some(known) = trusted.get(&sha256) else {
        return Err("receipt-not-registered-in-trusted-context");
    };
    if show(&resolve(&path)) != known.receipt {
        return Err("receipt-path-not-registered-copy");
    }
    Ok(known)
}

// ---------------------------------------------------------------------------
// one test's result
// ---------------------------------------------------------------------------

/// A summary row whose receipt bears it out.
struct Verified {
    selector: String,
    qualified: String,
    test_target_sha256: J,
}

/// Check one summary row against its receipt. An error is a fault of the
/// evidence: a missing, foreign or contradictory receipt.
fn verify_product_receipt(row: &J, manifest_head: Option<&str>) -> Result<Verified, String> {
    let Some(sel) = row.at("selector").text() else {
        return Err("summary-selector-missing".to_string());
    };
    let why = |what: &str| format!("{sel}:{what}");
    let Some(receipt) = row.at("receipt").text() else {
        return Err(why("summary-receipt-path-missing"));
    };
    let path = path_of(receipt);
    if !path.is_file() {
        return Err(why("receipt-file-missing"));
    }
    let Some(rc) = read_strict(&path).ok().and_then(|t| J::parse(&t).ok()) else {
        return Err(why("receipt-unreadable"));
    };
    if !rc.is_obj() {
        return Err(why("receipt-not-object"));
    }
    if !rc.at("receipt_kind").is("cargo-test-execution") {
        return Err(why(&format!(
            "receipt-kind-not-cargo-test-execution({})",
            rc.at("receipt_kind").repr()
        )));
    }
    if !rc.at("selector").is(sel) {
        return Err(why(&format!(
            "receipt-selector-mismatch({})",
            rc.at("selector").repr()
        )));
    }
    let matched = rc.at("matched_tests");
    let Some(names) = matched.items().filter(|names| !names.is_empty()) else {
        return Err(why("receipt-matched-tests-missing"));
    };
    let names: Vec<&str> = names.iter().filter_map(J::str).collect();
    if !names
        .iter()
        .any(|name| last_segment(name) == sel || *name == sel)
    {
        return Err(why("receipt-matched-tests-not-selector"));
    }
    let observed = row.at("observed");
    let Some(seen) = observed.str().filter(|o| ["pass", "fail"].contains(o)) else {
        return Err(why(&format!(
            "summary-observed-invalid({})",
            observed.repr()
        )));
    };
    if !rc.at("observed").is(seen) {
        return Err(why(&format!(
            "receipt-observed-mismatch({})",
            rc.at("observed").repr()
        )));
    }
    let Some(head) = row.at("head").text() else {
        return Err(why("summary-head-missing"));
    };
    if !rc.at("head_before").is(head) || !rc.at("head_after").is(head) {
        return Err(why("receipt-head-mismatch"));
    }
    if let Some(theirs) = manifest_head.filter(|theirs| *theirs != head) {
        let short = |text: &str| text.chars().take(8).collect::<String>();
        return Err(why(&format!(
            "summary-head-not-manifest({}!={})",
            short(head),
            short(theirs)
        )));
    }
    let cargo_exit = row.at("cargo_exit");
    if seen == "fail" {
        if !real_int(cargo_exit) || zero(cargo_exit) {
            return Err(why("summary-fail-with-zero-exit"));
        }
        if !rc.at("exit_code").same(cargo_exit) {
            return Err(why("receipt-exit-mismatch"));
        }
    } else {
        if !zero(rc.at("exit_code")) {
            return Err(why("receipt-pass-with-nonzero-exit"));
        }
        // A pass says exit 0 itself; `false` is not 0.
        if matches!(cargo_exit, J::Bool(_)) || !zero(cargo_exit) {
            return Err(why(&format!(
                "summary-pass-with-cargo-exit({})",
                cargo_exit.repr()
            )));
        }
    }
    let artifacts = rc.at("artifacts");
    if !artifacts.is_obj() {
        return Err(why("receipt-artifacts-missing"));
    }
    for key in ["stdout", "stderr"] {
        let Some(artifact) = artifacts.at(key).text() else {
            return Err(why(&format!("receipt-artifact-{key}-missing")));
        };
        let file = path_of(artifact);
        if !file.is_file() {
            return Err(why(&format!("receipt-artifact-{key}-file-missing")));
        }
        let declared = rc.at(&format!("{key}_sha256")).str();
        if declared.is_none() || sha256_file(&file).ok().as_deref() != declared {
            return Err(why(&format!("receipt-artifact-{key}-hash-mismatch")));
        }
    }
    // What the test did is read from the output itself, never from what
    // the summary or the receipt say about it.
    let qualified = match rc.at("selector_qualified").text() {
        Some(qualified) => Some(qualified),
        None if names.len() == 1 => names.first().copied(),
        None => None,
    };
    let Some(qualified) = qualified.filter(|q| !q.is_empty()) else {
        return Err(why("receipt-qualified-selector-missing"));
    };
    let stdout = artifacts.at("stdout").str().unwrap_or_default();
    let Ok(text) = read_lossy(&path_of(stdout)) else {
        return Err(why("receipt-artifact-stdout-unreadable"));
    };
    let statuses = test_statuses(&text, qualified);
    let (passed, failed) = (statuses.contains(&"ok"), statuses.contains(&"FAILED"));
    if seen == "pass" {
        if !(passed && !failed && has_line_starting(&text, "test result: ok.")) {
            return Err(why("receipt-pass-claim-contradicted-by-stdout"));
        }
    } else if !(failed && !passed && has_line_starting(&text, "test result: FAILED.")) {
        return Err(why("receipt-fail-claim-contradicted-by-stdout"));
    }
    Ok(Verified {
        selector: sel.to_string(),
        qualified: qualified.to_string(),
        test_target_sha256: rc.at("test_target_sha256").clone(),
    })
}

// ---------------------------------------------------------------------------
// the same probe on base and head
// ---------------------------------------------------------------------------

/// A slot that holds: both runs failed, and both are registered.
struct SlotProof {
    base_exit: J,
    head_exit: J,
    pr_head: String,
    probe: String,
    classification: String,
    base_log: String,
    head_log: String,
}

/// The names of the functions in the slot's probe source.
fn probe_functions(slot: &J) -> Vec<String> {
    let Some(text) = slot
        .at("probe")
        .text()
        .map(path_of)
        .filter(|p| p.is_file())
        .and_then(|p| read_lossy(&p).ok())
    else {
        return Vec::new();
    };
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut names = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("fn") {
        let before = rest[..at].chars().next_back();
        let after = &rest[at + 2..];
        rest = after;
        // `fn` as a word of its own, then spaces, a name, and `(`.
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let name_start = after.trim_start();
        if name_start.len() == after.len() {
            continue;
        }
        let len = name_start.find(|c| !word(c)).unwrap_or(name_start.len());
        if len > 0 && name_start[len..].trim_start().starts_with('(') {
            names.push(name_start[..len].to_string());
        }
    }
    names
}

/// The full name of the test a failure is about, for matching it in the
/// slot's logs: the receipt's qualified name when its last part is the
/// selector; without one, the selector when the probe declares it.
fn qualified_selector(slot: &J, failure: &Verified) -> Option<String> {
    if !failure.qualified.is_empty() {
        return (last_segment(&failure.qualified) == failure.selector)
            .then(|| failure.qualified.clone());
    }
    probe_functions(slot)
        .contains(&failure.selector)
        .then(|| failure.selector.clone())
}

/// Check a slot for one failing test of one pull request. None when
/// anything is missing or does not match: then there is no comparison.
fn verify_slot(
    slot: &J,
    log_dir: Option<&Path>,
    manifest_head: Option<&str>,
    manifest_base: Option<&str>,
    qualified: &str,
    probe_sha256: &J,
    trusted: &Registry,
) -> Option<SlotProof> {
    let int_like = |value: &J| value.int().is_some() || matches!(value, J::Bool(_));
    let (base_exit, head_exit) = (slot.at("base_exit"), slot.at("head_exit"));
    let base_log_sha = slot.at("base_log_sha256").str()?;
    let head_log_sha = slot.at("head_log_sha256").str()?;
    if !int_like(base_exit) || !int_like(head_exit) {
        return None;
    }
    // This pull request's own head and base, not just any in the manifest.
    let pr_head = slot.at("pr_head").text()?;
    if Some(pr_head) != manifest_head {
        return None;
    }
    let slot_base = slot.at("base");
    if !slot_base.is_null() && slot_base.str() != manifest_base {
        return None;
    }
    // The probe's source is there and is the one that was run.
    let probe = slot.at("probe").text()?;
    let probe_path = path_of(probe);
    let probe_sha = slot.at("probe_sha256").str()?;
    if !probe_path.is_file() || sha256_file(&probe_path).ok()? != probe_sha {
        return None;
    }
    if !probe_sha256.is_null() && !probe_sha256.is(probe_sha) {
        return None;
    }
    // The two logs are found by their hashes. Two honest runs may well
    // print the same bytes, so one file can be both.
    let log_dir = log_dir.filter(|dir| dir.is_dir())?;
    let mut logs: Vec<PathBuf> = std::fs::read_dir(log_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "log"))
        .collect();
    logs.sort();
    let (mut base_log, mut head_log) = (None, None);
    for log in logs {
        let digest = sha256_file(&log).ok()?;
        if digest == base_log_sha {
            base_log = Some(log.clone());
        }
        if digest == head_log_sha {
            head_log = Some(log);
        }
    }
    let (base_log, head_log) = (base_log?, head_log?);
    // Each log must show this very test failing — not some test, and not
    // a build that did not compile.
    if qualified.is_empty() {
        return None;
    }
    for (log, exit) in [(&base_log, base_exit), (&head_log, head_exit)] {
        let text = read_lossy(log).ok()?;
        let statuses = test_statuses(&text, qualified);
        if !statuses.contains(&"FAILED")
            || statuses.contains(&"ok")
            || !has_line_starting(&text, "test result: FAILED.")
            || zero(exit)
        {
            return None;
        }
    }
    // Both runs are registered: the receipt of each side is one a trusted
    // review recorded, of that commit, that test, that probe, failing, and
    // its output is the log.
    for (side, commit, log_sha) in [
        ("base_receipt", slot_base.clone(), base_log_sha),
        ("head_receipt", J::from(pr_head), head_log_sha),
    ] {
        let executed = &provenance(slot.at(side), trusted).ok()?.executed;
        let bound = executed.at("head_before").same(&commit)
            && executed.at("selector_qualified").is(qualified)
            && (probe_sha256.is_null() || executed.at("test_target_sha256").same(probe_sha256))
            && executed.at("observed").is("fail")
            && real_int(executed.at("exit_code"))
            && !zero(executed.at("exit_code"))
            && executed.at("stdout_sha256").is(log_sha);
        if !bound {
            return None;
        }
    }
    Some(SlotProof {
        base_exit: base_exit.clone(),
        head_exit: head_exit.clone(),
        pr_head: pr_head.to_string(),
        probe: probe.to_string(),
        classification: slot.get("classification").map(J::shown).unwrap_or_default(),
        base_log: show(&base_log),
        head_log: show(&head_log),
    })
}

// ---------------------------------------------------------------------------
// per pull request
// ---------------------------------------------------------------------------

fn aggregate(
    manifest_path: &Path,
    replay_path: &Path,
    slot_path: Option<&Path>,
    slot_log_dir: Option<&Path>,
    trusted: &Registry,
) -> Res<J> {
    let manifest = load_object(manifest_path, "classify-manifest-invalid")?;
    let Some(prs) = manifest.at("prs").pairs().filter(|prs| !prs.is_empty()) else {
        return Err(fail("classify-manifest-invalid", "MANIFEST 缺 prs 对象"));
    };
    let replay = load_object(replay_path, "classify-replay-invalid")?;
    let Some(results) = replay.at("results").items().filter(|r| !r.is_empty()) else {
        return Err(fail(
            "classify-replay-invalid",
            "replay summary 缺 results 数组",
        ));
    };
    // The slot is read once and checked again for every failing test of
    // every pull request.
    let slot = match slot_path.filter(|path| path.is_file()) {
        Some(path) => Some(load_object(path, "classify-slot-invalid")?),
        None => None,
    };
    let log_dir = slot_log_dir.or(slot_path.and_then(Path::parent));

    let mut classes = Vec::new();
    for (pr, meta) in prs {
        if !meta.is_obj() {
            continue;
        }
        let rows: Vec<&J> = results
            .iter()
            .filter(|row| row.is_obj() && row.get("pr").is_some_and(|p| &p.shown() == pr))
            .collect();
        let head = meta.at("head").text();
        let base = meta.at("base").text();
        let mut failures: Vec<Verified> = Vec::new();
        let mut faults: Vec<J> = Vec::new();
        for row in &rows {
            let (observed, adapter_exit, cargo_exit) = (
                row.at("observed"),
                row.at("adapter_exit"),
                row.at("cargo_exit"),
            );
            // The receipt must also be one a trusted review recorded.
            let verified = verify_product_receipt(row, head).and_then(|verified| {
                provenance(row.at("receipt"), trusted)
                    .map(|_| verified)
                    .map_err(str::to_string)
            });
            // The adapter itself finished cleanly; what cargo returned is
            // the product's result. `false` is not 0.
            let harness_ok = real_int(adapter_exit) && zero(adapter_exit) && real_int(cargo_exit);
            match verified {
                Ok(failure) if observed.is("fail") && harness_ok && !zero(cargo_exit) => {
                    failures.push(failure);
                }
                Ok(_) if observed.is("pass") && harness_ok && zero(cargo_exit) => {}
                other => faults.push(obj(vec![
                    ("selector", row.at("selector").clone()),
                    ("observed", observed.clone()),
                    ("adapter_exit", adapter_exit.clone()),
                    ("cargo_exit", cargo_exit.clone()),
                    (
                        "reason",
                        J::from(
                            other
                                .err()
                                .unwrap_or_else(|| "summary-fields-invalid".to_string()),
                        ),
                    ),
                ])),
            }
        }
        let all_pass = !rows.is_empty() && failures.is_empty() && faults.is_empty();

        // A failure is `existing` only when the slot proves it for this
        // very test; the pull request is, when every failure is.
        let mut intro = "unassessed";
        let mut bound = J::Null;
        if let (Some(slot), false) = (&slot, failures.is_empty()) {
            let proofs: Vec<Option<SlotProof>> = failures
                .iter()
                .map(|failure| {
                    let qualified = qualified_selector(slot, failure)?;
                    verify_slot(
                        slot,
                        log_dir,
                        head,
                        base,
                        &qualified,
                        &failure.test_target_sha256,
                        trusted,
                    )
                    .filter(|proof| {
                        !zero(&proof.base_exit)
                            && !zero(&proof.head_exit)
                            && proof.classification.contains("existing")
                    })
                })
                .collect();
            let covered: Vec<String> = failures
                .iter()
                .zip(&proofs)
                .filter(|(_, proof)| proof.is_some())
                .map(|(failure, _)| failure.selector.clone())
                .collect();
            if let Some(first) = proofs.iter().flatten().next() {
                let whole = proofs.iter().all(Option::is_some);
                let mut evidence = obj(vec![
                    ("pr_head", J::from(first.pr_head.as_str())),
                    ("probe", J::from(first.probe.as_str())),
                    ("base_exit", first.base_exit.clone()),
                    ("head_exit", first.head_exit.clone()),
                    ("classification", J::from(first.classification.as_str())),
                    ("base_log", J::from(first.base_log.as_str())),
                    ("head_log", J::from(first.head_log.as_str())),
                    ("covered_selectors", strings(&covered)),
                ]);
                if whole {
                    intro = "existing";
                } else {
                    // Some failures have no comparison: what is covered is
                    // recorded, the pull request stays blocked.
                    evidence.set("partial", J::from(true));
                }
                bound = evidence;
            }
        }

        let mut state = if !failures.is_empty() || !faults.is_empty() {
            let residual = !bound.is_null()
                && !bound.at("partial").truthy()
                && intro == "existing"
                && faults.is_empty();
            if !residual && !faults.is_empty() {
                // With faulty evidence, `existing` cannot be relied on.
                intro = "unassessed";
            }
            if residual { "residual" } else { "blocked" }
        } else if all_pass {
            "clean"
        } else {
            "blocked"
        };
        // Unassessed is never clean: all passing without the comparison
        // that would show it is still blocked.
        if intro == "unassessed" && state == "clean" {
            state = "blocked";
        }

        let recommendation = meta.at("outer_recommendation");
        let aligned = match recommendation.text() {
            Some(text) => J::from((state == "clean") == (text == "approve")),
            None => J::Null,
        };
        let failed: Vec<String> = failures.iter().map(|f| f.selector.clone()).collect();
        classes.push((
            pr.clone(),
            obj(vec![
                ("classification", J::from(state)),
                ("introduced_vs_existing", J::from(intro)),
                ("outer_recommendation", recommendation.clone()),
                ("product_fail_selectors", strings(&failed)),
                (
                    "harness_fault_selectors",
                    J::Arr(faults.iter().map(|f| f.at("selector").clone()).collect()),
                ),
                ("dual_execution_evidence", bound),
                ("recommendation_aligned", aligned),
            ]),
        ));
    }
    Ok(obj(vec![
        ("protocol", J::from(PROTOCOL)),
        ("aggregation", J::from("pr-classification")),
        ("manifest", J::from(show(manifest_path))),
        ("replay_summary", J::from(show(replay_path))),
        ("slot", slot_path.map_or(J::Null, |p| J::from(show(p)))),
        ("prs", J::Obj(classes)),
    ]))
}

pub(super) fn classify(opts: &Opts) -> Res<J> {
    let manifest = path_of(opts.value("--manifest").unwrap_or_default());
    if !manifest.is_file() {
        return Err(fail(
            "classify-manifest-invalid",
            format!("MANIFEST 不存在: {}", manifest.display()),
        ));
    }
    let replay = path_of(opts.value("--replay-summary").unwrap_or_default());
    if !replay.is_file() {
        return Err(fail(
            "classify-replay-invalid",
            format!("replay summary 不存在: {}", replay.display()),
        ));
    }
    let given = |name: &str| opts.value(name).filter(|v| !v.is_empty()).map(path_of);
    let (slot, slot_log_dir) = (given("--slot"), given("--slot-log-dir"));
    // Each trusted review once, in a fixed order so that two runs never
    // wait on each other's locks; a missing one fails before any lock
    // would create it.
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in opts.all("--review-dir") {
        let real = resolve(&path_of(dir));
        if !dirs.contains(&real) {
            dirs.push(real);
        }
    }
    dirs.sort_by_key(|dir| show(dir));
    if let Some(missing) = dirs.iter().find(|dir| !dir.is_dir()) {
        return Err(fail(
            "classify-context-missing",
            format!("受信上下文不存在: {}", missing.display()),
        ));
    }
    // Held from loading the registry to the last receipt read.
    let _locks: Vec<ReviewLock> = dirs
        .iter()
        .map(|dir| review_lock(dir, false))
        .collect::<Res<_>>()?;
    let trusted = load_trusted(&dirs)?;
    aggregate(
        &manifest,
        &replay,
        slot.as_deref(),
        slot_log_dir.as_deref(),
        &trusted,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tests_status_is_read_from_its_own_line() {
        let text = "running 2 tests\ntest a::probe ... ok\ntest a::other ... FAILED\n";
        assert_eq!(test_statuses(text, "a::probe"), vec!["ok"]);
        assert_eq!(test_statuses(text, "a::other"), vec!["FAILED"]);
        assert!(test_statuses(text, "probe").is_empty());
        let folded = "test a::probe ... \nthread panicked at src/x.rs:1:1\nFAILED\n";
        assert_eq!(test_statuses(folded, "a::probe"), vec!["FAILED"]);
    }

    #[test]
    fn probe_functions_are_whole_words() {
        let dir = tempfile::tempdir().unwrap();
        let probe = dir.path().join("p.rs");
        std::fs::write(
            &probe,
            "#[test]\nfn probe_one() {}\nfn  two (x: u8) {}\nlet defn = 1;\n",
        )
        .unwrap();
        let slot = obj(vec![("probe", J::from(show(&probe)))]);
        assert_eq!(probe_functions(&slot), vec!["probe_one", "two"]);
    }
}
