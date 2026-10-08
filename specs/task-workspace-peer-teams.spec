spec: task
name: "Automatic shared workspace peer teams"
inherits: project
tags: [tui, oup, peers, workspace]
---

## Intent

Independent local launches share an authenticated WebSocket server and keep
separate conversations. `/agents` discovers members, changes coordinator and
sends messages using server-owned peer state. Explicit stdio remains supported.

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
