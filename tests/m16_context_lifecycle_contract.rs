use octos_core::SessionKey;
use octoscode::model::{
    APPUI_FEATURE_CONTEXT_LIFECYCLE_V1, APPUI_FEATURE_CONTEXT_SEMANTIC_CACHE_V1,
    APPUI_METHOD_CONTEXT_COMPACTION_COMPLETED, APPUI_METHOD_CONTEXT_NORMALIZATION_REPORTED,
    ContextCacheDiagnostics, ContextCompactionSummary, ContextLifecycleState,
    ContextNormalizationSummary, SessionContextLifecycle,
};
use serde_json::Value;

/// Guard test: the M16-G2 capability and notification method constants
/// the TUI uses to gate compact-context UX must stay on-spec
/// (`context.lifecycle.v1`, `context/compaction_completed`,
/// `context/normalization_reported`).
#[test]
fn m16_capability_and_notification_constants_stay_on_spec() {
    assert_eq!(APPUI_FEATURE_CONTEXT_LIFECYCLE_V1, "context.lifecycle.v1");
    assert_eq!(
        APPUI_FEATURE_CONTEXT_SEMANTIC_CACHE_V1,
        "context.semantic_cache.v1"
    );
    assert_eq!(
        APPUI_METHOD_CONTEXT_COMPACTION_COMPLETED,
        "context/compaction_completed"
    );
    assert_eq!(
        APPUI_METHOD_CONTEXT_NORMALIZATION_REPORTED,
        "context/normalization_reported"
    );
}

/// Guard test: the compaction + normalization wire shapes round-trip
/// into the TUI's lifecycle ledger without losing the fields the M16-G2
/// status surface needs (generation, retained/dropped counts,
/// token estimates, normalization counts).
#[test]
fn compaction_and_normalization_events_round_trip_into_ledger() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/m16_context_lifecycle.json"))
            .expect("fixture parses");
    let events = fixture["events"].as_array().expect("events array");

    let mut ledger = SessionContextLifecycle::default();

    // Empty ledger renders no status (so the TUI hides the surface
    // until the server says something).
    assert!(ledger.summary_line().is_none());

    for event in events {
        match event["method"].as_str().expect("method") {
            "context/compaction_completed" => {
                let state: ContextLifecycleState =
                    serde_json::from_value(event["params"]["context_state"].clone())
                        .expect("context_state shape");
                let compaction: ContextCompactionSummary =
                    serde_json::from_value(event["params"]["compaction"].clone())
                        .expect("compaction shape");
                ledger.apply_compaction(state, compaction);
            }
            "context/normalization_reported" => {
                let state: ContextLifecycleState =
                    serde_json::from_value(event["params"]["context_state"].clone())
                        .expect("context_state shape");
                let normalization: ContextNormalizationSummary =
                    serde_json::from_value(event["params"]["normalization"].clone())
                        .expect("normalization shape");
                ledger.apply_normalization(state, normalization);
            }
            other => panic!("unexpected method in fixture: {other}"),
        }
    }

    let state = ledger.state.as_ref().expect("state populated");
    assert_eq!(state.generation, 4);
    assert_eq!(state.item_count, 42);
    assert_eq!(state.token_estimate, 9100);
    assert_eq!(state.last_compaction_id.as_deref(), Some("comp-001"));

    let compaction = ledger.last_compaction.as_ref().expect("compaction stored");
    assert_eq!(compaction.input_generation, 3);
    assert_eq!(compaction.output_generation, Some(4));
    assert_eq!(compaction.retained_count, 42);
    assert_eq!(compaction.dropped_count, 88);
    assert_eq!(compaction.token_estimate_before, 31200);
    assert_eq!(compaction.token_estimate_after, Some(9100));

    let normalization = ledger
        .last_normalization
        .as_ref()
        .expect("normalization stored");
    assert_eq!(normalization.generation, 4);
    assert_eq!(normalization.repaired_count, 2);
    assert_eq!(normalization.synthetic_count, 1);
    assert_eq!(normalization.dropped_count, 0);
    assert_eq!(normalization.truncated_count, 0);
}

