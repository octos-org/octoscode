use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use crate::theme::Palette;

const WIDE_PREVIEW_WIDTH: u16 = 100;
const MAX_ITEMS: u16 = 8;
const NARROW_MAX_ITEMS: u16 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectionItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub shortcut: Option<String>,
    pub disabled_reason: Option<String>,
    pub current: bool,
    pub default: bool,
    pub toggle: Option<bool>,
    pub loading: bool,
    pub required_valid: Option<bool>,
}

impl SelectionItem {
    pub(crate) fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            shortcut: None,
            disabled_reason: None,
            current: false,
            default: false,
            toggle: None,
            loading: false,
            required_valid: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectionPreview {
    pub title: String,
    pub lines: Vec<String>,
    /// One line per row: overlong lines are ellipsized to the pane width
    /// instead of wrapping. Set for key/value previews (the Snapshot pane),
    /// where a wrapped row shunts every row below it down and off the bottom.
    /// Prose bodies leave it false and wrap as before.
    pub single_line: bool,
    /// Row offset of the preview's scroll window (PgUp/PgDn). See
    /// [`crate::menu::preview_layout`].
    pub scroll: usize,
}

impl SelectionPreview {
    pub(crate) fn new(title: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            title: title.into(),
            lines,
            single_line: false,
            scroll: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectionView {
    pub title: String,
    pub subtitle: Option<String>,
    pub search_query: Option<String>,
    pub search_placeholder: Option<String>,
    pub footer_hint: Option<String>,
    pub items: Vec<SelectionItem>,
    pub selected: usize,
    pub scroll: usize,
    pub preview: Option<SelectionPreview>,
}

impl SelectionView {
    pub(crate) fn new(title: impl Into<String>, items: Vec<SelectionItem>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            search_query: None,
            search_placeholder: None,
            footer_hint: None,
            items,
            selected: 0,
            scroll: 0,
            preview: None,
        }
    }

    pub(crate) fn height_hint(&self, terminal_width: u16) -> u16 {
        let header_rows = u16::from(self.subtitle.is_some()) + u16::from(self.has_search_row());
        let max_items = if terminal_width >= WIDE_PREVIEW_WIDTH {
            MAX_ITEMS
        } else {
            NARROW_MAX_ITEMS
        };
        let item_rows = self.items.len().max(1).min(usize::from(max_items)) as u16;
        let stacked_preview_rows =
            u16::from(self.preview.is_some() && terminal_width < WIDE_PREVIEW_WIDTH) * 3;
        2 + header_rows + item_rows + stacked_preview_rows + 1
    }

    pub(crate) fn widget(&self, palette: Palette) -> SelectionViewWidget<'_> {
        SelectionViewWidget {
            view: self,
            palette,
        }
    }

    fn has_search_row(&self) -> bool {
        self.search_query.is_some() || self.search_placeholder.is_some()
    }

    fn selected_index(&self) -> usize {
        self.selected.min(self.items.len().saturating_sub(1))
    }
}

pub(crate) struct SelectionViewWidget<'a> {
    view: &'a SelectionView,
    palette: Palette,
}

impl Widget for SelectionViewWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .style(
                Style::default()
                    .fg(self.palette.text)
                    .bg(self.palette.surface),
            )
            .border_style(self.palette.border())
            .title(Line::from(Span::styled(
                self.view.title.clone(),
                self.palette.title().add_modifier(Modifier::BOLD),
            )));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        render_footer(
            chunks[1],
            buf,
            self.palette,
            self.view.footer_hint.as_deref(),
            &t!("menu.footer.select"),
        );

        if self.view.preview.is_some() && inner.width >= WIDE_PREVIEW_WIDTH {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
                // 2-col gutter so a clipped list item can't butt straight into
                // the preview text (the "Aliasubmission" collision).
                .spacing(2)
                .split(chunks[0]);
            render_selection_list(self.view, body[0], buf, self.palette);
            render_preview(self.view.preview.as_ref(), body[1], buf, self.palette);
        } else if self.view.preview.is_some() && chunks[0].height >= 8 {
            let preview_height = chunks[0].height.min(4);
            let body = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(preview_height)])
                .split(chunks[0]);
            render_selection_list(self.view, body[0], buf, self.palette);
            render_preview(self.view.preview.as_ref(), body[1], buf, self.palette);
        } else {
            render_selection_list(self.view, chunks[0], buf, self.palette);
        }
    }
}

