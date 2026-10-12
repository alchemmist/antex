//! Backtrack selection preserves cached layouts and prompt rails in the shared viewport.

use super::*;
use crate::history_cell::UserHistoryCell;
use crate::keymap::RuntimeKeymap;
use pretty_assertions::assert_eq;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

#[derive(Debug)]
struct MeasuredCell {
    measurements: AtomicUsize,
}

impl HistoryCell for MeasuredCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.measurements.fetch_add(1, Ordering::Relaxed);
        vec!["history".into()]
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        vec!["history".into()]
    }
}

#[test]
fn moving_highlight_preserves_unaffected_height_caches() {
    let cells: Vec<_> = (0..4)
        .map(|_| {
            Arc::new(MeasuredCell {
                measurements: AtomicUsize::new(0),
            })
        })
        .collect();
    let mut overlay = TranscriptOverlay::new(
        cells
            .iter()
            .map(|cell| cell.clone() as Arc<dyn HistoryCell>)
            .collect(),
        RuntimeKeymap::defaults().pager,
    );
    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 40, /*height*/ 20,
    );
    overlay.render(area, &mut Buffer::empty(area));
    let measurements = || {
        cells
            .iter()
            .map(|cell| cell.measurements.load(Ordering::Relaxed))
            .collect::<Vec<_>>()
    };
    let before = measurements();
    for selection in [Some(3), Some(2), Some(2), None] {
        overlay.set_highlight_cell(selection);
        overlay.render(area, &mut Buffer::empty(area));
    }
    assert_eq!(measurements(), before);
}

#[test]
fn moving_highlight_matches_full_rebuild_with_live_tail() {
    let cells: Vec<Arc<dyn HistoryCell>> = ["first prompt", "second prompt"]
        .into_iter()
        .map(|message| {
            Arc::new(UserHistoryCell {
                message: message.to_string(),
                text_elements: Vec::new(),
                local_image_paths: Vec::new(),
                remote_image_urls: Vec::new(),
            }) as Arc<dyn HistoryCell>
        })
        .collect();
    let mut actual = TranscriptOverlay::new(cells.clone(), RuntimeKeymap::defaults().pager);
    let key = Some(ActiveCellTranscriptKey {
        cacheable: true,
        revision: 1,
        is_stream_continuation: false,
        animation_tick: None,
    });
    for width in [40, 24, 40] {
        let area = Rect::new(/*x*/ 0, /*y*/ 0, width, /*height*/ 20);
        for selection in [Some(0), Some(1), Some(1), None, Some(99), Some(0)] {
            let mut expected =
                TranscriptOverlay::new(cells.clone(), RuntimeKeymap::defaults().pager);
            for overlay in [&mut actual, &mut expected] {
                overlay
                    .sync_live_tail(width, key, |_| Some(vec![HyperlinkLine::from("live tail")]));
                overlay.set_highlight_cell(selection);
            }
            let mut actual_buffer = Buffer::empty(area);
            let mut expected_buffer = Buffer::empty(area);
            actual.render(area, &mut actual_buffer);
            expected.render(area, &mut expected_buffer);
            assert_eq!(actual_buffer, expected_buffer);
        }
    }
    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 40, /*height*/ 20,
    );
    let mut buffer = Buffer::empty(area);
    actual.render(area, &mut buffer);
    let content = (1..area.height - 4)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(content);
}
