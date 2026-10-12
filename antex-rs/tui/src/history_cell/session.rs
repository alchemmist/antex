//! Session headers, onboarding guidance, and transcript cards.

use super::*;
use crate::line_truncation::line_width;
use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::style::accent_color;
use crate::width::display_width;
use antex_config::types::StartupMascotSkin;
use antex_config::types::StartupPanelConfig;
use antex_config::types::StartupPanelStyle;

use super::startup_mascot::MascotFrame;
use super::startup_mascot::StartupMascotMotion;

/// Render `lines` inside a border whose inner width is at least `inner_width`.
///
/// This is useful when callers have already clamped their content to a
/// specific width and want the border math centralized here instead of
/// duplicating padding logic in the TUI widgets themselves.
pub(crate) fn with_border_with_inner_width(
    lines: Vec<Line<'static>>,
    inner_width: usize,
) -> Vec<Line<'static>> {
    let max_line_width = lines.iter().map(line_width).max().unwrap_or(0);
    let content_width = inner_width.max(max_line_width);

    let mut out = Vec::with_capacity(lines.len() + 2);
    let border_inner_width = content_width + 2;
    out.push(vec![format!("╭{}╮", "─".repeat(border_inner_width)).dim()].into());

    for line in lines.into_iter() {
        let used_width = line_width(&line);
        let span_count = line.spans.len();
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(span_count + 4);
        spans.push(Span::from("│ ").dim());
        spans.extend(line);
        if used_width < content_width {
            spans.push(Span::from(" ".repeat(content_width - used_width)).dim());
        }
        spans.push(Span::from(" │").dim());
        out.push(Line::from(spans));
    }

    out.push(vec![format!("╰{}╯", "─".repeat(border_inner_width)).dim()].into());

    out
}

/// Brand title shared by the session header and the status card; each owns its own indentation.
pub(crate) fn codex_title(version: &str) -> Vec<Span<'static>> {
    vec![
        ">_ ".fg(accent_color()),
        "Antex".bold(),
        format!(" (v{version})").dim(),
    ]
}

#[derive(Debug)]
struct TooltipHistoryCell {
    tip: String,
    cwd: PathBuf,
}

impl TooltipHistoryCell {
    fn new(tip: String, cwd: &Path) -> Self {
        Self {
            tip,
            cwd: cwd.to_path_buf(),
        }
    }
}

impl HistoryCell for TooltipHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }

    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        let indent = "  ";
        let indent_width = display_width(indent);
        let wrap_width = usize::from(width.max(1))
            .saturating_sub(indent_width)
            .max(1);
        let lines = crate::tooltips::render_tooltip_lines(&self.tip, wrap_width, &self.cwd);

        prefix_hyperlink_lines(lines, indent.into(), indent.into())
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        vec![Line::from(format!("Tip: {}", self.tip))]
    }
}

/// Startup metadata, including prior-session summaries and available usage resets.
#[derive(Debug)]
pub(crate) struct SessionNoticeCell(pub(crate) PlainHistoryCell);

impl HistoryCell for SessionNoticeCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.0.display_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.0.raw_lines()
    }
}

#[derive(Debug)]
struct FeatureTipHistoryCell {
    tip: String,
}

impl HistoryCell for FeatureTipHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let text = format!("✦ alchemmist feature: {}", self.tip);
        let wrap_width = usize::from(width.max(1)).saturating_sub(2).max(1);
        textwrap::wrap(&text, wrap_width)
            .into_iter()
            .map(|line| Line::from(vec!["  ".into(), line.into_owned().dim()]))
            .collect()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        vec![Line::from(format!("alchemmist feature: {}", self.tip))]
    }
}

#[derive(Debug)]
pub struct SessionInfoCell(CompositeHistoryCell);