fn render_selection_list(view: &SelectionView, area: Rect, buf: &mut Buffer, palette: Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let header_height = u16::from(view.subtitle.is_some()) + u16::from(view.has_search_row());
    let chunks = if header_height == 0 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(0), Constraint::Min(1)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(1)])
            .split(area)
    };

    let mut header = Vec::new();
    if let Some(subtitle) = &view.subtitle {
        header.push(Line::from(Span::styled(subtitle.clone(), palette.muted())));
    }
    if view.has_search_row() {
        let query = view
            .search_query
            .as_deref()
            .filter(|query| !query.is_empty())
            .or(view.search_placeholder.as_deref())
            .unwrap_or_default();
        header.push(Line::from(vec![
            Span::styled(t!("menu.search.label").to_string(), palette.title()),
            Span::styled(query.to_string(), palette.text()),
        ]));
    }
    Paragraph::new(Text::from(header))
        .style(Style::default().bg(palette.surface))
        .render(chunks[0], buf);

    let lines = selection_rows(
        view,
        chunks[1].height,
        usize::from(chunks[1].width),
        palette,
    );
    // Rows arrive pre-wrapped to the pane width; letting the Paragraph wrap
    // again would let a stray long line shift every row below it and break the
    // visual-row budget `selection_rows` just computed.
    Paragraph::new(Text::from(lines))
        .style(Style::default().bg(palette.surface))
        .render(chunks[1], buf);
}

fn selection_rows(
    view: &SelectionView,
    height: u16,
    width: usize,
    palette: Palette,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    if view.items.is_empty() {
        return vec![Line::from(Span::styled(
            t!("menu.empty").to_string(),
            palette.muted(),
        ))];
    }

    let selected = view.selected_index();
    // Rows now WRAP, so an item is no longer one screen row. Everything below
    // budgets in visual rows: `heights` is what each item actually costs, and
    // the scroll math keeps the selected item on screen in those terms.
    let budget = usize::from(height);
    let rendered: Vec<Vec<Line<'static>>> = view
        .items
        .iter()
        .enumerate()
        .map(|(idx, item)| selection_row(item, idx == selected, width, palette))
        .collect();
    let heights: Vec<usize> = rendered.iter().map(Vec::len).collect();
    let start = visible_start(&heights, selected, view.scroll, budget);

    let mut rows = Vec::new();
    for item_rows in rendered.into_iter().skip(start) {
        if rows.len() >= budget {
            break;
        }
        // A single item taller than the whole pane is clipped rather than
        // dropped — better a partial row than a blank list.
        let room = budget - rows.len();
        rows.extend(item_rows.into_iter().take(room));
    }
    rows
}

/// First item to draw, in VISUAL rows: honours the stored scroll but always
/// pulls back far enough that the selected item's wrapped rows fit in the
/// pane, and never scrolls past the point where the remaining rows still fill
/// it. `heights[i]` is the number of rows item `i` occupies.
fn visible_start(heights: &[usize], selected: usize, scroll: usize, height: usize) -> usize {
    if heights.is_empty() || height == 0 {
        return 0;
    }
    // Furthest start that still fills the pane, so the list stays bottom-
    // anchored instead of scrolling into empty space.
    let mut max_start = 0usize;
    let mut tail = 0usize;
    for (idx, rows) in heights.iter().enumerate().rev() {
        tail += rows;
        if tail >= height {
            max_start = idx;
            break;
        }
    }

    let mut start = scroll.min(max_start).min(selected);
    // Walk forward until the selected item's rows fit in the remaining budget.
    while start < selected {
        let used: usize = heights[start..=selected].iter().sum();
        if used <= height {
            break;
        }
        start += 1;
    }
    start
}

