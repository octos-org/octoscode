spec: task
name: "Keep the latest submitted question fixed above the TUI transcript"
inherits: project
tags: [tui, rendering, scroll-mode, composer]
---

## Behavior

The default launch scroll mode is sticky. Explicit saved/CLI native or pinned
preferences remain authoritative. Sticky chat uses the alternate screen with
a separate question header, scrolling transcript, and fixed composer. The
existing native scrollback insertion path is unchanged. Bare /scrollmode toggles
sticky/native; explicit sticky/native/pinned and /saveconfig remain supported.

The selected session's newest submitted question replaces the header. A queued
question is labeled Queued, an in-flight steer is labeled Sending. Drafts and
background sessions do not replace it. Completion does not clear it. The
canonical transcript retains the original user message for history and copying.

The header displays at most three wrapped content rows, shrinking on small
terminals. F2/click opens a full read-only question viewer; arrows, PageUp,
PageDown, Home, End and wheel navigate, while Esc/F2/Enter closes without
editing the composer or sending. Approval and question dialogs retain priority.

## Acceptance

Scenario: Question remains fixed while the response changes
  Test: question_remains_at_top_through_scroll_stream_completion_and_resize
  Scrolling moves the answer but not the header. Completing a turn retains the
  question. The next submitted question replaces it. Resizing preserves the draft.

Scenario: Selected session and delivery state own the header
  Test: question_tracks_selected_session_queue_and_steering_without_using_the_draft
  Tab queues and updates the label; typing only changes the draft. Switching
  sessions restores the corresponding question/queue. Steering updates immediately.

Scenario: Read all of a long question without changing the draft
  Test: full_question_reader_reaches_the_end_without_editing_draft_or_scrolling_answer
  A long CJK/Unicode question has a reachable end. Typing/paste cannot mutate
  the hidden draft. Closing the reader does not interrupt or submit.

Scenario: Narrow terminals and mouse access
  Test: question_header_click_opens_reader_and_narrow_screens_keep_composer_reachable
  Question and draft remain visible at 20x8, 40x12 and 80x24. Clicking the header
  opens the reader. Changing session closes the old session's reader.

Scenario: Default, persistence, reconnect and native fallback
  Test: sticky_default_can_be_saved_restored_and_switched_back_to_native
  Saveconfig roundtrips sticky and reconnect keeps it. Explicit native restores
  the old mouse policy and removes the header.

Scenario: Launch settings precedence
  Test: sticky_scroll_default_respects_saved_and_explicit_preferences
  Empty config launches sticky; a saved native preference wins over the new
  default, and explicit --scroll-mode sticky overrides that saved preference.

Scenario: Incoming approval owns rendering and input above the question reader
  Test: incoming_approval_keeps_priority_over_the_question_reader
  An approval received while reading the question becomes visible and accepts
  the deny key without changing the draft or dismissing the question reader.
