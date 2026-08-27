//! Contract tests for pager visual continuity
//! (`specs/task-pager-visual-continuity.spec`).
//!
//! Pinned-mode wheel scrolling enters the pager seamlessly, so the pager must
//! not flip the screen to the theme surface color ("screen went black") and
//! must signal the read position in the status row and right-side scrollbar
//! lane, since the alt-screen has no native scrollbar.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use octos_core::{Message, SessionKey};
use octoscode::app;
use octoscode::cli::ThemeName;
use octoscode::event_loop::handle_terminal_event;
use octoscode::model::{AppState, SessionView};
use octoscode::store::Store;
use octoscode::theme::Palette;
use octoscode::tui_terminal::FrameLike;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Color;
use ratatui::widgets::Widget;

fn chat_store(message_count: usize) -> Store {
    let messages = (1..=message_count)
        .flat_map(|idx| {
            [
                Message::user(format!("ask number {idx:02}")),
                Message::assistant(format!("history message {idx:02}")),
            ]
        })
        .collect();
    Store {
        state: AppState::new(
            vec![SessionView {
                id: SessionKey("local:pager-visual-test".into()),
                title: "pager-visual-test".into(),
                profile_id: Some("coding".into()),
                messages,
                tasks: vec![],
                live_reply: None,
            }],
            0,
            "ready".into(),
            None,
            false,
        ),
    }
}

struct BufferFrame {
    area: Rect,
    buffer: Buffer,
}

impl BufferFrame {
    fn new(width: u16, height: u16) -> Self {
        let area = Rect::new(0, 0, width, height);
        Self {
            area,
            buffer: Buffer::empty(area),
        }
    }
}

impl FrameLike for BufferFrame {
    fn area(&self) -> Rect {
        self.area
    }

    fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, &mut self.buffer);
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, _position: P) {}

    fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }
}

fn rendered_frame(state: &AppState, width: u16, height: u16) -> BufferFrame {
    let mut frame = BufferFrame::new(width, height);
    app::render(&mut frame, state, Palette::for_theme(ThemeName::default()));
    frame
}

fn status_row(state: &AppState, width: u16, height: u16) -> String {
    let frame = rendered_frame(state, width, height);
    (0..width)
        .map(|x| frame.buffer[(x, height - 1)].symbol())
        .collect()
}

fn scrollbar_lane_symbols(state: &AppState, width: u16, height: u16) -> Vec<String> {
    let frame = rendered_frame(state, width, height);
    let layout = app::chat_layout_areas(state, Rect::new(0, 0, width, height));
    let x = layout.transcript.x + layout.transcript.width - 1;
    (layout.transcript.y..layout.transcript.y + layout.transcript.height)
        .map(|y| frame.buffer[(x, y)].symbol().to_string())
        .collect()
}

fn scrollbar_thumb_top(state: &AppState, width: u16, height: u16) -> Option<usize> {
    scrollbar_lane_symbols(state, width, height)
        .iter()
        .position(|symbol| symbol == "█")
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl_t() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
}

#[test]
fn pager_transcript_uses_default_background() {
    let mut store = chat_store(10);
    handle_terminal_event(&mut store, ctrl_t());
    assert!(store.state.transcript_pager_active);

    let frame = rendered_frame(&store.state, 60, 20);
    // A cell well inside the transcript pane (above the composer rows).
    let cell = &frame.buffer[(30, 2)];
    assert_eq!(
        cell.bg,
        Color::Reset,
        "the pager transcript must blend with the terminal default background"
    );
}

#[test]
fn non_pager_fullscreen_keeps_surface_background() {
    let store = chat_store(10);
    assert!(!store.state.transcript_pager_active);

    let frame = rendered_frame(&store.state, 60, 20);
    let cell = &frame.buffer[(30, 2)];
    assert_eq!(
        cell.bg,
        Palette::for_theme(ThemeName::default()).surface_alt,
        "non-pager full-screen surfaces keep the existing surface_alt background"
    );
}

