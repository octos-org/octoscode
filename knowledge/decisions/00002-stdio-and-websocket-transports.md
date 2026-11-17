---
kind: decision
id: ADR-002
title: "Use stdio by default and WebSocket for explicit endpoints"
status: Accepted
date: 2026-10-03
---

## Context

This records octoscode revision
`eff531212ce5953fc95720e4b9ae3089d90e0c68`. A bare local launch should reach a
real coding runtime, while an explicit endpoint must support an independently
running or remote server. Transport selection should not create two different
interaction contracts.

## Decision

A protocol launch without a configured endpoint or stdio command uses
`octos serve --stdio --solo`. Before entering terminal mode,
`backend_ensure` locates or provisions a compatible backend for the default
local launch. An explicit `--stdio-command` selects a child command; an explicit
`--endpoint` selects the WebSocket AppUI endpoint. The two transport options
are mutually exclusive.

Both paths use the AppUI/UI Protocol contract through `AppUiBackend` and
`ProtocolBackend`. Stdio is an implemented transport, not a future design
proposal. The live WebSocket path is `/api/ui-protocol/ws`; the older `/api/ws`
gateway is a different interface.

Connection bootstrap negotiates capabilities and establishes Session scope.
Reconnect must restore the relevant scoped opens before releasing deferred
Session commands. Hydrate and replay use server-owned cursors and Session
identity, including when more than one Session is active.

Mock mode is explicit. It supplies deterministic fixture behavior for rendering
and tests; it cannot validate real provider calls, server permissions, sandbox
policy, or durable execution.

## Consequences

Good, because a local installation can start without a separately managed daemon,
while remote deployments keep the same command and event boundary.

Bad, because the client must manage child-process startup/exit as well as socket
failure. Successful framing alone is insufficient: reconnect must preserve
capability and Session barriers before deferred work resumes.

## Alternatives Considered

- Require every user to run a WebSocket daemon before starting the TUI.
  That adds setup to the default local workflow.
- Default to mock behavior when transport is absent. A bare launch would appear
  functional without reaching a real coding runtime.
- Replay deferred commands immediately after reconnect. That can run a command
  before its intended Session scope is restored.

## Evidence

- [CLI defaults and transport selection](../../src/cli.rs), including
  `should_default_bare_launch_to_stdio_protocol` and
  `rejects_endpoint_and_stdio_command_together`
- [Backend provisioning](../../src/backend_ensure.rs) and
  [transport implementation](../../src/transport.rs)
- [Scoped reconnect contract](../../specs/task-stdio-reconnect-scope-barrier.spec)
- [Negotiation and Session affinity contract](../../specs/task-stdio-negotiation-and-session-affinity.spec.md)

## Next

Maintain the existing transport contracts when changing bootstrap or reconnect.
Additional transports require an explicit decision and behavior contract; this
record does not claim Unix sockets or named pipes are implemented.
