spec: task
name: "wasm_mock_server fiddler mock for /context compaction plus a scenario loop runner"
tags: [mock, wasm-mock-server, fiddler, context, compaction, automation, upstream]
depends: []
estimate: 2d
---

## Intent

`/context` is the one octoscode menu whose entire surface is decided by data the
mock corpus does not carry: availability gates on `config/capabilities/list`
advertising `session/compact`, the two mode rows gate on
`session/compact/mode/set`, the subtitle renders live context-window usage, and
the compaction block renders a `context/compaction_completed` notification. The
recorded baseline in `examples/automation/octos` has `context.compaction`
`{count: 0, last: null}` and `last_compaction_id: null` — no compaction was ever
captured — so both methods fall through to the guest's `-32000` and the client's
whole compaction path has never run against served data. This task adds the
`/context` responses plus a replayable compaction notification stream to that
corpus, and the scenarios that mutate them. Implementation repo is
**wasm_mock_rust** (`/Users/alanpoon/Documents/go/wasm_mock_rust`); this contract
ships with the workstream.

## Decisions

- Two new result files, each holding the bare `result` body with no JSON-RPC
  envelope, plus one `index.json` entry each — no Rust edited, per the corpus
  rule in `examples/automation/octos/README.md`:

  | file | method | const |
  | `session_compact.json` | `session/compact` | `SESSION_COMPACT` |
  | `session_compact_mode_set.json` | `session/compact/mode/set` | `SESSION_COMPACT_MODE_SET` |

- `context/compaction_completed` is a **notification**, not an RPC —
  `src/model.rs:209` states the client must never call it as a method. It gets no
  `results` entry in `index.json`. It lives in a new `compaction_stream.json`
  replayed after the `session/compact` reply, structurally identical to the way
  `turn_stream.json` is replayed after the `turn/start` ack
  (`mock_octos.rs:309`), and declared in `index.json` as a sibling of
  `turn_stream` rather than inside `results`.
- Stream order, one compaction: `context/compaction_completed` carrying a
  `ContextCompactionSummary` → `session/status` reflecting the new generation →
  `progress/updated` with a `token_cost_update`. Envelope `seq` is renumbered
  contiguously from 1, the same rule `turn_stream.json` already follows, because
  the client reports `seq` gaps as `protocol/replay_lossy`.
- The summary is pinned to the captured baseline so it is continuous with the
  rest of the data set: `input_generation` equals `context.state.generation`
  (`68`), `output_generation` is greater than it, `token_estimate_before` equals
  the captured `1937`, and `token_estimate_after` is lower. `status` is
  `"completed"`, `error` absent.
- One body answers both modes. `session/compact/mode/set` echoes the `mode` it
  was given by lifting it out of the live params, the same retarget trick
  `retarget_loop_id` uses to make one body answer every `loop/pause`
  (`mock_octos.rs:280`). Session and profile ids keep their captured values and
  are rewritten by the existing `retarget` (`mock_octos.rs:156`).
- Capability gating is exercised as **data, not code**: the `/context` menu
  reads `supported_methods`, so dropping `session/compact` from
  `config_capabilities_list.json` must render the menu unavailable, and dropping
  only `session/compact/mode/set` must leave the compact row and remove the two
  mode rows.
- `context` is a **new** scenario area. `gen_scenario.py` today registers eight
  — `agents`, `capabilities`, `goal`, `hydrate`, `llm`, `loops`, `status`,
  `turn` — and none of them owns the compaction surface. `FILES` gains
  `compaction` → `compaction_stream.json`; the existing `status` key already
  maps `session_status_read.json`.
- A compaction scenario is a **multi-file** scenario returning
  `{"status": …, "compaction": …}`, the shape `gen_scenario.py` already emits
  elsewhere (line 385), so the served `context_state` and the replayed
  notification never disagree about the generation.
- Each scenario carries an `expectation` sentence that the guest files as a
  deliberately failing step. `pristine/` is refreshed so `--restore` restores the
  extended baseline rather than the pre-task one.
- The loop runner is `examples/automation/octos/run_campaign.py <area>`, taking
  the area as an argument so one driver serves every area rather than a copy per
  task; this task passes `context`. It uses repo-relative paths only, replacing
  the scratchpad driver the corpus README describes as unshippable because it
  points at absolute paths on one machine. Per scenario it runs
  `setup_scenario.py <name>`, deploys with `bash cli/mock_octos.sh`, reads
  `server/heartbeat`, asserts the reported `scenario_id` equals the one just
  installed, drives the client, collects the report, and moves on. It calls
  `setup_scenario.py --restore` in a finally block so an aborted run does not
  leave a mutated data set behind.
