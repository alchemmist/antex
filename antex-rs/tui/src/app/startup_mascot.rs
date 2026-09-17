use super::App;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryRenderMode;
use crate::history_cell::StartupMascotMotion;
use crate::insert_history::wrap_history_hyperlink_lines;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::tui::Tui;
use color_eyre::eyre::Result;
use std::sync::Arc;
use std::sync::Weak;

pub(super) struct StartupMascotAnimation {
    pub(super) motion: StartupMascotMotion,
    pub(super) header: Option<Weak<dyn HistoryCell>>,
    pub(super) previous_lines: Vec<HyperlinkLine>,
    pub(super) width: u16,
}

impl StartupMascotAnimation {
    pub(super) fn new(motion: StartupMascotMotion) -> Self {
        Self {
            motion,
            header: None,
            previous_lines: Vec::new(),
            width: 0,
        }
    }
}

impl App {
    pub(super) fn refresh_startup_mascot(&mut self, tui: &mut Tui) -> Result<()> {
        let Some(mut animation) = self.startup_mascot_animation.take() else {
            return Ok(());
        };
        let finished = animation.motion.is_finished();
        if self.overlay.is_some() || animation.header.is_none() {
            if !finished {
                self.startup_mascot_animation = Some(animation);
            }
            return Ok(());
        }
        let Some(header) = animation.header.as_ref().and_then(Weak::upgrade) else {
            return Ok(());
        };
        let Some(index) = self
            .transcript_cells
            .iter()
            .position(|cell| Arc::ptr_eq(cell, &header))
        else {
            return Ok(());
        };
        let mode = self.chat_widget.history_render_mode();
        if mode == HistoryRenderMode::Raw {
            return Ok(());
        }
        let width = self
            .chat_widget
            .history_wrap_width(tui.terminal.last_known_screen_size.width);
        let wrap_policy = self.history_line_wrap_policy();
        let mut rows_after = 0usize;
        for cell in self.transcript_cells[index + 1..].iter().rev() {
            let lines = cell.display_hyperlink_lines_for_mode(width, mode);
            if lines.is_empty() {
                continue;
            }
            rows_after += wrap_history_hyperlink_lines(
                &lines,
                usize::from(tui.terminal.last_known_screen_size.width.max(1)),
                wrap_policy,
            )
            .1 + usize::from(!cell.is_stream_continuation());
            if rows_after >= usize::from(tui.terminal.last_known_screen_size.height) {
                return Ok(());
            }
        }
        let lines = header.display_hyperlink_lines_for_mode(width, mode);
        if animation.width == width && lines != animation.previous_lines {
            tui.repaint_visible_history_block(
                &animation.previous_lines,
                &lines,
                rows_after,
                wrap_policy,
            )?;
        }
        animation.previous_lines = lines;
        animation.width = width;
        if !finished {
            self.startup_mascot_animation = Some(animation);
        }
        Ok(())
    }
}