/// One menu item as the rows it occupies: word-wrapped to the pane width, so
/// a long label (an MCP row listing every server) is readable in full instead
/// of being clipped at the pane edge.
fn selection_row(
    item: &SelectionItem,
    selected: bool,
    width: usize,
    palette: Palette,
) -> Vec<Line<'static>> {
    let disabled = item.disabled_reason.is_some();
    let semantic = item.required_valid.map(|valid| {
        if valid {
            palette.success
        } else {
            palette.danger
        }
    });
    let base = if let Some(color) = semantic {
        let style = Style::default().fg(color);
        if selected {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    } else if disabled {
        palette.muted()
    } else if selected {
        palette.selected().add_modifier(Modifier::BOLD)
    } else {
        palette.text()
    };
    let style = if selected {
        base.bg(palette.surface_alt)
    } else {
        base.bg(palette.surface)
    };
    let muted = if selected {
        palette.muted().bg(palette.surface_alt)
    } else {
        palette.muted().bg(palette.surface)
    };
    let reason_style = if selected {
        Style::default().fg(palette.danger).bg(palette.surface_alt)
    } else {
        Style::default().fg(palette.danger).bg(palette.surface)
    };

    let marker = if selected { ">" } else { " " };
    // A `*` column marks the active/current selection (e.g. the active model,
    // theme, or thinking level) — clearer than a trailing "current" word. It is
    // a separate column from the `>` navigation cursor so both can show at once.
    let current_marker = if item.current { "*" } else { " " };
    let mut text = format!("{marker}{current_marker} ");
    if let Some(checked) = item.toggle {
        text.push_str(if checked { "[x] " } else { "[ ] " });
    }
    if let Some(shortcut) = &item.shortcut {
        text.push_str(shortcut);
        text.push(' ');
    }
    if item.loading {
        text.push_str("[..] ");
    }
    text.push_str(&item.label);
    if let Some(description) = &item.description {
        text.push_str(" - ");
        text.push_str(description);
    }
    if item.default {
        text.push_str(&t!("menu.item.default_suffix"));
    }

    // The row's full text as STYLED SEGMENTS. It is wrapped as one string
    // below and the styles re-sliced across the wrapped rows, so a disabled
    // reason keeps its danger colour even when the wrap falls inside it.
    let mut spans = vec![Span::styled(text.clone(), style)];
    if let Some(reason) = &item.disabled_reason {
        let reason = format!(" ({reason})");
        text.push_str(&reason);
        spans.push(Span::styled(reason, reason_style));
    } else if item.id.is_empty() {
        spans.push(Span::styled("", muted));
    }

    let row_style = if selected {
        Style::default().bg(palette.surface_alt)
    } else {
        Style::default().bg(palette.surface)
    };
    let chunks = wrap_row(&text, width, CONTINUATION_INDENT);
    crate::app::markdown_highlight::split_highlighted_spans(&spans, &chunks, style)
        .into_iter()
        .enumerate()
        .map(|(index, mut row)| {
            if index > 0 {
                // Continuations sit past the `>` / `*` marker columns so a
                // wrapped tail never reads as a separate menu item.
                row.insert(0, Span::styled(" ".repeat(CONTINUATION_INDENT), style));
            }
            Line::from(row).style(row_style)
        })
        .collect()
}

/// Columns a wrapped continuation row is indented by.
const CONTINUATION_INDENT: usize = 5;

/// Word-wrap `text` to `width` display columns, with every row after the first
/// reserving `indent` columns for its hanging indent. Breaks on spaces; a
/// single word wider than the budget is hard-split rather than overflowing.
/// Widths are display columns, so CJK (2 columns per glyph) cannot spill past
/// the pane edge.
fn wrap_row(text: &str, width: usize, indent: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let rest_width = width.saturating_sub(indent).max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut row = String::new();
    let mut used = 0usize;

    let budget = |rows: &Vec<String>| if rows.is_empty() { width } else { rest_width };

    for word in split_keeping_spaces(text) {
        let word_width = unicode_width::UnicodeWidthStr::width(word);
        if used + word_width > budget(&rows) && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            used = 0;
            // A break consumes the space that caused it — no leading blank.
            if word.trim().is_empty() {
                continue;
            }
        }
        if word_width > budget(&rows) {
            // Longer than a whole row: hard-split it across rows.
            for ch in word.chars() {
                let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if used + ch_width > budget(&rows) && !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
                row.push(ch);
                used += ch_width;
            }
            continue;
        }
        row.push_str(word);
        used += word_width;
    }
    rows.push(row);
    rows
}