#[test]
fn pager_status_shows_reviewing_indicator() {
    let mut store = chat_store(30);
    handle_terminal_event(&mut store, ctrl_t());
    for _ in 0..3 {
        handle_terminal_event(&mut store, key(KeyCode::PageUp));
    }
    assert!(store.state.transcript_scroll > 0);

    let row = status_row(&store.state, 220, 24);
    assert!(
        row.contains("Reviewing"),
        "scrolled pager must surface the reviewing indicator; status row: {row:?}"
    );
}

#[test]
fn pager_status_hides_indicator_at_bottom() {
    let mut store = chat_store(30);
    handle_terminal_event(&mut store, ctrl_t());
    assert_eq!(store.state.transcript_scroll, 0);

    let row = status_row(&store.state, 220, 24);
    assert!(
        !row.contains("Reviewing"),
        "at the bottom the indicator must disappear; status row: {row:?}"
    );
    assert!(
        row.contains("PgUp/PgDn"),
        "the plain pager key hint must remain; status row: {row:?}"
    );
}

#[test]
fn pager_message_blocks_have_no_span_background() {
    let mut store = chat_store(10);
    handle_terminal_event(&mut store, ctrl_t());

    // Every cell in the transcript pane must sit on the terminal default
    // background — message-block "bubble" colors would paint text-shaped
    // stripes over the terminal theme (the reported "black backgrounds").
    let frame = rendered_frame(&store.state, 60, 20);
    let transcript_rows = 20 - 8; // above composer block + status row
    for y in 0..transcript_rows {
        for x in 0..60 {
            let cell = &frame.buffer[(x, y)];
            assert_eq!(
                cell.bg,
                Color::Reset,
                "cell ({x},{y}) must keep the default background, found {:?}",
                cell.bg
            );
        }
    }
}

#[test]
fn pager_scrollbar_renders_when_transcript_overflows() {
    let mut store = chat_store(50);
    handle_terminal_event(&mut store, ctrl_t());
    assert!(store.state.transcript_pager_active);

    let lane = scrollbar_lane_symbols(&store.state, 80, 24);

    assert!(
        lane.iter().any(|symbol| symbol == "│"),
        "overflowing pager should render a scrollbar track; lane: {lane:?}"
    );
    assert!(
        lane.iter().any(|symbol| symbol == "█"),
        "overflowing pager should render a scrollbar thumb; lane: {lane:?}"
    );
}

#[test]
fn pager_scrollbar_thumb_moves_when_scrolled_up() {
    let mut store = chat_store(60);
    handle_terminal_event(&mut store, ctrl_t());
    let bottom_top = scrollbar_thumb_top(&store.state, 80, 24).expect("bottom state renders thumb");

    for _ in 0..2 {
        handle_terminal_event(&mut store, key(KeyCode::PageUp));
    }
    assert!(store.state.transcript_scroll > 0);
    let scrolled_top =
        scrollbar_thumb_top(&store.state, 80, 24).expect("scrolled state renders thumb");

    assert!(
        scrolled_top < bottom_top,
        "scrolling up should move thumb upward; bottom={bottom_top}, scrolled={scrolled_top}"
    );
}

fn left_click(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })
}

