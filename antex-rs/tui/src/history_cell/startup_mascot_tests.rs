use antex_config::types::StartupMascotSkin;
use pretty_assertions::assert_eq;

use super::*;

#[test]
fn animation_runs_once_and_settles_on_rest_frame() {
    let frames = (0..FRAME_SEQUENCE.len())
        .map(|index| {
            frame_at_elapsed(FRAME_TICK * u32::try_from(index).unwrap()).map(|(frame, _)| frame)
        })
        .collect::<Vec<_>>();
    assert_eq!(frames, FRAME_SEQUENCE.map(Some));
    assert_eq!(
        frame_at_elapsed(FRAME_TICK * u32::try_from(FRAME_SEQUENCE.len()).unwrap()),
        None
    );
}

#[test]
fn both_mascot_skins_render_within_the_fixed_canvas() {
    for skin in [StartupMascotSkin::Ant01, StartupMascotSkin::Ant03] {
        let lines = render_mascot(skin, MascotFrame::Rest);
        assert_eq!(lines.len(), 6);
        assert!(lines.iter().all(|line| line.width() <= MASCOT_WIDTH));
    }
}

#[test]
fn none_skin_renders_no_lines() {
    assert_eq!(
        render_mascot(StartupMascotSkin::None, MascotFrame::Rest),
        Vec::new()
    );
}
