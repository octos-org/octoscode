//! Shared layout for the menu preview pane (the Snapshot pane on the right).
//!
//! `selection_view` and `multi_select_view` render the same pane, so the
//! scroll window, the row truncation and the position indicator live here
//! rather than being duplicated — and stay in step by construction.

use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};

use crate::theme::Palette;

/// How far one PgUp/PgDn press moves the preview.
pub(crate) const PREVIEW_SCROLL_STEP: usize = 4;

/// The furthest the preview may scroll: the last row reaches the TOP of the
/// pane, `less`-style. Deliberately independent of the pane height, which only
/// the renderer knows — so the key handler and the renderer clamp to the same
/// bound and can never disagree about whether a press does anything.
pub(crate) fn max_preview_scroll(row_count: usize) -> usize {
    row_count.saturating_sub(1)
}

/// The pane's rows: a title (with a `3-10/14` position indicator whenever
/// content is out of view) followed by the visible slice of `lines`.
pub(crate) fn preview_lines(
    title: &str,
    lines: &[String],
    single_line: bool,
    scroll: usize,
    area: Rect,
    palette: Palette,
) -> Vec<Line<'static>> {
    let capacity = usize::from(area.height.saturating_sub(1));
    let scroll = scroll.min(max_preview_scroll(lines.len()));
    let visible = &lines[scroll.min(lines.len())..];
    let shown = visible.len().min(capacity);

    let mut rows = vec![Line::from(Span::styled(
        preview_title(title, lines.len(), scroll, shown),
        palette.title().add_modifier(Modifier::BOLD),
    ))];
    rows.extend(visible.iter().take(capacity).map(|line| {
        let text = if single_line {
            crate::app::truncate_to_display_width(line, usize::from(area.width))
        } else {
            line.clone()
        };
        Line::from(Span::styled(text, palette.text()))
    }));
    rows
}

/// `Snapshot  3-10/14` while rows are out of view, plain `Snapshot` when the
/// whole preview fits — an indicator on a complete pane is just noise.
fn preview_title(title: &str, total: usize, scroll: usize, shown: usize) -> String {
    // `shown == 0` is a pane with no room for content at all; a range there
    // would read as nonsense ("3-2/3"), so it stays a plain title.
    if shown == 0 || shown >= total {
        return title.to_string();
    }
    format!("{title}  {}-{}/{total}", scroll + 1, scroll + shown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ThemeName;

    fn rows(count: usize) -> Vec<String> {
        (1..=count).map(|i| format!("row {i}")).collect()
    }

    fn render(lines: &[String], scroll: usize, height: u16) -> Vec<String> {
        preview_lines(
            "Snapshot",
            lines,
            true,
            scroll,
            Rect::new(0, 0, 40, height),
            Palette::for_theme(ThemeName::Slate),
        )
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
    }

    #[test]
    fn scroll_offsets_the_visible_window() {
        // height 5 => title + 4 rows.
        let lines = rows(14);
        assert_eq!(render(&lines, 0, 5)[1], "row 1");
        assert_eq!(render(&lines, 4, 5)[1], "row 5");
        assert_eq!(render(&lines, 8, 5)[1], "row 9");
    }

    #[test]
    fn scroll_clamps_so_the_last_row_reaches_the_top() {
        let lines = rows(14);
        // Anything past the bound lands on the same final window: row 14 alone.
        let at_end = render(&lines, 13, 5);
        assert_eq!(at_end[1], "row 14");
        assert_eq!(at_end.len(), 2, "only the last row remains below the title");
        assert_eq!(
            render(&lines, 99, 5),
            at_end,
            "overshoot clamps, never panics"
        );
    }

    #[test]
    fn indicator_reports_the_window_only_while_content_is_hidden() {
        let lines = rows(14);
        assert_eq!(render(&lines, 0, 5)[0], "Snapshot  1-4/14");
        assert_eq!(render(&lines, 4, 5)[0], "Snapshot  5-8/14");
        // A preview that fits entirely gets no indicator.
        assert_eq!(render(&rows(3), 0, 5)[0], "Snapshot");
    }

    #[test]
    fn empty_and_degenerate_panes_do_not_panic() {
        assert_eq!(render(&[], 0, 5), vec!["Snapshot".to_string()]);
        // Height 1 leaves no room for content: title only, no bogus range.
        assert_eq!(render(&rows(3), 9, 1), vec!["Snapshot".to_string()]);
    }
}
