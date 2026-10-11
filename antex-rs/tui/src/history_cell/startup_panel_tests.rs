use super::*;
use crate::history_cell::HistoryCell;
use crate::history_cell::SessionHeaderHistoryCell;

#[test]
fn startup_panel_displays_all_server_speed_tiers() {
    let frames = ["default", "priority", "ultrafast", "flex"]
        .into_iter()
        .map(|tier| {
            let cell = SessionHeaderHistoryCell::new(
                "Test model".to_string(),
                /*reasoning_effort*/ None,
                std::path::PathBuf::from("workspace"),
                "test",
            )
            .with_startup_panel(
                StartupPanelConfig {
                    show_commit: false,
                    show_feature_tip: false,
                    ..StartupPanelConfig::default()
                },
                /*context_window*/ None,
            )
            .with_service_tier(Some(tier.to_string()));
            format!(
                "{tier}:\n{}",
                ratatui::text::Text::from(cell.display_lines(/*width*/ 48))
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n");
    insta::assert_snapshot!(frames);
}