fn bottom_row_symbols(state: &AppState, width: u16, height: u16) -> String {
    let frame = rendered_frame(state, width, height);
    let layout = app::chat_layout_areas(state, Rect::new(0, 0, width, height));
    let y = layout.transcript.y + layout.transcript.height - 1;
    (layout.transcript.x..layout.transcript.x + layout.transcript.width)
        .map(|x| frame.buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn pager_scroll_to_bottom_button_appears_when_scrolled_up() {
    let mut store = chat_store(50);
    handle_terminal_event(&mut store, ctrl_t());
    for _ in 0..2 {
        handle_terminal_event(&mut store, key(KeyCode::PageUp));
    }
    assert!(store.state.transcript_scroll > 0);

    let row = bottom_row_symbols(&store.state, 80, 24);
    assert!(
        row.contains('▼'),
        "scrolled-up pager must show the jump-to-latest arrow; bottom row: {row:?}"
    );
    assert!(
        store.state.scroll_to_bottom_button.get().is_some(),
        "the renderer must record the button's hit rect"
    );
}

#[test]
fn pager_scroll_to_bottom_button_hidden_at_bottom() {
    let mut store = chat_store(50);
    handle_terminal_event(&mut store, ctrl_t());
    assert_eq!(store.state.transcript_scroll, 0);

    let row = bottom_row_symbols(&store.state, 80, 24);
    assert!(
        !row.contains('▼'),
        "at the bottom the arrow must disappear; bottom row: {row:?}"
    );
    assert!(
        store.state.scroll_to_bottom_button.get().is_none(),
        "no hit rect may be recorded while the button is hidden"
    );
}

#[test]
fn pager_scroll_to_bottom_button_click_jumps_to_latest() {
    let mut store = chat_store(50);
    handle_terminal_event(&mut store, ctrl_t());
    for _ in 0..3 {
        handle_terminal_event(&mut store, key(KeyCode::PageUp));
    }
    assert!(store.state.transcript_scroll > 0);

    // Render once so the button's hit rect is recorded, as the live loop does.
    rendered_frame(&store.state, 80, 24);
    let hit = store
        .state
        .scroll_to_bottom_button
        .get()
        .expect("scrolled pager records the button rect");

    // A click just outside the button must not move the view.
    handle_terminal_event(&mut store, left_click(hit.x.saturating_sub(1), hit.y));
    assert!(store.state.transcript_scroll > 0);

    handle_terminal_event(&mut store, left_click(hit.x, hit.y));
    assert_eq!(
        store.state.transcript_scroll, 0,
        "clicking the arrow must jump to the latest output"
    );
    assert!(
        store.state.transcript_pager_active,
        "the jump stays in the pager, matching the End binding"
    );

    // The next frame is at the bottom, so the button withdraws its hit rect.
    rendered_frame(&store.state, 80, 24);
    assert!(store.state.scroll_to_bottom_button.get().is_none());
}

#[test]
fn pager_page_up_without_overflow_never_enters_reviewing() {
    // Content fits one screen (max_scroll == 0): PageUp must be a no-op.
    // Without the render-fed clamp, the bare `saturating_add` kept growing
    // `transcript_scroll` — the status row claimed "Reviewing history" while
    // the view, scrollbar and ▼ button (all gated on the real offset) stayed
    // frozen at the bottom, and a later PageDown first had to unwind the
    // phantom offset (dead zone).
    let mut store = chat_store(1);
    handle_terminal_event(&mut store, ctrl_t());
    assert!(store.state.transcript_pager_active);

    // Render once so the pager records the real bound, as the live loop does.
    rendered_frame(&store.state, 80, 24);
    assert_eq!(
        store.state.transcript_scroll_max.get(),
        0,
        "one exchange must fit a 80x24 transcript pane"
    );

    for _ in 0..3 {
        handle_terminal_event(&mut store, key(KeyCode::PageUp));
    }
    assert_eq!(
        store.state.transcript_scroll, 0,
        "PageUp on a non-overflowing transcript must be clamped to the bottom"
    );

    let row = status_row(&store.state, 220, 24);
    assert!(
        !row.contains("Reviewing"),
        "no reviewing indicator when nothing overflows; status row: {row:?}"
    );

    // No dead zone: the clamped offset is still exactly at the bottom, so a
    // PageDown is absorbed immediately instead of unwinding phantom scroll.
    handle_terminal_event(&mut store, key(KeyCode::PageDown));
    assert_eq!(store.state.transcript_scroll, 0);
}

#[test]
fn pager_scrollbar_hidden_without_overflow() {
    let mut store = chat_store(1);
    handle_terminal_event(&mut store, ctrl_t());

    let lane = scrollbar_lane_symbols(&store.state, 80, 24);

    assert!(
        !lane.iter().any(|symbol| symbol == "│" || symbol == "█"),
        "non-overflowing pager must not draw a scrollbar; lane: {lane:?}"
    );
}