impl SessionInfoCell {
    pub(crate) fn animate_startup(&mut self, motion: StartupMascotMotion) {
        if let Some(header) = self
            .0
            .parts
            .first_mut()
            .and_then(|part| part.as_any_mut().downcast_mut::<SessionHeaderHistoryCell>())
        {
            motion.trace_handoff("session_header_attachment");
            header.startup_mascot_motion = Some(motion);
            tracing::debug!(
                target: "antex_tui::startup_mascot",
                visible_motion = header.animated_startup_mascot().is_some(),
                "startup mascot attached to session header"
            );
        } else {
            tracing::debug!(
                target: "antex_tui::startup_mascot",
                "startup mascot attachment missing session header part"
            );
        }
    }
}

/// Fullscreen transcript presentation omits tips; scrollback retains the original cells.
pub(crate) fn fullscreen_session_lines(
    cell: &dyn HistoryCell,
    width: u16,
    detailed: bool,
    mode: HistoryRenderMode,
) -> Vec<HyperlinkLine> {
    if let Some(info) = cell.as_any().downcast_ref::<SessionInfoCell>() {
        let mut lines = Vec::new();
        for part in &info.0.parts {
            if part.as_any().is::<TooltipHistoryCell>() {
                continue;
            }
            let part_lines = fullscreen_session_lines(part.as_ref(), width, detailed, mode);
            if !part_lines.is_empty() {
                if !lines.is_empty() {
                    lines.push(HyperlinkLine::from(""));
                }
                lines.extend(part_lines);
            }
        }
        lines
    } else if detailed || mode == HistoryRenderMode::Rich {
        cell.retained_hyperlink_lines(width, detailed)
    } else {
        cell.display_hyperlink_lines_for_mode(width, mode)
    }
}

impl HistoryCell for SessionInfoCell {
    fn transcript_animation_tick(&self) -> Option<u64> {
        // The source-backed header owns startup motion; tips and notices remain stable.
        self.0
            .parts
            .first()
            .and_then(|header| header.transcript_animation_tick())
    }

    fn compact_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.0.compact_hyperlink_lines(width)
    }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.0.display_lines(width)
    }

    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.0.display_hyperlink_lines(width)
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.0.desired_height(width)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.0.transcript_lines(width)
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.0.transcript_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.0.raw_lines()
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "keep local preferences separate while the legacy Config parameter is still required"
)]
pub(crate) fn new_session_info(
    config: &Config,
    local_settings: &crate::local_settings::LocalSettings,
    requested_model: &str,
    model_display_name: &str,
    session: &ThreadSessionState,
    is_first_event: bool,
    tooltip_override: Option<String>,
    auth_plan: Option<PlanType>,
) -> SessionInfoCell {
    // Header rendered as history (so it appears at the very top).
    let header = SessionHeaderHistoryCell::new(
        model_display_name.to_string(),
        session.reasoning_effort.clone(),
        config.cwd.to_path_buf(),
        ANTEX_CLI_VERSION,
    )
    .with_startup_panel(
        local_settings.tui.startup_panel.clone(),
        config.model_context_window,
    )
    .with_service_tier(session.service_tier.clone())
    .with_yolo_mode(has_yolo_permissions(
        session.approval_policy,
        &session.permission_profile,
    ));
    let mut parts: Vec<Box<dyn HistoryCell>> = vec![Box::new(header)];

    if let Some(tip) = super::startup_panel::random_feature_tip(&local_settings.tui.startup_panel) {
        parts.push(Box::new(FeatureTipHistoryCell { tip }));
    }

    if is_first_event {
        // Help lines below the header (new copy and list)
        let help_lines: Vec<Line<'static>> = vec![
            "  To get started, describe a task or try one of these commands:"
                .dim()
                .into(),
            Line::from(""),
            Line::from(vec![
                "  ".into(),
                "/init".into(),
                " - create an AGENTS.md file with instructions for Antex".dim(),
            ]),
            Line::from(vec![
                "  ".into(),
                "/status".into(),
                " - show current session configuration".dim(),
            ]),
            Line::from(vec![
                "  ".into(),
                "/permissions".into(),
                " - choose what Antex is allowed to do".dim(),
            ]),
            Line::from(vec![
                "  ".into(),
                "/model".into(),
                " - choose what model and reasoning effort to use".dim(),
            ]),
            Line::from(vec![
                "  ".into(),
                "/review".into(),
                " - review any changes and find issues".dim(),
            ]),
        ];

        parts.push(Box::new(PlainHistoryCell { lines: help_lines }));
    } else {
        if !local_settings.tui.startup_panel.show_feature_tip
            && local_settings.tui.show_tooltips
            && let Some(tooltips) = tooltip_override
                .or_else(|| tooltips::get_tooltip(auth_plan, &local_settings.tui.keymap))
                .map(|tip| TooltipHistoryCell::new(tip, &config.cwd))
        {
            parts.push(Box::new(tooltips));
        }
        if requested_model != session.model.as_str() {
            let lines = vec![
                "model changed:".magenta().bold().into(),
                format!("requested: {requested_model}").into(),
                format!("used: {}", session.model).into(),
            ];
            parts.push(Box::new(PlainHistoryCell { lines }));
        }
    }

    SessionInfoCell(CompositeHistoryCell { parts })
}

