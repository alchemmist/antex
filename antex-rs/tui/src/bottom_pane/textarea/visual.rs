use super::KillBufferKind;
use super::TextArea;
use super::VimMode;
use super::VimOperator;
use super::VimPending;
use super::vim::VimFindMotion;
use crate::key_hint::KeyBindingListExt;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VimVisualKind {
    Character,
    Line,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VimVisualState {
    pub(super) anchor: usize,
    pub(super) kind: VimVisualKind,
}

impl TextArea {
    pub(super) fn start_vim_visual(&mut self, kind: VimVisualKind) {
        self.vim_visual = Some(VimVisualState {
            anchor: self.cursor_pos,
            kind,
        });
        self.vim_pending = VimPending::None;
    }

    pub(super) fn handle_vim_visual(&mut self, event: KeyEvent) {
        if event.code == KeyCode::Esc {
            self.enter_vim_normal_mode();
            return;
        }

        let event = self.normalize_vim_command_event(event);
        if matches!(self.vim_pending, VimPending::Find { .. }) {
            let pending = std::mem::replace(&mut self.vim_pending, VimPending::None);
            self.handle_vim_pending_command(pending, event);
            return;
        }

        if visual_key(event, 'v') {
            self.toggle_vim_visual(VimVisualKind::Character);
            return;
        }
        if visual_shift_key(event, 'v') {
            self.toggle_vim_visual(VimVisualKind::Line);
            return;
        }
        if visual_control_key(event, 'v') {
            self.toggle_vim_visual(VimVisualKind::Block);
            return;
        }

        if self.vim_normal_keymap.move_left.is_pressed(event) {
            self.move_cursor_left();
        } else if self.vim_normal_keymap.move_right.is_pressed(event) {
            self.move_cursor_right();
        } else if self.vim_normal_keymap.move_down.is_pressed(event) {
            self.move_cursor_down();
        } else if self.vim_normal_keymap.move_up.is_pressed(event) {
            self.move_cursor_up();
        } else if self.vim_normal_keymap.move_word_forward.is_pressed(event) {
            self.set_cursor(self.beginning_of_next_word());
        } else if self.vim_normal_keymap.move_word_backward.is_pressed(event) {
            self.set_cursor(self.beginning_of_previous_word());
        } else if self.vim_normal_keymap.move_word_end.is_pressed(event) {
            self.set_cursor(self.vim_word_end_cursor());
        } else if self.vim_normal_keymap.move_line_start.is_pressed(event) {
            self.set_cursor(self.beginning_of_current_line());
        } else if self.vim_normal_keymap.move_line_end.is_pressed(event) {
            self.set_cursor(self.vim_line_end_cursor());
        } else if self.vim_normal_keymap.find_forward.is_pressed(event) {
            self.start_vim_find(VimFindMotion::Forward, /*operator*/ None);
        } else if self.vim_normal_keymap.find_backward.is_pressed(event) {
            self.start_vim_find(VimFindMotion::Backward, /*operator*/ None);
        } else if self.vim_normal_keymap.till_forward.is_pressed(event) {
            self.start_vim_find(VimFindMotion::TillForward, /*operator*/ None);
        } else if self.vim_normal_keymap.till_backward.is_pressed(event) {
            self.start_vim_find(VimFindMotion::TillBackward, /*operator*/ None);
        } else if self.vim_normal_keymap.delete_char.is_pressed(event)
            || self
                .vim_normal_keymap
                .start_delete_operator
                .is_pressed(event)
            || self.vim_normal_keymap.delete_to_line_end.is_pressed(event)
        {
            self.apply_vim_visual_operator(VimOperator::Delete);
        } else if self.vim_normal_keymap.start_yank_operator.is_pressed(event)
            || self.vim_normal_keymap.yank_line.is_pressed(event)
        {
            self.apply_vim_visual_operator(VimOperator::Yank);
        } else if self
            .vim_normal_keymap
            .start_change_operator
            .is_pressed(event)
            || self.vim_normal_keymap.change_to_line_end.is_pressed(event)
            || self.vim_normal_keymap.substitute_char.is_pressed(event)
        {
            self.apply_vim_visual_operator(VimOperator::Change);
        }
    }

    pub(super) fn vim_visual_ranges(&self) -> Vec<Range<usize>> {
        let Some(visual) = self.vim_visual else {
            return Vec::new();
        };
        match visual.kind {
            VimVisualKind::Character => self.character_visual_range(visual.anchor),
            VimVisualKind::Line => self.line_visual_range(visual.anchor),
            VimVisualKind::Block => self.block_visual_ranges(visual.anchor),
        }
    }

    pub(super) fn vim_visual_label(&self) -> Option<&'static str> {
        Some(match self.vim_visual?.kind {
            VimVisualKind::Character => "Visual",
            VimVisualKind::Line => "Visual Line",
            VimVisualKind::Block => "Visual Block",
        })
    }

    fn toggle_vim_visual(&mut self, kind: VimVisualKind) {
        if self.vim_visual.is_some_and(|visual| visual.kind == kind) {
            self.enter_vim_normal_mode();
        } else if let Some(visual) = self.vim_visual.as_mut() {
            visual.kind = kind;
        }
    }

    fn character_visual_range(&self, anchor: usize) -> Vec<Range<usize>> {
        let start = anchor.min(self.cursor_pos);
        let cursor_end = self.next_atomic_boundary(anchor.max(self.cursor_pos));
        (start < cursor_end)
            .then_some(start..cursor_end)
            .into_iter()
            .collect()
    }

    fn line_visual_range(&self, anchor: usize) -> Vec<Range<usize>> {
        let anchor_start = self.beginning_of_line(anchor);
        let cursor_start = self.beginning_of_current_line();
        let start = anchor_start.min(cursor_start);
        let last_line_start = anchor_start.max(cursor_start);
        let mut end = self.end_of_line(last_line_start);
        if end < self.text.len() {
            end += 1;
        }
        (start < end).then_some(start..end).into_iter().collect()
    }

    fn block_visual_ranges(&self, anchor: usize) -> Vec<Range<usize>> {
        let anchor_line = self.beginning_of_line(anchor);
        let cursor_line = self.beginning_of_current_line();
        let first_line = anchor_line.min(cursor_line);
        let last_line = anchor_line.max(cursor_line);
        let anchor_col = super::display_width(&self.text[anchor_line..anchor]);
        let cursor_col = super::display_width(&self.text[cursor_line..self.cursor_pos]);
        let start_col = anchor_col.min(cursor_col);
        let end_col = anchor_col.max(cursor_col);
        let mut ranges = Vec::new();
        let mut line_start = first_line;
        loop {
            let line_end = self.end_of_line(line_start);
            if let Some(range) =
                display_column_range(&self.text, line_start, line_end, start_col, end_col)
            {
                ranges.push(range);
            }
            if line_start >= last_line || line_end >= self.text.len() {
                break;
            }
            line_start = line_end + 1;
        }
        ranges
    }

    fn apply_vim_visual_operator(&mut self, operator: VimOperator) {
        let Some(visual) = self.vim_visual else {
            return;
        };
        let ranges = self
            .vim_visual_ranges()
            .into_iter()
            .map(|range| self.expand_range_to_element_boundaries(range))
            .filter(|range| range.start < range.end)
            .collect::<Vec<_>>();
        let Some(first) = ranges.first() else {
            self.enter_vim_normal_mode();
            return;
        };
        let cursor = first.start;
        let kind = if visual.kind == VimVisualKind::Line {
            KillBufferKind::Linewise
        } else {
            KillBufferKind::Characterwise
        };
        let removed = ranges
            .iter()
            .map(|range| self.text[range.clone()].to_string())
            .collect::<Vec<_>>()
            .join(if visual.kind == VimVisualKind::Block {
                "\n"
            } else {
                ""
            });
        self.store_kill_buffer(removed, kind);
        if operator == VimOperator::Yank {
            self.pending_system_clipboard_yank = Some(self.kill_buffer.clone());
        }
        if operator != VimOperator::Yank {
            for range in ranges.into_iter().rev() {
                self.replace_range_raw(range, "");
            }
        }
        self.set_cursor(cursor.min(self.text.len()));
        self.vim_visual = None;
        self.vim_pending = VimPending::None;
        self.vim_mode = if operator == VimOperator::Change {
            VimMode::Insert
        } else {
            VimMode::Normal
        };
    }
}

