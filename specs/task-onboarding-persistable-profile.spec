spec: task
name: "Provider saves require a persistable profile identity"
inherits: project
tags: [tui, onboarding, provider, server-truth]
depends: []
estimate: 0.5d
---

## Intent

An unresolved provider save omits `profile_id`, causing the server to use its
runtime identity `_main`. That identity cannot be persisted by the profile
store. A `profile/llm/list` response can also populate the client cache with
`_main`, so checking only for `Some` is insufficient.

Primary, fallback, and research provider saves require a nonempty profile id
containing only lowercase ASCII letters, digits, and hyphens. A refused save
shows guidance to create or select a profile and leaves no pending save spinner.
Provider test and model discovery remain usable before a profile is resolved;
this rule applies at the persistence boundary, not to runtime reads.

## Acceptance Criteria

场景: An unresolved primary provider save stays local
  测试: onboarding_save_refuses_to_dispatch_without_a_resolved_profile
  假设 No profile id is resolved and a provider is selected
  当 The user submits /onboard save
  那么 No upsert is dispatched and the wizard explains how to resolve a profile
  并且 No provider save remains pending

场景: An unresolved fallback provider save stays local
  测试: provider_fallback_save_refuses_to_dispatch_without_a_resolved_profile
  假设 No profile id is resolved and a fallback provider is selected
  当 The user submits /provider add-fallback
  那么 No upsert is dispatched and no provider save remains pending

场景: A runtime identity backfilled by a list response cannot enable persistence
  测试: onboarding_save_refuses_a_runtime_only_profile_id_from_the_list_response
  假设 profile/llm/list has populated the client cache with _main
  当 The user submits /onboard save
  那么 No upsert is dispatched and the wizard requests a persistable profile

场景: The persistence gate distinguishes valid profile slugs from runtime ids
  测试: persistable_profile_id_rejects_runtime_and_malformed_slugs
  假设 A profile id is supplied to the persistence gate
  当 The gate validates that id
  那么 Lowercase ASCII letters, digits, and hyphens are accepted in nonempty ids
  并且 Empty ids, runtime ids, uppercase, underscores, spaces, and dots are refused

场景: A resolved primary provider save retains its profile and masks its secret
  测试: onboarding_slash_flow_builds_profile_llm_upsert_and_masks_secret
  假设 A persistable profile and provider are selected
  当 The user submits /onboard save
  那么 The upsert carries the selected profile and the displayed state masks the key

场景: A resolved fallback provider save remains available
  测试: provider_slash_flow_builds_fallback_llm_upsert
  假设 A persistable profile and fallback provider are selected
  当 The user submits /provider add-fallback
  那么 A fallback upsert retains the selected provider and does not replace the primary