pub(crate) fn is_yolo_mode(config: &Config) -> bool {
    has_yolo_permissions(
        AskForApproval::from(config.permissions.approval_policy.value()),
        &config.permissions.effective_permission_profile(),
    )
}

pub(crate) fn has_yolo_permissions(
    approval_policy: AskForApproval,
    permission_profile: &PermissionProfile,
) -> bool {
    approval_policy == AskForApproval::Never
        && matches!(
            permission_profile,
            PermissionProfile::Disabled
                | PermissionProfile::Managed {
                    file_system: ManagedFileSystemPermissions::Unrestricted,
                    network: NetworkSandboxPolicy::Enabled,
                }
        )
}
/// Session banner with a model label already resolved for presentation by its caller.
#[derive(Debug)]
pub(crate) struct SessionHeaderHistoryCell {
    version: &'static str,
    model: String,
    reasoning_effort: Option<ReasoningEffortConfig>,
    service_tier: Option<String>,
    directory: PathBuf,
    yolo_mode: bool,
    startup_panel: Option<StartupPanelConfig>,
    context_window: Option<i64>,
    startup_updates: Option<super::startup_panel::StartupUpdates>,
    startup_mascot_motion: Option<StartupMascotMotion>,
}

impl SessionHeaderHistoryCell {
    pub(crate) fn new(
        model: String,
        reasoning_effort: Option<ReasoningEffortConfig>,
        directory: PathBuf,
        version: &'static str,
    ) -> Self {
        Self {
            version,
            model,
            reasoning_effort,
            service_tier: None,
            directory,
            yolo_mode: false,
            startup_panel: None,
            context_window: None,
            startup_updates: None,
            startup_mascot_motion: None,
        }
    }

    pub(crate) fn with_startup_panel(
        mut self,
        startup_panel: StartupPanelConfig,
        context_window: Option<i64>,
    ) -> Self {
        self.startup_panel = Some(startup_panel);
        self.context_window = context_window;
        self
    }

    pub(crate) fn with_service_tier(mut self, service_tier: Option<String>) -> Self {
        self.service_tier = service_tier;
        self
    }

    pub(crate) fn with_yolo_mode(mut self, yolo_mode: bool) -> Self {
        self.yolo_mode = yolo_mode;
        self
    }

    pub(crate) fn with_startup_mascot_animation(mut self, motion: StartupMascotMotion) -> Self {
        self.startup_mascot_motion = Some(motion);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_startup_updates(
        mut self,
        startup_updates: super::startup_panel::StartupUpdates,
    ) -> Self {
        self.startup_updates = Some(startup_updates);
        self
    }

    fn format_directory(&self, max_width: Option<usize>) -> String {
        Self::format_directory_inner(&self.directory, max_width)
    }

    pub(crate) fn format_directory_inner(directory: &Path, max_width: Option<usize>) -> String {
        let formatted = if let Some(rel) = relativize_to_home(directory) {
            if rel.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~{}{}", std::path::MAIN_SEPARATOR, rel.display())
            }
        } else {
            directory.display().to_string()
        };

        if let Some(max_width) = max_width {
            if max_width == 0 {
                return String::new();
            }
            if display_width(formatted.as_str()) > max_width {
                return crate::text_formatting::center_truncate_path(&formatted, max_width);
            }
        }

        formatted
    }

