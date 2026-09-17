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

#[test]
fn animation_starts_on_first_display_and_survives_handoff() {
    let motion = StartupMascotMotion::new(FrameRequester::test_dummy());
    let first_display = Instant::now() + Duration::from_secs(5);
    assert_eq!(motion.current_frame_at(first_display), MascotFrame::Rest);
    let handed_off = motion.clone();
    assert_eq!(
        handed_off.current_frame_at(first_display + FRAME_TICK),
        MascotFrame::AntennaeWide
    );
    assert_eq!(
        motion.current_frame_at(first_display + FRAME_TICK * 7),
        MascotFrame::Blink
    );
    assert_eq!(
        handed_off.current_frame_at(first_display + FRAME_TICK * 8),
        MascotFrame::Rest
    );
}

#[test]
fn startup_mascot_animation_frames_snapshot() {
    let rendered = [StartupMascotSkin::Ant01, StartupMascotSkin::Ant03]
        .into_iter()
        .flat_map(|skin| {
            [
                MascotFrame::Rest,
                MascotFrame::AntennaeWide,
                MascotFrame::Blink,
            ]
            .into_iter()
            .map(move |frame| {
                format!(
                    "{skin:?} {frame:?}\n{}",
                    render_mascot(skin, frame)
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    insta::assert_snapshot!(rendered);
}
