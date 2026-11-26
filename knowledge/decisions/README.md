# Decisions

MADR-style decision records. One decision per file, `NNNNN-slug.md`.
These records explain implemented boundaries, their trade-offs, and alternatives.
The initial records capture existing main-branch behavior on 2026-10-03; their
dates are recording dates, not claims about when the architecture first shipped.

| Record | Decision |
|---|---|
| [ADR-001](00001-client-runtime-boundary.md) | Keep agent/runtime authority in `octos serve`, with explicit TUI-host operations. |
| [ADR-002](00002-stdio-and-websocket-transports.md) | Default to a local stdio backend and use WebSocket for explicit endpoints. |

Start with [the architecture guide](../../docs/ARCHITECTURE.md) for the current
command and source-file maps. Use [the template](adr-template.md) for a new
decision, include code/spec evidence, and assign the next sequence and ADR id.
Supersede an accepted decision with a new linked record when its boundary
changes; preserve the original rationale.

Routine implementation choices with no real trade-off belong in code/comments.
