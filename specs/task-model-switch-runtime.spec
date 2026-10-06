spec: task
name: "Model selection reports saved and live state separately"
inherits: project
tags: [tui, model, protocol, server-truth]
---

## Contract

`profile/llm/select` saves a profile default shared by that profile's sessions.
`applied` alone does not prove a live model switch. Decode `restart_required`,
`runtime_disposition`, and `runtime_error` without breaking older replies.

Report `reloaded` as effective for the next message; report restart, deferred,
and failed replacement explicitly. A legacy saved reply has unconfirmed runtime
activation. Update the saved model catalog separately from the runtime status:
only an explicit reload without a restart requirement updates the latter.
A restart requirement retains the prior catalog marker until server readback,
as specified in task-model-selection-restart-required.spec.
Preserve request-to-session attribution and identify catalog rows by provider,
model, and route.

## Verification

- `model_select_messages_report_runtime_disposition`: reload, restart (including
  a legacy boolean hint), deferred, failed, unchanged, and legacy save messages.
- `model_select_keeps_saved_and_runtime_models_distinct`: a saved selection
  cannot overwrite the old runtime on restart, deferred, failed, or legacy replies.
- `model_select_result_lands_on_the_initiating_session` and
  `out_of_order_select_replies_land_on_their_initiating_sessions`: preserve routing.
