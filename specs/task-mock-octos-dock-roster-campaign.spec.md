spec: task
name: "wasm_mock_server fiddler mock for /dock roster plus a scenario loop runner"
tags: [mock, wasm-mock-server, fiddler, dock, agents, automation, campaign, upstream]
depends: []
estimate: 2d
---

## Intent

`/dock` (alias `/ag`, `src/menu/registry.rs:825`) is `CommandAvailability::always()`
and issues **no AppUI method at all** — the picker reads a client-side roster
mirror fed by `agent/updated` notification upserts, and renders `Unavailable`
when that mirror is empty. The corpus's `agent_list.json` is empty in the
capture and nothing in `examples/automation/octos` ever pushes `agent/updated`,
so every `/dock` surface — the per-agent rows, the `(viewing)` marker, the unread
`●`, the counts subtitle and the collapse toggle — has never rendered against
served data. This task gives the corpus a roster and a push stream that
populates it, and replaces the scratchpad campaign driver with a loop runner
that walks the dock scenarios in-repo. Implementation repo is **wasm_mock_rust**
(`/Users/alanpoon/Documents/go/wasm_mock_rust`); this contract ships with the
workstream.

## Decisions

- `agent_list.json` stops being the capture's empty `agents[]` and carries
  records built field-for-field to `UiAgentRecord` as the client decodes it
  (`agent_id`, `parent_agent_id`, `session_id`, `task_id`, `path`, `role`,
  `nickname`, `title`, `backend_kind`, `status`, `last_task`, `summary`,
  `output_tail`, `cwd`, `profile_id`, `runtime_policy_stamp`, `artifact_count`,
  `artifacts`, `created_at_ms`, `updated_at_ms`). The baseline holds three: one
  `running`, one `completed`, one `failed`.
- `agent/updated` is a **notification**. It gets no `results` entry in
  `index.json`; it lives in a new `agent_stream.json` declared as a sibling of
  `turn_stream`, the same shape `turn_stream.json` already uses.
- The picker sends nothing, so the mock cannot answer — it must **push
  unprompted**. The guest replays `agent_stream.json` after the
  `session/hydrate` reply, the first point at which the client holds a session
  for the roster to attach to. There is no `/dock` request on the wire to
  trigger on, and inventing one would contradict `registry.rs:825`.
- `agent/list` keeps being answered from its file for the separate `/agents`
  autonomy command. Its `agent_id` set must equal the set the stream upserts, so
  a client reading either surface lands on the same agents.
- Envelope `seq` is renumbered contiguously from 1, the rule `turn_stream.json`
  already follows, because the client reports `seq` gaps as
  `protocol/replay_lossy`.
- The guest rewrites `session_id` and `profile_id` through the existing
  `retarget` (`mock_octos.rs:156`); data files keep the captured literals.
- New scenarios extend the existing `agents` area of `gen_scenario.py`, which
  holds six today and already maps that area key to `agent_list.json` (line 24).
  One area key maps one file, so `FILES` gains a second key,
  `agent_stream` → `agent_stream.json`, and a dock scenario returns the
  multi-file `{"agents": …, "agent_stream": …}` — the shape `gen_scenario.py`
  already emits elsewhere (line 385) — so the list body and the pushed roster
  never disagree.
- The loop runner is `examples/automation/octos/run_campaign.py <area>`, taking
  the area as an argument so one driver serves every area rather than a copy per
  task; this task passes `agents`. It uses repo-relative paths only. Per
  scenario it runs `setup_scenario.py <name>`, deploys with
  `bash cli/mock_octos.sh`, reads `server/heartbeat`, asserts the reported
  `scenario_id` equals the one just installed, drives the client, collects the
  report, and moves on. It calls `setup_scenario.py --restore` in a finally
  block so an aborted run does not leave a mutated data set behind.
- The heartbeat assertion is not optional bookkeeping. `shutil.copy2` preserves
  source mtimes, cargo then skips the rebuild, and a campaign once tested one
  stale build fifty times while every deploy reported `ok`. The runner treats a
  heartbeat that disagrees with the installed scenario as a hard failure before
  any driving happens.
- No octoscode source changes. A client that mishandles a served case is a
  finding this contract reports, not a fix it makes.

## Boundaries

