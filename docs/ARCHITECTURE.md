# octoscode Architecture

## Scope

`octoscode` is a standalone terminal client for the Octos UI Protocol.
In protocol mode it does not run the Octos agent, execute tools, approve
commands, maintain the durable ledger, or own provider/model configuration.
Those responsibilities belong to the `octos serve` process.

The TUI owns:

- terminal rendering and keyboard handling
- local view state, focus, scroll, expansion, composer draft, and staged input
- optimistic display of the user's submitted prompt
- local slash commands such as `/ps`, `/stop`, and `/help`
- translation between user interactions and stable `AppUiCommand` values

The server owns:

- session creation and session cwd validation
- agent/runtime execution
- shell/tool execution and sandbox policy
- approval requests, approval decisions, and approval scopes
- task supervisor state and background task registry
- durable UI event ledger, replay, and `protocol/replay_lossy` reporting
- diff preview and task output data sources

## Runtime Topology

```text
User keyboard
  |
  v
octoscode
  src/event_loop.rs       terminal draw/read/send loop
  src/store.rs            AppUI reducer and follow-up command builder
  src/app.rs              ratatui panes, markdown, tasks, diffs, approvals
  src/transport.rs        mock or protocol backend
  |
  | AppUiCommand -> JSON-RPC over WebSocket
  v
ws://HOST:PORT/api/ui-protocol/ws
  |
  v
octos serve
  crates/octos-cli/src/api/ui_protocol.rs
  crates/octos-core/src/app_ui.rs
  crates/octos-core/src/ui_protocol.rs
  |
  v
Octos runtime
  sessions, agent turns, tools, approvals, task supervisor, ledger
  |
  | UiNotification / UiProgressEvent / RPC results
  v
octoscode Store -> AppState -> ratatui render
```

`octoscode` and `octos-app` should both depend on the AppUI contract, not on
M9 or future M10 implementation details. As long as the AppUI API remains
compatible, client behavior should survive server-internal milestone changes.

## Server Endpoints

The AppUI endpoint is:

```text
/api/ui-protocol/ws
```

That route is implemented in the Octos repo under
`crates/octos-cli/src/api/ui_protocol.rs`. It accepts JSON-RPC messages over a
WebSocket and translates protocol commands into runtime actions.

WebSocket is the current deployed transport, not the Octos UI Protocol itself. The
transport refactor milestone is documented in the parent Octos repo at
`api/APPUI_TRANSPORT_PROTOCOL_REFACTOR_MILESTONE.md`. The intended long-term
shape is that the same `AppUiCommand` and `AppUiEvent` contract can run over
WebSocket, stdio, Unix sockets, local TCP streams, named pipes, or in-process
test channels.

The older endpoint:

```text
/api/ws
```

is the legacy web chat/gateway WebSocket. It is not the AppUI contract used by
`octoscode`.

## Shared API Types

The client consumes shared Rust types from the sibling Octos repo:

```text
../octos/crates/octos-core/src/app_ui.rs
../octos/crates/octos-core/src/ui_protocol.rs
```

`app_ui.rs` is the app-facing API layer. `ui_protocol.rs` is the JSON-RPC wire
protocol layer. The TUI should prefer the `AppUi*` types at its boundary and
keep wire-specific details inside `src/transport.rs`.

## Protocol Commands

Every variant of `AppUiCommand` (`src/model.rs`) and the wire method
`AppUiCommand::method()` maps it to. The transport turns each into a JSON-RPC
request and records the result kind it expects back.

This list is COMPLETE and kept that way by `tests/docs_drift.rs`, which fails
if a variant is added to the enum without being documented here. Do not prune
it by hand.

### `profile/`

| AppUI command | Wire method |
|---|---|
| `ListModels` | `profile/llm/list` |
| `ProfileLlmCatalog` | `profile/llm/catalog` |
| `ProfileLlmDelete` | `profile/llm/delete` |
| `ProfileLlmFetchModels` | `profile/llm/fetch_models` |
| `ProfileLlmList` | `profile/llm/list` |
| `ProfileLlmSelect` | `profile/llm/select` |
| `ProfileLlmTest` | `profile/llm/test` |
| `ProfileLlmUpsert` | `profile/llm/upsert` |
| `ProfileLocalCreate` | `profile/local/create` |
| `ProfileSkillsInstall` | `profile/skills/install` |
| `ProfileSkillsList` | `profile/skills/list` |
| `ProfileSkillsRegistrySearch` | `profile/skills/registry/search` |
| `ProfileSkillsRemove` | `profile/skills/remove` |
| `ProfileSubProvidersList` | `profile/sub_providers/list` |
| `ProfileSubProvidersRemove` | `profile/sub_providers/remove` |
| `ProfileSubProvidersUpsert` | `profile/sub_providers/upsert` |
| `SelectModel` | `profile/llm/select` |

