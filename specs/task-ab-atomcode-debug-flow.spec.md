spec: task
name: "atomcode vs octoscode debug-flow A/B under injected faults"
inherits: project
tags: [ab-testing, atomcode, debug, fault-injection, pty-capture, harness]
depends: [task-ab-atomcode-octoscode-harness]
estimate: 2d
---

## Intent

`task-ab-atomcode-octoscode-harness` compares the two clients on prompts that
succeed, which measures the surface a user sees when nothing goes wrong. The
interesting difference between two coding agents is what each one does when the
work FAILS — a tool exits non-zero, credit runs out, the stream dies mid-turn,
the user interrupts. That comparison exists in no form today: the scratch
material in `tmp/` drives only happy prompts. This task adds a second harness
that drives a declared battery of injected faults through both arms on the same
mock, captures each arm's TUI as PTY text, and classifies what each arm did
about each fault into a fixed taxonomy — recording the difference, not scoring
it.

## Decisions

- The harness reuses the round machinery this contract's `depends` establishes:
  one mock answering both arms, PTY capture, capture normalization, an
  `--out`-overridable per-run directory, and the `server/heartbeat`
  `scenario_id` equality check before anything is reported. This contract adds
  the fault battery and the classifier; it does not re-implement rounds.
- Two reuse facts are load-bearing and easy to get wrong.
  `scripts/capture-appui-ux-pty.sh` is not a driver: its command is fixed to
  `cargo test --test appui_ux_fixture` and it performs NO normalization. What is
  reused from it is the capture TECHNIQUE — `script -q -e -c` with the BSD
  fallback — and the `OCTOSCODE_UX_CAPTURE_DIR` override convention. The only
  normalization prior art in the repo is the ANSI/CR strip in
  `scripts/validate-tmux-ux-capture.sh`; everything past it (timestamps, session
  and turn ids, cursor positions, elapsed-time rows) is new work owned here.
- Rounds are driven through the upstream runner's existing `--driver` seam,
  which invokes `<CMD> <scenario-name>` per scenario and already aborts a round
  when the deployed guest's heartbeat disagrees with the installed scenario.
  This harness is such a driver; it does not fork the runner.
- A fault is declared in a manifest, `scripts/ab_debug_faults.toml`, one entry
  per fault: `id`, the mock `scenario` that serves it, the prompt that provokes
  it, and the observable `marker` each arm's capture is searched for. The
  battery is data, so adding a fault is an entry rather than a code change.
- The opening battery, chosen because each entry has a DIFFERENT correct
  response and collapsing any pair is a real client defect:

  | fault id | injected condition | why it separates the arms |
  | `tool-nonzero-exit` | a tool call returns a non-zero exit and stderr | retry, re-plan, ask, or silently continue |
  | `quota-unfunded` | provider answers `402` / `insufficient_quota` | "your card ran out" is not "your key is wrong" |
  | `quota-rate-limited` | provider answers `429` `rate_limit_exceeded` | a wait-and-retry, not a terminal failure |
  | `auth-revoked` | provider answers `401` | terminal; re-keying is the only fix |
  | `stream-dies-mid-turn` | the connection drops after partial output | a partial answer must not read as a complete one |
  | `frame-seq-gap` | a replayed frame seq is skipped | octoscode reports `protocol/replay_lossy`; silence is the defect |
  | `user-interrupt` | the driver interrupts mid-turn | what survives the interrupt, and what is lost |
  | `approval-park` | the backend parks on an approval | asymmetric by design, see below |
- Every row carries TWO fields, and conflating them is the mistake this split
  exists to prevent. `outcome` says whether the round produced evidence at all:
  `ok`, `not-fired`, `timeout`, `arm-failed`. `recovery` is the comparison axis
  and is read only when `outcome` is `ok`: `surfaced`, `retried`, `asked-user`,
  `silently-continued`, `aborted`, `not-applicable`. `silently-continued` is the
  finding this harness exists to catch — an arm that neither shows the fault nor
  acts on it — and a round that never fired must never be able to produce it.