pub(super) fn visual_key(event: KeyEvent, key: char) -> bool {
    event.code == KeyCode::Char(key) && event.modifiers == KeyModifiers::NONE
}

pub(super) fn visual_shift_key(event: KeyEvent, key: char) -> bool {
    matches!(
        event,
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers: KeyModifiers::SHIFT,
            ..
        } if ch == key.to_ascii_uppercase() || ch == key
    ) || matches!(
        event,
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers: KeyModifiers::NONE,
            ..
        } if ch == key.to_ascii_uppercase()
    )
}

pub(super) fn visual_control_key(event: KeyEvent, key: char) -> bool {
    event.code == KeyCode::Char(key) && event.modifiers == KeyModifiers::CONTROL
}

fn display_column_range(
    text: &str,
    line_start: usize,
    line_end: usize,
    start_col: usize,
    end_col: usize,
) -> Option<Range<usize>> {
    let line = &text[line_start..line_end];
    let mut display_col = 0;
    let mut start = None;
    let mut end = None;
    for (offset, grapheme) in line.grapheme_indices(true) {
        let next_col = display_col + super::display_width(grapheme);
        if start.is_none() && next_col > start_col {
            start = Some(line_start + offset);
        }
        if display_col <= end_col && next_col > start_col {
            end = Some(line_start + offset + grapheme.len());
        }
        display_col = next_col;
    }
    start.zip(end).map(|(start, end)| start..end)
}

#[cfg(test)]
#[path = "visual_tests.rs"]
mod tests;