### `session/`

| AppUI command | Wire method |
|---|---|
| `ClearSessionGoal` | `session/goal/clear` |
| `CompactContext` | `session/compact` |
| `GetSessionGoal` | `session/goal/get` |
| `HydrateSession` | `session/hydrate` |
| `ListSessions` | `session/list` |
| `OpenSession` | `session/open` |
| `ReadSessionStatus` | `session/status/read` |
| `SessionBtw` | `session/btw` |
| `SessionRollback` | `session/rollback` |
| `SetCompactionMode` | `session/compact/mode/set` |
| `SetSessionGoal` | `session/goal/set` |

### `agent/`

| AppUI command | Wire method |
|---|---|
| `CloseAgent` | `agent/close` |
| `InterruptAgent` | `agent/interrupt` |
| `ListAgentArtifacts` | `agent/artifact/list` |
| `ListAgents` | `agent/list` |
| `ReadAgentArtifact` | `agent/artifact/read` |
| `ReadAgentOutput` | `agent/output/read` |
| `ReadAgentStatus` | `agent/status/read` |

### `mcp/`

| AppUI command | Wire method |
|---|---|
| `DeleteMcpConfig` | `mcp/config/delete` |
| `ListMcpConfig` | `mcp/config/list` |
| `ListMcpStatus` | `mcp/status/list` |
| `SetMcpConfigEnabled` | `mcp/config/set_enabled` |
| `TestMcpConfig` | `mcp/config/test` |
| `UpsertMcpConfig` | `mcp/config/upsert` |

### `tool/`

| AppUI command | Wire method |
|---|---|
| `DeleteToolConfig` | `tool/config/delete` |
| `ListToolConfig` | `tool/config/list` |
| `ListToolStatus` | `tool/status/list` |
| `SetToolConfigEnabled` | `tool/config/set_enabled` |
| `TestToolConfig` | `tool/config/test` |
| `UpsertToolConfig` | `tool/config/upsert` |

### `loop/`

| AppUI command | Wire method |
|---|---|
| `CreateLoop` | `loop/create` |
| `DeleteLoop` | `loop/delete` |
| `FireLoopNow` | `loop/fire_now` |
| `ListLoops` | `loop/list` |
| `PauseLoop` | `loop/pause` |
| `ResumeLoop` | `loop/resume` |

### `task/`

| AppUI command | Wire method |
|---|---|
| `CancelTask` | `task/cancel` |
| `ListTasks` | `task/list` |
| `ReadTaskArtifact` | `task/artifact/read` |
| `ReadTaskOutput` | `task/output/read` |
| `RestartTaskFromNode` | `task/restart_from_node` |

### `auth/`

| AppUI command | Wire method |
|---|---|
| `AuthLogout` | `auth/logout` |
| `AuthMe` | `auth/me` |
| `AuthSendCode` | `auth/send_code` |
| `AuthStatus` | `auth/status` |
| `AuthVerify` | `auth/verify` |

### `turn/`

| AppUI command | Wire method |
|---|---|
| `GetTurnState` | `turn/state/get` |
| `InterruptTurn` | `turn/interrupt` |
| `SubmitPrompt` | `turn/start` |
| `TurnSteer` | `turn/steer` |

### `approval/`

| AppUI command | Wire method |
|---|---|
| `ListApprovalScopes` | `approval/scopes/list` |
| `RespondApproval` | `approval/respond` |

### `permission/`

| AppUI command | Wire method |
|---|---|
| `ListPermissionProfiles` | `permission/profile/list` |
| `SetPermissionProfile` | `permission/profile/set` |

### `snapshot/`

| AppUI command | Wire method |
|---|---|
| `SnapshotList` | `snapshot/list` |
| `SnapshotRestore` | `snapshot/restore` |

### `peer/`

| AppUI command | Wire method |
|---|---|
| `WorkspaceTeam` | `peer/team/list`, `peer/team/leader/set`, `peer/team/message` |
| `PeerGather` | `peer/gather` |
| `PeerPrepare` | `peer/prepare` |

### `user_question/`

| AppUI command | Wire method |
|---|---|
| `RespondUserQuestion` | `user_question/respond` |

### `diff/`

| AppUI command | Wire method |
|---|---|
| `GetDiffPreview` | `diff/preview/get` |

