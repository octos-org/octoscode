//! Contract tests for the `/rewind` pre-token-submit guard (#246).
//!
//! Between prompt submit and the server's `turn/started` there is no
//! `live_reply` yet, but the optimistic prompt row already counts as rewind
//! checkpoint 1. A rollback dispatched in that window targets a turn the
//! server has not committed and drops the wrong turns. The fresh
//! `pre_token_turns` marker must refuse `/rewind` exactly like a streaming
//! turn does; once the marker ages past `PRE_TOKEN_TURN_TTL` (dead submit),
//! rewind works again.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use octos_core::ui_protocol::{UiProtocolCapabilities, methods};
use octos_core::{Message, SessionKey};
use octoscode::event_loop::{KeyAction, handle_terminal_event};
use octoscode::model::{AppState, AppUiCommand, SessionView};
use octoscode::store::Store;

fn chat_store() -> Store {
    let mut store = Store {
        state: AppState::new(
            vec![SessionView {
                id: SessionKey("local:rewind-test".into()),
                title: "rewind-test".into(),
                profile_id: Some("coding".into()),
                messages: vec![
                    Message::user("first question"),
                    Message::assistant("ok"),
                    Message::user("second question"),
                    Message::assistant("ok"),
                ],
                tasks: vec![],
                live_reply: None,
            }],
            0,
            "ready".into(),
            Some("ws://example.test/ui-protocol".into()),
            false,
        ),
    };
    // `/rewind` is gated on the server advertising `session/rollback`.
    store.state.set_capabilities(UiProtocolCapabilities::new(
        &[methods::SESSION_ROLLBACK],
        &[],
    ));
    store
}

/// Type a composer command and press Enter through the real key-event path.
fn run_command(store: &mut Store, command: &str) -> KeyAction {
    store.state.set_composer_text(command);
    handle_terminal_event(
        store,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    )
}

fn arm_pre_token_submit(store: &mut Store) {
    store.state.pre_token_turns.insert(
        SessionKey("local:rewind-test".into()),
        std::time::Instant::now(),
    );
}

#[test]
fn rewind_inline_dispatches_rollback_when_idle() {
    let mut store = chat_store();

    match run_command(&mut store, "/rewind 1") {
        KeyAction::Send(command) => assert!(
            matches!(*command, AppUiCommand::SessionRollback(_)),
            "an idle session rewinds normally"
        ),
        _ => panic!("expected Send(SessionRollback)"),
    }
}

#[test]
fn rewind_inline_is_refused_during_pre_token_submit_window() {
    let mut store = chat_store();
    arm_pre_token_submit(&mut store);

    let action = run_command(&mut store, "/rewind 1");

    assert!(
        !matches!(
            action,
            KeyAction::Send(ref command)
                if matches!(**command, AppUiCommand::SessionRollback(_))
        ),
        "no rollback may leave for the wire while a submit is pre-first-token"
    );
    assert!(
        store.state.status.contains("Finish or stop"),
        "the refusal is surfaced on the status line: {}",
        store.state.status
    );
    assert!(
        store.state.pending_rewind_prefill.is_none(),
        "no prefill is stashed for a refused rewind"
    );
}

#[test]
fn rewind_picker_does_not_open_during_pre_token_submit_window() {
    let mut store = chat_store();
    arm_pre_token_submit(&mut store);

    run_command(&mut store, "/rewind");

    assert!(
        store.state.menu_stack.active().is_none(),
        "no picker opens while a submit is pre-first-token"
    );
    assert!(
        store.state.rewind_turns.is_empty(),
        "no rewind rows are snapshotted mid-submit"
    );
}

#[test]
fn rewind_dispatches_once_the_marker_ages_out() {
    let mut store = chat_store();
    // 11s > PRE_TOKEN_TURN_TTL (10s, `pub(crate)` — not visible to integration
    // tests); if the TTL is ever raised past 11s this test must move with it.
    store.state.pre_token_turns.insert(
        SessionKey("local:rewind-test".into()),
        std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(11))
            .expect("instant in the past"),
    );

    match run_command(&mut store, "/rewind 1") {
        KeyAction::Send(command) => assert!(
            matches!(*command, AppUiCommand::SessionRollback(_)),
            "a dead submit marker must not block rewinding"
        ),
        _ => panic!("expected Send(SessionRollback)"),
    }
}
