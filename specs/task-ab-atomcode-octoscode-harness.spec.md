spec: task
name: "reproducible atomcode vs octoscode A/B harness on mocked models with PTY capture"
inherits: project
tags: [ab-testing, atomcode, wasm-mock-server, pty-capture, harness, benchmark]
depends: [task-mock-octos-model-matrix]
estimate: 2d
---

## Intent

The atomcode vs octoscode comparison exists only as scratch in `tmp/`
(`ab_run.sh`, `ab_proxy.py`, `bench.py`, `bench_report.md`), and both arms call
the live DeepSeek API — so a rerun needs a key, spends money, and returns
different numbers each time. The prior report's headline latency column is
therefore a measurement of the provider, not of the two clients. Meanwhile
`examples/automation/ab_prompt_tap.rs` already records identical input for both
agents, but the `scripts/ab_replay.py` its own header names was never written.
This task puts both arms behind wasm_mock_server so one mocked model answers
both, captures each arm's TUI as PTY text, and emits a diffable report keyed by
the model scenario it ran under.

## Decisions

- Both arms are served by the same mock in the same round: atomcode through the
  HTTP MITM on `:20810` via its `HTTP_PROXY`/`HTTPS_PROXY`, octoscode through the
  octos WS mock. A difference in output is then a difference between the two
  clients rather than between two live calls.
- The model set is the matrix from `task-mock-octos-model-matrix`. Each round
  names a scenario, so a run is reproducible by name rather than by whatever the
  provider returned that afternoon.
- `scripts/ab_replay.py` is written — the file `ab_prompt_tap.rs` already
  documents as its consumer. It reads the tap's recorded steps and replays each
  prompt at both arms in the same order.
- The tap's privacy boundary is load-bearing and is preserved exactly: it walks
  to the last `role: "user"` entry and drops the rest, because `/v1/messages`
  carries the system prompt, every tool definition and the contents of every file
  the agent has read. The replay consumes only those recorded steps and never
  reads or forwards a full messages body.
- A TUI "screenshot" here is a **PTY text capture**, not an image, taken through
  the `script -q -e -c` path already used by `scripts/capture-appui-ux-pty.sh`.
  Text frames diff; a PNG does not, and the repo's existing
  `validate-tmux-ux-capture.sh` is row-oriented.
- Captures are normalized before diffing — timestamps, session ids, turn ids,
  cursor positions and elapsed-time rows are stripped — so a diff shows client
  behavior and not clock skew.
- Arm invocations are pinned to what the scratch harness established: atomcode
  runs headless as `atomcode -p <prompt> -y --dev --no-telemetry` under its own
  `ATOMCODE_HOME`; octoscode is driven by `turn/steer` over the UI-protocol
  WebSocket.
- Each run writes a unique run directory holding both arms' raw captures, the
  normalized captures, `report.md` and `report.json`. Nothing is overwritten in
  place, so two runs can be compared. The location is overridable with
  `--out <dir>`, matching the `OCTOSCODE_UX_CAPTURE_DIR` override that
  `scripts/capture-appui-ux-pty.sh` already offers; without it the run directory
  is derived from a run id.
- Latency is recorded but labelled **mock-served** in the report. With a mock
  upstream it measures client overhead only, and presenting it as a model
  benchmark is the mistake the prior report made.
- Before reporting, the harness reads `server/heartbeat` for both arms and
  refuses to emit a report if they were served different `scenario_id`s. An A/B
  where the arms saw different data is not an A/B.
- This contract is octoscode-side only. The guest that does the capturing already
  exists and is not touched.

## Boundaries

### Allowed Changes
- scripts/ab_replay.py
- scripts/run-ab-atomcode-octoscode.sh
- scripts/validate-ab-capture.sh
- scripts/tests/test_ab_replay.py
- docs/ab-atomcode-octoscode.md

### Forbidden
- Do not modify `examples/automation/ab_prompt_tap.rs`; it is a capture-only,
  byte-for-byte passthrough tap on a real session, and a tap that alters the
  traffic it observes no longer measures what it claims to.
- Do not read, log, or replay a full `/v1/messages` body — only the last-user-
  message steps the tap records.
