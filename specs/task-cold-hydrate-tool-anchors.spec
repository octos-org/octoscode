spec: task
name: "Cold hydrate restores canonical tool owners"
inherits: project
tags: [tui, hydrate, tools, identity, replay]
---

## Intent

A fresh client must render replayed tool rows under their proven canonical
user turn, not silently retain invisible unanchored activity logs.

## Contract

- Resolve ownership only from a user row's explicit turn ID or a unique typed
  hydrated-turn/thread mapping within that session. Reject contradictory or
  ambiguous mappings in either direction (including an inferred turn also
  attributed to a different thread), unmapped rows, userless continuations and multiple user
  rows that do not establish one initiating prompt for an owner.
- Use the actual projected user index, never the raw canonical row sequence,
  an envelope sequence, the latest user message, or a text-match guess.
- Rebase projected indices through actual optimistic insertions. Preserve
  local attachments and distinct equal-text user occurrences.
- Restore anchors before tool archival and repair already archived logs only
  for the same proven session and turn, even when replay dedup skips recapture.
- Repeated hydration renders neither tool rows nor dialogue again.

## Scenarios

Scenario: Fresh canonical hydrate displays tools for equal prompts in distinct turns
  Test: cold_hydrate_tools_render_at_exact_typed_user_anchors_once
  Then: each tool result appears once under its own projected user anchor
  And: repeated hydrate produces no additional native rows

Scenario: Local pending insertion shifts canonical owners structurally
  Test: cold_hydrate_anchor_repair_survives_optimistic_structural_insertion
  Then: prior unanchored logs bind to their shifted canonical users
  And: the local pending attachment remains intact

Scenario: Explicit row ownership cannot rebind another session's same turn
  Test: cold_hydrate_explicit_turn_anchors_do_not_rebind_a_sibling_session
  Then: only the authoritative session's owned logs gain anchors

Scenario: Missing or contradictory identity is not replaced by text guessing
  Test: cold_hydrate_anchors_reject_ambiguous_unmapped_and_conflicting_ownership
  Then: those logs remain unattributed