    fn animated_startup_mascot(&self) -> Option<&StartupMascotMotion> {
        let panel = self.startup_panel.as_ref()?;
        if panel.mascot_skin != StartupMascotSkin::None
            && matches!(
                panel.style,
                StartupPanelStyle::Cockpit | StartupPanelStyle::Hacker | StartupPanelStyle::Minimal
            )
        {
            self.startup_mascot_motion.as_ref()
        } else {
            None
        }
    }

    fn reasoning_label(&self) -> Option<&str> {
        self.reasoning_effort
            .as_ref()
            .map(ReasoningEffortConfig::as_str)
    }
}

impl HistoryCell for SessionHeaderHistoryCell {
    fn transcript_animation_tick(&self) -> Option<u64> {
        self.animated_startup_mascot()
            .and_then(StartupMascotMotion::transcript_animation_tick)
    }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        if let Some(startup_panel) = self.startup_panel.as_ref()
            && startup_panel.style != StartupPanelStyle::Classic
        {
            return super::startup_panel::StartupPanelView {
                config: startup_panel,
                model: &self.model,
                model_style: Style::default(),
                reasoning_effort: self.reasoning_effort.as_ref(),
                service_tier: self.service_tier.as_deref(),
                directory: &self.directory,
                yolo_mode: self.yolo_mode,
                context_window: self.context_window,
                updates: self.startup_updates.as_ref(),
                mascot_frame: self
                    .animated_startup_mascot()
                    .map(StartupMascotMotion::current_frame)
                    .unwrap_or(MascotFrame::Rest),
            }
            .display_lines(width);
        }
        let width = usize::from(width);
        let mut title = vec!["  ".into()];
        title.extend(codex_title(self.version));
        let mut lines = vec![
            Line::default(),
            Line::from(title),
            Line::from(vec![
                "     ".into(),
                self.format_directory(Some(width.saturating_sub(/*rhs*/ 5)))
                    .dim(),
            ]),
        ];
        if self.yolo_mode {
            lines.push(Line::from(vec![
                "  permissions: ".dim(),
                "YOLO mode".magenta().bold(),
            ]));
        }
        lines
            .into_iter()
            .map(|line| truncate_line_with_ellipsis_if_overflow(line, width))
            .collect()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        if let Some(startup_panel) = self.startup_panel.as_ref()
            && startup_panel.style != StartupPanelStyle::Classic
        {
            return super::startup_panel::StartupPanelView {
                config: startup_panel,
                model: &self.model,
                model_style: Style::default(),
                reasoning_effort: self.reasoning_effort.as_ref(),
                service_tier: self.service_tier.as_deref(),
                directory: &self.directory,
                yolo_mode: self.yolo_mode,
                context_window: self.context_window,
                updates: self.startup_updates.as_ref(),
                mascot_frame: MascotFrame::Rest,
            }
            .raw_lines();
        }
        let mut lines = vec![
            Line::from(format!("Antex (v{})", self.version)),
            Line::from(format!(
                "model: {}{}",
                self.model,
                self.reasoning_label()
                    .map(|reasoning| format!(" {reasoning}"))
                    .unwrap_or_default()
            )),
            Line::from(format!(
                "directory: {}",
                self.format_directory(/*max_width*/ None)
            )),
        ];
        if self.yolo_mode {
            lines.push(Line::from("permissions: YOLO mode"));
        }
        lines
    }
}

#[cfg(test)]
#[path = "session_transcript_tests.rs"]
mod transcript_tests;