- The heartbeat assertion is not optional bookkeeping. `shutil.copy2` preserves
  source mtimes, cargo then skips the rebuild, and a campaign once tested one
  stale build fifty times while every deploy reported `ok`. The runner treats a
  heartbeat that disagrees with the installed scenario as a hard failure before
  any driving happens.
- `setup_scenario.py`'s `data_files()` enumerates the data set as every
  `index.json` `results` entry plus the ONE hardcoded sibling
  `index["turn_stream"]["file"]` — deliberately read from the index rather than
  globbed, so a stray JSON never becomes part of the baseline. `compaction_stream`
  is declared as a second sibling and carries no `results` entry, so that function
  cannot see it: `--seed` would never copy it into `pristine/`, and `--restore`,
  which walks `pristine/`, would therefore never put it back — the "refreshed
  `pristine/`" this contract depends on cannot be produced by `--seed` as the
  script stands. `data_files()` therefore enumerates every declared stream sibling
  instead of `turn_stream` alone.
- No octoscode source changes. This task adds mock data and guest plumbing only;
  if the client turns out to mishandle a served case, that is a finding, not a
  fix inside this contract.

## Boundaries

### Allowed Changes
- examples/automation/octos/session_compact.json
- examples/automation/octos/session_compact_mode_set.json
- examples/automation/octos/compaction_stream.json
- examples/automation/octos/index.json
- examples/automation/octos/README.md
- examples/automation/octos/gen_scenario.py
- examples/automation/octos/run_campaign.py
- examples/automation/octos/setup_scenario.py
- examples/automation/octos/pristine/**
- examples/automation/mock_octos.rs
- build.rs

### Forbidden
- Do not answer `context/compaction_completed` as a JSON-RPC result, and do not
  add it to `index.json` `results` — it is a notification only.
- Do not hand-write or hand-edit the generated `canned(method)` lookup; it is
  produced by `build.rs` from `index.json`.
- Do not wrap a data file in a JSON-RPC envelope; files hold the bare `result`.
- Do not renumber, reorder, or otherwise modify `turn_stream.json`.
- Do not hardcode an absolute machine path anywhere in the runner.
- Do not change the captured `session_id` or `profile_id` literals in any data
  file — the guest rewrites them at run time.
- Do not modify any file under the octoscode repository's `src/`.

## Out of Scope

- Checkpoint and rewind (`last_checkpoint_id`) coverage.
- A `context/normalization_reported` stream; only the compaction notification is
  in scope.
- Running against a real octos server, or recording a fresh `tcp_fiddler` capture.
- Any change to how the TUI renders the compaction block.
- The `session/open` assembly path and the four code-answered methods.

## Completion Criteria

Scenario: session compact answers from its own body
  Test: test_session_compact_answers_from_its_own_body
  Level: integration
  Test Double: baked-in corpus, no live octos server
  Given `index.json` lists `session/compact` against `session_compact.json`
  When the guest receives a `session/compact` request
  Then the reply result is the parsed body of that file
  And the reply id equals the live request id

Scenario: the compaction stream follows the compact reply
  Test: test_compaction_stream_follows_the_compact_reply
  Level: integration
  Test Double: baked-in corpus, no live octos server
  Given a `session/compact` request from the live client
  When the guest has sent the reply
  Then the next frames are the `compaction_stream.json` notifications in order
  And the first notification method is "context/compaction_completed"
  And the session id in every notification is the live session id

Scenario: one body answers both compaction modes
  Test: test_compact_mode_set_echoes_requested_mode
  Given `session_compact_mode_set.json` holds a single captured body
  When the client sends `session/compact/mode/set` with the following params:
    | mode      |
    | llm       |
    | heuristic |
  Then each reply echoes the mode it was sent
  And both replies come from the same file

Scenario: a new case is a file plus an index entry
  Test: test_index_entries_generate_consts
  Given the two new entries in `index.json`
  When `build.rs` runs
  Then the generated lookup resolves "session/compact" and "session/compact/mode/set"
  And no hand-written arm was added to the generated lookup

Scenario: the compaction notification is never answered as a result
  Test: test_compaction_completed_is_never_answered_as_result
  Given `context/compaction_completed` has no `results` entry
  When a client calls it as a JSON-RPC method
  Then the reply is a JSON-RPC error "-32000" naming the method
  And no body from `compaction_stream.json` is returned as a result

Scenario: a malformed body fails the build naming the file
  Test: test_malformed_body_fails_build_naming_file
  Given `session_compact.json` contains invalid JSON
  When `build.rs` runs
  Then the build fails
  And the failure message contains "session_compact.json"

Scenario: replayed compaction seq is contiguous
  Test: test_compaction_stream_seq_is_contiguous
  Given `compaction_stream.json` as baked into the guest
  When the stream is replayed
  Then the envelope seq values run from "1" with no gap and no repeat
  And the client emits no "protocol/replay_lossy"

Scenario: dropping the compact method makes the menu unavailable
  Test: test_scenario_context_compact_method_absent
  Level: integration
  Test Double: mutated `config_capabilities_list.json`, deployed guest
  Given the scenario "context-compact-method-absent" is installed
  When `supported_methods` omits "session/compact"
  Then `server/heartbeat` reports that scenario id
  And the heartbeat `modified` names "config_capabilities_list.json"
  And the expectation states the menu renders unavailable with a reason

Scenario: dropping only the mode method keeps the compact row
  Test: test_scenario_context_mode_set_absent
  Level: integration
  Test Double: mutated `config_capabilities_list.json`, deployed guest
  Given the scenario "context-mode-set-absent" is installed
  When `supported_methods` omits "session/compact/mode/set" but keeps "session/compact"
  Then the expectation states the compact row remains and the two mode rows are gone
  And no dead row is offered

Scenario: a failed compaction is served as a failure
  Test: test_scenario_context_compaction_failed
  Given the scenario "context-compaction-failed" is installed
  When the summary carries status "error" with an error string
  Then `token_estimate_after` is absent
  And `output_generation` is absent
  And the expectation states the client surfaces the failure rather than a saving

Scenario: a regressing generation is served without being repaired
  Test: test_scenario_context_compaction_generation_regress
  Given the scenario "context-compaction-generation-regress" is installed
  When `output_generation` is lower than `input_generation`
  Then the data set is deployed unchanged
  And the expectation states what the client is claimed to do with a regressed generation

Scenario: the new area is registered alongside the existing eight
  Test: test_context_area_is_registered
  Given `gen_scenario.py` registering the areas "agents, capabilities, goal, hydrate, llm, loops, status, turn"
  When the context scenarios are added
  Then `setup_scenario.py --list` reports "context" as a ninth area
  And `FILES` maps "compaction" to "compaction_stream.json"

Scenario: the loop runner walks every context scenario
  Test: test_context_campaign_walks_every_scenario
  Level: integration
  Test Double: stub deploy and stub client driver
  Given the context scenarios listed by `setup_scenario.py --list context`
  When `run_campaign.py context` runs
  Then each scenario is installed, deployed and driven exactly once
  And the run reports one result row per scenario

Scenario: a stale deploy aborts the run before driving
  Test: test_campaign_aborts_on_heartbeat_scenario_mismatch
  Level: integration
  Test Double: stub deploy that returns ok without rebuilding
  Given a scenario is installed
  And the deployed guest reports a different `scenario_id`
  When `run_campaign.py context` reaches that scenario
  Then the run fails naming both the installed and the reported scenario
  And the client is never driven for that scenario

Scenario: the compaction stream is part of the restorable baseline
  Test: test_compaction_stream_is_seeded_and_restored
  Level: integration
  Test Double: temporary data dir seeded from the committed corpus
  Given `compaction_stream.json` declared as an `index.json` sibling with no `results` entry
  When `setup_scenario.py --seed` runs and then a context scenario mutates the stream
  Then `pristine/` contains "compaction_stream.json"
  And `setup_scenario.py --restore` returns the file to its baseline bytes

Scenario: an aborted run restores the baseline
  Test: test_campaign_restores_baseline_on_failure
  Level: integration
  Test Double: stub driver that raises mid-campaign
  Given a campaign that fails partway through
  When the runner exits
  Then `setup_scenario.py --current` reports the baseline
  And the three new files match `pristine/`

Scenario: restore returns the extended baseline
  Test: test_restore_returns_extended_baseline
  Given a context scenario has been installed over the data set
  When `setup_scenario.py --restore` runs
  Then the three new files are present and match `pristine/`
  And every restored file is written with a current mtime
