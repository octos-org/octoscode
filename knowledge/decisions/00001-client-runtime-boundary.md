---
kind: decision
id: ADR-001
title: "Keep agent runtime authority in octos serve"
status: Accepted
date: 2026-10-03
---

## Context

This records the architecture implemented at octoscode revision
`eff531212ce5953fc95720e4b9ae3089d90e0c68`. The terminal and browser clients
need compatible session, turn, approval, task, and replay semantics without
each implementing an agent runtime.

## Decision

The TUI owns input, terminal rendering, local view state, drafts, queued input,
and optimistic presentation. The reducer turns interactions into
`AppUiCommand` values; the transport translates protocol commands into
JSON-RPC and folds responses and notifications back into client state.

`octos serve` owns agent turns, model/provider configuration, model-requested
tools, sandbox policy, approvals, tasks, and the durable event ledger. The TUI
displays approval requests and sends the user's decisions; it does not become
the authority for the server's approval policy.

Runtime controls use the connected server's advertised methods and features.
On reconnect, local optimistic state cannot substitute for server replay and
hydrate. Session identity and replay attribution must remain correct for
foreground and background Sessions.

There are explicit client-host operations outside this runtime boundary. The TUI
can provision and spawn a local backend process. A user-entered `!command`
runs on the TUI host through `LocalShellExec`: the event loop lends the terminal
to a local shell and restores it afterwards. This is not the server's sandboxed
shell tool and is never sent as a model prompt or an AppUI RPC.

The `octos-core` dependency is pinned in `Cargo.toml`. Local client command and
event adapters can add presentation behavior, but server protocol and runtime
changes belong in the owning Octos repository. Unmerged PR features are not
part of this recorded main-branch contract.

## Consequences

Good, because multiple clients can share server-owned work and durable history,
and runtime policy has one owner.

Bad, because a feature can require coordinated client and server releases.
Capability checks, scoped recovery, and honest unavailable states are necessary.
Local shell escapes also have a different execution location and policy boundary
from model-requested server tools.

## Alternatives Considered

- Embed the agent loop, model provider, and durable task store in the TUI.
  That duplicates server authority and changes behavior between clients.
- Treat optimistic UI state as execution truth after reconnect. A sent prompt
  or displayed task does not establish the server's durable outcome.
- Send local shell escapes through the model or server tool runner. That changes
  the explicit host-local meaning of the user's command.

## Evidence

- [Architecture guide](../../docs/ARCHITECTURE.md)
- [Command model](../../src/model.rs), [reducer](../../src/store.rs), and
  [transport](../../src/transport.rs)
- [Local shell interception](../../src/event_loop.rs), covered by
  `every_bang_command_is_run_through_the_terminal_handoff`
- [Documentation coverage checks](../../tests/docs_drift.rs)

## Next

Preserve this ownership boundary when adding protocol features. New behavior
requires its corresponding requirement/specification and real test coverage;
this record documents existing behavior and creates no new runtime requirement.
