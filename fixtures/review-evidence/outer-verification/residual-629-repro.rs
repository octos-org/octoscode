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
