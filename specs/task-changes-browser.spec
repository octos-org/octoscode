# Changes browser

- `/diff` opens a file list for the selected session preview, falling back to the server-bound workspace diff when no preview exists. `/diff workspace` explicitly includes all uncommitted changes, without attributing them to this turn.
- Up/Down selects a file, Enter opens its diff, Esc returns to the list and then the composer. Tab changes scope. Empty workspaces and errors remain closable.
- `c` quotes the selected hunk into the composer for editing, including during a live turn. It never automatically queues or sends that question. Ctrl+X remains Send now.
- Inline previews have no persistent shortcut toolbar or width warning; unified view is the default and narrow side-by-side falls back silently.
- A newer workspace request, scope change, session change or close invalidates older results.
- Workspace reads carry the active session's profile so legacy session IDs resolve to the same server-bound workspace as `session/open`; no client path is sent.

Tests:
- event_loop::tests::changes_browser_lists_opens_quotes_and_returns_to_files
- event_loop::tests::ctrl_x_never_quotes_diff_from_composer_focus
- store::tests::workspace_diff_drops_old_scope_and_session_responses
- transport::tests::workspace_diff_wire_carries_session_profile_and_preserves_local_request_identity
- store::tests::selected_diff_hunk_context_is_quoted_without_autoqueue_during_live_turn
- app::tests::tests::render_unified_diff_has_no_persistent_shortcut_toolbar
- app::tests::tests::render_side_by_side_falls_back_to_unified_below_min_width
