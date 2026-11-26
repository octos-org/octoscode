spec: task
name: "#526:中断 ack 了结回合并按序排空暂存队列"
inherits: project
tags: [tui, turn, interrupt, queue]
estimate: 0.5d
---

## Intent

#526: a staged prompt jammed permanently when the active turn was
interrupted — the client waited for a wire `TurnError` that may never
arrive, while the `turn/interrupt` RPC result (which the server always
sends) was flattened to a status string. The ack is now correlated to its
turn and settles it through the ordinary interrupted-terminal path: the
partial answer is committed, the deferred prompt restore applies, and the
ordinary drain serves Esc, Ctrl+C, Ctrl+X and `/stop` in order — never
bypassing the server's turn slot while the old turn is alive.

## Acceptance

Scenario: the ack settles the turn and drains the queue in order
  Test: interrupt_ack_settles_the_turn_and_drains_the_staged_prompt
  Partial answer committed before the staged prompt is submitted; run state
  reflects the new turn; the old prompt is not restored over the composer.

Scenario: every settled ack form serves the drain
  Test: interrupt_ack_timeout_and_terminal_state_also_settle
  ack_timeout (wire terminal lost) and an already-terminal report both
  settle and drain.

Scenario: a refused ack settles nothing
  Test: a_refused_interrupt_ack_settles_nothing
  interrupted=false without a terminal state keeps the live turn and queue.

Scenario: ack and wire terminal never settle twice
  Test: interrupt_ack_and_wire_terminal_never_settle_twice
  The late wire `TurnError` after an ack-settle drains without a duplicate
  "Turn interrupted" note.

Scenario: the transport correlates the ack to its turn
  Test: protocol_backend_maps_interrupt_success_to_cancel_status
  The typed TurnInterruptAck carries the request's session and turn.
