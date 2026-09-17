use super::Tui;
use crate::custom_terminal::Terminal;
use crate::insert_history::HistoryLineWrapPolicy;
use crate::insert_history::wrap_history_hyperlink_lines;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_palette::terminal_background;
use crate::terminal_palette::terminal_foreground;
use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::io;
use std::io::Write;

impl Tui {
    pub(crate) fn repaint_visible_history_block(
        &mut self,
        previous: &[HyperlinkLine],
        replacement: &[HyperlinkLine],
        rows_after: usize,
        wrap_policy: HistoryLineWrapPolicy,
    ) -> io::Result<()> {
        let screen_size = self.terminal.last_known_screen_size;
        Self::flush_pending_history_lines(
            &mut self.terminal,
            &mut self.pending_history_lines,
            self.scrollback,
            screen_size,
        )?;
        repaint_history_block(
            &mut self.terminal,
            previous,
            replacement,
            rows_after,
            wrap_policy,
        )
    }
}

fn repaint_history_block<B>(
    terminal: &mut Terminal<B>,
    previous: &[HyperlinkLine],
    replacement: &[HyperlinkLine],
    rows_after: usize,
    wrap_policy: HistoryLineWrapPolicy,
) -> io::Result<()>
where
    B: Backend<Error = io::Error> + Write,
{
    let width = terminal.viewport_area.width.max(1);
    let (previous, previous_rows) =
        wrap_history_hyperlink_lines(previous, usize::from(width), wrap_policy);
    let (replacement, replacement_rows) =
        wrap_history_hyperlink_lines(replacement, usize::from(width), wrap_policy);
    if previous_rows != replacement_rows
        || previous_rows != previous.len()
        || replacement_rows != replacement.len()
        || previous_rows + rows_after > usize::from(terminal.viewport_area.y)
    {
        return Ok(());
    }
    let top = terminal.viewport_area.y - (previous_rows + rows_after) as u16;
    let area = Rect::new(0, top, width, previous_rows as u16);
    let mut old = Buffer::empty(area);
    let mut new = Buffer::empty(area);
    for (buffer, lines) in [(&mut old, previous), (&mut new, replacement)] {
        for (row, line) in lines.iter().enumerate() {
            buffer.set_line(0, top + row as u16, &line.line, width);
        }
        for cell in &mut buffer.content {
            cell.fg = terminal_foreground(cell.fg);
            cell.bg = terminal_background(cell.bg);
        }
    }
    let cursor = terminal.last_known_cursor_pos;
    terminal.backend_mut().draw(old.diff(&new).into_iter())?;
    terminal.backend_mut().set_cursor_position(cursor)?;
    Backend::flush(terminal.backend_mut())
}

#[cfg(test)]
#[path = "history_animation_tests.rs"]
mod tests;
