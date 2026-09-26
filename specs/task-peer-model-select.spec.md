spec: task
name: "/peer --model selects a stronger model for the new peer session"
inherits: project
tags: [tui, peer, model, oup]
estimate: 0.25d
---

## Intent

`/peer` had no way to request a non-default model for the session it spins
off — `PeerPrepareParams` carries `brief`/`worktree`/`cwd`/`n`/`session_id`/
`profile_id` but nothing model-related, so every peer ran on the profile's
default model regardless of task difficulty. Per CONTRIBUTING.md, octoscode
must not invent a client-only wire field on `peer/prepare` for this — the
real contract for changing a session's model is the existing `model/select`
OUP method (`AppUiCommand::SelectModel(ModelSelectParams)`), already used by
the `/model` menu. This adds a `--model <id>` flag to `/peer` that stashes
the requested id client-side (the same way `--go` already does) and fires
`model/select` at the peer's own session id right after its `session/opened`
lands — no new wire field, no server change.

## Decisions

- `--model <id>` takes a raw model id string (e.g. `deepseek-v4-pro`), the
  same vocabulary `ModelSelectParams::model` already uses — no tier keyword,
  no catalog validation at parse time.
- The id is stashed on `PendingPeerPrepare::model` (client-local, mirrors
  `go`), carried into `PeerKickoff::model` when `apply_peer_prepared_event`
  mints the kickoff(s), and consumed exactly once: when `session/opened`
  lands for that peer key, a `model/select` targeting that session id is
  pushed onto `pending_autonomy_hydration` — the same generic follow-up
  queue used for fleet extra-opens and hydration probes.
- Applies identically to `--go` and background peers, and to every member of
  a `--n <K>` fleet (each fleet member's kickoff carries the same `model`).
- `peer/staged` (agent-initiated peers via `peer_handoff`) has no `/peer`
  flags to read `--model` from, so its `PeerKickoff::model` is always `None`
  — agent-staged peers keep the profile's default model.
- No `--model` flag ⇒ `PeerKickoff::model` is `None` ⇒ no `model/select` is
  queued at all (the profile default is left untouched, not re-selected).

## Boundaries

### Allowed Changes
- src/model.rs
- src/store.rs
- src/app/tests.rs
- locales/en.yml
- locales/zh.yml
- specs/task-peer-model-select.spec.md

### Forbidden
- Do not add a `model` field to `PeerPrepareParams` or any other `peer/prepare`
  wire type — the requested id stays client-local until `model/select`.
- Do not validate or reject the `--model` value against a session's model
  catalog at parse time — parsing only rejects a missing value, exactly like
  `--cwd`.

## Completion Criteria

Scenario: `--model` is parsed and stashed, not sent over the `peer/prepare` wire
  Test: peer_slash_parses_model_flag
  Given a `/peer --go --model deepseek-v4-pro fix the thing` dispatch
  When the composer submits it
  Then the emitted `PeerPrepare` command's params contain no model field
  And `pending_peer_prepare.model` is "deepseek-v4-pro"

Scenario: `--model` with no value is rejected like `--cwd`
  Test: peer_slash_rejects_model_flag_without_value
  Given a `/peer --model` dispatch with nothing after the flag
  When the composer submits it
  Then the dispatch is rejected
  And the status is the `/peer` usage line

Scenario: a `--go` peer's session-open queues a `model/select` for its own id
  Test: peer_session_opened_with_model_enqueues_select_model
  Given a `/peer --go --model deepseek-v4-pro fix the nav` was prepared
  When that peer's `session/opened` lands
  Then the kickoff prompt is still returned directly as `SubmitPrompt`
  And a `SelectModel` command for the peer's session id and "deepseek-v4-pro" is queued on `pending_autonomy_hydration`

Scenario: a background (non-`--go`) peer's session-open also queues `model/select`
  Test: peer_session_opened_without_go_still_enqueues_select_model
  Given a `/peer --model deepseek-v4-pro fix the nav` was prepared without `--go`
  When that peer's `session/opened` lands
  Then a `SelectModel` command for the peer's session id and "deepseek-v4-pro" is queued on `pending_autonomy_hydration`

Scenario: no `--model` flag queues no `model/select`
  Test: peer_session_opened_without_model_flag_enqueues_no_select_model
  Given a `/peer --go fix the nav` was prepared with no `--model` flag
  When that peer's `session/opened` lands
  Then `pending_autonomy_hydration` contains no `SelectModel` command
