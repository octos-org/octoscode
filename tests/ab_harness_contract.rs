//! Contract tests for `specs/task-ab-atomcode-octoscode-harness.spec.md`.
//!
//! ## Why these are Rust tests for a Python and shell harness
//!
//! The harness itself is `scripts/ab_replay.py`, `scripts/run-ab-atomcode-octoscode.sh`
//! and `scripts/validate-ab-capture.sh` — the contract pins those filenames. But
//! `cargo test` is the only runner the verification gate invokes, and a bound
//! selector naming a test that does not exist is reported as a **pass**, not a
//! skip. That combination is not theoretical: before this file existed the
//! contract's thirteen scenarios all verified green against a repository in
//! which not one of the five harness files had been written.
//!
//! So every scenario binds to a test here, and each test drives the harness as a
//! subprocess and asserts on what it actually wrote. `tests/olp_contract.rs` and
//! `tests/docs_drift.rs` set the precedent: a contract test that reads artifacts
//! rather than linking against them.
//!
//! ## The harness surface these tests pin
//!
//! `scripts/run-ab-atomcode-octoscode.sh`
//!   `--out <dir>`             write the run directory here instead of deriving one
//!   `--prompts <file>`        newline-delimited prompt battery
//!   env `AB_RUN_ROOT`         parent for derived run directories when `--out` is absent
//!   env `AB_ARM_STUB`         `1` selects stub arms that emit canned PTY text
//!   env `AB_ATOMCODE_BIN`     path the atomcode arm is launched from
//!   env `AB_ATOMCODE_EXIT`    stub-arm exit code, for the failing-arm scenario
//!   env `AB_HEARTBEAT_ATOMCODE` / `AB_HEARTBEAT_OCTOSCODE`
//!                             fixture `server/heartbeat` payloads, one per arm
//!   env `AB_GUEST_MANIFEST`   fixture guest manifest, read for `modify` hooks
//!
//! `scripts/ab_replay.py --tap-report <file>`
//!   prints one recorded prompt per line, in recorded order, on stdout
//!
//! `scripts/validate-ab-capture.sh --normalize <file>`
//!   prints the normalized capture on stdout
//!
//! Every test is hermetic — stub arms, fixture heartbeats, a scratch directory.
//! None deploys a wasm guest or executes a real atomcode binary, because the
//! inherited project spec forbids binding an acceptance scenario to a live soak.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// Path to a harness entry point, with a failure that names the missing file
/// rather than surfacing as a confusing "No such file or directory" from the
/// spawn. An absent harness is the expected state until it is implemented.
fn harness(rel: &str) -> PathBuf {
    let path = repo_root().join(rel);
    assert!(
        path.exists(),
        "{} does not exist; the contract lists it under Allowed Changes",
        path.display()
    );
    path
}