### Allowed Changes
- examples/automation/octos/agent_list.json
- examples/automation/octos/agent_stream.json
- examples/automation/octos/index.json
- examples/automation/octos/README.md
- examples/automation/octos/gen_scenario.py
- examples/automation/octos/run_campaign.py
- examples/automation/octos/pristine/**
- examples/automation/mock_octos.rs
- build.rs

### Forbidden
- Do not add `agent/updated` to `index.json` `results` — it is a notification.
- Do not make `/dock` issue an AppUI request; the command is local by contract.
- Do not hand-write or hand-edit the generated `canned(method)` lookup.
- Do not wrap a data file in a JSON-RPC envelope; files hold the bare `result`.
- Do not modify `turn_stream.json` or `compaction_stream.json`.
- Do not hardcode an absolute machine path anywhere in the runner.
- Do not modify any file under the octoscode repository's `src/`.

## Out of Scope

- The Peer Dock (Alt+P / Ctrl+L) — a different surface with its own roster.
- `agent/output/delta` and `agent/output/read` live-view streaming.
- `agent/artifact/list`, `agent/artifact/read`, `agent/artifact/updated`.
- `agent/close` and `agent/interrupt`.
- The `/agents` autonomy command's server-side RPC paths.

## Completion Criteria

Scenario: the roster populates from the pushed stream
  Test: test_dock_roster_populates_from_agent_updated_stream
  Level: integration
  Test Double: baked-in corpus, no live octos server
  Given the client has been answered a `session/hydrate` request
  When the guest replays `agent_stream.json`
  Then the frames are `agent/updated` notifications in order
  And the roster carries "3" agents
  And the session id in every notification is the live session id

Scenario: the list body and the stream name the same agents
  Test: test_agent_list_and_stream_agree_on_ids
  Given `agent_list.json` and `agent_stream.json` as baked into the guest
  When the agent ids of each are collected
  Then the two sets are equal
  And neither set is empty

Scenario: the stream is baked from an index sibling entry
  Test: test_agent_stream_is_baked_from_index
  Given `agent_stream.json` declared as a sibling of `turn_stream`
  When `build.rs` runs
  Then the generated const for the stream is emitted
  And `agent/updated` resolves to no entry in the `canned` lookup

Scenario: the loop runner walks every dock scenario
  Test: test_dock_campaign_walks_every_scenario
  Level: integration
  Test Double: stub deploy and stub client driver
  Given the dock scenarios listed by `setup_scenario.py --list agents`
  When `run_campaign.py agents` runs
  Then each scenario is installed, deployed and driven exactly once
  And the run reports one result row per scenario

Scenario: the notification is never answered as a result
  Test: test_agent_updated_is_never_answered_as_result
  Level: integration
  Test Double: baked-in corpus, no live octos server
  Given `agent/updated` has no `results` entry
  When a client calls it as a JSON-RPC method
  Then the reply is a JSON-RPC error "-32000" naming the method
  And no body from `agent_stream.json` is returned as a result

Scenario: an empty roster still renders the placeholder
  Test: test_scenario_dock_roster_empty
  Given the scenario "dock-roster-empty" is installed
  When `agent_list.json` and the stream are both emptied
  Then `server/heartbeat` reports that scenario id
  And the expectation states the picker renders Unavailable rather than an empty list

Scenario: replayed roster seq is contiguous
  Test: test_agent_stream_seq_is_contiguous
  Level: integration
  Test Double: baked-in corpus, no live octos server
  Given `agent_stream.json` as baked into the guest
  When the stream is replayed
  Then the envelope seq values run from "1" with no gap and no repeat
  And the client emits no "protocol/replay_lossy"

Scenario: a stale deploy aborts the run before driving
  Test: test_campaign_aborts_on_heartbeat_scenario_mismatch
  Level: integration
  Test Double: stub deploy that returns ok without rebuilding
  Given a scenario is installed
  And the deployed guest reports a different `scenario_id`
  When `run_campaign.py agents` reaches that scenario
  Then the run fails naming both the installed and the reported scenario
  And the client is never driven for that scenario

Scenario: an aborted run restores the baseline
  Test: test_campaign_restores_baseline_on_failure
  Level: integration
  Test Double: stub driver that raises mid-campaign
  Given a campaign that fails partway through
  When the runner exits
  Then `setup_scenario.py --current` reports the baseline
  And the three dock files match `pristine/`

Scenario: a malformed body fails the build naming the file
  Test: test_malformed_agent_body_fails_build_naming_file
  Given `agent_stream.json` contains invalid JSON
  When `build.rs` runs
  Then the build fails
  And the failure message contains "agent_stream.json"

Scenario: an unknown agent status is served unchanged
  Test: test_scenario_dock_unknown_status
  Given the scenario "dock-unknown-status" is installed
  When a record carries a status outside the known set
  Then the data set is deployed unchanged
  And the expectation states what the picker is claimed to render for it

Scenario: a repeated agent id upserts rather than duplicating
  Test: test_scenario_dock_duplicate_agent_id
  Given the scenario "dock-duplicate-agent-id" is installed
  When the stream pushes two `agent/updated` frames carrying one agent id
  Then both frames are replayed in order
  And the expectation states the roster holds one row for that id
