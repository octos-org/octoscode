spec: task
name: Cache unchanged committed transcript fingerprints
inherits: project
tags: [tui, performance, scrollback, history]
---

## Intent

The production inline loop reuses a committed-history fingerprint while input,
animation, and streaming reply text change. Revision-based invalidation must
observe every committed message mutation, including equal-length replacements.
The public raw fingerprint and default ScrollbackTracker retain fresh reads for
callers that directly mutate the public message Vec.

## Acceptance Criteria

Scenario: Unchanged committed messages are not rehashed on live frames
  Test: committed_fingerprint_cache_hits_during_input_animation_and_live_deltas
  Given the production revision-aware tracker has rendered committed history
  When the Store receives live deltas and the draft changes
  Then the fingerprint allocation is reused without reading history content

Scenario: Equal-length authoritative rewrites invalidate cached history
  Test: committed_fingerprint_cache_invalidates_same_length_hydrate_and_snapshot_rewrites
  Given committed history is cached
  When hydrate or snapshot replaces content with the same count and byte length
  Then the next fingerprint agrees with a fresh raw fingerprint

Scenario: Optimistic insertion and withdrawal update history identity
  Test: committed_fingerprint_cache_tracks_optimistic_append_withdraw_and_canonical_echo
  Given committed history is cached
  When an optimistic prompt is inserted, withdrawn, or a canonical user row arrives
  Then the next fingerprint reflects the exact message sequence

Scenario: Session changes and viewport resets preserve correctness
  Test: committed_fingerprint_cache_keys_sessions_and_survives_viewport_reset
  Given two sessions with equal message counts
  When selection changes and the viewport resets
  Then fingerprints follow the selected session and caching remains enabled
  And default public trackers still detect direct equal-length Vec rewrites

Scenario: Committing live text invalidates cached history
  Test: committed_fingerprint_cache_invalidates_completed_failed_and_switched_turns
  Given a cached committed transcript and an active live reply
  When the turn completes, fails, or is finalized by a successor turn
  Then the committed answer is included in the next fingerprint
