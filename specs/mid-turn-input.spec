Feature: Deliver new user input while a turn is working

Scenario: Enter steers; Tab explicitly queues
Test: tab_queues_draft_even_when_enter_steers_by_default
Given an accepted live turn on a steering-capable server
When the user presses Enter with text
Then send turn/steer with the active turn ID
When the user presses Tab with text
Then retain it as a separate FIFO turn and clear the admitted draft
And preserve slash/bang command dispatch and empty-Tab agent navigation

Scenario: Send pending input now without duplicating or replacing drafts
Test: ctrl_x_sends_pending_once_and_preserves_composer_draft
Given an accepted live turn and pending input
When Ctrl+X is pressed
Then interrupt once and keep the unfinished draft
And wait for terminal before sending queued work
And do not restore the interrupted request into the composer

Scenario: An explicit preference still queues
Test: mid_turn_prompts_stage_fifo_when_steering_disabled
Given /steer off
Then mid-turn input remains FIFO even on a capable server

Scenario: Diff preview retains its shortcut
Test: ctrl_x_stages_the_selected_hunk_from_composer_focus
Given an active diff preview
Then Ctrl+X stages selected diff context instead of interrupting

Scenario: Esc releases queued input without waiting for a terminal
Test: esc_interrupts_when_staged_messages_are_waiting
Given an active turn and explicitly queued user input
When Esc interrupts the active turn
Then enqueue the queued input behind the interrupt as a fresh turn
And keep later input FIFO behind the fresh submit gate

Scenario: A late interrupt terminal reconciles without double-settling or draining
Test: late_interrupt_terminal_after_fresh_turn_started_does_not_double_settle_or_drain
Given Esc has released queued input and its fresh turn has started
When the interrupted turn's real terminal arrives late or replays
Then preserve the already-settled partial answer and the fresh live turn
And do not drain another queued prompt
