spec: task
name: "Automatic shared workspace peer teams"
inherits: project
tags: [tui, oup, peers, workspace]
---

## Intent

Independent local launches share an authenticated WebSocket server and keep
separate conversations. `/agents` discovers members, changes coordinator and
sends messages using server-owned peer state. Explicit stdio remains supported.
A message another member sends is shown to the recipient's user as a card
attributed to the sending agent, never as a prompt that user typed.

## Acceptance

Scenario: transport selection preserves explicit choices
  Test: workspace_default_launch_is_shared_and_explicit_transports_are_preserved
  Bare launch chooses shared discovery; explicit stdio and remote WS retain their transports.

Scenario: fresh launch versus reconnect identity
  Test: workspace_launches_are_distinct_but_bootstrap_retry_keeps_identity
  New clients get different session IDs; bootstrap retry reuses its ID; older servers keep legacy behavior.

Scenario: coordinator and messages are capability gated
  Test: workspace_agent_controls_require_capability_revision_and_mutable_client
  Missing snapshots cannot elect; read-only clients may list but cannot elect or broadcast.

Scenario: picker uses authoritative state
  Test: workspace_picker_leader_action_uses_displayed_revision_and_readonly_has_no_mutations
  Election sends the displayed revision and read-only pickers expose no mutation.

Scenario: slash command parsing
  Test: workspace_agents_parse_leader_message_broadcast_and_reject_empty
  Leader, direct message and broadcast parse; empty requests are refused locally.

Scenario: discovery refuses mismatched endpoints before authentication
  Test: discovery_rejects_remote_or_wrong_runtime_before_sending_credentials
  A remote endpoint or another runtime cannot receive the local discovery credential.

Scenario: concurrent attachment verifies process identity
  Test: shared_discovery_checks_server_identity_and_concurrent_clients_reuse_owner
  Two clients attach to one locked runtime, never spawn a competing child, and reject a mismatched server identity.

Scenario: simultaneous cold starts share a real backend
  Test: shared_discovery_two_cold_launches_start_one_real_server
  Explicit integration run with OCTOS_WORKSPACE_TEST_BINARY verifies two concurrent launches share one authenticated server and one database-owner lock.

Scenario: simultaneous first-use activation stays in separate conversations
  Test: workspace_activation_menus_and_profile_switch_keep_clients_distinct
  Activate and cross-profile menus use the same per-client launch identity; profile switching cannot collapse independent clients into the legacy coding topic.

Scenario: older installed backends still launch
  Test: implicit_launch_probes_shared_support_and_preserves_explicit_transport
  Only implicit launches probe serve --help; matching backends use shared mode, older backends retain private stdio with a notice, and explicit/configured transports are unchanged.

Scenario: inherited data root selects the server's profile registry
  Test: inherited_octos_home_matches_server_profile_and_runtime_roots
  Implicit shared launches honor inherited OCTOS_HOME for profile discovery and runtime placement; an explicit data-dir still wins, and remote launches do not scan local profiles.

Scenario: an incoming peer message is an attributed card
  Test: peer_message_received_adds_one_card_to_the_recipient_session
  With peer.team_message_record.v1 the recipient session shows one card naming the sending agent; the durable replay adds no second card and another session gets none.

Scenario: the card survives reconnect and never shows the stored prompt
  Test: hydrated_peer_message_row_is_the_card_and_never_the_stored_prompt
  The hydrated record row becomes the same card in place, with the prompt the model was given nowhere in the transcript.

Scenario: an unreadable record is hidden
  Test: hydrated_peer_message_row_without_a_readable_record_is_hidden
  A recorded row whose sender cannot be read is omitted instead of appearing as a prompt the user typed.

Scenario: the card is drawn apart from the user's prompts
  Test: peer_message_card_is_attributed_and_never_drawn_as_a_user_prompt
  The card has its own header and gutter, labels an assignment, and shares no marker with the user block.

Scenario: another agent's text cannot drive the terminal
  Test: peer_message_card_strips_terminal_controls_and_requires_a_sender
  Control sequences are removed from the sender and the body, the sender is one bounded line, and a card with no sender is not created.

Scenario: a card is not the user's question or a rewind target
  Test: rewind_turns_skip_peer_message_cards
  The rewind picker neither offers nor counts a card, matching the server, and the pinned question header keeps the prompt the user typed (latest_question_skips_a_peer_message_card).

Scenario: the record feature is requested and decoded locally
  Test: should_decode_peer_team_message_received_when_the_frame_arrives
  The notification decodes before the pinned codec, hydrate and rollback rows are marked before the typed decode (should_mark_hydrated_peer_message_rows_before_the_typed_decode), and the feature is withheld from the old-server baseline (should_request_peer_message_records_only_outside_the_old_server_baseline).