- Markers are matched on LOCALE-INDEPENDENT evidence. octoscode renders its
  failure text through `t!()` against `locales/en.yml` and `locales/zh.yml`, so
  a harness grepping English status strings silently reclassifies every octoscode
  row the moment the capture runs under `zh`. Each manifest entry therefore
  matches a glyph or a structural token that both locale files share — the `✗`
  failure bullet, the `Error [` status prefix's bracketed code, the approval
  card's `┌─ ⚠` frame — and the harness pins `OCTOSCODE_LOCALE` for the
  octoscode arm and records the pinned locale in the report.
- Capability asymmetry is recorded, not scored. atomcode runs headless as
  `atomcode -p <prompt> -y --dev --no-telemetry`, where `-y` resolves to
  skip-permissions mode and prints its own `all tool calls are auto-approved`
  banner — so that arm cannot park on an approval, and its per-run `.ui.json`
  event channel stays empty. Its `recovery` for `approval-park` is
  `not-applicable` and the report states the reason. An arm that cannot express
  a behaviour has not lost a comparison.
- The atomcode arm's evidence is read from its PTY capture AND from its session
  artifacts under `ATOMCODE_HOME`, because the two disagree in exactly the case
  this harness cares about: **a turn that fails writes no per-turn `.jsonl` file
  at all**. Its only durable trace is `turn_stats[].errored` in the session
  `.meta` plus a `.snapshot` truncated before any assistant message. A collector
  that globs `*.jsonl` therefore drops every failed turn and reports the fault
  as though it never fired. Evidence collection reads `.meta` first and treats a
  missing `.jsonl` beside an `errored` turn as a failure, never as an absence.
- Where an arm exposes them, each row also records `rounds` and `tool_calls`
  (for atomcode, `turn_stats[].round_count` and `tool_call_count`), and marks
  them absent otherwise. They are recorded because recovery has a cost that is
  visible without reading any text — the one observed failed-edit recovery ran
  3 rounds / 2 tool calls against 2 / 1 for the clean runs of the same prompt —
  and they are never turned into a score or a ranking.
- A fault that did not fire is not a data point. Each round asserts the fault's
  `marker` appears in the mock's own served transcript before either capture is
  classified; a fault that never reached an arm sets `outcome` to `not-fired`
  for both arms. This is the same trap the sibling contract's heartbeat check
  exists for — an unfired fault would otherwise read as the strongest possible
  finding against both arms.
- Each arm gets a per-fault deadline; an arm still running at the deadline is
  killed, its `outcome` is `timeout`, and its partial capture is retained. A
  hung arm is a result, not a reason to lose the round's other arm.
- Redaction runs before anything is written: API keys, bearer tokens and the
  `ATOMCODE_HOME` absolute path are stripped from captures, `debug-report.md`
  and `debug-report.json`. A battery that provokes auth errors is exactly the
  run most likely to print a credential.
- Latency is not recorded at all. The sibling contract labels it mock-served;
  under injected faults it measures the injected delay, so this report omits the
  column rather than qualifying it.
- No octoscode source changes. A client that mishandles an injected fault is a
  finding this harness reports, not a fix it makes.

## Boundaries

### Allowed Changes
- scripts/ab_debug_flow.py
- scripts/ab_debug_faults.toml
- scripts/run-ab-debug-flow.sh
- scripts/tests/test_ab_debug_flow.py
- docs/ab-atomcode-debug-flow.md

### Forbidden
- Do not modify any file under `src/`.
- Do not call a real provider endpoint from either arm.
- Do not modify `scripts/capture-appui-ux-pty.sh` or
  `scripts/validate-tmux-ux-capture.sh`; reuse them as they are.
- Do not match a marker against a localized string from `locales/`.
- Do not score, rank, or grade an arm's answer quality.
- Do not report a capability an arm cannot express as a failure of that arm.
- Do not read a `recovery` value from a row whose `outcome` is not `ok`.
- Do not commit a run directory, a capture, or a report.
- Do not write an absolute machine path or a credential into any artifact.
- Do not record a latency figure in the report.

## Out of Scope

