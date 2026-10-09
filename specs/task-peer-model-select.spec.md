spec: task
name: "/peer --model applies an isolated override before the first turn"
inherits: project
tags: [tui, peer, model, oup]
estimate: 0.25d
---

## Intent

`/peer --model <id>` requests a configured model for the newly staged peer.
The request carries `model_override: {model_id}` on `peer/prepare`, only
when the server advertises `peer.model_override.v1`. The backend resolves
and validates the configured model, persists the choice for every fleet
member, and returns only after staging succeeds. The ordinary session-open
and kickoff flow then starts on that model, including background peers.

`profile/llm/select` changes the profile default and must never implement
this option. No model-selection command is queued after session-open.
Older servers refuse the option at dispatch before a peer is staged.

## Decisions

- Accept a model ID, not a lane key. The server validates it against the
  profile's configured primary, fallback and sub-provider models. Unknown
  or ambiguous IDs fail clearly; the client does not guess a provider.
- No `--model` omits `model_override` and preserves the existing wire shape.
- The override is scoped to the staged peer (all members for `--n`), survives
  reconnects, and leaves the master and other peers' profile default alone.
- A missing value or another option in place of the value is a usage error.
- Keep the additive wire type local, as with other AppUI peer types, until
  the shared octos-core pin is advanced. The backend specification defines
  the same `PeerModelOverride` shape; the capability gate protects old servers.

## Boundaries

### Allowed Changes
- src/model.rs
- src/store.rs
- src/transport.rs
- src/app/tests.rs
- locales/en.yml
- locales/zh.yml
- specs/task-peer-model-select.spec.md

## Completion Criteria

Scenario: The model is included in preparation
  Test: peer_slash_parses_model_flag
  Given a capable server and `/peer --go --model deepseek-v4-pro fix it`
  When the composer submits it
  Then peer/prepare carries model_override with model_id deepseek-v4-pro

Scenario: An old server never receives an unsupported model override
  Test: peer_slash_model_requires_backend_support_before_staging
  Given peer/prepare support without peer.model_override.v1
  When `/peer --model strong fix it` is submitted
  Then dispatch rejects before stashing or staging a peer
  And `/peer fix it` remains available

Scenario: A model value is required
  Test: peer_slash_rejects_model_flag_without_value
  Given `/peer --model` without a value
  When the composer submits it
  Then dispatch shows the usage line

Scenario: An option is not a model ID
  Test: peer_slash_model_rejects_another_flag_as_value
  Given `/peer --model --go fix it`
  When the composer submits it
  Then dispatch rejects before staging

Scenario: Fleet staging carries one selection for every member
  Test: peer_slash_model_is_part_of_fleet_preparation
  Given `/peer --n 3 --model strong review it`
  When the composer submits it
  Then one peer/prepare request carries n 3 and model_override strong

Scenario: Foreground and background kickoffs never mutate the profile model
  Test: peer_session_opened_with_model_never_changes_profile_model
  Given a model override was included in successful preparation
  When the peer session opens with or without --go
  Then the kickoff is submitted without any queued profile model change

Scenario: Omitted model preserves default behavior
  Test: peer_session_opened_without_model_flag_enqueues_no_select_model
  Given a prepared peer without --model
  When its session opens
  Then no profile model selection is queued

Scenario: The wire codec preserves the scoped override
  Test: peer_prepare_model_override_encodes_before_session_open
  Given a prepare command with model_override strong
  When the transport encodes it
  Then the method is peer/prepare with model_override.model_id strong
  And an absent override is omitted entirely