/// Guard test: the bounded `summary_line` matches the documented
/// status surface (the issue's "render active context generation,
/// compacted/rebuilt status, and last compaction summary in a bounded
/// status surface" — without raw transcript hashes or per-item lists).
#[test]
fn summary_line_is_bounded_and_does_not_leak_raw_hashes_or_item_ids() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/m16_context_lifecycle.json"))
            .expect("fixture parses");
    let mut ledger = SessionContextLifecycle::default();
    let comp_event = &fixture["events"][0];
    let state: ContextLifecycleState =
        serde_json::from_value(comp_event["params"]["context_state"].clone())
            .expect("context_state shape");
    let compaction: ContextCompactionSummary =
        serde_json::from_value(comp_event["params"]["compaction"].clone())
            .expect("compaction shape");
    ledger.apply_compaction(state, compaction);

    let summary = ledger.summary_line().expect("summary present");
    assert_eq!(
        summary,
        fixture["expected_summary_after_compaction"]
            .as_str()
            .unwrap()
    );

    // The bounded summary must not leak raw transcript hashes,
    // checkpoint internals, summary item ids, or per-record field
    // names. Those stay inside the ledger struct for diagnostics, but
    // never reach the chat-adjacent status surface.
    for forbidden in fixture["expected_no_raw_record_text"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
    {
        assert!(
            !summary.contains(forbidden),
            "summary leaked raw lifecycle field {forbidden}: {summary}"
        );
    }
}

/// Guard test: applying a normalization event when the state recovery
/// label is not `"healthy"` surfaces that label in the summary, so the
/// status surface can show "(recovering)" / "(degraded)" without the
/// TUI inventing a label of its own.
#[test]
fn recovery_state_label_appears_in_summary_when_not_healthy() {
    let mut ledger = SessionContextLifecycle::default();
    ledger.apply_normalization(
        ContextLifecycleState {
            session_id: SessionKey("local:test".into()),
            thread_id: None,
            generation: 12,
            transcript_hash: "h".into(),
            item_count: 30,
            token_estimate: 4000,
            recovery_state: "recovering".into(),
            last_checkpoint_id: None,
            last_compaction_id: None,
        },
        ContextNormalizationSummary {
            generation: 12,
            model_capability_id: "anthropic/sonnet-4.7".into(),
            prompt_message_count: 30,
            token_estimate: 4000,
            repaired_count: 0,
            dropped_count: 0,
            synthetic_count: 0,
            truncated_count: 0,
        },
    );
    let summary = ledger.summary_line().expect("summary");
    assert!(summary.contains("(recovering)"), "summary={}", summary);
    // Compaction segment must NOT appear when no compaction has been
    // observed yet.
    assert!(!summary.contains("compacted"), "summary={}", summary);
}

