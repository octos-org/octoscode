# Model review feedback implementation plan

Approved scope: retain strict report-turn identity while making incomplete
ledger recovery and legacy cross collection actionable to the operator.

1. Extend `tests/olp_review_models.py` with two retained log segments and a
   deleted-prefix case. Full segments must verify; a continuous tail must not
   become turn 1. The error must explain starting a new short reviewer peer.
2. Extend `olp_review_model_gate_composes_with_live_cargo` to collect legacy
   cross through the actual CLI, assert a structured JSON warning plus stderr
   guidance, and verify final acceptance remains false. Verified resubmission
   must clear the warning and restore acceptance with the real Cargo evidence.
3. In `scripts/olp-review-evidence.py`, keep sequence/ordinal validation intact,
   explain recovery on missing evidence, and emit collection warnings after
   successful legacy persistence. Do not change the final acceptance gate.
4. Update `docs/OLP_REVIEW_EVIDENCE.md` and the corresponding behavior contract:
   reviewer peers are short dedicated sessions; retain all ledger segments
   through acceptance, or restart initial reviews and cross in a new context.
5. Run Python fixtures, the live-Cargo integration, fmt, clippy and all-targets;
   review the final diff, commit owned files, and update PR #637 and its CI.
