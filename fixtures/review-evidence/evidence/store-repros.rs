fn outer_review_goal_get(store: &mut Store, id: &str, status: &str) -> Option<AppUiCommand> {
    let mut goal = goal_record(status);
    goal.goal_id = id.into();
    store.apply_client_event(ClientEvent::Autonomy(crate::client_event::AutonomyClientEvent {
        result: crate::client_event::AutonomyResult::GoalGet(serde_json::from_value(serde_json::json!({
            "session_id": "local:test", "profile_id": "coding", "goal": goal
        })).unwrap()),
    }))
}

#[test]
fn outer_review_627_failed_archive_must_not_replay_on_new_goal() {
    let mut store = protocol_store_with_autonomy();
    store.state.set_session_goal(&SessionKey("local:test".into()), Some(goal_record("complete")), None);
    store.state.composer = "/goal archive".into();
    assert!(matches!(store.compose_command(), Some(AppUiCommand::GetSessionGoal(_))));
    store.apply_event(AppUiEvent::Error(octos_core::app_ui::AppUiError {
        code: "goal_unavailable".into(), message: "session/goal/get request tui-1 failed: goal unavailable".into(),
    }));
    store.state.composer = "/goal new task".into();
    assert!(matches!(store.compose_command(), Some(AppUiCommand::SetSessionGoal(_))));
    let mut fresh = goal_record("active"); fresh.goal_id = "new-goal".into();
    store.apply_client_event(ClientEvent::Autonomy(crate::client_event::AutonomyClientEvent {
        result: crate::client_event::AutonomyResult::GoalSet(serde_json::from_value(serde_json::json!({
            "session_id":"local:test", "profile_id":"coding", "ok":true, "goal":fresh, "transition_actor":"user"
        })).unwrap()),
    }));
    store.state.composer = "/goal".into();
    assert!(matches!(store.compose_command(), Some(AppUiCommand::GetSessionGoal(_))));
    let follow_up = outer_review_goal_get(&mut store, "new-goal", "active");
    assert!(follow_up.is_none(), "Read-only goal query replayed cancelled operator intent: {follow_up:?}");
}

#[test]
fn outer_review_627_cancelled_archive_must_not_replay_after_reconnect() {
    let mut store = protocol_store_with_autonomy();
    store.state.set_session_goal(&SessionKey("local:test".into()), Some(goal_record("active")), None);
    store.state.composer = "/goal archive".into();
    assert!(matches!(store.compose_command(), Some(AppUiCommand::GetSessionGoal(_))));
    store.apply_event(AppUiEvent::Error(octos_core::app_ui::AppUiError {
        code:"request_cancelled".into(), message:"session/goal/get request tui-1 cancelled because connection was lost".into(),
    }));
    store.apply_client_event(ClientEvent::BackendConnectionEpoch);
    store.apply_client_event(ClientEvent::BackendRelaunched);
    let follow_up = outer_review_goal_get(&mut store, "goal_01", "active");
    assert!(follow_up.is_none(), "Reconnect replayed cancelled archive: {follow_up:?}");
}

#[test]
fn outer_review_628_idle_rejection_must_render_beyond_status_bar() {
    let mut store = protocol_store_with_autonomy();
    store.state.sessions[0].messages = vec![Message::user("old request"), Message::assistant("old answer")];
    store.state.composer = "/goal archive --reason".into();
    assert!(store.compose_command().is_none());
    let reason = store.state.activity.last().unwrap().status.clone();
    assert!(reason.contains("reason"));
    store.state.status = "idle".into(); // Exclude status-bar text from the transcript assertion.
    let rendered = crate::app::debug_render_text(&store.state);
    assert!(rendered.contains(&reason), "Warning absent from actual rendered transcript. reason={reason:?}\n{rendered}");
}

#[test]
fn outer_review_628_active_turn_rejection_must_render() {
    let config = protocol_store_with_autonomy();
    let mut store = store_with_live_reply(TurnId::new(), "active request");
    store.state.target = config.state.target;
    store.state.capabilities = config.state.capabilities;
    store.state.composer = "/goal archive --reason".into();
    assert!(store.compose_command().is_none());
    let reason = store.state.activity.last().unwrap().status.clone();
    store.state.status = "working".into();
    let rendered = crate::app::debug_render_text(&store.state);
    assert!(rendered.contains(&reason), "Active-turn filter hides rejection: {reason:?}\n{rendered}");
}

#[test]
fn outer_review_628_repeat_rejection_must_keep_specific_reason() {
    let mut store = protocol_store_with_autonomy();
    store.state.composer = "/goal archive --reason".into();
    assert!(store.compose_command().is_none());
    let first = store.state.activity.last().unwrap().status.clone();
    store.state.composer = "/goal archive --reason".into();
    assert!(store.compose_command().is_none());
    assert_eq!(store.state.activity.last().unwrap().status, first, "Repeated parse error lost its explicit reason");
}

#[test]
fn outer_review_629_runtime_main_list_must_not_enable_invalid_save() {
    let mut store = protocol_store_with_methods(&[crate::model::APPUI_METHOD_PROFILE_LLM_UPSERT]);
    store.state.sessions[0].profile_id = None;
    store.apply_client_event(ClientEvent::ProfileLlmList(crate::client_event::ProfileLlmListClientEvent {
        result: serde_json::from_value(serde_json::json!({"profile_id":"_main", "primary":null, "fallbacks":[]})).unwrap(),
        message:"Configured providers refreshed".into(),
    }));
    store.state.composer = "/onboard select moonshot kimi-k2.5 autodl https://example.test/v1 AUTODL_API_KEY".into();
    assert!(store.compose_command().is_none());
    store.state.composer = "/onboard key test-placeholder".into();
    assert!(store.compose_command().is_none());
    store.state.composer = "/onboard save".into();
    let command = store.compose_command();
    assert!(command.is_none(), "Runtime-only _main profile passed save gate: {command:?}");
}
