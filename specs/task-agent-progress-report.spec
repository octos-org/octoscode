spec: task
name: "Agent progress reports in the transcript"
inherits: project
tags: [tui, oup, activity, progress]
---

## Intent

With `event.agent_report.v1` (octos UPCR-2026-R3DB2ECCA) a running agent can
publish its own progress report. The server delivers it as a
`background/activity` row whose `origin_kind` is `agent` and which carries a
`report` object. OctosCode shows it in the session that owns the agent while
the turn is still running, as the agent's own statement: attributed, with its
kind spelled out, and never drawn as a verified result or an approval.

## Decisions

- The report rides on the existing background-activity store and section. No
  new event, store or pane.
- An agent group is drawn apart from observed monitor and fleet groups: a
  header naming who reports, text in reading colour, and a kind label
  (`Progress`, `Blocked`, `Correction`, `Self-check`). `Self-check` is a check
  the agent ran itself; no kind is drawn as a pass.
- Collapsed, the newest report is shown only while the turn it came from is
  running. Once that turn is over the group folds to its header and count, so
  a finished turn's blocker does not sit under a later answer as if it still
  held. Ctrl+O shows every retained report in order.
- A report never rewrites an earlier one. A correction is a later row.
- The capability is requested only from servers outside the old-server
  baseline. Without it the server sends no agent rows and offers the model no
  report tool, so nothing changes.

## Out of scope

- Reports from other sessions, staged peers' reports in the originator's
  session, pacing and heartbeats. These are server changes that follow.
- Localising the existing `event(s)`, `dropped` and `ctrl+o to expand` words
  of the activity section.

## Acceptance

Scenario: the capability is requested and the frame decodes
  Test: agent_report_frame_decodes_with_its_kind_turn_and_details
  The report object decodes with its kind, turn and details, a kind this client does not know still decodes, and the capability is requested from modern servers only (agent_report_capability_is_requested_from_modern_servers_only).

Scenario: a report names its kind, next step and evidence
  Test: agent_report_names_its_kind_next_step_and_evidence
  A blocker is drawn under an "Agent reports" header with its kind label, the agent's next step and its evidence pointer, and is not counted as an observed event.

Scenario: an unknown kind and a peer origin still read correctly
  Test: agent_report_draws_an_unknown_kind_as_progress_under_the_peer_name
  A kind this client does not know is drawn as plain progress, and a staged peer's group is headed by the peer's name.

Scenario: a correction is shown as a correction
  Test: agent_report_group_keeps_the_newest_report_and_shows_a_correction_in_order
  Collapsed, the newest report is visible. Expanded, the correction follows the report it corrects and neither replaces the other.

Scenario: a finished turn's report is not shown as current
  Test: agent_report_folds_to_its_header_when_its_turn_is_over
  The newest report is shown while its turn runs, folds to the header and count when the turn is over, and stays readable when expanded.

Scenario: a replayed report is drawn once
  Test: replayed_agent_report_is_drawn_once
  A report the client already holds is not added again on replay, and observed rows keep their existing behaviour.

Scenario: the cap marker is honest
  Test: agent_report_cap_marker_is_shown_and_not_counted_as_a_report
  The server's marker row for an agent origin is shown with the drop total and is not counted as a report.
