# Terminal answer integrity

Given a turn reaches a completed terminal without visible assistant text
When the terminal is applied, including an empty live reply
Then do not insert a synthetic Session Summary assistant message
And clear terminal busy state so subsequent prompts can run
And display a missing-answer diagnostic instead of a successful Done state

Given the master turn ends while its peer fleet remains active
Then show the fleet wait in status without fabricating an assistant message
And do not call the missing answer a failure while the fleet is outstanding

Given actual assistant text was streamed or persisted for the completed turn
Then retain it without appending a guessed partial-answer summary
And never borrow another turn's assistant text to suppress a missing-answer diagnostic

Given a stale or duplicate terminal is delivered
Then retain existing exact-turn idempotence and newer-turn protections

Historical Session Summary text and genuine error cards remain renderable.

Given whitespace-only peer setup, two commentary iterations, and the final answer stream before a delayed canonical commit batch
When each canonical row retains its own streamed segment identity and terminal/replay arrives
Then the real final answer and each commentary appear in native scrollback exactly once
And no Session Summary substitutes for that answer
Test: delayed_peer_commit_batch_keeps_final_visible_once_through_terminal_replay

Given a native assistant iteration streams only whitespace before a tool, a new segment, or turn completion
When that closed iteration has no displayable canonical row
Then omit only its exact empty segment from the committed dialogue without trimming any meaningful segment
And same-segment leading indentation, internal spacing, Unicode, and trailing whitespace remain intact
And compatibility v1 identities that span tool phases retain their existing content semantics
And delayed canonical text for an empty observed phase remains authoritative without erasing later segments or closing their in-flight indentation
And same-client reconnect preserves all previously hydrated and live native history exactly once
And genuine history rewrites still use the unchanged exact-prefix guard
Test: whitespace_only_native_iterations_do_not_replay_cold_history_after_hydrate
Test: whitespace_segment_cleanup_preserves_native_indentation_and_compatibility_lanes
Test: closed_whitespace_native_segment_retains_late_canonical_authority
Test: late_canonical_empty_phase_does_not_close_next_native_indentation
Test: trailing_whitespace_native_iteration_is_removed_at_terminal_or_turn_switch
Test: canonical_whitespace_native_row_stays_empty_and_boundaries_remain_bounded

Given a completed live turn combines several uniquely identified assistant iterations
When reconnect hydrates the exact canonical iteration rows more than once
Then reconcile the existing whole-turn aggregate without re-emitting its native scrollback
And preserve equal-text iterations, media, and reasoning without merging across user, steer, background, or turn boundaries
And keep fresh canonical history and genuinely rewritten history distinct
Test: reconnect_hydrate_of_multisegment_turn_keeps_native_answer_once
Test: hydrate_aggregate_reconciliation_preserves_identity_boundaries_and_media

Given a submitted input is visible before its canonical user echo
When a delayed empty active-turn hydrate arrives
Then preserve that session's unconfirmed full input rows before comparing scrollback fingerprints
And do not confirm or discard another session's optimistic inputs
And keep repeated identical inputs and their attachments distinct
And do not resurrect withdrawn inputs or explicitly rolled-back history
Test: hydrate_preserves_only_its_unconfirmed_optimistic_rows_with_media
Test: hydrate_preserves_two_identical_inputs_and_does_not_restore_withdrawn_one
Test: reconnect_hydrate_of_multisegment_turn_keeps_native_answer_once

Given the client has observed a turn's terminal and canonical answer
When a delayed hydrate still reports that same turn active or interrupting
Then keep the newer terminal history and request a fresh hydrate at most once per session, contradicted turn, and connection epoch
And a repeated stale response in either hydrate form must not create an automatic request loop
And do not compare different-domain cursors or reject a fresh terminal canonical rewrite
Test: delayed_active_hydrate_after_terminal_cannot_erase_canonical_answer

Given a protocol client starts with a selected session
Then connection and read-only state belong to status, not synthetic conversation rows
When the user submits before the initial canonical hydrate arrives
Then the real protocol bootstrap and native scrollback render the prompt exactly once
Test: protocol_bootstrap_delayed_hydrate_keeps_native_prompt_once
Test: protocol_snapshot_honors_requested_session

Given finalized conversation history exceeds the visible terminal height
When context and compact-confirm menus open and close repeatedly
Then repaint only the owned viewport without emitting prior user prompts or answers again
And preserve live turn coverage and the visible-history extent
And keep the vacated menu gap bounded across cycles and fill it with subsequent output
And retain the separate handling of true terminal resize and canonical history rewrite
Test: menu_open_close_with_long_native_history_never_reemits_old_turns
Test: menu_close_frame_clears_only_viewport_and_preserves_committed_transcript
Test: menu_close_frame_does_not_duplicate_orchestrating_chip_during_live_turn
Test: menu_open_close_cycles_do_not_accumulate_blank_gap

