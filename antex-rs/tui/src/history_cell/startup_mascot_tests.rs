use antex_config::types::StartupMascotSkin;
use pretty_assertions::assert_eq;

use super::*;

#[test]
fn retained_animation_ticks_track_frames_and_finish_once() {
    let (requester, mut requests) = FrameRequester::test_channel();
    let motion = StartupMascotMotion::new(requester);
    let first_display = Instant::now() + Duration::from_secs(60);
    let handed_off = motion.clone();
    for (index, frame) in FRAME_SEQUENCE.iter().copied().enumerate() {
        let now = first_display + FRAME_TICK * u32::try_from(index).unwrap();
        let tick = handed_off.animation_tick_at(now);
        assert_eq!(tick, Some(frame as u64));
        assert!(
            requests.try_recv().is_ok(),
            "cache lookup must request the next frame"
        );
        // Reusing a cached frame must still leave an animation wakeup pending.
        assert_eq!(handed_off.animation_tick_at(now), tick);
        assert!(requests.try_recv().is_ok());
        assert_eq!(motion.current_frame_at(now), frame);
        assert!(requests.try_recv().is_ok());
    }
    let finished = first_display + FRAME_TICK * u32::try_from(FRAME_SEQUENCE.len()).unwrap();
    for now in [finished, finished + FRAME_TICK] {
        assert_eq!(handed_off.animation_tick_at(now), None);
        assert_eq!(motion.current_frame_at(now), MascotFrame::Rest);
        assert!(requests.try_recv().is_err());
    }
}

#[test]
fn retained_header_ticks_only_when_a_mascot_can_be_displayed() {
    use crate::history_cell::HistoryCell;
    use crate::history_cell::SessionHeaderHistoryCell;
    use antex_config::types::StartupPanelConfig;
    use antex_config::types::StartupPanelStyle;

    let motion = StartupMascotMotion::new(FrameRequester::test_dummy());
    // Keep the shared motion on its first frame without wall-clock sleeps.
    motion.current_frame_at(Instant::now() + Duration::from_secs(60));
    for (style, skin, expected) in [
        (
            StartupPanelStyle::Cockpit,
            StartupMascotSkin::Ant01,
            Some(0),
        ),
        (StartupPanelStyle::Hacker, StartupMascotSkin::Ant03, Some(0)),
        (
            StartupPanelStyle::Minimal,
            StartupMascotSkin::Ant01,
            Some(0),
        ),
        (StartupPanelStyle::Classic, StartupMascotSkin::Ant01, None),
        (StartupPanelStyle::Hidden, StartupMascotSkin::Ant01, None),
        (StartupPanelStyle::Cockpit, StartupMascotSkin::None, None),
    ] {
        let header = SessionHeaderHistoryCell::new(
            "model".into(),
            /*reasoning_effort*/ None,
            std::path::PathBuf::from("workspace"),
            "test",
        )
        .with_startup_panel(
            StartupPanelConfig {
                style,
                mascot_skin: skin,
                ..Default::default()
            },
            /*context_window*/ None,
        )
        .with_startup_mascot_animation(motion.clone());
        let source = header.raw_lines();
        assert_eq!(header.transcript_animation_tick(), expected);
        assert!(header.has_stable_transcript_height());
        header.display_lines(/*width*/ 100);
        assert_eq!(header.raw_lines(), source);
    }
}

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
fn handoff_during_last_antenna_pose_preserves_the_pending_transition() {
    let (requester, mut requests) = FrameRequester::test_channel();
    let motion = StartupMascotMotion::new(requester);
    let first_display = Instant::now();
    assert_eq!(motion.current_frame_at(first_display), MascotFrame::Rest);
    assert!(requests.try_recv().is_ok());

    // Model a clock starting at the diagnostic's first 267ms observation, then
    // its 1189ms antenna and 1203ms readiness. The actual clock may start BEFORE
    // that first observation; this tests the mid-pose case, not the runtime cause.
    let before_handoff = first_display + Duration::from_millis(/*millis*/ 922);
    assert_eq!(
        motion.current_frame_at(before_handoff),
        MascotFrame::AntennaeWide
    );
    assert!(requests.try_recv().is_ok());
    let handed_off = motion.clone();
    let ready = first_display + Duration::from_millis(/*millis*/ 936);
    for now in [before_handoff, ready] {
        assert_eq!(handed_off.current_frame_at(now), MascotFrame::AntennaeWide);
        assert!(requests.try_recv().is_ok());
    }
    assert!(Arc::ptr_eq(&motion.started_at, &handed_off.started_at));
    assert_eq!(handed_off.started_at.get(), Some(&first_display));

    // A shared handle still has the antenna-to-rest transition AFTER ready,
    // followed by the color-only blink and the final rest. It cannot settle at ready.
    let remaining = [6, 7, 8].map(|index| {
        let now = first_display + FRAME_TICK * index;
        let frame = handed_off.current_frame_at(now);
        let requested = requests.try_recv().is_ok();
        (frame, requested)
    });
    assert_eq!(
        remaining,
        [
            (MascotFrame::Rest, true),
            (MascotFrame::Blink, true),
            (MascotFrame::Rest, false),
        ]
    );
}

#[test]
fn handoff_on_last_rest_frame_still_has_visible_color_only_motion() {
    let (requester, mut requests) = FrameRequester::test_channel();
    let motion = StartupMascotMotion::new(requester);
    let first_display = Instant::now();
    motion.current_frame_at(first_display);
    assert!(requests.try_recv().is_ok());
    let handed_off = motion;
    let ready = first_display + FRAME_TICK * 6;
    let rest = handed_off.current_frame_at(ready);
    assert!(requests.try_recv().is_ok());
    let blink = handed_off.current_frame_at(ready + FRAME_TICK);
    assert!(requests.try_recv().is_ok());
    let settled = handed_off.current_frame_at(ready + FRAME_TICK * 2);
    assert!(requests.try_recv().is_err());
    assert_eq!(
        (rest, blink, settled),
        (MascotFrame::Rest, MascotFrame::Blink, MascotFrame::Rest)
    );
    assert_eq!(handed_off.started_at.get(), Some(&first_display));

    crate::terminal_palette::with_test_default_colors(
        crate::terminal_probe::DefaultColors {
            fg: (255, 255, 255),
            bg: (0, 0, 0),
        },
        || {
            for skin in [StartupMascotSkin::Ant01, StartupMascotSkin::Ant03] {
                let rest = render_mascot(skin, rest);
                let blink = render_mascot(skin, blink);
                // Text-only PTY observations lose this real visible transition.
                assert_eq!(
                    rest.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    blink.iter().map(ToString::to_string).collect::<Vec<_>>()
                );
                pretty_assertions::assert_ne!(rest, blink);
                assert_eq!(render_mascot(skin, settled), rest);
            }
        },
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
