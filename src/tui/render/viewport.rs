use ratatui::{
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Wrap},
};

use crate::tui::custom_terminal::Frame;

pub(crate) struct TranscriptViewport {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) scroll_offset: u16,
}

impl TranscriptViewport {
    pub(crate) fn new(lines: Vec<Line<'static>>, scroll_offset: u16) -> Self {
        Self {
            lines,
            scroll_offset,
        }
    }

    pub(crate) fn visible_window(&self, width: u16, height: u16) -> (Vec<Line<'static>>, u16) {
        if self.lines.is_empty() || height == 0 {
            return (Vec::new(), 0);
        }

        let wrap_width = usize::from(width.max(1));
        let target_start = usize::from(self.scroll_offset);
        let target_end = target_start.saturating_add(usize::from(height));

        let mut row_cursor = 0usize;
        let mut first_idx = None;
        let mut first_inner_scroll = 0u16;
        let mut last_exclusive_idx = self.lines.len();

        for (idx, line) in self.lines.iter().enumerate() {
            let line_rows = visual_rows_for_line(line, wrap_width);
            let line_end = row_cursor.saturating_add(line_rows);

            if first_idx.is_none() && line_end > target_start {
                first_idx = Some(idx);
                first_inner_scroll = target_start.saturating_sub(row_cursor) as u16;
            }

            if line_end >= target_end {
                last_exclusive_idx = idx + 1;
                break;
            }

            row_cursor = line_end;
        }

        let Some(first_idx) = first_idx else {
            return (Vec::new(), 0);
        };

        (
            self.lines[first_idx..last_exclusive_idx].to_vec(),
            first_inner_scroll,
        )
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect) {
        let (visible_lines, inner_scroll) = self.visible_window(area.width, area.height);
        f.render_widget(
            Paragraph::new(visible_lines)
                .wrap(Wrap { trim: false })
                .scroll((inner_scroll, 0)),
            area,
        );
    }
}

fn visual_rows_for_line(line: &Line<'static>, wrap_width: usize) -> usize {
    crate::tui::layout_utils::line_visual_rows(line, wrap_width)
}