### `launch/`

| AppUI command | Wire method |
|---|---|
| `LaunchResolve` | `launch/resolve` |

### `thread/`

| AppUI command | Wire method |
|---|---|
| `GetThreadGraph` | `thread/graph/get` |

### `review/`

| AppUI command | Wire method |
|---|---|
| `StartReview` | `review/start` |

### `config/`

| AppUI command | Wire method |
|---|---|
| `ListConfigCapabilities` | `config/capabilities/list` |

### `local/`

| AppUI command | Wire method |
|---|---|
| `LocalShellExec` | `local/shell_exec` |

## Protocol Notifications

Every `UiNotification` variant the pinned `octos-core` defines, and the wire
method it arrives as. `Store::apply_notification` currently handles all of
them — the match is exhaustive on purpose, so a variant added to `octos-core`
becomes a compile error here rather than silently unhandled.

An unknown or future notification must never crash the UI. Where it cannot be
rendered it should degrade to a visible warning or status item.

This list is kept complete by `tests/docs_drift.rs`.

### `session/`

| Notification | Wire method |
|---|---|
| `SessionEventBridged` | `session/event` |
| `SessionGoalCleared` | `session/goal/cleared` |
| `SessionGoalUpdated` | `session/goal/updated` |
| `SessionOpened` | `session/open` |
| `SessionOrchestration` | `session/orchestration` |

### `turn/`

| Notification | Wire method |
|---|---|
| `TurnCompleted` | `turn/completed` |
| `TurnError` | `turn/error` |
| `TurnSteerDropped` | `turn/steer_dropped` |
| `TurnSpawnComplete` | `turn/spawn_complete` |
| `TurnStarted` | `turn/started` |

### `monitor/`

| Notification | Wire method |
|---|---|
| `MonitorExpired` | `monitor/expired` |
| `MonitorFired` | `monitor/fired` |
| `MonitorUpdated` | `monitor/updated` |

### `background/`

| Notification | Wire method |
|---|---|
| `BackgroundActivity` | `background/activity` |

### `judge/`

| Notification | Wire method |
|---|---|
| `JudgeDecision` | `judge/decision` |

### `approval/`

| Notification | Wire method |
|---|---|
| `ApprovalAutoResolved` | `approval/auto_resolved` |
| `ApprovalCancelled` | `approval/cancelled` |
| `ApprovalDecided` | `approval/decided` |
| `ApprovalRequested` | `approval/requested` |

### `visual/`

| Notification | Wire method |
|---|---|
| `VisualFailed` | `visual/failed` |
| `VisualGenerating` | `visual/generating` |
| `VisualSucceeded` | `visual/succeeded` |

### `tool/`

| Notification | Wire method |
|---|---|
| `ToolCompleted` | `tool/completed` |
| `ToolProgress` | `tool/progress` |
| `ToolStarted` | `tool/started` |

### `agent/`

| Notification | Wire method |
|---|---|
| `AgentArtifactUpdated` | `agent/artifact/updated` |
| `AgentOutputDelta` | `agent/output/delta` |
| `AgentUpdated` | `agent/updated` |

### `loop/`

| Notification | Wire method |
|---|---|
| `LoopCompleted` | `loop/completed` |
| `LoopFired` | `loop/fired` |
| `LoopUpdated` | `loop/updated` |

### `context/`

| Notification | Wire method |
|---|---|
| `ContextCompactionCompleted` | `context/compaction_completed` |
| `ContextCompactionStarted` | `context/compaction_started` |
| `ContextNormalizationReported` | `context/normalization_reported` |
| `ContextStateReported` | `context/state_reported` (ignored; `context.state.v1` is not negotiated) |

### `message/`

| Notification | Wire method |
|---|---|
| `MessageDelta` | `message/delta` |
| `ReasoningDelta` | `message/reasoning_delta` |

### `voice/`

| Notification | Wire method |
|---|---|
| `VoiceAudioChunk` | `voice/audio_chunk` |
| `VoiceExit` | `voice/exit` |

### `task/`

| Notification | Wire method |
|---|---|
| `TaskOutputDelta` | `task/output/delta` |
| `TaskUpdated` | `task/updated` |

### `router/`

| Notification | Wire method |
|---|---|
| `RouterFailover` | `router/failover` |
| `RouterStatus` | `router/status` |

### `projection/`

| Notification | Wire method |
|---|---|
| `Envelope` | `projection/envelope` |
| `EnvelopeV2` | `projection/envelope` |
| `PeerClosed` | `peer/closed` |
| `SkillActionJobUpdated` | `skill/action_job_updated` |

