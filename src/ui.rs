use std::collections::HashSet;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::tree::FlatRow;

pub struct View<'a> {
    pub query: &'a str,
    pub rows: Vec<VisibleRow<'a>>,
    pub selected: usize,
    pub total_rows: usize,
    pub status: ViewStatus<'a>,
}

pub struct VisibleRow<'a> {
    pub row: &'a FlatRow,
    pub match_positions: &'a [usize],
}

pub enum ViewStatus<'a> {
    Ready,
    Reconnecting { message: &'a str },
}

pub fn draw(frame: &mut Frame<'_>, view: &View<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    draw_prompt(frame, chunks[0], view.query);
    match view.status {
        ViewStatus::Ready => draw_rows(frame, chunks[1], view),
        ViewStatus::Reconnecting { message } => draw_reconnect(frame, chunks[1], message),
    }
    draw_count(frame, chunks[2], view);
}

fn draw_prompt(frame: &mut Frame<'_>, area: Rect, query: &str) {
    let width = area.width.saturating_sub(3) as usize;
    let query = tail_chars(query, width);
    let line = Line::from(vec![
        Span::raw("> "),
        Span::styled(query, Style::new().add_modifier(Modifier::BOLD)),
        Span::raw("█"),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_rows(frame: &mut Frame<'_>, area: Rect, view: &View<'_>) {
    if area.height == 0 {
        return;
    }

    if view.rows.is_empty() {
        frame.render_widget(Paragraph::new("No matches"), area);
        return;
    }

    let visible_height = area.height as usize;
    let offset = scroll_offset(view.selected, visible_height, view.rows.len());

    for (line_idx, visible) in view
        .rows
        .iter()
        .skip(offset)
        .take(visible_height)
        .enumerate()
    {
        let y = area.y + line_idx as u16;
        let row_area = Rect::new(area.x, y, area.width, 1);
        let selected = offset + line_idx == view.selected;
        let line = row_line(visible, area.width as usize, selected);
        frame.render_widget(Paragraph::new(line), row_area);
    }
}

fn draw_reconnect(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let width = area.width as usize;
    let lines = [
        "Reconnecting to cmux",
        message,
        "Set CMUX_TUI_SOCKET to the cmux-tui JSON-lines socket path.",
    ];
    for (idx, line) in lines.iter().enumerate() {
        if idx >= area.height as usize {
            break;
        }
        let truncated = middle_truncate(line, width).into_iter().collect::<String>();
        frame.render_widget(
            Paragraph::new(Line::from(truncated)),
            Rect::new(area.x, area.y + idx as u16, area.width, 1),
        );
    }
}

fn draw_count(frame: &mut Frame<'_>, area: Rect, view: &View<'_>) {
    let text = format!("{}/{}", view.rows.len(), view.total_rows);
    frame.render_widget(Paragraph::new(text), area);
}

fn row_line(row: &VisibleRow<'_>, width: usize, selected: bool) -> Line<'static> {
    let prefix = format!("{}#{} ", row.row.kind.as_str(), row.row.id);
    let active = if row.row.active { "* " } else { "  " };
    let prefix_width = prefix.chars().count() + active.chars().count();
    let label_width = width.saturating_sub(prefix_width).max(1);
    let label_chars = middle_truncate_with_positions(&row.row.label, label_width);
    let matches = row.match_positions.iter().copied().collect::<HashSet<_>>();
    let base_style = if selected {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };

    let mut spans = vec![
        Span::styled(active.to_string(), base_style),
        Span::styled(prefix, base_style.add_modifier(Modifier::DIM)),
    ];

    for (ch, original_idx) in label_chars {
        let mut style = base_style;
        if original_idx.is_some_and(|idx| matches.contains(&idx)) {
            style = style.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(ch.to_string(), style));
    }

    Line::from(spans)
}

fn scroll_offset(selected: usize, visible_height: usize, total: usize) -> usize {
    if visible_height == 0 || total <= visible_height {
        return 0;
    }
    if selected < visible_height {
        return 0;
    }
    (selected + 1)
        .saturating_sub(visible_height)
        .min(total - visible_height)
}

fn tail_chars(input: &str, max_chars: usize) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return input.to_string();
    }
    chars[chars.len().saturating_sub(max_chars)..]
        .iter()
        .collect()
}

fn middle_truncate(input: &str, max_chars: usize) -> Vec<char> {
    middle_truncate_with_positions(input, max_chars)
        .into_iter()
        .map(|(ch, _)| ch)
        .collect()
}

fn middle_truncate_with_positions(input: &str, max_chars: usize) -> Vec<(char, Option<usize>)> {
    let chars = input.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return chars
            .into_iter()
            .enumerate()
            .map(|(idx, ch)| (ch, Some(idx)))
            .collect();
    }

    if max_chars == 0 {
        return Vec::new();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars).chars().map(|ch| (ch, None)).collect();
    }

    let marker = ['.', '.', '.'];
    let keep = max_chars - marker.len();
    let front = keep.div_ceil(2);
    let back = keep / 2;
    let mut out = Vec::with_capacity(max_chars);

    for (idx, ch) in chars.iter().copied().take(front).enumerate() {
        out.push((ch, Some(idx)));
    }
    out.extend(marker.into_iter().map(|ch| (ch, None)));
    let start_back = chars.len() - back;
    for (idx, ch) in chars.iter().copied().enumerate().skip(start_back) {
        out.push((ch, Some(idx)));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_truncates_with_position_map() {
        let mapped = middle_truncate_with_positions("abcdefghi", 7);
        let text = mapped.iter().map(|(ch, _)| *ch).collect::<String>();
        assert_eq!(text, "ab...hi");
        assert_eq!(mapped[0].1, Some(0));
        assert_eq!(mapped[3].1, None);
        assert_eq!(mapped[6].1, Some(8));
    }
}