Given a persisted background completion has a canonical session message index
And its replay envelope has unrelated per-thread and ledger cursor sequence numbers
When reconnect hydration replaces the canonical row with its exact nonempty message ID
Then retain the row's canonical position, background provenance, and media
And do not re-emit the previously rendered native history on repeated hydrates
And subsequent output still appends once
Test: hydrate_background_message_order_uses_exact_canonical_row_position
Test: hydrate_background_message_order_reconnect_preserves_native_prefix_once

Given background rows and replay envelopes lack a matching nonempty message ID
Then do not delete or coalesce canonical rows based on equal text, source, thread, or unrelated sequence numbers
And retain unanchored envelopes after canonical rows in received order without guessing a message position
And an absent messages snapshot does not replace existing history with an empty snapshot
Test: hydrate_background_message_order_preserves_distinct_identity_even_with_equal_text
Test: hydrate_background_message_order_retains_unmatched_rows_and_appends_unknown_envelopes
Test: hydrate_background_message_order_unanchored_envelopes_keep_received_order

Given a background completion with a nonempty canonical message ID already owns a displayed row
When the same completion replays before reconnect hydration or through the other legacy/v2 lane
Then do not append another card, duplicate activity, or force native history replay
And retain the original media and exact session ownership
And distinct IDs with equal text and events without IDs remain separate
Test: hydrate_background_message_order_reconnect_preserves_native_prefix_once
Test: background_replay_identity_is_shared_by_legacy_and_v2_with_media
Test: background_replay_identity_scope_and_distinct_equal_text_are_not_coalesced

Given canonical hydration or a legacy snapshot replaces displayed history
Then background identity ownership follows retained rows rather than an ever-seen receipt cache
And hydrate-only background rows and repeated exact-ID envelopes are idempotent
And assistant aggregation and optimistic row insertion/removal preserve structural ownership indexes
And an absent messages snapshot keeps existing ownership
And rollback, row replacement, or removed sessions release ownership so a later completion can display
Test: background_replay_identity_hydrate_first_rows_only_and_absent_snapshot
Test: background_replay_identity_deduplicates_same_id_inside_one_hydrate_only
Test: background_replay_identity_tracks_current_snapshot_and_rollback_ownership
Test: background_replay_identity_moves_with_optimistic_insert_and_withdrawal
Test: background_replay_identity_uses_display_index_after_assistant_aggregation
Test: background_replay_identity_does_not_record_missing_session_or_keep_removed_owner
Test: background_replay_identity_snapshot_restores_optimistic_prefix_before_rebinding

Given a parent turn already archived its Spawn invocation
When a durable background completion arrives later and reconnect replays the originating ToolEnd
Then update only the exact session/turn Tool row, never the Progress row sharing its call ID
And recapturing terminal activity retains the original invocation and separately owns the canonical background activity
And repeated hydration never re-emits unchanged prompts or answers
Test: late_background_archive_preserves_spawn_and_native_history_after_replayed_tool_end
Test: tool_activity_updates_require_same_session_turn_and_kind_with_reused_call_id
Test: hydrate_tool_eligibility_does_not_share_call_ids_between_turns_or_sessions

Given two sessions have activity with the same turn ID and tool call ID
When hydration reports a terminal turn for only one session, with or without replayed tool envelopes
Then reconcile and archive only that session's explicitly attributed activity
And preserve the other session's running rows and a later turn's rows
And legacy activity without session attribution retains turn-only terminal reconciliation
Test: terminal_hydrate_activity_capture_isolates_sessions_with_same_turn_id

Given archived activity outlives the eight-turn live reply cache
When old activity grows or its render anchor moves while new messages append
Then append only unseen activity and new dialogue without replaying old messages or tools
And metadata-only activity changes do not reset native dialogue
And same-count rewrites, rewrite-plus-append, rollback shrink, and session switch retain real reset behavior
Test: late_activity_after_ten_turns_appends_only_new_activity_and_keeps_rewrite_guard
Test: late_activity_and_new_messages_share_a_frame_without_losing_delta_or_rewrite_guard
Test: archived_activity_anchor_move_on_first_answer_does_not_repeat_old_tool
Test: late_activity_log_archive_appends_activity_without_reflushing_messages

Given a committed activity log already rendered its turn footer
When its anchor moves to a newly appended assistant answer
Then retain separate footer ownership and do not repeat the old footer or tool
And a footer first arriving after dialogue still appends exactly once even without activity
And an ordinary live-settle flush still emits its first committed footer
Test: archived_activity_anchor_move_on_first_answer_does_not_repeat_old_tool
Test: late_summary_without_activity_is_appended_once_without_replaying_dialogue
Test: unflushed_activity_section_still_emits_turn_summary

Given an archived tool is still running when committed live coverage refreshes
Then do not acknowledge its item key before terminal output has actually rendered
And append the eventual terminal output once without replaying the answer
Test: archived_running_tool_is_not_acknowledged_before_its_terminal_output

Given an archived legacy activity log has no message anchor
Then exact session-and-turn live coverage still permits its full child rows to append once
And retain that coverage with the log after the shorter live-reply cache expires
And unknown unanchored logs are neither rendered nor pre-acknowledged by another turn's coverage
Test: scrollback_flush_keeps_children
Test: unanchored_archived_activity_requires_exact_owner_and_retains_once_coverage