/// A scratch directory, removed on drop.
///
/// Deliberately not `tempfile`: the project spec forbids adding a crate
/// dependency without cause, and a counter plus the clock is enough for a
/// directory name that no concurrent test reuses.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "octoscode-ab-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("create scratch: {err}"));
        Self { dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    /// Write a file under the scratch directory and return its path.
    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|err| panic!("create parent: {err}"));
        }
        std::fs::write(&path, contents)
            .unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
        path
    }

    /// File names directly inside `rel`, sorted.
    fn list(&self, rel: &str) -> BTreeSet<String> {
        let dir = self.path(rel);
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("read_dir {}: {err}", dir.display()));
        entries
            .map(|entry| {
                entry
                    .expect("directory entry is readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A round invocation of the shell harness, built up fluently.
struct Round {
    args: Vec<String>,
    envs: Vec<(String, String)>,
}

impl Round {
    /// A stubbed round: canned PTY output, no deploy, no real binary.
    fn stubbed() -> Self {
        Self {
            args: Vec::new(),
            envs: vec![("AB_ARM_STUB".into(), "1".into())],
        }
    }

    fn arg(mut self, value: impl AsRef<OsStr>) -> Self {
        self.args
            .push(value.as_ref().to_string_lossy().into_owned());
        self
    }

    fn out(self, dir: &Path) -> Self {
        self.arg("--out").arg(dir)
    }

    fn prompts(self, file: &Path) -> Self {
        self.arg("--prompts").arg(file)
    }

    fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.envs.push((
            key.to_string(),
            value.as_ref().to_string_lossy().into_owned(),
        ));
        self
    }

    /// Both arms served the same scenario id — the ordinary case.
    fn heartbeats(self, scenario_id: &str, scratch: &Scratch) -> Self {
        self.divergent_heartbeats(scenario_id, scenario_id, scratch)
    }

    fn divergent_heartbeats(self, atomcode: &str, octoscode: &str, scratch: &Scratch) -> Self {
        let a = scratch.write(
            "heartbeat_atomcode.json",
            &format!("{{\"scenario_id\": \"{atomcode}\", \"status\": \"ok\"}}\n"),
        );
        let b = scratch.write(
            "heartbeat_octoscode.json",
            &format!("{{\"scenario_id\": \"{octoscode}\", \"status\": \"ok\"}}\n"),
        );
        self.env("AB_HEARTBEAT_ATOMCODE", &a)
            .env("AB_HEARTBEAT_OCTOSCODE", &b)
    }

    fn run(self) -> Output {
        let script = harness("scripts/run-ab-atomcode-octoscode.sh");
        let mut cmd = Command::new("bash");
        cmd.arg(&script).args(&self.args).current_dir(repo_root());
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }
        cmd.output()
            .unwrap_or_else(|err| panic!("spawn {}: {err}", script.display()))
    }
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Four prompts, the battery size the contract's scenarios count against.
const BATTERY: &str = "explain this repository\nadd a test for the parser\nwhy did the last build fail\nrename the config field\n";

fn write_battery(scratch: &Scratch) -> PathBuf {
    scratch.write("prompts.txt", BATTERY)
}

/// Read `report.json` from a run directory as loosely-parsed rows.
///
/// Parsed by scanning rather than with a JSON crate: the repository carries no
/// serde_json dev-dependency for integration tests, and the assertions here are
/// about row counts and field presence, not about a typed shape.
fn report_json(dir: &Path) -> String {
    let path = dir.join("report.json");
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// Count of `"arm"` keys, one per comparison row.
fn row_count(report: &str) -> usize {
    report.matches("\"arm\"").count()
}

/// Hostnames that would mean an arm reached a real provider. The mock serves
/// both arms on loopback, so any of these appearing in a run's own record is
/// the failure the contract's first scenario exists to catch.
const PROVIDER_HOSTS: &[&str] = &[
    "api.anthropic.com",
    "api.openai.com",
    "api.deepseek.com",
    "api.moonshot.cn",
    "open.bigmodel.cn",
];

// ---------------------------------------------------------------------------
// Scenario: both arms are answered by the same scenario
// ---------------------------------------------------------------------------

#[test]
fn test_both_arms_served_same_scenario() {
    let scratch = Scratch::new("same-scenario");
    let out = scratch.path("run");
    let prompts = write_battery(&scratch);

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .run();

    assert!(
        output.status.success(),
        "a round whose arms agree exits zero; stderr:\n{}",
        stderr_of(&output)
    );

    let report = report_json(&out);
    assert!(
        report.contains("model-kimi-no-reasoning"),
        "the report names the scenario both arms were served; got:\n{report}"
    );

    // The two arms' recorded scenario ids are equal: the id appears against
    // both arms rather than only against one.
    assert_eq!(
        report.matches("model-kimi-no-reasoning").count(),
        row_count(&report),
        "every row names the one scenario served; got:\n{report}"
    );

    for host in PROVIDER_HOSTS {
        assert!(
            !report.contains(host),
            "no arm reached the real provider {host}; got:\n{report}"
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario: each arm is captured for each prompt
// ---------------------------------------------------------------------------

#[test]
fn test_capture_per_arm_per_prompt() {
    let scratch = Scratch::new("capture-per-arm");
    let out = scratch.path("run");
    let prompts = write_battery(&scratch);

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .run();
    assert!(
        output.status.success(),
        "stubbed round exits zero; stderr:\n{}",
        stderr_of(&output)
    );

    let names = scratch.list("run");
    let raw: Vec<&String> = names.iter().filter(|n| n.ends_with(".raw.txt")).collect();
    assert_eq!(
        raw.len(),
        8,
        "four prompts across two arms produce eight raw captures; got {raw:?}"
    );

    for arm in ["atomcode", "octoscode"] {
        let for_arm = raw.iter().filter(|n| n.contains(arm)).count();
        assert_eq!(
            for_arm, 4,
            "each capture names its arm; {arm} has {for_arm} of 4 in {raw:?}"
        );
    }

    // Each capture also names its prompt: the eight names are distinct, which
    // they cannot be if the prompt is not part of the name.
    let distinct: BTreeSet<&&String> = raw.iter().collect();
    assert_eq!(
        distinct.len(),
        8,
        "each capture names its prompt, so no two names collide; got {raw:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario: the report carries a row per prompt per arm
// ---------------------------------------------------------------------------

#[test]
fn test_report_row_per_prompt_per_arm() {
    let scratch = Scratch::new("report-rows");
    let out = scratch.path("run");
    let prompts = write_battery(&scratch);

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .heartbeats("model-glm-small-window", &scratch)
        .run();
    assert!(
        output.status.success(),
        "stubbed round exits zero; stderr:\n{}",
        stderr_of(&output)
    );

    let report = report_json(&out);
    assert_eq!(
        row_count(&report),
        8,
        "four prompts across two arms produce eight rows; got:\n{report}"
    );
    assert_eq!(
        report.matches("model-glm-small-window").count(),
        8,
        "every row names the scenario it ran under; got:\n{report}"
    );
}

// ---------------------------------------------------------------------------
// Scenario: the replay reads the tap's recorded steps
// ---------------------------------------------------------------------------

/// A tap report holding three recorded steps, in the shape `ab_prompt_tap.rs`
/// files them: one row per request, carrying only the last user message.
const TAP_REPORT_THREE_STEPS: &str = r#"{
  "steps": [
    {"name": "prompt", "value": "first recorded prompt"},
    {"name": "prompt", "value": "second recorded prompt"},
    {"name": "prompt", "value": "third recorded prompt"}
  ]
}
"#;

fn replay(tap_report: &Path) -> Output {
    let script = harness("scripts/ab_replay.py");
    Command::new("python3")
        .arg(&script)
        .arg("--tap-report")
        .arg(tap_report)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|err| panic!("spawn {}: {err}", script.display()))
}

#[test]
fn test_replay_reads_tap_steps() {
    let scratch = Scratch::new("replay-steps");
    let report = scratch.write("tap_report.json", TAP_REPORT_THREE_STEPS);

    let output = replay(&report);
    assert!(
        output.status.success(),
        "the replay reads a well-formed tap report; stderr:\n{}",
        stderr_of(&output)
    );

    let stdout = stdout_of(&output);
    let lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    assert_eq!(
        lines,
        [
            "first recorded prompt",
            "second recorded prompt",
            "third recorded prompt"
        ],
        "one prompt per recorded step, in recorded order"
    );
}

// ---------------------------------------------------------------------------
// Scenario: mismatched scenarios refuse to produce a report
// ---------------------------------------------------------------------------

#[test]
fn test_refuses_report_on_scenario_mismatch() {
    let scratch = Scratch::new("mismatch");
    let out = scratch.path("run");
    let prompts = write_battery(&scratch);

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .divergent_heartbeats(
            "model-kimi-no-reasoning",
            "model-glm-small-window",
            &scratch,
        )
        .run();

    assert!(
        !output.status.success(),
        "an A/B whose arms saw different data exits non-zero"
    );

    let message = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        message.contains("model-kimi-no-reasoning") && message.contains("model-glm-small-window"),
        "the failure names both scenario ids; got:\n{message}"
    );

    for artifact in ["report.md", "report.json"] {
        assert!(
            !out.join(artifact).exists(),
            "no {artifact} is written when the arms disagree"
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario: the replay never reads a full messages body
// ---------------------------------------------------------------------------

/// A tap report that ALSO carries a full `/v1/messages` body. The tap does not
/// produce this, but the replay must not be the thing that starts forwarding it
/// if some future report shape does: the system prompt, the tool definitions
/// and the prior messages are the contents of every file the agent has read.
const TAP_REPORT_WITH_FULL_BODY: &str = r#"{
  "steps": [
    {"name": "prompt", "value": "the only thing that may leave"}
  ],
  "raw_body": {
    "system": "SYSTEM_PROMPT_CANARY you are a coding agent",
    "tools": [{"name": "TOOL_DEFINITION_CANARY", "description": "reads files"}],
    "messages": [
      {"role": "user", "content": "PRIOR_MESSAGE_CANARY here is my api key sk-CANARY"},
      {"role": "assistant", "content": "ASSISTANT_TURN_CANARY"},
      {"role": "user", "content": "the only thing that may leave"}
    ]
  }
}
"#;

#[test]
fn test_replay_never_reads_full_message_body() {
    let scratch = Scratch::new("replay-privacy");
    let report = scratch.write("tap_report.json", TAP_REPORT_WITH_FULL_BODY);

    let output = replay(&report);
    assert!(
        output.status.success(),
        "the replay reads the recorded steps and ignores the rest; stderr:\n{}",
        stderr_of(&output)
    );

    let emitted = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        emitted.contains("the only thing that may leave"),
        "the recorded step is replayed; got:\n{emitted}"
    );

    for canary in [
        "SYSTEM_PROMPT_CANARY",
        "TOOL_DEFINITION_CANARY",
        "PRIOR_MESSAGE_CANARY",
        "ASSISTANT_TURN_CANARY",
        "sk-CANARY",
    ] {
        assert!(
            !emitted.contains(canary),
            "no {canary} reaches either arm; got:\n{emitted}"
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario: a modifying tap aborts the run
// ---------------------------------------------------------------------------

/// A guest manifest advertising a `modify` hook on the messages path. A tap that
/// rewrites the traffic it observes is no longer measuring the session it claims
/// to, so the run must not proceed to drive either arm.
const GUEST_MANIFEST_WITH_MODIFY_HOOK: &str = r#"{
  "hooks": [
    {"kind": "modify http_req", "path": "/v1/messages"}
  ]
}
"#;

#[test]
fn test_harness_aborts_if_tap_guest_modifies_requests() {
    let scratch = Scratch::new("modify-hook");
    let out = scratch.path("run");
    let prompts = write_battery(&scratch);
    let manifest = scratch.write("guest_manifest.json", GUEST_MANIFEST_WITH_MODIFY_HOOK);

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .env("AB_GUEST_MANIFEST", &manifest)
        .run();

    assert!(!output.status.success(), "a modifying tap aborts the run");

    let message = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        message.contains("modify http_req"),
        "the failure names the hook it found; got:\n{message}"
    );

    // Aborted before either arm was driven: no capture exists.
    let captured = out
        .read_dir()
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().contains(".raw.txt"))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(captured, 0, "the run aborts before either arm is driven");
}

// ---------------------------------------------------------------------------
// Scenario: normalization strips volatile rows before diffing
// ---------------------------------------------------------------------------

/// The same arm, the same answer, captured twice. Everything that differs is
/// volatile: the wall clock, the session and turn ids, the cursor position and
/// the elapsed-time row. A diff that shows these shows clock skew, not client
/// behaviour.
const CAPTURE_FIRST: &str = "\
2026-08-29T09:14:02Z  session 0f3a91c2-4d55-4d0a-9f11-6a2b7c8d9e01
\x1b[12;40Hturn 8c1d2e3f
> explain this repository
octoscode read 14 files and answered.
elapsed 1.284s
";

const CAPTURE_SECOND: &str = "\
2026-08-30T22:47:51Z  session bb77ee01-1122-4c33-8d44-55ff66aa77bb
\x1b[7;3Hturn 41aa90bc
> explain this repository
octoscode read 14 files and answered.
elapsed 9.007s
";

fn normalize(capture: &Path) -> Output {
    let script = harness("scripts/validate-ab-capture.sh");
    Command::new("bash")
        .arg(&script)
        .arg("--normalize")
        .arg(capture)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|err| panic!("spawn {}: {err}", script.display()))
}

#[test]
fn test_capture_normalization_strips_volatile_rows() {
    let scratch = Scratch::new("normalize");
    let first = scratch.write("first.raw.txt", CAPTURE_FIRST);
    let second = scratch.write("second.raw.txt", CAPTURE_SECOND);

    let a = normalize(&first);
    let b = normalize(&second);
    assert!(
        a.status.success() && b.status.success(),
        "normalization succeeds on both captures; stderr:\n{}\n{}",
        stderr_of(&a),
        stderr_of(&b)
    );

    let (a, b) = (stdout_of(&a), stdout_of(&b));
    assert_eq!(
        a, b,
        "two captures of the same arm normalize to the same text"
    );

    for volatile in [
        "2026-08-29T09:14:02Z",
        "2026-08-30T22:47:51Z",
        "0f3a91c2",
        "bb77ee01",
        "8c1d2e3f",
        "41aa90bc",
        "\x1b[12;40H",
        "\x1b[7;3H",
        "1.284s",
        "9.007s",
    ] {
        assert!(
            !a.contains(volatile),
            "{volatile:?} is stripped before diffing; got:\n{a}"
        );
    }

    assert!(
        a.contains("octoscode read 14 files and answered."),
        "the behaviour being compared survives normalization; got:\n{a}"
    );
}

// ---------------------------------------------------------------------------
// Scenario: a failing arm fails its round
// ---------------------------------------------------------------------------

#[test]
fn test_atomcode_nonzero_exit_fails_the_round() {
    let scratch = Scratch::new("arm-fails");
    let out = scratch.path("run");
    let prompts = scratch.write("prompts.txt", "explain this repository\n");

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .env("AB_ATOMCODE_EXIT", "3")
        .run();

    let report = report_json(&out);
    assert!(
        report.contains("\"failed\""),
        "a round whose atomcode arm exits non-zero is recorded as failed; got:\n{report}\nstderr:\n{}",
        stderr_of(&output)
    );

    // No comparison row claims a result for the failed arm: the atomcode row
    // carries the failure rather than an answer.
    let atomcode_row = report
        .split("\"arm\"")
        .find(|chunk| chunk.contains("atomcode"))
        .unwrap_or_else(|| panic!("the report holds an atomcode row; got:\n{report}"));
    assert!(
        !atomcode_row.contains("\"ok\""),
        "the failed arm's row claims no result; got:\n{atomcode_row}"
    );
}

// ---------------------------------------------------------------------------
// Scenario: a missing atomcode binary is named
// ---------------------------------------------------------------------------

#[test]
fn test_missing_atomcode_binary_reports_clearly() {
    let scratch = Scratch::new("missing-binary");
    let out = scratch.path("run");
    let prompts = write_battery(&scratch);
    let absent = scratch.path("nowhere/atomcode");

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .env("AB_ATOMCODE_BIN", &absent)
        .run();

    assert!(
        !output.status.success(),
        "a missing arm binary exits non-zero"
    );

    let message = format!("{}{}", stdout_of(&output), stderr_of(&output));
    assert!(
        message.contains(&absent.to_string_lossy().into_owned()),
        "the message names the path it looked for; got:\n{message}"
    );

    // It exits before deploying anything, so nothing was captured.
    assert!(
        !out.join("report.json").exists(),
        "the run exits before deploying anything"
    );
}

// ---------------------------------------------------------------------------
// Scenario: latency is labelled mock-served
// ---------------------------------------------------------------------------

#[test]
fn test_report_labels_latency_as_mock_served() {
    let scratch = Scratch::new("latency-label");
    let out = scratch.path("run");
    let prompts = write_battery(&scratch);

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .run();
    assert!(
        output.status.success(),
        "stubbed round exits zero; stderr:\n{}",
        stderr_of(&output)
    );

    let markdown = std::fs::read_to_string(out.join("report.md"))
        .unwrap_or_else(|err| panic!("read report.md: {err}"));
    let lowered = markdown.to_lowercase();

    assert!(
        lowered.contains("mock-served"),
        "the report labels its latency mock-served; got:\n{markdown}"
    );
    assert!(
        lowered.contains("not a provider benchmark") || lowered.contains("not a model benchmark"),
        "the report states it is not a provider benchmark; got:\n{markdown}"
    );

    // Every line carrying a latency figure carries the label with it, so a row
    // read on its own cannot be mistaken for a provider measurement.
    for line in markdown.lines() {
        let lowered = line.to_lowercase();
        let mentions_latency = lowered.contains("latency") || lowered.contains(" ms");
        if mentions_latency && !lowered.contains("mock-served") {
            panic!("latency figure without the mock-served label: {line:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario: a run writes exactly the declared artifact set
// ---------------------------------------------------------------------------

#[test]
fn test_run_writes_declared_artifact_set() {
    let scratch = Scratch::new("artifact-set");
    let out = scratch.path("run");
    std::fs::create_dir_all(&out).expect("create the empty output directory");
    let prompts = write_battery(&scratch);
    let run_root = scratch.path("derived-root");

    let output = Round::stubbed()
        .out(&out)
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .env("AB_RUN_ROOT", &run_root)
        .run();
    assert!(
        output.status.success(),
        "stubbed round exits zero; stderr:\n{}",
        stderr_of(&output)
    );

    let names = scratch.list("run");
    assert!(
        names.contains("report.md") && names.contains("report.json"),
        "the output directory holds report.md and report.json; got {names:?}"
    );

    let raw = names.iter().filter(|n| n.ends_with(".raw.txt")).count();
    let normalized = names
        .iter()
        .filter(|n| n.ends_with(".normalized.txt"))
        .count();
    assert_eq!(
        (raw, normalized),
        (8, 8),
        "one raw and one normalized capture for each of the eight arm-prompt pairs; got {names:?}"
    );
    assert_eq!(
        names.len(),
        18,
        "the declared artifact set and nothing else; got {names:?}"
    );

    assert!(
        !run_root.exists(),
        "with --out given, no file is written outside that output directory"
    );
}

// ---------------------------------------------------------------------------
// Scenario: a run never overwrites a prior run
// ---------------------------------------------------------------------------

#[test]
fn test_run_dir_is_unique_per_run() {
    let scratch = Scratch::new("unique-run");
    let prompts = scratch.write("prompts.txt", "explain this repository\n");
    let run_root = scratch.path("runs");

    let first = Round::stubbed()
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .env("AB_RUN_ROOT", &run_root)
        .run();
    assert!(
        first.status.success(),
        "the first run exits zero; stderr:\n{}",
        stderr_of(&first)
    );

    let after_first = scratch.list("runs");
    assert_eq!(
        after_first.len(),
        1,
        "the first run derives one run directory; got {after_first:?}"
    );
    let first_dir = run_root.join(after_first.iter().next().expect("one run directory"));
    let first_report = std::fs::read(first_dir.join("report.json"))
        .unwrap_or_else(|err| panic!("read the first run's report: {err}"));
    let first_captures: BTreeSet<String> = std::fs::read_dir(&first_dir)
        .expect("the first run directory is readable")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    let second = Round::stubbed()
        .prompts(&prompts)
        .heartbeats("model-kimi-no-reasoning", &scratch)
        .env("AB_RUN_ROOT", &run_root)
        .run();
    assert!(
        second.status.success(),
        "the second run exits zero; stderr:\n{}",
        stderr_of(&second)
    );

    let after_second = scratch.list("runs");
    assert_eq!(
        after_second.len(),
        2,
        "the second run writes to a different directory; got {after_second:?}"
    );

    let still_there: BTreeSet<String> = std::fs::read_dir(&first_dir)
        .expect("the first run directory survives the second run")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        still_there, first_captures,
        "the prior run's captures are unchanged"
    );
    assert_eq!(
        std::fs::read(first_dir.join("report.json")).expect("the prior report survives"),
        first_report,
        "the prior run's report is unchanged"
    );
}

// ---------------------------------------------------------------------------
// Scenario: every bound selector names a test that exists
// ---------------------------------------------------------------------------

/// The guard against the failure that motivated this whole file.
///
/// `agent-spec` reports a selector naming a nonexistent test as a **pass**, not
/// a skip, so a typo in a `Test:` line does not fail the gate — it silently
/// removes a scenario from it. This test reads the contract's selectors and the
/// test functions in this file and fails if they ever drift apart.
#[test]
fn test_every_bound_selector_resolves() {
    let spec = read("specs/task-ab-atomcode-octoscode-harness.spec.md");
    let source = read("tests/ab_harness_contract.rs");

    let selectors: BTreeSet<String> = spec
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Test:"))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();

    assert!(
        selectors.len() >= 14,
        "the contract binds every scenario to a selector; found {}",
        selectors.len()
    );

    let defined: BTreeSet<String> = source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("fn "))
        .filter_map(|rest| rest.split('(').next())
        .map(|name| name.trim().to_string())
        .filter(|name| name.starts_with("test_"))
        .collect();

    let unresolved: Vec<&String> = selectors.difference(&defined).collect();
    assert!(
        unresolved.is_empty(),
        "these selectors resolve to nothing and would be scored as passes: {unresolved:?}"
    );

    let unbound: Vec<&String> = defined.difference(&selectors).collect();
    assert!(
        unbound.is_empty(),
        "these tests are not bound by any scenario: {unbound:?}"
    );
}
