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

## Compatibility

WebSocket negotiation remains header-based. Older stdio servers that do not
implement `client_hello` retain their prior default feature behavior; no new
server method is assumed beyond the already-supported OUP `client_hello`.