- Authoring the mock-side fault scenarios; that corpus data ships in the
  wasm_mock_rust contracts, and this harness consumes it by scenario name.
- The happy-path prompt battery and its report, which the sibling contract owns.
- Automatic judgement of which recovery class is the better behaviour.
- Real provider faults, real rate limits, and real billing state.
- Any agent other than atomcode and octoscode.
- Image or pixel-diffed screenshots.

## Completion Criteria

Scenario: every declared fault produces a row per arm
  Test: test_debug_report_row_per_fault_per_arm
  Level: integration
  Test Double: stub arms writing canned PTY output
  Given a manifest declaring "8" faults
  When the harness completes a run
  Then `debug-report.json` holds "16" rows
  And every row names its fault id, its arm, and its scenario id

Scenario: a run writes exactly the declared artifact set
  Test: test_debug_run_writes_declared_artifact_set
  Level: integration
  Test Double: stub arms writing canned PTY output
  Given a completed run invoked with "--out" pointing at an empty directory
  When that output directory is listed
  Then it holds the file "debug-report.md" and the file "debug-report.json"
  And it holds one raw and one normalized capture for each of the "16" arm-fault pairs
  And no file is written outside that output directory

Scenario: outcome and recovery come from their own fixed vocabularies
  Test: test_debug_outcome_and_recovery_use_fixed_vocabularies
  Level: integration
  Test Double: canned run fixture, no live arms
  Given the outcome vocabulary holds "ok", "not-fired", "timeout", "arm-failed"
  And the recovery vocabulary holds "surfaced", "retried", "asked-user", "silently-continued", "aborted", "not-applicable"
  When every row of a completed run is read
  Then each row's outcome is a member of the outcome vocabulary
  And each row's recovery is a member of the recovery vocabulary
  And no row carries a free-text classification

Scenario: an unfired fault fails its round instead of reading as silence
  Test: test_debug_unfired_fault_is_not_classified_as_silence
  Level: integration
  Test Double: mock guest serving the scenario without emitting the fault marker
  Given the fault "tool-nonzero-exit" never reaches either arm
  When the round completes
  Then both rows for that fault carry the outcome "not-fired"
  And neither row carries the recovery "silently-continued"
  And the run exits non-zero

Scenario: a failed atomcode turn is detected without a per-turn jsonl file
  Test: test_debug_errored_turn_without_jsonl_is_still_detected
  Level: integration
  Test Double: canned `ATOMCODE_HOME` holding an errored turn and no matching `.jsonl`
  Given a session `.meta` whose `turn_stats` entry carries `errored` set to "true"
  And no per-turn `.jsonl` file exists beside it
  When the atomcode arm's evidence is collected
  Then the row carries the outcome "arm-failed"
  And the row does not carry the outcome "not-fired"

Scenario: a row that produced no evidence carries no recovery reading
  Test: test_debug_non_ok_outcome_carries_no_recovery
  Level: integration
  Test Double: canned run fixture holding one row per outcome
  Given a run holding rows with the outcomes "not-fired", "timeout" and "arm-failed"
  When each such row is read
  Then its recovery field is absent
  And the report states which outcome suppressed the reading

Scenario: arms served different scenarios produce no report
  Test: test_debug_scenario_mismatch_writes_no_report
  Level: integration
  Test Double: deployed mock guest reporting divergent heartbeats
  Given the two arms report different `scenario_id` values for one fault
  When the harness reaches the reporting step
  Then no report file is written
  And the run exits non-zero naming both scenario ids

Scenario: markers survive a locale change
  Test: test_debug_markers_are_locale_independent
  Level: integration
  Test Double: stub octoscode arm rendering the zh locale
  Given one capture of the octoscode arm taken under "en" and one under "zh"
  When both are classified for the fault "tool-nonzero-exit"
  Then both rows carry the same recovery
  And the report names the locale each arm was pinned to