### `user_question/`

| Notification | Wire method |
|---|---|
| `UserQuestionCancelled` | `user_question/cancelled` (ignored; the picker is already cleared when the asking turn ends) |
| `UserQuestionRequested` | `user_question/requested` |

### `plan/`

| Notification | Wire method |
|---|---|
| `PlanUpdated` | `plan/updated` |

### `progress/`

| Notification | Wire method |
|---|---|
| `ProgressUpdated` | `progress/updated` |

### `warning/`

| Notification | Wire method |
|---|---|
| `Warning` | `warning` |

### `protocol/`

| Notification | Wire method |
|---|---|
| `ReplayLossy` | `protocol/replay_lossy` |

### `file/`

| Notification | Wire method |
|---|---|
| `FileAttached` | `file/attached` |

### `queue/`

| Notification | Wire method |
|---|---|
| `QueueState` | `queue/state` |

### `peer/`

| Notification | Wire method |
|---|---|
| `PeerStaged` | `peer/staged` |
| `WorkspaceTeamUpdated` | `peer/team/updated` |

## Client Layers

One row per file under `src/`. `tests/docs_drift.rs` fails if a source file is
added without a row here.

### Entry and configuration

| File | Lines | Responsibility |
|---|---:|---|
| `src/cli.rs` | 1053 | `--config` JSON launch defaults plus CLI overrides. Must not own provider/model settings; those stay in Octos server config. |
| `src/cmd/config.rs` | 152 | `octoscode config`: read-only inspection of the client's startup config |
| `src/cmd/doctor.rs` | 2460 | `octoscode doctor` — flutter-doctor-style diagnostics (design §B). |
| `src/cmd/github.rs` | 155 | Minimal GitHub Releases client for `update --check` and `doctor`. |
| `src/cmd/install_method.rs` | 746 | Install-method detection for `octoscode update`/`doctor` (design §A.3). |
| `src/cmd/mod.rs` | 273 | `octoscode` subcommands: `update` and `doctor` (design doc). |
| `src/cmd/update.rs` | 606 | `octoscode update` — install-method-aware updater (design §A). |
| `src/cmd/olp.rs` | 681 | `octoscode olp <组> <动作>` — the one entry point for the OctoLoop tools (docs/OLP_CLI.md); phase one runs the scripts bundled into the binary from a per-version directory. |
| `src/cmd/olp_mcp.rs` | 20 | `octoscode olp-mcp-serve` entry — OUTER_LOOP_REVIEW #31 OLP-MCP server subcommand. |
| `src/cmd/outer_duty.rs` | 98 | `octoscode outer-duty` entry — OUTER_LOOP_REVIEW #38 per-project duty lock (hold/check). |
| `src/cmd/olp_runtime_watch.rs` | 1008 | `octoscode olp-runtime-watch` — observation-only runtime guard: bounded process probes, the target turn's ledger alerts, and a no-progress stall advisory, as NDJSON; never accepts a task. |
| `src/cmd/olp_runtime_watch_ledger.rs` | 945 | Runtime-guard ledger reader — incremental, session/turn-scoped JSONL polling into whitelisted alerts (read-only; Unix-only); consumed by `olp_runtime_watch.rs`. |
| `src/lib.rs` | 317 | Crate root — module declarations and the shared public surface. |
| `src/main.rs` | 53 | Binary entry point: subcommand dispatch, then `event_loop::run`. |

### Core loop and state

