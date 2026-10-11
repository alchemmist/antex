//! Session information appears only once all older transcript pages are available.

use super::*;
use crate::pager_overlay::TranscriptHistoryState;
use crate::transcript_view::TranscriptView;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use pretty_assertions::assert_eq;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

#[derive(Debug)]
struct AnimationCacheProbe {
    tick: Option<Arc<AtomicU64>>,
    renders: Arc<AtomicUsize>,
}

impl HistoryCell for AnimationCacheProbe {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.renders.fetch_add(1, Ordering::Relaxed);
        vec![format!("frame {:?}", self.transcript_animation_tick()).into()]
    }

    fn transcript_animation_tick(&self) -> Option<u64> {
        self.tick
            .as_ref()
            .map(|tick| tick.load(Ordering::Relaxed))
            .filter(|tick| *tick != 0)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        vec!["retained source".into()]
    }
}

#[test]
fn session_animation_refreshes_only_its_retained_layout() {
    let tick = Arc::new(AtomicU64::new(1));
    let header_renders = Arc::new(AtomicUsize::new(0));
    let other_renders = Arc::new(AtomicUsize::new(0));
    let header: Arc<dyn HistoryCell> = Arc::new(SessionInfoCell(CompositeHistoryCell {
        parts: vec![Box::new(AnimationCacheProbe {
            tick: Some(tick.clone()),
            renders: header_renders.clone(),
        })],
    }));
    let source = header.raw_lines();
    let cells = vec![
        header.clone(),
        Arc::new(AnimationCacheProbe {
            tick: None,
            renders: other_renders.clone(),
        }) as Arc<dyn HistoryCell>,
    ];
    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 60, /*height*/ 12,
    );
    let mut view = TranscriptView::default();
    for (index, revision) in [1, 2, 0].into_iter().enumerate() {
        tick.store(revision, Ordering::Relaxed);
        let expected_tick = (revision != 0).then_some(revision);
        assert_eq!(header.transcript_animation_tick(), expected_tick);
        assert!(header.has_stable_transcript_height());
        let mut before = Buffer::empty(area);
        view.render(area, &mut before, &cells);
        let mut repeated = Buffer::empty(area);
        view.render(area, &mut repeated, &cells);
        assert_eq!(repeated, before);
        assert_eq!(header.raw_lines(), source);
        assert_eq!(
            (
                header_renders.load(Ordering::Relaxed),
                other_renders.load(Ordering::Relaxed)
            ),
            (index + 1, 1),
        );
    }
}

#[test]
fn wheel_at_hidden_session_header_keeps_the_loaded_message_anchor() {
    let header: Arc<dyn HistoryCell> = Arc::new(SessionInfoCell(CompositeHistoryCell {
        parts: vec![Box::new(PlainHistoryCell::new(vec![
            "session announcement".into(),
        ]))],
    }));
    let recent: Arc<dyn HistoryCell> = Arc::new(PlainHistoryCell::new(
        (0..6)
            .map(|row| format!("recent row {row}").into())
            .collect(),
    ));
    let mut cells = vec![header, recent];
    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 30, /*height*/ 3,
    );
    let mut view = TranscriptView::default();
    view.history = TranscriptHistoryState::Partial;
    let mut before = Buffer::empty(area);
    view.render(area, &mut before, &cells);
    for _ in 0..2 {
        view.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            &cells,
        );
        view.render(area, &mut before, &cells);
    }
    assert!(view.needs_history(&cells));
    let page = (0..3).map(|row| {
        Arc::new(PlainHistoryCell::new(vec![
            format!("older row {row}").into(),
        ])) as Arc<dyn HistoryCell>
    });
    cells.splice(1..1, page);
    view.history_loaded(&cells, 1..4);
    assert!(
        !view.needs_history(&cells),
        "one wheel gesture must not keep requesting older pages"
    );
    let mut after = Buffer::empty(area);
    view.render(area, &mut after, &cells);
    assert_eq!(after, before);
}

#[test]
fn session_information_waits_for_older_pages_and_reappears_after_completion() {
    let header: Arc<dyn HistoryCell> = Arc::new(SessionInfoCell(CompositeHistoryCell {
        parts: vec![Box::new(PlainHistoryCell::new(vec![
            "session announcement".into(),
        ]))],
    }));
    let cells = vec![header];
    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 30, /*height*/ 2,
    );
    let mut view = TranscriptView::default();
    let mut output = Vec::new();
    for history in [
        TranscriptHistoryState::Complete,
        TranscriptHistoryState::Partial,
        TranscriptHistoryState::LoadingOlder,
        TranscriptHistoryState::LoadingBeginning,
        TranscriptHistoryState::Failed,
        TranscriptHistoryState::Complete,
    ] {
        view.history = history;
        let mut buffer = Buffer::empty(area);
        view.render(area, &mut buffer, &cells);
        output.push(
            buffer
                .content
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
                .trim()
                .to_owned(),
        );
    }
    assert_eq!(
        output,
        [
            "session announcement",
            "",
            "",
            "",
            "",
            "session announcement"
        ]
    );
}