Scenario: an arm that cannot express a behaviour is not scored against
  Test: test_debug_asymmetric_capability_is_not_a_loss
  Level: integration
  Test Double: stub atomcode arm invoked with the skip-permissions flag
  Given the fault "approval-park" and an atomcode arm invoked with "-y"
  When the run is classified
  Then the atomcode row carries the recovery "not-applicable"
  And that row carries the reason the arm cannot park
  And no row for that fault is marked failed

Scenario: the two quota faults stay distinct
  Test: test_debug_quota_faults_are_not_collapsed
  Level: integration
  Test Double: mock guest serving both quota scenarios
  Given the faults "quota-unfunded" and "quota-rate-limited"
  When both rounds are classified
  Then the two faults hold separate rows for each arm
  And no row for "quota-unfunded" is merged into "quota-rate-limited"

Scenario: an auth failure is separated from a funding failure
  Test: test_debug_auth_revoked_is_distinct_from_unfunded
  Level: integration
  Test Double: mock guest serving the auth and funding scenarios
  Given the faults "auth-revoked" and "quota-unfunded"
  When both rounds are classified
  Then each fault holds its own row per arm
  And the report states the distinct condition each one injected

Scenario: a partial answer from a dead stream is marked partial
  Test: test_debug_stream_death_marks_the_answer_partial
  Level: integration
  Test Double: mock guest closing the connection mid-turn
  Given the fault "stream-dies-mid-turn"
  When the round is classified
  Then each arm's row records whether its capture ends mid-answer
  And a capture ending mid-answer never carries the recovery "surfaced" alone

Scenario: an arm exiting non-zero fails only its own row
  Test: test_debug_arm_nonzero_exit_fails_only_that_row
  Level: integration
  Test Double: stub atomcode arm exiting non-zero
  Given the atomcode arm exits non-zero on one fault
  When the round completes
  Then that arm's row for that fault carries the outcome "arm-failed"
  And the octoscode row for the same fault carries the outcome "ok"

Scenario: a hung arm is killed and recorded as a timeout
  Test: test_debug_hung_arm_is_killed_and_recorded
  Level: integration
  Test Double: stub arm that never exits
  Given an arm still running at its per-fault deadline
  When the deadline passes
  Then that arm is killed
  And its row carries the outcome "timeout"
  And its partial capture is retained in the run directory

Scenario: a missing atomcode binary stops the run before anything deploys
  Test: test_debug_missing_atomcode_binary_is_named
  Level: integration
  Test Double: configured binary path pointing at an absent file
  Given the atomcode binary is absent from its configured path
  When the harness starts
  Then it exits non-zero before deploying a scenario
  And the message names the path it looked for

Scenario: credentials never reach an artifact
  Test: test_debug_artifacts_are_redacted
  Level: integration
  Test Double: stub arm emitting a canned key and home path
  Given an arm capture containing the api key "sk-live-000000000000abcd" and an absolute `ATOMCODE_HOME` path
  When the run directory is written
  Then no artifact contains "sk-live-000000000000abcd"
  And no artifact contains the absolute home path

Scenario: normalization strips volatile rows before diffing
  Test: test_debug_normalization_strips_volatile_rows
  Level: integration
  Test Double: two canned captures of one arm
  Given two captures of the same arm and fault taken at different times
  When both are normalized
  Then the normalized captures are identical
  And timestamps, session ids and cursor positions are absent from both

Scenario: neither arm opens a connection to a real provider
  Test: test_debug_no_real_provider_connection
  Level: integration
  Test Double: deployed mock guest with the provider hosts unroutable
  Given a full battery run
  When the connections each arm opened are read
  Then every connection targets the mock
  And no connection targets a provider host

Scenario: the report carries no latency figure
  Test: test_debug_report_omits_latency
  Level: integration
  Test Double: canned run fixture, no live arms
  Given a completed run
  When `debug-report.md` and `debug-report.json` are read
  Then neither artifact carries a latency column or field
  And the report states latency is omitted because the faults inject their own delay

Scenario: a run never overwrites a prior run
  Test: test_debug_run_dir_is_unique_per_run
  Given a prior run directory exists
  When a second run starts
  Then it writes to a different directory
  And the prior run's captures and report are unchanged
