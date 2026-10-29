//! The latest submitted question is chrome, never part of the scrolling pane.
use super::*;

pub(crate) struct Question<'a> {
    pub content: &'a str,
    pub label: &'static str,
}

pub(crate) fn detail_visible(app: &AppState) -> bool {
    app.question_detail.active
        && !app.activity_navigator.active
        && !app.approval.as_ref().is_some_and(|value| value.visible)
        && !app
            .user_question
            .as_ref()
            .is_some_and(|value| value.visible)
        && !app.task_output.active
        && !app.artifact_detail.active
        && !app.thread_graph_detail.active
        && !app.turn_state_detail.active
        && !app.diff_preview.overlay_active()
        && !app.menu_stack.is_active()
}

pub(crate) fn latest(app: &AppState) -> Option<Question<'_>> {
    let session = app.active_session()?;
    // A nonempty FIFO prevents new steering, so its tail is the newest ask.
    // This is the selected session's queue; background queues must not leak in.
    if let Some(content) = app.pending_messages.last() {
        return Some(Question {
            content,
            label: "question_header.queued",
        });
    }
    if let Some(steer) = app
        .pending_turn_steers
        .iter()
        .rev()
        .find(|steer| steer.session_id == session.id)
    {
        return Some(Question {
            content: &steer.prompt,
            label: "question_header.sending",
        });
    }
    session
        .messages
        .iter()
        .rev()
        .find(|message| message.role.as_str() == "user" && !message.content.trim().is_empty())
        .map(|message| Question {
            content: &message.content,
            label: "question_header.latest",
        })
}

fn preview(question: &Question<'_>, width: u16) -> Vec<Line<'static>> {
    // Bound preview work even for a very large pasted question. The expanded
    // reader and canonical transcript retain the complete text.
    let prefix: String = question
        .content
        .chars()
        .take(usize::from(width).max(1) * 12)
        .collect();
    let text = format!("{}: {}", t!(question.label), prefix);
    text.lines()
        .flat_map(|line| {
            crate::insert_history::wrap_line(
                &Line::raw(crate::insert_history::sanitize_span_content(line)),
                usize::from(width).max(1),
            )
        })
        .take(3)
        .collect()
}

pub(super) fn header_height(app: &AppState, width: u16, height: u16) -> u16 {
    if app.scroll_mode != crate::cli::ScrollMode::Sticky || height < 5 {
        return 0;
    }
    let Some(question) = latest(app) else {
        return 0;
    };
    // Keep room for the composer and at least one answer row on small panes.
    (preview(&question, width).len() as u16 + 1).min(height.saturating_sub(6).max(1))
}

pub(super) fn render_header(
    frame: &mut impl FrameLike,
    app: &AppState,
    palette: Palette,
    area: Rect,
) {
    if area.is_empty() {
        return;
    }
    let Some(question) = latest(app) else {
        return;
    };
    let block = if area.height > 1 {
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(palette.border())
            .title_bottom(t!("question_header.expand").into_owned())
    } else {
        Block::default()
    };
    frame.render_widget(
        Paragraph::new(preview(&question, area.width))
            .style(
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .block(block),
        area,
    );
}

/// Capturing a question for reading does not change session history or drafts.
pub(crate) fn open(app: &mut AppState) {
    if let Some(question) = latest(app) {
        let title = t!(question.label).into_owned();
        let content = question
            .content
            .lines()
            .map(crate::insert_history::sanitize_span_content)
            .collect::<Vec<_>>()
            .join("\n");
        app.question_detail = crate::model::QuestionDetailState {
            active: true,
            title,
            content,
            ..Default::default()
        };
    }
}

pub(super) fn render_detail(frame: &mut impl FrameLike, app: &AppState, palette: Palette) {
    let area = frame.area();
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(palette.surface).fg(palette.text))
        .border_style(palette.border())
        .title(app.question_detail.title.clone())
        .title_bottom(t!("question_header.close").into_owned());
    let inner = block.inner(area);
    let paragraph = Paragraph::new(app.question_detail.content.clone()).wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(inner.width)
        .saturating_sub(usize::from(inner.height));
    app.question_detail.max_scroll.set(max_scroll);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(
        paragraph.scroll((
            app.question_detail
                .scroll
                .min(max_scroll)
                .min(u16::MAX as usize) as u16,
            0,
        )),
        inner,
    );
}
