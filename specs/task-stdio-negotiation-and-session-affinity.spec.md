# Stdio negotiation and session-affinity barriers

## Contract

- A stdio connection sends the supported OUP `client_hello` RPC before any
  ordinary request. Its `supported_features` are the same feature tokens used
  by WebSocket negotiation; `OCTOSCODE_OLD_SERVER_FEATURES=1` sends only the
  compatibility baseline.
- A server that rejects, malforms, or does not answer `client_hello` cannot
  wedge startup. OctosCode falls back to the server's legacy stdio defaults
  and releases queued requests after a bounded timeout.
- Every `session/open`, including a reconnect reopen and a local A-to-B tab
  switch, is a request-id-correlated barrier. No later session-bound command
  may be written until the matching response opens the expected session.
- An errored, malformed, mismatched, or timed-out `session/open` fails every
  queued command explicitly and recycles the connection. It must never leave
  an unbounded FIFO behind an uncleared barrier.
- A local tab switch queues `session/open(B)` before any B status probe or
  restored staged `turn/start`, and updates the transport's reconnect target.
- A connection that has not produced a capability set re-asks
  `config/capabilities/list` on the SAME connection. One unanswered request —
  e.g. one flushed into a stdio child that was still booting when the startup
  grace expired — must not leave the session capability-blind until it
  reconnects, hiding `/onboard`, `/login` and the permission menu behind
  "Octos UI capabilities are not available". The retry is clocked from the
  wire (not from `bootstrap`, since the request is routinely deferred behind
  `client_hello`), is suppressed while the child is still booting or the hello
  barrier is armed, and is bounded by a fixed attempt budget so a server that
  withholds the method is asked a few times and then left alone.

## Scenarios

- `unanswered_capabilities_request_is_reasked_on_the_same_connection` — a stdio
  child that rejects `client_hello` and swallows the first
  `config/capabilities/list` is asked a second time, and that answer reaches the
  store as a `Capabilities` event.
- `capabilities_retry_stops_after_the_attempt_budget` — a probe that has spent
  its budget starts no further attempt.

## Compatibility

WebSocket negotiation remains header-based. Older stdio servers that do not
implement `client_hello` retain their prior default feature behavior; no new
server method is assumed beyond the already-supported OUP `client_hello`.
