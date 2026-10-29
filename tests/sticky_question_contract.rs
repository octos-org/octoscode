//! Observable rendering/input contract for the default fixed-question layout.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use octos_core::app_ui::{AppUiEvent, AppUiSnapshot};
use octos_core::{Message, SessionKey, ui_protocol::TurnId};
use octoscode::{
    app,
    cli::{ScrollMode, load_config_file},
    event_loop::{KeyAction, handle_terminal_event},
    model::{AppState, LiveReply, PendingTurnSteer, SessionView},
    store::Store,
    theme::Palette,
    tui_terminal::FrameLike,
};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    widgets::Widget,
};

struct Frame {
    area: Rect,
    buffer: Buffer,
}
impl FrameLike for Frame {
    fn area(&self) -> Rect {
        self.area
    }
    fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, &mut self.buffer);
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, _: P) {}
    fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }
}

fn session(name: &str) -> SessionView {
    SessionView {
        id: SessionKey(name.into()),
        title: name.into(),
        profile_id: Some("coding".into()),
        messages: vec![
            Message::user(format!("QUESTION_{name}")),
            Message::assistant(
                (0..80)
                    .map(|n| format!("Answer line {n}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
        ],
        tasks: vec![],
        live_reply: None,
    }
}
fn store() -> Store {
    let mut state = AppState::new(
        vec![session("A"), session("B")],
        0,
        "ready".into(),
        None,
        false,
    );
    state.scroll_mode = ScrollMode::Sticky;
    Store { state }
}
fn render(store: &mut Store, width: u16, height: u16) -> Vec<String> {
    store.state.last_terminal_width = width;
    store.state.last_terminal_height = height;
    let area = Rect::new(0, 0, width, height);
    let mut frame = Frame {
        area,
        buffer: Buffer::empty(area),
    };
    app::render(
        &mut frame,
        &store.state,
        Palette::for_theme(store.state.theme),
    );
    (0..height)
        .map(|y| (0..width).map(|x| frame.buffer[(x, y)].symbol()).collect())
        .collect()
}
fn key(store: &mut Store, code: KeyCode) -> KeyAction {
    handle_terminal_event(store, Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}
fn wheel(store: &mut Store, kind: MouseEventKind) {
    handle_terminal_event(
        store,
        Event::Mouse(MouseEvent {
            kind,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }),
    );
}

#[test]
fn question_remains_at_top_through_scroll_stream_completion_and_resize() {
    let mut s = store();
    s.state.set_composer_text("UNFINISHED_DRAFT");
    s.state.sessions[0]
        .messages
        .push(Message::user("QUESTION_A"));
    let turn = TurnId::new();
    s.state.sessions[0].live_reply = Some(LiveReply {
        turn_id: turn,
        text: "STREAMING_ANSWER".into(),
    });
    let first = render(&mut s, 80, 24);
    assert!(first[0].contains("QUESTION_A"));
    assert!(first.join("\n").contains("STREAMING_ANSWER"));
    for _ in 0..12 {
        wheel(&mut s, MouseEventKind::ScrollUp);
    }
    assert!(s.state.transcript_scroll > 0);
    assert!(
        !s.state.transcript_pager_active,
        "wheel scrolling must not disable composer shortcuts"
    );
    let scrolled = render(&mut s, 80, 24);
    assert_eq!(first[0], scrolled[0]);
    assert_ne!(first[8], scrolled[8], "the answer pane actually scrolls");
    s.state.sessions[0].live_reply = None;
    s.state.sessions[0]
        .messages
        .push(Message::assistant("FINAL_ANSWER"));
    s.state.set_run_state_success();
    for (width, height) in [(40, 12), (80, 24), (100, 32)] {
        let rows = render(&mut s, width, height);
        assert!(rows[0].contains("QUESTION_A"));
        assert!(rows.join("\n").contains("UNFINISHED_DRAFT"));
    }
    s.state.record_submitted_user_prompt(
        SessionKey("A".into()),
        TurnId::new(),
        "NEW_QUESTION".into(),
    );
    let rows = render(&mut s, 80, 24);
    assert!(rows[0].contains("NEW_QUESTION"));
    assert!(!rows[0].contains("QUESTION_A"));
}

#[test]
fn question_tracks_selected_session_queue_and_steering_without_using_the_draft() {
    let mut s = store();
    let turn = TurnId::new();
    s.state.sessions[0].live_reply = Some(LiveReply {
        turn_id: turn.clone(),
        text: "working".into(),
    });
    s.state.set_composer_text("QUEUED_QUESTION");
    key(&mut s, KeyCode::Tab);
    assert_eq!(s.state.pending_messages, ["QUEUED_QUESTION"]);
    let rows = render(&mut s, 80, 24);
    assert!(rows[0].contains("Queued: QUEUED_QUESTION"));
    s.state.set_composer_text("DRAFT_ONLY");
    assert_eq!(rows[0], render(&mut s, 80, 24)[0]);
    s.state.switch_selected_session(1);
    assert!(render(&mut s, 80, 24)[0].contains("QUESTION_B"));
    s.state.switch_selected_session(0);
    assert!(render(&mut s, 80, 24)[0].contains("QUEUED_QUESTION"));
    s.state.pending_messages.clear();
    s.state.pending_turn_steers.push_back(PendingTurnSteer {
        session_id: SessionKey("A".into()),
        turn_id: turn.clone(),
        prompt: "STEERING_QUESTION".into(),
    });
    assert!(render(&mut s, 80, 24)[0].contains("Sending: STEERING_QUESTION"));
    s.state.pending_turn_steers.clear();
    s.state
        .record_submitted_user_prompt(SessionKey("A".into()), turn, "STEERING_QUESTION".into());
    assert!(render(&mut s, 80, 24)[0].contains("You: STEERING_QUESTION"));
}

#[test]
fn full_question_reader_reaches_the_end_without_editing_draft_or_scrolling_answer() {
    let mut s = store();
    let question = format!(
        "长问题 中文 👩‍💻 {}\nQUESTION_END",
        "正文 mixed content\n".repeat(100)
    );
    s.state.sessions[0]
        .messages
        .push(Message::user(question.clone()));
    s.state.set_composer_text("KEEP_DRAFT");
    render(&mut s, 40, 16);
    s.state.scroll_transcript_up(7);
    let answer_scroll = s.state.transcript_scroll;
    key(&mut s, KeyCode::F(2));
    assert!(s.state.question_detail.active);
    render(&mut s, 40, 16);
    key(&mut s, KeyCode::End);
    assert!(render(&mut s, 40, 16).join("\n").contains("QUESTION_END"));
    key(&mut s, KeyCode::Char('x'));
    handle_terminal_event(&mut s, Event::Paste("DO_NOT_INSERT".into()));
    assert_eq!(s.state.composer, "KEEP_DRAFT");
    assert_eq!(s.state.transcript_scroll, answer_scroll);
    assert!(matches!(key(&mut s, KeyCode::Esc), KeyAction::Continue));
    assert!(!s.state.question_detail.active);
    assert_eq!(
        s.state.sessions[0].messages.last().unwrap().content,
        question
    );
}

#[test]
fn question_header_click_opens_reader_and_narrow_screens_keep_composer_reachable() {
    let mut s = store();
    s.state.set_composer_text("DRAFT");
    for (width, height) in [(20, 8), (40, 12), (80, 24)] {
        let rows = render(&mut s, width, height);
        assert!(rows[0].contains("QUESTION_A"), "{width}x{height}: {rows:?}");
        assert!(
            rows.join("\n").contains("DRAFT"),
            "{width}x{height}: {rows:?}"
        );
    }
    handle_terminal_event(
        &mut s,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 4,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert!(s.state.question_detail.active);
    s.state.switch_selected_session(1);
    assert!(
        !s.state.question_detail.active,
        "a viewer from another session must close"
    );
}

#[test]
fn incoming_approval_keeps_priority_over_the_question_reader() {
    let mut s = store();
    s.state.set_composer_text("KEEP_DRAFT");
    render(&mut s, 80, 24);
    key(&mut s, KeyCode::F(2));
    s.state.scroll_transcript_up(50);
    assert!(s.state.transcript_scroll > 0);
    s.apply_event(AppUiEvent::Protocol(
        octos_core::ui_protocol::UiNotification::ApprovalRequested(
            octos_core::ui_protocol::ApprovalRequestedEvent::generic(
                SessionKey("A".into()),
                octos_core::ui_protocol::ApprovalId::new(),
                TurnId::new(),
                "shell",
                "APPROVAL_PRIORITY",
                "May I run it?",
            ),
        ),
    ));
    assert_eq!(s.state.transcript_scroll, 0, "arrival reveals the decision");
    assert!(
        render(&mut s, 80, 24)
            .join("\n")
            .contains("APPROVAL_PRIORITY")
    );
    key(&mut s, KeyCode::Char('n'));
    assert!(
        s.state
            .approval
            .as_ref()
            .is_none_or(|approval| !approval.visible)
    );
    assert!(
        s.state.question_detail.active,
        "answering approval must not dismiss the question reader"
    );
    assert_eq!(s.state.composer, "KEEP_DRAFT");
}

#[test]
fn sticky_default_can_be_saved_restored_and_switched_back_to_native() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();
    let mut s = store();
    s.state.config_path = Some(config.clone());
    s.state.set_composer_text("/saveconfig");
    s.compose_command();
    assert_eq!(
        load_config_file(&config).unwrap().scroll_mode,
        Some(ScrollMode::Sticky)
    );
    let sessions = s.state.sessions.clone();
    s.apply_event(AppUiEvent::Snapshot(AppUiSnapshot {
        sessions,
        selected_session: 0,
        status: "restored".into(),
        target: None,
        readonly: false,
    }));
    assert_eq!(s.state.scroll_mode, ScrollMode::Sticky);
    s.state.set_composer_text("/scrollmode native");
    s.compose_command();
    assert!(!app::wants_mouse_capture(&s.state));
    assert!(!app::wants_fullscreen_overlay(&s.state));
    assert_eq!(
        app::chat_layout_areas(&s.state, Rect::new(0, 0, 80, 24))
            .question
            .height,
        0
    );
    s.state.set_composer_text("/saveconfig");
    s.compose_command();
    assert_eq!(
        load_config_file(&config).unwrap().scroll_mode,
        Some(ScrollMode::Native)
    );
}