| File | Lines | Responsibility |
|---|---:|---|
| `src/client_event.rs` | 377 | Decoded RPC results and autonomy results, in the shape the store reduces. |
| `src/event_loop.rs` | 7047 | Terminal raw mode, alternate screen, draw loop, keyboard dispatch, backend polling, send-error handling. |
| `src/model.rs` | 11612 | `AppState`, the TUI view models, `AppUiCommand`, and the mapping from AppUI snapshots/tasks/messages into renderable state. |
| `src/outer_duty.rs` | 358 | Outer-duty kernel lock: fs2 try_lock_exclusive, VACANT/HELD/ERROR, atomic 0600 metadata sidecar (#38); death coupling via PDEATHSIG on Linux and a guardian process on macOS. |
| `src/olp/init.rs` | 447 | `octoscode olp init` and `olp skill install`: lays the two-loop scaffold in a project (dependency check, `.octos/loop.md`, the blackboard, `.gitignore`, the onboarding card, the single-file helpers) without overwriting anything, and installs the path-free `/octoloop` entry; the card and helpers are compiled in; replaces `scripts/olp-init.sh` and `scripts/olp-install-skill.sh` for the CLI |
| `src/olp/keepalive.rs` | 998 | `octoscode olp keepalive heartbeat\|install\|uninstall\|list\|render\|doctor`: registers what must always run on a machine as jobs of the current user — launchd agents on macOS, systemd user units on Linux — and reports (never changes) the settings that keep them from running unattended; replaces `scripts/olp-launchd.py` and `scripts/olp-systemd.py` |
| `src/olp/localpr.rs` | 603 | `octoscode olp repo status\|promote\|push`: the state of each `task/<N>` branch on the hub worked out from refs and the blackboard (Markdown or event log), the gate that lets only an accepted, verified head go to GitHub, and the unit's push with its verification record; replaces `scripts/olp-local-pr.py` and `scripts/olp-task-push.sh` |
| `src/olp/log.rs` | 490 | The JSON Lines event log, the default store: append-only segments per project, every write decided and appended under one lock (idempotency keys, claim-once, numbering), torn tails cut off, corrupt lines refused. |
| `src/olp/mod.rs` | 16 | OctoLoop tools in Rust: the `scripts/olp-*` commands move here one at a time, command line, output and exit codes unchanged (docs/OLP_CLI.md). |
| `src/olp/answer.rs` | 422 | `octoscode olp ask list\|show\|answer\|serve`: the way back for an `ask_outer` question — the hub reaches the unit's mailbox over ssh (or in-process when the unit is this machine), where `serve` is the forced command; only a question still waiting can be answered; replaces `scripts/olp-answer.py` |
| `src/olp/append.rs` | 311 | `octoscode olp board append` and `olp id alloc`: the two writes that hand out numbers, under the same `<file>.lock` that `flock(1)` takes — so they exclude the scripts they replace and need no `flock` program |
| `src/olp/archive.rs` | 414 | `octoscode olp board archive`: moves the text of settled entries to an archive file and leaves numbered stubs that keep every machine-read line; archive first, then an atomic replace under the append lock, given up if the board changed underneath; replaces `scripts/olp-board-archive.py` |
| `src/olp/board.rs` | 462 | The one reader of the blackboard's Markdown: lines outside code fences, the live (non-historical) part, numbered entries and the signed line-start forms (verdicts, ACKs, pushes, questions). |
| `src/olp/card.rs` | 396 | `octoscode olp card open/ack/accept/return/void/snooze`: a card's life written as events, each decided and appended under the log's lock (next number, key once, no writes to a missing or voided card). |
| `src/olp/cards.rs` | 219 | Cards from the event log: replays events into the same picture a scan of the Markdown board gives (sequence numbers for line numbers), so every command judges both stores by one set of rules. |
| `src/olp/dispatch.rs` | 378 | `octoscode olp card dispatch --log`: one-step dispatch, to an event log (`--log`) or a Markdown board (`--board`, with a herdr wake) — the unit's health, whether it can run the card's reviewers, its load and the verify-of rule are checked and the card written under the log's lock; carries the card's `avoid` list |
| `src/olp/client.rs` | 627 | The unit's side of the event log: `remote ack / progress / mirror` send one request to the hub's forced command (ssh, or run locally when `hub = local`); keyed so a retry is recorded once; the mirror is this unit's cards only, fetched incrementally. |
| `src/olp/doctor.rs` | 867 | `octoscode olp doctor`: checks the OctoLoop commands written into ssh forced commands, hub hooks, local push settings and keepalive jobs (pinned to one version's tool directory, missing, or found only through PATH), the hub remote of work repositories, where the skill reads from, old-format unit heartbeats, and processes still running a replaced binary; read-only, with a fix per finding; replaces `scripts/olp-doctor.py` |
| `src/olp/duty.rs` | 41 | The outer-duty lease check every writing command makes first (R7); same rule as the scripts' `duty_gate`, calling the Rust lease verification directly. |
| `src/olp/forward.rs` | 253 | `octoscode olp guard forward`: reads the runtime guard's records and appends one alert line per new event to the hub's blackboard, whitelisted fields only, once while the event lasts; replaces `scripts/olp-alert-forward.py` |
| `src/olp/import.rs` | 190 | `octoscode olp board import`: a Markdown board into a new event log, events numbered in source-line order; one-time, refuses a non-empty log. |
| `src/olp/inbox.rs` | 526 | `octoscode olp board inbox`: each entry's state from the last thing said about it (inbox / working / snoozed / settled), unit occupancy; output identical to `scripts/olp-inbox.py`. |
| `src/olp/mdremote.rs` | 576 | `octoscode olp remote append\|ack\|flush\|queue\|mirror\|serve`: a unit writing to a hub whose blackboard is a Markdown file — the write runs on the hub under the board's lock through the forced command `serve` (own keys only, no outer-signed lines, no numbered headings), and is queued and resent in order when the hub is unreachable; replaces `scripts/olp-board-remote.py` |
| `src/olp/outer.rs` | 872 | `octoscode olp outer supervise\|resume\|install`: brings the outer loop back when its duty lock goes VACANT (never a second outer, never over a live lock, paused after repeated failures), hands the restarted agent a resume prompt, prints the take-over briefing, and registers itself as a keepalive job; replaces `scripts/olp-outer.py` |
| `src/olp/outerwatch.rs` | 236 | `octoscode olp outer watch`: the outer loop's watcher on the event log — when a unit's ACK, a recorded push, a question, a failed delivery or a cluster stall waits for the outer loop and its agent is idle, one fixed sentence is typed into its pane; keeps its own cursor so nothing that arrived while the outer loop was busy is skipped |
| `src/olp/render.rs` | 178 | `octoscode olp board render`: the event log as a read-only Markdown board that the inbox reads to the same conclusions as the log. |
| `src/olp/repo.rs` | 677 | `octoscode olp repo init\|serve\|connect-local\|local-pack\|hook`: the hub's bare git repository — the ssh forced command that pins the unit, and the receive hooks that let a unit push only its own `task/<N>` with a verification record signed by every reviewer the card names; reads the card from a Markdown board or an event log and records the push there; replaces `scripts/olp-git-hub.py` |
| `src/olp/sentinel.rs` | 140 | `octoscode olp board watch`: the board sentinel — fires on a token in lines appended after arming, skipping signed notes; `--harvest` runs the evolution harvest on each hit and keeps watching; replaces `scripts/olp-watch-board.sh` |
| `src/olp/serve.rs` | 266 | `octoscode olp remote serve-log`: the hub side of a unit's ssh forced command on the event log — ACKs and progress nodes only for cards dispatched to that unit, and a view of its own cards' events only. |
| `src/olp/stall.rs` | 292 | `octoscode olp board stall`: whether the whole cluster has gone quiet with cards open — no working inner loop on any unit that holds one and no new event for a while — and, per card, what it waits on, since which event and who can move it; the same silence is reported three times at most |
| `src/olp/summary.rs` | 381 | `octoscode olp board summary`: cards, counts, units, local pull requests, alerts on open cards and pending questions in one read-only report, from a Markdown board or an event log; replaces `scripts/olp-summary.py` |
| `src/olp/unit.rs` | 1359 | `octoscode olp unit`: the unit registry and heartbeats — `register`, `set-state`, `receive`, `status`, `check`, `where` on the hub, `heartbeat` on a unit; same files and output as `scripts/olp-unit.py`, which it replaces |
| `src/olp_mcp.rs` | 397 | OLP-MCP outer-loop server: newline JSON-RPC stdio, mailbox quota/timeout, board audit (#31). |
| `src/store.rs` | 37021 | The AppUI reducer: snapshots, RPC results, notifications, local commands, approvals, diffs, task output and queued prompts folded into `AppState`. |

### Transport and backend

| File | Lines | Responsibility |
|---|---:|---|
| `src/shared_instance.rs` | 280 | Authenticated shared local WebSocket server discovery, startup locking and detached launch. |
| `src/workspace_team.rs` | 160 | Workspace peer team wire types and the `/agents` member/coordinator picker. |
| `src/backend_ensure.rs` | 1140 | Auto-provision the `octos` server backend so a fresh octoscode install |
| `src/profiles.rs` | 544 | Phase 3 startup profile discovery. |
| `src/transport.rs` | 10721 | `AppUiBackend`, the mock and protocol backends, WebSocket/stdio framing, auth, reconnect status and in-memory cursors. |

### Rendering

| File | Lines | Responsibility |
|---|---:|---|
| `src/app.rs` | 5505 | ratatui surfaces — transcript, composer, docks, approvals, diff preview, status bar — and the re-exports for the `app/*` submodules. |
| `src/app/activity_nav.rs` | 558 | `activity_nav` — extracted from `app.rs` (#365 step 2). Items keep their |
| `src/app/markdown_highlight.rs` | 516 | Style-only markdown highlighting for the composer draft. |
| `src/app/render.rs` | 2172 | `render` — extracted from `app.rs` (#365 step 2). Items keep their |
| `src/app/pinned_question.rs` | 165 | Latest submitted question header and read-only expansion for sticky scrolling. |
| `src/app/tests.rs` | 12480 | Test module for [`crate::app`] (#365): moved out of `app.rs`, which was |
| `src/app/transcript_build.rs` | 3615 | `transcript_build` — extracted from `app.rs` (#365 step 2). Items keep their |
| `src/highlight.rs` | 172 | Fenced-code-block syntax highlighting for the transcript renderer |
| `src/insert_history.rs` | 1603 | Insert finalized history lines into the terminal's **normal scrollback**, |
| `src/sanitize.rs` | 160 | Terminal control-sequence sanitisation for server-supplied text. |
| `src/splash.rs` | 292 | Startup splash: a ttfx-rendered OCTOS logo animation played on the main screen before the event loop claims the terminal. |
| `src/terminal_probe.rs` | 243 | Terminal detection and color adaptation for octoscode. |
| `src/theme.rs` | 204 | Terminal-aware palettes and theme-specific colors. |
| `src/tui_terminal.rs` | 1171 | Inline-viewport terminal — ported and trimmed from codex-rs `tui/src/custom_terminal.rs`. |
| `src/viewport.rs` | 576 | Inline-viewport driver: owns the scrollback-flush bookkeeping that turns |

### Menu framework

| File | Lines | Responsibility |
|---|---:|---|
| `src/menu/availability.rs` | 695 | Capability gating — which menus the connected server advertises. |
| `src/menu/mod.rs` | 21 | Menu framework model and generic render surfaces. |
| `src/menu/multi_select_view.rs` | 501 | Multi-select menu surface. |
| `src/menu/preview_layout.rs` | 133 | Preview-pane scroll window, row truncation and the range indicator, shared by both menu surfaces. |
| `src/menu/providers.rs` | 11969 | Local and capability-backed menu providers for the M9.34 framework. |
| `src/menu/registry.rs` | 1583 | The canonical slash-command registry: names, aliases and capability gating. |
| `src/menu/render.rs` | 441 | Generic menu renderer shared by every provider. |
| `src/menu/selection_view.rs` | 843 | Single-select menu surface. |
| `src/menu/types.rs` | 921 | `MenuSpec` / `MenuItem` / `MenuAction` — the framework core types. |
| `src/menu/wizard.rs` | 417 | First-run setup wizard step model. |

### Composer and input

| File | Lines | Responsibility |
|---|---:|---|
| `src/autonomy.rs` | 1097 | M15-E autonomy command parsing for `/agents`, `/goal`, and `/loop`. |
| `src/clipboard.rs` | 378 | Clipboard copy support for the TUI. |
| `src/file_picker.rs` | 254 | `@` composer file picker (#363, v1: path insert only). |
| `src/history.rs` | 743 | Composer command-history navigation (codex / claude-code style). |
| `src/keymap.rs` | 1 | The status-bar key-hint string. |

## Menu Framework

Codex-style menus should be implemented as a reusable TUI framework, not as
one-off slash handlers. The milestone plan lives in
`docs/M9_34_MENU_FRAMEWORK_MILESTONE.md`.

The intended boundary is:

- generic command registry, slash popup, selection views, and menu stack live in
  `octoscode`
- local menus such as `/theme`, `/statusline`, `/title`, and `/keymap` remain
  local TUI concerns
- server-backed menus such as `/model`, `/status`, `/permissions`, and `/mcp`
  must use AppUI capabilities and typed `AppUiCommand` values
- menu content providers must plug into the framework without changing generic
  renderer or composer logic

## Protocol Startup Flow

1. `octoscode` parses CLI launch preferences.
2. `build_backend()` creates either `MockAppUiBackend` or
   `ProtocolAppUiBackend`.
3. In protocol mode, `bootstrap()` connects to `/api/ui-protocol/ws`.
4. If a session id is present, the TUI sends `session/open` with
   `session_id`, `profile_id`, requested `cwd`, and any known replay cursor.
5. The server validates the session and cwd against its policy, replays durable
   ledger events after the cursor, and sends the current session view.
6. `Store::from_snapshot()` hydrates local state and the renderer draws the
   first frame.

## Turn Flow

1. User presses Enter in the composer.
2. `Store::compose_command()` creates a new `turn_id`, appends the submitted
   user message locally, and returns `AppUiCommand::SubmitPrompt`.
3. The transport sends `turn/start`.
4. The server emits turn, message, tool, approval, task, progress, and warning
   events.
5. `Store::apply_client_event()` applies each event and may return a follow-up
   command, for example `diff/preview/get` after a diff approval request.
6. `src/app.rs` renders active work separately from completed activity so the
   user can see what is running and what already finished.

## Approval Flow

Approval decisions are server-owned. The TUI renders approval details and sends
one of the server-supported choices:

- approve this request
- approve an allowed scope, such as session or tool, when advertised
- deny this request

The TUI must stop the visible waiting state when the server emits
`approval/decided`, `approval/auto_resolved`, `approval/cancelled`, or a turn
interrupt/error that invalidates the approval.

## Task and Output Flow

Task lifecycle is server-owned. The TUI renders task state from
`task/updated` and task snapshots. On-demand task details use
`task/output/read`; live output deltas use `task/output/delta` when the server
emits them.

The UX target is Codex-style task visibility:

- active work is sticky near the composer
- completed work appears in the transcript with past-tense labels such as
  `Ran`, `Explored`, and `Waited`
- long command output, diffs, and file/document previews collapse by default
- expanded cards expose as much useful command/file detail as the terminal can
  fit

## Durability and Replay

The durable source of truth is the server ledger, not TUI memory. The TUI keeps
only local presentation state and in-memory replay cursors.

Server durability requirements:

- append durable lifecycle events before sending them
- replay missed durable events after a client cursor
- never apply stale disk replay over a newer live session snapshot
- surface lossy delivery with `protocol/replay_lossy`
- keep terminal task states deliverable under backpressure

Client replay requirements:

- request replay using the latest known cursor when opening a session
- tolerate duplicate durable notifications
- treat `protocol/replay_lossy` as a signal to refresh/reopen rather than as a
  normal chat message
- never infer runtime truth from local optimistic UI state after reconnect

## Mock Mode

`--mode mock` is a deterministic local fixture backend for rendering,
keyboard, and harness tests. It does not represent the live Octos runtime and
must not be used to validate server policy, sandboxing, provider setup, ledger
durability, or tool execution behavior.

## Readonly Mode

`--readonly` opens a protocol session as a viewer. Mutating commands should be
blocked locally, and unavailable protocol connections may fall back to a
readonly offline snapshot for inspection.

## Codex-Style Reference Architecture

This section is an observable product-level reference model for comparison. It
is not a statement about private OpenAI implementation details.

The Codex CLI surface exposes an integrated local coding-agent process with
interactive TUI, non-interactive `exec`, code `review`, MCP support,
configurable sandbox policy, configurable approval policy, optional web search,
local working-directory selection, and optional remote/app-server modes.

A useful architecture model is:

```text
User terminal
  |
  v
Codex CLI/TUI process
  |-- renderer, composer, status line, transcript, tool cards
  |-- local session/config/history
  |-- approval policy and sandbox policy
  |-- local tool runner for shell/files/MCP
  |-- optional remote/app-server connection
  |
  | model requests and streamed responses
  v
model/provider service
  |
  | tool-call decisions, text deltas, plan/status updates
  v
Codex CLI executes approved local tools and updates the transcript
```

The important product difference is where the runtime lives:

| Area | Codex-style local CLI | Octos AppUI architecture |
|---|---|---|
| UI | Local CLI/TUI process. | `octoscode` or `octos-app`. |
| Runtime owner | Mostly the local CLI process, with model service calls. | `octos serve`. |
| Tool execution | Local CLI sandbox/tool runner. | Server-side Octos runtime/tool system. |
| Approval policy | Local CLI approval flow. | Server-owned approval requests plus client rendering/response. |
| Durable replay | Local CLI/session behavior. | Server ledger and replay cursors. |
| Client/server contract | CLI implementation boundary. | Stable Octos UI Protocol boundary. |

For Octos, the product goal is to keep Codex-quality coding UX while preserving
a cleaner split: the terminal app is replaceable, and all clients speak the
same AppUI API to the Octos server.

## Architectural Invariants

- `octoscode` must not call Octos runtime internals directly.
- `octoscode` must not rely on M9-specific server internals outside AppUI.
- `octoscode` must treat the server as authoritative for tasks, approvals,
  diffs, tool results, cwd policy, sandbox policy, and replay.
- The server must not require TUI-specific behavior for protocol correctness.
- New client-visible runtime features should land in `octos-core` AppUI/UI
  Protocol types before TUI-specific rendering.
- Prompt shaping for better coding UX belongs in the server profile or harness
  prompt contract, documented separately in
  `docs/CODING_UX_PROMPT_CONTRACT.md`.
