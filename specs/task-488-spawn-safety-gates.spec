spec: task
name: "/agents spawn 安全门(#488):同伴只读闸、父会话上限、错误不误报"
inherits: project
tags: [tui, agents, permissions]
estimate: 0.5d
---

## Intent

#488: `/agents spawn` bypassed the peer read-only gate, the spawn cap was
per-command while the server caps per parent session, and a count above u32
reported the wrong error. The read-only gate is now central (every
server-mutating command on a focused peer), the cap counts live children of
the focused session against the env-configurable server limit, refusals keep
the composer's draft, and the two terminal-status predicates are one.

## Acceptance

Scenario: spawn count parse and the configurable limit
  Test: agents_spawn_count_above_cap_is_rejected
  Test: agents_spawn_count_beyond_u32_reports_the_cap_not_invalid
  Test: max_children_per_parent_follows_the_env_with_the_server_default
  Counts above the per-session limit report the cap (localized), never the
  generic integer error; the limit follows OCTOS_MAX_CHILDREN_PER_PARENT.

Scenario: the cap counts live children of the parent session
  Test: agents_spawn_refused_when_live_agents_hit_the_parent_cap
  live + requested over the limit is refused with the draft kept; terminal
  agents do not count as live.

Scenario: every mutating command on a focused peer is read-only
  Test: peer_focused_mutating_commands_are_read_only_and_keep_the_draft
  Test: agents_spawn_on_peer_focused_session_is_read_only
  /agents spawn, /review, /goal, /loop and /gather refuse on a focused peer
  with the read-only status; autonomy refusals keep the draft.

Scenario: one terminal predicate
  Test: agent_status_terminal_predicates_agree
  Test: closed_agent_record_reconciles_a_stuck_task_to_cancelled
  The list count and the dock agree on every status; a closed record flips a
  stuck running task to cancelled, case-insensitively.
