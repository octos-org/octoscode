spec: task
name: Correlate input replies and preserve failed decisions
inherits: project
tags: [input, transport, reliability]
---

## Intent

Every steering reply or rejection settles only its originating input. Approval
and question responses retain recoverable state until the matching request or
server lifecycle confirms that the decision has settled. Local correlation
does not change the OUP wire schema.

Recovered prompts remain visible immediately. Automatic staged draining waits
until the same session's pending steer replies settle; precise success, refusal,
or disconnect cancellation releases that wait. Generic RPC deadlines and
recovery from a server that never answers remain outside this contract (tracked
in issue #232, item 8); this change does not introduce a general timeout.

## Acceptance Criteria

Scenario: A local rejection cannot consume an earlier steering request
  Test: input_response_local_rejection_does_not_consume_another_steer
  Given two distinct local steering requests, including identical prompt text
  When the second request is refused by the transport pending limit
  Then only its text is staged and the first request remains in flight
  And the first reply settles the first input without consuming the second

Scenario: Out of order replies retain session ownership
  Test: input_response_reordered_steers_stay_in_their_sessions
  Given steering requests from two sessions
  When the second succeeds before the first is rejected
  Then acceptance and recovery apply only to their originating sessions

Scenario: A rejected approval response can be retried
  Test: input_response_failed_approval_restores_the_original_card_and_can_retry
  Given an approval response is prepared
  When the transport refuses to send it
  Then the original card returns with its identity preserved
  And a later successful response clears its backup

Scenario: A rejected question preserves the selected answer
  Test: input_response_failed_question_preserves_answers
  Given a question with an answer selected
  When its response is rejected
  Then the original picker and answer are restored
  And a successful retry clears the retained state

Scenario: A decided approval cannot be resurrected by a late failure
  Test: input_response_late_approval_error_does_not_resurrect_a_decided_card
  Given an approval response is in flight
  When a decided notification precedes a late transport failure
  Then the card remains settled

Scenario: A failed peer approval remains owned by its background session
  Test: input_response_peer_approval_failure_stays_in_background
  Given a peer approval is answered without selecting that peer
  When the response is rejected
  Then its original card returns only to that peer's pending stash

Scenario: Disconnect cancellation recovers each input exactly once
  Test: input_response_disconnect_cancels_only_the_matching_inputs
  Given multiple steering requests are in flight
  When the transport cancels them on disconnect
  Then each originating input is staged once

Scenario: Cancelling across request id digit boundaries preserves dispatch order
  Test: input_response_cancel_9_10_restores_local_dispatch_order
  Given local steering requests have transport ids tui-9 and tui-10
  When disconnect cancellation arrives in a different order
  Then both prompts recover in their local dispatch order

Scenario: Rejected duplicate prompts retain their individual queue positions
  Test: input_response_reverse_rejections_preserve_duplicate_dispatch_order
  Given three steers including duplicate text precede a staged prompt
  When their RPC rejections arrive in reverse order
  Then recovery preserves local dispatch order ahead of the later staged prompt

Scenario: Accepted replies cannot reorder legacy terminal recovery
  Test: input_response_reverse_acceptance_then_legacy_terminal_preserves_dispatch_order
  Given a legacy server accepts two steers in reverse reply order
  When the active turn terminates without their canonical echoes
  Then the first dispatched prompt drains first and the second stays ahead of later staged input

Scenario: Snapshot replay preserves session-owned recovery order
  Test: input_response_snapshot_preserves_background_queue_order_and_session_owner
  Given one steer is rejected and another is pending for session A
  When a snapshot selects session B before the earlier A steer is rejected
  Then the A queue remains owned by A and recovers in dispatch order
  And B's staged input remains independent

Scenario: Raw public queue edits discard stale ordering metadata safely
  Test: input_response_staged_order_metadata_recovers_from_raw_vec_edits_and_front_priority
  Given a caller edits or clears the public queue Vec
  When recovery or explicit front staging runs
  Then stale positional metadata is discarded and explicit front priority survives

Scenario: Terminal cannot drain a later rejection before an earlier pending steer
  Test: input_response_terminal_waits_for_earlier_steer_before_draining_recovery
  Given B was rejected while earlier dispatched A is still pending
  When the active turn terminates
  Then B is visible in the staged queue but automatic draining waits
  And rejecting A later releases the backstop with A first

Scenario: A successful late reply also releases the pending drain gate
  Test: input_response_terminal_late_acceptance_releases_pending_drain_gate
  Given B is staged and terminal arrived before A's successful legacy reply
  When A's success settles the remaining pending request
  Then the idle backstop can drain B