/// The semantic-cache extension is additive: an old context_state without the
/// four fields decodes to an empty diagnostic mirror, while a new one retains
/// every value. Neither case changes the existing lifecycle summary surface —
/// and, driven through the real store + transcript render path, the four
/// values land ONLY in the `/context` ledger, never in a transcript line.
#[test]
fn cache_diagnostics_are_optional_and_do_not_leak_into_lifecycle_summary() {
    let legacy: ContextCacheDiagnostics =
        serde_json::from_value(serde_json::json!({})).expect("legacy shape decodes");
    assert!(legacy.is_empty());

    let diagnostics: ContextCacheDiagnostics = serde_json::from_value(serde_json::json!({
        "cache_epoch_id": "sha256:epoch",
        "last_cache_invalidation_reason": "tool_schema_changed",
        "semantic_head_id": "semblk_000004",
        "semantic_head_kind": "tool_interaction"
    }))
    .expect("new additive shape decodes");
    assert_eq!(diagnostics.cache_epoch_id.as_deref(), Some("sha256:epoch"));
    assert_eq!(
        diagnostics.last_cache_invalidation_reason.as_deref(),
        Some("tool_schema_changed")
    );

    let mut ledger = SessionContextLifecycle {
        cache_diagnostics: Some(diagnostics),
        ..SessionContextLifecycle::default()
    };
    ledger.state = Some(ContextLifecycleState {
        session_id: SessionKey("local:test".into()),
        thread_id: None,
        generation: 1,
        transcript_hash: "sha256:transcript".into(),
        item_count: 2,
        token_estimate: 100,
        recovery_state: "healthy".into(),
        last_checkpoint_id: None,
        last_compaction_id: None,
    });
    let summary = ledger.summary_line().expect("summary");
    assert!(!summary.contains("sha256:epoch"));
    assert!(!summary.contains("semblk_000004"));

    // `summary_line()` never reads `cache_diagnostics`, so the two asserts
    // above cannot fail on their own. Lock the contract against the REAL
    // render path: apply a diagnostics-bearing lifecycle event through the
    // store and check the rendered transcript against the `/context` ledger.
    use octos_core::Message;
    use octos_core::app_ui::AppUiEvent;
    use octos_core::ui_protocol::{
        ContextNormalizationReportedEvent, UiContextNormalizationReport, UiContextState,
        UiNotification,
    };
    use octoscode::app::finalized_history_lines;
    use octoscode::cli::ThemeName;
    use octoscode::client_event::{ClientEvent, ContextLifecycleClientEvent};
    use octoscode::model::{AppState, SessionView};
    use octoscode::store::Store;
    use octoscode::theme::Palette;

    const CACHE_EPOCH: &str = "sha256:epoch-9";
    const INVALIDATION_REASON: &str = "compaction_installed";
    const SEMANTIC_HEAD_ID: &str = "semblk_000020";
    const SEMANTIC_HEAD_KIND: &str = "assistant_final";

    let session_id = SessionKey("local:test".into());
    let mut store = Store {
        state: AppState::new(
            vec![SessionView {
                id: session_id.clone(),
                title: "test".into(),
                profile_id: Some("coding".into()),
                messages: vec![
                    Message::user("compact the context"),
                    Message::assistant("done, context compacted"),
                ],
                tasks: vec![],
                live_reply: None,
            }],
            0,
            "ready".into(),
            None,
            false,
        ),
    };
    let event = AppUiEvent::Protocol(UiNotification::ContextNormalizationReported(
        ContextNormalizationReportedEvent {
            session_id: session_id.clone(),
            context_state: UiContextState {
                cache_epoch_id: None,
                last_cache_invalidation_reason: None,
                semantic_head_id: None,
                semantic_head_kind: None,
                session_id: session_id.clone(),
                thread_id: None,
                generation: 9,
                transcript_hash: "sha256:transcript".into(),
                item_count: 20,
                token_estimate: 7200,
                recovery_state: "healthy".into(),
                last_checkpoint_id: None,
                last_compaction_id: Some("comp-9".into()),
            },
            normalization: UiContextNormalizationReport {
                generation: 9,
                input_transcript_hash: "sha256:transcript".into(),
                output_prompt_hash: "sha256:prompt".into(),
                model_capability_id: "openai/gpt-5".into(),
                prompt_message_count: 20,
                token_estimate: 7200,
                repaired_count: 0,
                dropped_count: 0,
                synthetic_count: 0,
                truncated_count: 0,
            },
        },
    ));
    store.apply_client_event(ClientEvent::ContextLifecycle(ContextLifecycleClientEvent {
        event: Box::new(event),
        session_id: session_id.clone(),
        diagnostics: Some(ContextCacheDiagnostics {
            cache_epoch_id: Some(CACHE_EPOCH.into()),
            last_cache_invalidation_reason: Some(INVALIDATION_REASON.into()),
            semantic_head_id: Some(SEMANTIC_HEAD_ID.into()),
            semantic_head_kind: Some(SEMANTIC_HEAD_KIND.into()),
        }),
        // The same response advertised the feature, so no separate
        // capabilities negotiation is needed for the store to keep them.
        semantic_cache_advertised: Some(true),
    }));

    let stored = store
        .state
        .context_lifecycle_for(&session_id)
        .and_then(|ledger| ledger.cache_diagnostics.clone())
        .expect("the /context ledger keeps the diagnostics");
    assert_eq!(stored.cache_epoch_id.as_deref(), Some(CACHE_EPOCH));
    assert_eq!(
        stored.last_cache_invalidation_reason.as_deref(),
        Some(INVALIDATION_REASON)
    );
    assert_eq!(stored.semantic_head_id.as_deref(), Some(SEMANTIC_HEAD_ID));
    assert_eq!(
        stored.semantic_head_kind.as_deref(),
        Some(SEMANTIC_HEAD_KIND)
    );

    let transcript =
        finalized_history_lines(&store.state, Palette::for_theme(ThemeName::Codex), 100)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
    assert!(
        transcript.contains("done, context compacted"),
        "sanity: the real transcript rendered, got {transcript:?}"
    );
    for value in [
        CACHE_EPOCH,
        INVALIDATION_REASON,
        SEMANTIC_HEAD_ID,
        SEMANTIC_HEAD_KIND,
    ] {
        assert!(
            !transcript.contains(value),
            "diagnostic {value:?} must never enter a transcript line, got {transcript:?}"
        );
    }
    let ledger_summary = store
        .state
        .context_lifecycle_for(&session_id)
        .and_then(|ledger| ledger.summary_line())
        .expect("lifecycle summary after the event");
    for value in [
        CACHE_EPOCH,
        INVALIDATION_REASON,
        SEMANTIC_HEAD_ID,
        SEMANTIC_HEAD_KIND,
    ] {
        assert!(
            !ledger_summary.contains(value),
            "diagnostic {value:?} must stay out of the lifecycle summary line"
        );
    }
}
