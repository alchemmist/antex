use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use antex_config::types::StartupMascotSkin;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use crate::mascot_palette::MascotColor;
use crate::mascot_palette::mascot_color;
use crate::tui::FrameRequester;

pub(super) const MASCOT_WIDTH: usize = 13;
const MASCOT_LOGICAL_HEIGHT: usize = 12;
const FRAME_TICK: Duration = Duration::from_millis(180);
const FRAME_SEQUENCE: [MascotFrame; 8] = [
    MascotFrame::Rest,
    MascotFrame::AntennaeWide,
    MascotFrame::Rest,
    MascotFrame::AntennaeWide,
    MascotFrame::Rest,
    MascotFrame::AntennaeWide,
    MascotFrame::Rest,
    MascotFrame::Blink,
];

const ANT_01: [&str; MASCOT_LOGICAL_HEIGHT] = [
    "..2.......2..",
    "...3.....3...",
    "....3...3....",
    "...1111111...",
    "..111111111..",
    ".114.111.411.",
    ".11..111..11.",
    "..333333333..",
    "...33...33...",
    "..33.....33..",
    ".33.......33.",
    ".............",
];

const ANT_03: [&str; MASCOT_LOGICAL_HEIGHT] = [
    ".2.........2.",
    "..3.......3..",
    "...3111113...",
    "..111111111..",
    ".11111111111.",
    "114.11111.411",
    "11..11111..11",
    ".33333333333.",
    "..33.....33..",
    "..33..3..33..",
    "...333.333...",
    ".............",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MascotFrame {
    Rest,
    AntennaeWide,
    Blink,
}

#[derive(Clone, Debug)]
pub(crate) struct StartupMascotMotion {
    request_frame: FrameRequester,
    started_at: Arc<OnceLock<Instant>>,
}

impl StartupMascotMotion {
    pub(crate) fn new(request_frame: FrameRequester) -> Self {
        Self {
            request_frame,
            started_at: Arc::new(OnceLock::new()),
        }
    }

    pub(super) fn current_frame(&self) -> MascotFrame {
        self.current_frame_at(Instant::now())
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.started_at
            .get()
            .is_some_and(|started| frame_at_elapsed(started.elapsed()).is_none())
    }

    fn current_frame_at(&self, now: Instant) -> MascotFrame {
        let started = self.started_at.get_or_init(|| now);
        let Some((frame, next_frame_in)) =
            frame_at_elapsed(now.saturating_duration_since(*started))
        else {
            return MascotFrame::Rest;
        };
        self.request_frame.schedule_frame_in(next_frame_in);
        frame
    }
}

pub(super) fn render_mascot(skin: StartupMascotSkin, frame: MascotFrame) -> Vec<Line<'static>> {
    if skin == StartupMascotSkin::None {
        return Vec::new();
    }

    (0..MASCOT_LOGICAL_HEIGHT)
        .step_by(2)
        .map(|row| {
            Line::from(render_row_pair(
                sprite_row(skin, row, frame),
                sprite_row(skin, row + 1, frame),
            ))
        })
        .collect()
}

fn frame_at_elapsed(elapsed: Duration) -> Option<(MascotFrame, Duration)> {
    let tick_ms = FRAME_TICK.as_millis();
    let elapsed_ms = elapsed.as_millis();
    let frame_index = usize::try_from(elapsed_ms / tick_ms).unwrap_or(usize::MAX);
    let frame = FRAME_SEQUENCE.get(frame_index).copied()?;
    let remainder_ms = elapsed_ms % tick_ms;
    let next_frame_ms = if remainder_ms == 0 {
        tick_ms
    } else {
        tick_ms - remainder_ms
    };
    let next_frame_ms = u64::try_from(next_frame_ms).unwrap_or(u64::MAX);
    Some((frame, Duration::from_millis(next_frame_ms)))
}

fn sprite_row(skin: StartupMascotSkin, row: usize, frame: MascotFrame) -> &'static str {
    match (skin, frame, row) {
        (StartupMascotSkin::Ant01, MascotFrame::AntennaeWide, 0) => ".2.........2.",
        (StartupMascotSkin::Ant01, MascotFrame::AntennaeWide, 1) => "..3.......3..",
        (StartupMascotSkin::Ant01, MascotFrame::AntennaeWide, 2) => "...3.....3...",
        (StartupMascotSkin::Ant03, MascotFrame::AntennaeWide, 0) => "..2.......2..",
        (StartupMascotSkin::Ant03, MascotFrame::AntennaeWide, 1) => "...3.....3...",
        (StartupMascotSkin::Ant01, MascotFrame::Blink, 5) => ".113.111.311.",
        (StartupMascotSkin::Ant01, MascotFrame::Blink, 6) => ".11..111..11.",
        (StartupMascotSkin::Ant03, MascotFrame::Blink, 5) => "113.11111.311",
        (StartupMascotSkin::Ant03, MascotFrame::Blink, 6) => "11..11111..11",
        (StartupMascotSkin::Ant01, _, row) => ANT_01[row],
        (StartupMascotSkin::Ant03, _, row) => ANT_03[row],
        (StartupMascotSkin::None, _, _) => "",
    }
}

fn render_row_pair(top: &str, bottom: &str) -> Vec<Span<'static>> {
    let mut spans = top
        .bytes()
        .zip(bottom.bytes())
        .map(|(top, bottom)| render_pixel_pair(pixel(top), pixel(bottom)))
        .collect::<Vec<_>>();
    while spans
        .last()
        .is_some_and(|span| span.content.as_ref() == " ")
    {
        spans.pop();
    }
    spans
}

fn pixel(value: u8) -> Option<MascotColor> {
    match value {
        b'1' => Some(MascotColor::Chestnut),
        b'2' => Some(MascotColor::Sand),
        b'3' => Some(MascotColor::DarkBrown),
        b'4' => Some(MascotColor::Teal),
        b'.' => None,
        _ => None,
    }
}

fn render_pixel_pair(top: Option<MascotColor>, bottom: Option<MascotColor>) -> Span<'static> {
    match (top, bottom) {
        (None, None) => " ".into(),
        (Some(top), None) => Span::styled("▀", Style::default().fg(mascot_color(top))),
        (None, Some(bottom)) => Span::styled("▄", Style::default().fg(mascot_color(bottom))),
        (Some(top), Some(bottom)) if top == bottom => {
            Span::styled("█", Style::default().fg(mascot_color(top)))
        }
        (Some(top), Some(bottom)) => Span::styled(
            "▀",
            Style::default()
                .fg(mascot_color(top))
                .bg(mascot_color(bottom)),
        ),
    }
}

#[cfg(test)]
#[path = "startup_mascot_tests.rs"]
mod tests;