- Do not call a real provider endpoint from either arm.
- Do not commit a run directory, a capture, or a report.
- Do not present mock-served latency as a model or provider benchmark.
- Do not modify any file under `src/`.

## Out of Scope

- Adding or changing model scenarios; they come from the matrix contract.
- Scoring answer quality or correctness automatically.
- Benchmarking real provider latency, throughput, or cost.
- Comparing any agent other than atomcode and octoscode.
- Image-based or pixel-diffed screenshots.

## Completion Criteria

Scenario: both arms are answered by the same scenario
  Test: test_both_arms_served_same_scenario
  Level: integration
  Test Double: deployed mock guest, no live provider
  Given the scenario "model-kimi-no-reasoning" is installed
  When a round runs both arms
  Then the heartbeat `scenario_id` read for each arm is equal
  And neither arm opened a connection to a real provider host

Scenario: each arm is captured for each prompt
  Test: test_capture_per_arm_per_prompt
  Level: integration
  Test Double: stub arms writing canned PTY output
  Given a battery of "4" prompts
  When the harness runs
  Then the run directory holds "8" raw captures
  And each capture names its arm and its prompt

Scenario: the report carries a row per prompt per arm
  Test: test_report_row_per_prompt_per_arm
  Given a completed run over "4" prompts
  When `report.json` is read
  Then it holds "8" rows
  And every row names the scenario it ran under

Scenario: the replay reads the tap's recorded steps
  Test: test_replay_reads_tap_steps
  Given a report holding recorded last-user-message steps
  When `ab_replay.py` loads it
  Then one prompt is produced per recorded step
  And the prompts are replayed in recorded order

Scenario: mismatched scenarios refuse to produce a report
  Test: test_refuses_report_on_scenario_mismatch
  Level: integration
  Test Double: deployed mock guest reporting divergent heartbeats
  Given the two arms report different `scenario_id` values
  When the harness reaches the reporting step
  Then no report file is written
  And the run exits non-zero naming both scenario ids

Scenario: the replay never reads a full messages body
  Test: test_replay_never_reads_full_message_body
  Given a tap report that also contains a full `/v1/messages` body
  When `ab_replay.py` loads it
  Then only last-user-message steps are read
  And no system prompt, tool definition, or prior message reaches either arm

Scenario: a modifying tap aborts the run
  Test: test_harness_aborts_if_tap_guest_modifies_requests
  Level: integration
  Test Double: deployed guest advertising a modify hook
  Given the deployed guest advertises a `modify http_req` hook on the messages path
  When the harness starts a round
  Then the run aborts before either arm is driven
  And the failure names the hook it found

Scenario: normalization strips volatile rows before diffing
  Test: test_capture_normalization_strips_volatile_rows
  Given two captures of the same arm taken at different times
  When both are normalized
  Then the normalized captures are identical
  And timestamps, session ids and cursor positions are absent from both

Scenario: a failing arm fails its round
  Test: test_atomcode_nonzero_exit_fails_the_round
  Given the atomcode arm exits non-zero
  When the round completes
  Then that round is recorded as failed
  And no comparison row claims a result for it

Scenario: a missing atomcode binary is named
  Test: test_missing_atomcode_binary_reports_clearly
  Given the atomcode binary is absent from its configured path
  When the harness starts
  Then it exits non-zero before deploying anything
  And the message names the path it looked for

Scenario: latency is labelled mock-served
  Test: test_report_labels_latency_as_mock_served
  Given a completed run
  When `report.md` is read
  Then every latency figure is labelled mock-served
  And the report states it is not a provider benchmark

Scenario: a run writes exactly the declared artifact set
  Test: test_run_writes_declared_artifact_set
  Level: integration
  Test Double: stub arms writing canned PTY output
  Given a completed run over "4" prompts invoked with "--out" pointing at an empty directory
  When that output directory is listed
  Then it holds the file "report.md" and the file "report.json"
  And it holds one raw and one normalized capture file for each of the "8" arm-prompt pairs
  And no file is written outside that output directory

Scenario: a run never overwrites a prior run
  Test: test_run_dir_is_unique_per_run
  Given a prior run directory exists
  When a second run starts
  Then it writes to a different directory
  And the prior run's captures and report are unchanged
