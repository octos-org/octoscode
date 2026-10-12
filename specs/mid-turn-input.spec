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