/// Split into words with their trailing spaces attached, so a break can drop
/// the separator without losing interior spacing.
fn split_keeping_spaces(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut in_space = text.starts_with(' ');
    for (idx, ch) in text.char_indices() {
        let is_space = ch == ' ';
        if is_space != in_space {
            out.push(&text[start..idx]);
            start = idx;
            in_space = is_space;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn render_preview(
    preview: Option<&SelectionPreview>,
    area: Rect,
    buf: &mut Buffer,
    palette: Palette,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(preview) = preview else {
        return;
    };
    let lines = crate::menu::preview_layout::preview_lines(
        &preview.title,
        &preview.lines,
        preview.single_line,
        preview.scroll,
        area,
        palette,
    );
    let paragraph = Paragraph::new(Text::from(lines))
        .style(Style::default().fg(palette.text).bg(palette.surface_alt));
    // Truncated rows already fit the pane width; wrapping them would only
    // re-introduce the overflow the truncation exists to prevent.
    if preview.single_line {
        paragraph.render(area, buf);
    } else {
        paragraph.wrap(Wrap { trim: false }).render(area, buf);
    }
}

fn render_footer(
    area: Rect,
    buf: &mut Buffer,
    palette: Palette,
    hint: Option<&str>,
    fallback: &str,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let text = hint.unwrap_or(fallback);
    Paragraph::new(Line::from(Span::styled(
        fit_text(text, usize::from(area.width)),
        palette.muted().bg(palette.surface),
    )))
    .style(Style::default().bg(palette.surface))
    .render(area, buf);
}

fn fit_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    // `width` is a COLUMN budget, not a char count: CJK glyphs occupy 2
    // display columns. Accumulate unicode display width so translated/CJK
    // labels truncate on a column boundary instead of overflowing the row.
    use unicode_width::UnicodeWidthChar;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ThemeName;
    use ratatui::{Terminal, backend::TestBackend};

    /// Word wrap is the row's own layout: breaks land on spaces, an
    /// over-long word is hard-split rather than overflowing, and every budget
    /// is in DISPLAY COLUMNS so CJK cannot spill past the pane edge.
    #[test]
    fn wrap_row_breaks_on_words_with_a_hanging_indent_budget() {
        // Width 12, indent 5 => first row 12 cols, later rows 7.
        assert_eq!(
            wrap_row("alpha beta gamma delta", 12, 5),
            vec!["alpha beta ", "gamma ", "delta"]
        );
        // A word wider than the budget is hard-split, never overflowed: 12
        // columns on the first row, then 12-5=7 on each continuation.
        let split = wrap_row("supercalifragilistic", 12, 5);
        assert_eq!(split, vec!["supercalifra", "gilisti", "c"]);
        assert_eq!(
            split.concat(),
            "supercalifragilistic",
            "a hard split loses nothing"
        );
        // Display columns, not chars: CJK is 2 columns per glyph.
        assert_eq!(wrap_row("中文测试", 4, 0), vec!["中文", "测试"]);
        assert_eq!(wrap_row("fits", 20, 5), vec!["fits"]);
    }

    /// The reported bug: a long MCP row was clipped at the pane edge. It must
    /// now continue onto indented rows instead.
    #[test]
    fn long_item_wraps_instead_of_being_clipped() {
        let view = SelectionView::new(
            "Status",
            vec![SelectionItem::new(
                "mcp",
                "MCP - GitHub (connected, 4 tools), Gmail2 (connected, 0 tools), \
                 GDrive (connected, 2 tools)",
            )],
        );
        let rows = render_rows(&view, 60, 12);
        let first = row_index_containing(&rows, "MCP - GitHub");

        assert!(
            rows[first + 1].contains("tools)"),
            "the tail continues on the next row, got {:?}",
            rows[first + 1]
        );
        assert!(
            rows[first + 1].starts_with(&format!("│{}", " ".repeat(CONTINUATION_INDENT))),
            "continuations are indented past the marker columns, got {:?}",
            rows[first + 1]
        );
        assert!(
            rows.iter().any(|row| row.contains("GDrive")),
            "no text is lost off the pane edge"
        );
    }

    #[test]
    fn visible_start_budgets_in_visual_rows_not_items() {
        // Four items costing 1, 3, 1, 1 rows in a 4-row pane.
        let heights = [1, 3, 1, 1];
        // Selecting the tall item pulls the start forward so its rows fit.
        assert_eq!(visible_start(&heights, 1, 0, 4), 0, "1+3 fits exactly");
        assert_eq!(
            visible_start(&heights, 2, 0, 4),
            1,
            "1+3+1 does not fit, so the first item scrolls off"
        );
        // Keeping the SELECTION visible overrides the bottom-anchor clamp:
        // reaching item 3 scrolls the 3-row item off rather than hiding it.
        assert_eq!(visible_start(&heights, 3, 99, 4), 2);
        // With no selection pressure the clamp holds: item 0 stays on screen
        // because the tail alone cannot fill the pane.
        assert_eq!(visible_start(&heights, 0, 99, 4), 0);
        // Degenerate inputs are safe.
        assert_eq!(visible_start(&[], 0, 0, 4), 0);
        assert_eq!(visible_start(&heights, 0, 0, 0), 0);
    }

    /// i18n/CJK: `fit_text`'s `width` is a column budget. CJK glyphs are
    /// double-width, so truncation must count display columns, not chars —
    /// otherwise translated menu labels overflow the row and misalign.
    #[test]
    fn fit_text_truncates_on_column_width_not_char_count() {
        assert_eq!(fit_text("hello", 3), "hel"); // ASCII: 1 col/char
        assert_eq!(fit_text("中文测试", 4), "中文"); // each CJK = 2 cols → 2 glyphs
        assert_eq!(fit_text("中文测试", 5), "中文"); // 3rd glyph would be col 6 > 5
        assert_eq!(fit_text("a中b", 3), "a中"); // 1 + 2 = 3 cols exactly
        assert_eq!(fit_text("中", 1), ""); // a 2-col glyph cannot fit in 1 col
    }

    fn render_buffer(view: &SelectionView, width: u16, height: u16, palette: Palette) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| frame.render_widget(view.widget(palette), frame.area()))
            .expect("render succeeds");
        terminal.backend().buffer().clone()
    }

    fn render_view(view: &SelectionView, width: u16, height: u16) -> String {
        render_buffer(view, width, height, Palette::for_theme(ThemeName::Slate))
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn style_for_text(buffer: &Buffer, needle: &str) -> Option<Style> {
        let width = usize::from(buffer.area.width);
        let height = usize::from(buffer.area.height);
        for y in 0..height {
            let row_start = y * width;
            let row = buffer.content[row_start..row_start + width]
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            if let Some(x) = row.find(needle) {
                let cell = &buffer.content[row_start + x];
                return Some(
                    Style::default()
                        .fg(cell.fg)
                        .bg(cell.bg)
                        .add_modifier(cell.modifier),
                );
            }
        }
        None
    }

    #[test]
    fn renders_selected_disabled_and_marked_rows() {
        let mut current = SelectionItem::new("current", "Current model");
        current.current = true;
        let mut disabled = SelectionItem::new("disabled", "Disabled model");
        disabled.disabled_reason = Some("server unavailable".into());
        let mut default = SelectionItem::new("default", "Default model");
        default.default = true;
        let mut view = SelectionView::new("Model", vec![current, disabled, default]);
        view.selected = 1;

        let text = render_view(&view, 80, 10);

        assert!(text.contains(">  Disabled model"));
        assert!(text.contains("server unavailable"));
        // The active selection is marked with a leading `*` (not a trailing word).
        assert!(text.contains("* Current model"));
        assert!(text.contains("Default model default"));
    }

    #[test]
    fn renders_required_rows_with_success_and_danger_colors() {
        let mut missing = SelectionItem::new("missing", "API key: not set");
        missing.required_valid = Some(false);
        let mut ready = SelectionItem::new("ready", "Model: deepseek-reasoner");
        ready.required_valid = Some(true);
        let view = SelectionView::new("Provider", vec![missing, ready]);
        let palette = Palette::for_theme(ThemeName::Codex);
        let buffer = render_buffer(&view, 80, 8, palette);

        let missing_style = style_for_text(&buffer, "API key").expect("missing row style");
        let ready_style = style_for_text(&buffer, "Model").expect("ready row style");

        assert_eq!(missing_style.fg, Some(palette.danger));
        assert_eq!(ready_style.fg, Some(palette.success));
    }

    fn row_index_containing(rows: &[String], needle: &str) -> usize {
        rows.iter()
            .position(|row| row.contains(needle))
            .unwrap_or_else(|| panic!("row containing {needle:?} in {rows:#?}"))
    }

    fn render_rows(view: &SelectionView, width: u16, height: u16) -> Vec<String> {
        let buffer = render_buffer(view, width, height, Palette::for_theme(ThemeName::Slate));
        let width = usize::from(buffer.area.width);
        (0..usize::from(buffer.area.height))
            .map(|y| {
                buffer.content[y * width..(y + 1) * width]
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// A key/value preview row is ONE row. Wrapping pushed every following row
    /// down — a long `session_id` shoved `session:` and `health:` down the
    /// Snapshot pane and off the bottom.
    #[test]
    fn single_line_preview_truncates_long_rows_instead_of_wrapping() {
        let mut view = SelectionView::new("Status", vec![SelectionItem::new("refresh", "Refresh")]);
        let mut preview = SelectionPreview::new(
            "Snapshot",
            vec![
                format!("session_id: alan:local:tui#coding{}", "x".repeat(200)),
                "session: Protocol session".into(),
                "health: ok".into(),
            ],
        );
        preview.single_line = true;
        view.preview = Some(preview);

        let rows = render_rows(&view, 120, 12);
        let id_row = rows
            .iter()
            .position(|row| row.contains("session_id:"))
            .expect("session_id row");

        assert!(
            rows[id_row].contains('…'),
            "the long value is ellipsized: {:?}",
            rows[id_row]
        );
        assert!(
            rows[id_row + 1].contains("session: Protocol session"),
            "the next row keeps its slot, got {:?}",
            rows[id_row + 1]
        );
        assert!(
            rows[id_row + 2].contains("health: ok"),
            "and so does the one after it, got {:?}",
            rows[id_row + 2]
        );
        assert_eq!(
            rows.iter().filter(|row| row.contains('x')).count(),
            1,
            "the value occupies exactly one row — no wrapped remainder"
        );
    }

    /// Text previews still wrap: their bodies are prose, not key/value rows.
    #[test]
    fn wrapping_preview_still_wraps_long_lines() {
        let mut view = SelectionView::new("Theme", vec![SelectionItem::new("slate", "Slate")]);
        view.preview = Some(SelectionPreview::new(
            "Preview",
            vec![format!("body {}", "y".repeat(200))],
        ));

        let rows = render_rows(&view, 120, 12);
        assert!(
            rows.iter().filter(|row| row.contains('y')).count() > 1,
            "a Text preview body wraps across rows as before"
        );
    }

    #[test]
    fn wide_layout_renders_side_preview() {
        let mut view = SelectionView::new(
            "Theme",
            vec![
                SelectionItem::new("slate", "Slate"),
                SelectionItem::new("terminal", "Terminal"),
            ],
        );
        view.preview = Some(SelectionPreview::new(
            "Preview",
            vec!["Surface".into(), "Accent".into()],
        ));

        let text = render_view(&view, 120, 12);

        assert!(text.contains("Preview"));
        assert!(text.contains("Surface"));
        assert!(text.contains("Slate"));
    }
}
