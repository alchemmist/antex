use super::SESSION_HEADER_MAX_INNER_WIDTH;
use super::card_inner_width;
use super::startup_mascot::MASCOT_WIDTH;
use super::startup_mascot::MascotFrame;
use super::startup_mascot::render_mascot;
use crate::exec_command::relativize_to_home;
use crate::line_truncation::line_width;
use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::status::format_tokens_compact;
use crate::text_formatting::center_truncate_path;
use crate::width::display_width;
use antex_config::types::StartupPanelConfig;
use antex_config::types::StartupPanelStyle;
use antex_protocol::openai_models::ReasoningEffort;
use rand::Rng as _;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::RwLock;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StartupUpdates {
    pub(crate) fork: Option<VersionUpdate>,
    pub(crate) upstream: Option<VersionUpdate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VersionUpdate {
    pub(crate) current: String,
    pub(crate) latest: String,
}

impl VersionUpdate {
    #[cfg(any(not(debug_assertions), test))]
    pub(crate) fn new(current: impl Into<String>, latest: impl Into<String>) -> Self {
        Self {
            current: current.into(),
            latest: latest.into(),
        }
    }
}

static STARTUP_UPDATES: OnceLock<RwLock<StartupUpdates>> = OnceLock::new();

#[cfg(not(debug_assertions))]
pub(crate) fn set_startup_updates(updates: StartupUpdates) {
    let lock = STARTUP_UPDATES.get_or_init(|| RwLock::new(StartupUpdates::default()));
    *lock
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = updates;
}

fn startup_updates() -> StartupUpdates {
    STARTUP_UPDATES
        .get_or_init(|| RwLock::new(StartupUpdates::default()))
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

const FEATURE_TIPS: &[&str] = &[
    "Ctrl-S stashes your current prompt across restarts and restores it on the next press.",
    "/cd changes the working directory without restarting the session.",
    "/dump exports the conversation as a minimal HTML page.",
    "/context shows a compact summary of the active model context.",
    "/system-prompt opens the complete model request in Neovim.",
    "/agents shows what every background agent is working on.",
    "/subagents explicitly delegates the next prompt.",
    "/tmux-command-log mirrors shell activity into a dedicated tmux window.",
    "/todo opens the complete active checklist.",
    "Force pushes always require an explicit Yes or No.",
];

pub(super) struct StartupPanelView<'a> {
    pub config: &'a StartupPanelConfig,
    pub model: &'a str,
    pub model_style: Style,
    pub reasoning_effort: Option<&'a ReasoningEffort>,
    pub show_fast_status: bool,
    pub directory: &'a Path,
    pub yolo_mode: bool,
    pub context_window: Option<i64>,
    pub updates: Option<&'a StartupUpdates>,
    pub mascot_frame: MascotFrame,
}

impl StartupPanelView<'_> {
    pub(super) fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let panel = match self.config.style {
            StartupPanelStyle::Cockpit => self.framed_lines(width, FrameStyle::Cockpit),
            StartupPanelStyle::Hacker => self.framed_lines(width, FrameStyle::Hacker),
            StartupPanelStyle::Minimal => self.minimal_lines(width),
            StartupPanelStyle::Classic | StartupPanelStyle::Hidden => Vec::new(),
        };
        self.with_mascot(panel, width)
    }

    fn with_mascot(&self, panel: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
        let mascot = render_mascot(self.config.mascot_skin, self.mascot_frame);
        if panel.is_empty() || mascot.is_empty() {
            return panel;
        }
        const GAP: usize = 2;
        let panel_width = panel.iter().map(line_width).max().unwrap_or(0);
        if MASCOT_WIDTH + GAP + panel_width > usize::from(width) {
            return panel;
        }

        let height = mascot.len().max(panel.len());
        let mascot_top = height.saturating_sub(mascot.len()) / 2;
        let panel_top = height.saturating_sub(panel.len()) / 2;
        (0..height)
            .map(|row| {
                let mascot_line = mascot
                    .get(row.saturating_sub(mascot_top))
                    .filter(|_| row >= mascot_top);
                let panel_line = panel
                    .get(row.saturating_sub(panel_top))
                    .filter(|_| row >= panel_top);
                let mut spans = mascot_line
                    .map(|line| line.spans.clone())
                    .unwrap_or_default();
                match (mascot_line, panel_line) {
                    (Some(mascot_line), Some(line)) if line_width(line) > 0 => {
                        let mascot_width = line_width(mascot_line);
                        spans.push(" ".repeat(MASCOT_WIDTH - mascot_width + GAP).into());
                        spans.extend(line.spans.clone());
                    }
                    (None, Some(line)) if line_width(line) > 0 => {
                        spans.push(" ".repeat(MASCOT_WIDTH + GAP).into());
                        spans.extend(line.spans.clone());
                    }
                    (Some(_), Some(_)) | (Some(_), None) | (None, Some(_)) | (None, None) => {}
                }
                Line::from(spans)
            })
            .collect()
    }

    pub(super) fn raw_lines(&self) -> Vec<Line<'static>> {
        if self.config.style == StartupPanelStyle::Hidden {
            return Vec::new();
        }
        let mut lines = vec![Line::from(self.title())];
        lines.extend(self.content_lines(FrameStyle::Cockpit));
        lines
    }

    fn framed_lines(&self, width: u16, frame_style: FrameStyle) -> Vec<Line<'static>> {
        let Some(max_inner_width) = card_inner_width(width, SESSION_HEADER_MAX_INNER_WIDTH) else {
            return Vec::new();
        };
        let max_inner_width = max_inner_width.saturating_sub(2);
        let title = self.title();
        let content = self.content_lines(frame_style);
        let content_width = content
            .iter()
            .map(line_width)
            .chain(std::iter::once(display_width(&title).saturating_add(4)))
            .max()
            .unwrap_or(0)
            .min(max_inner_width);
        let title = truncate_title(&title, content_width.saturating_sub(4).max(1));
        let title_width = display_width(&title);
        let border_width = content_width.saturating_add(4);
        let remaining = border_width.saturating_sub(title_width.saturating_add(3));
        let (title_prefix, title_suffix) = match frame_style {
            FrameStyle::Cockpit => ("╭─ ", " "),
            FrameStyle::Hacker => ("╭─[", "]"),
        };
        let mut lines = vec![Line::from(vec![
            title_prefix.dim(),
            title.bold(),
            title_suffix.dim(),
            format!("{}╮", "─".repeat(remaining)).dim(),
        ])];
        for line in content {
            let line = truncate_line_with_ellipsis_if_overflow(line, content_width);
            let used_width = line_width(&line);
            let mut spans = vec!["│  ".dim()];
            spans.extend(line.spans);
            spans.push(" ".repeat(content_width.saturating_sub(used_width)).into());
            spans.push("  │".dim());
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(format!("╰{}╯", "─".repeat(border_width)).dim()));
        lines
    }

    fn minimal_lines(&self, width: u16) -> Vec<Line<'static>> {
        let max_width = usize::from(width.max(1));
        let mut lines = vec![truncate_line_with_ellipsis_if_overflow(
            Line::from(self.title()).bold(),
            max_width,
        )];
        lines.push(Line::from(""));
        lines.extend(
            self.content_lines(FrameStyle::Cockpit)
                .into_iter()
                .map(|line| truncate_line_with_ellipsis_if_overflow(line, max_width)),
        );
        lines
    }

    fn content_lines(&self, frame_style: FrameStyle) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if self.config.show_model {
            let mut spans = Vec::new();
            if frame_style == FrameStyle::Hacker {
                spans.push("model  ".green().dim());
            }
            spans.push(Span::styled(self.model.to_string(), self.model_style));
            if let Some(reasoning) = self.reasoning_effort {
                spans.extend([" · ".dim(), reasoning.as_str().to_string().into()]);
            }
            if self.show_fast_status {
                spans.extend([" · ".dim(), "fast".magenta()]);
            }
            lines.push(Line::from(spans));
        }
        if self.config.show_directory {
            let directory = format_directory(self.directory);
            let spans = if frame_style == FrameStyle::Hacker {
                vec!["cwd    ".green().dim(), directory.into()]
            } else {
                vec![directory.into()]
            };
            lines.push(Line::from(spans));
        }
        let mut status = Vec::new();
        if self.config.show_permissions && self.yolo_mode {
            status.push("YOLO".to_string());
        }
        if self.config.show_context
            && let Some(context_window) = self.context_window
        {
            status.push(format!("context {}", format_tokens_compact(context_window)));
        }
        if !status.is_empty() {
            let mut spans = Vec::new();
            if frame_style == FrameStyle::Hacker {
                spans.push("mode   ".green().dim());
            }
            spans.push(status.join(" · ").into());
            lines.push(Line::from(spans));
        }
        let updates = self.updates.cloned().unwrap_or_else(startup_updates);
        if let Some(update) = &updates.fork {
            lines.push(update_line("fork", update));
        }
        if let Some(update) = &updates.upstream {
            lines.push(update_line("upstream", update));
        }
        lines
    }

    fn title(&self) -> String {
        if !self.config.show_commit {
            return self.config.title.clone();
        }
        format!("{} · {}", self.config.title, build_commit())
    }
}

fn update_line(label: &str, update: &VersionUpdate) -> Line<'static> {
    Line::from(vec![
        "update · ".dim(),
        label.to_string().cyan(),
        " ".into(),
        update.current.clone().dim(),
        " → ".dim(),
        update.latest.clone().cyan().bold(),
    ])
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FrameStyle {
    Cockpit,
    Hacker,
}

pub(super) fn random_feature_tip(config: &StartupPanelConfig) -> Option<String> {
    if !config.show_feature_tip || config.style == StartupPanelStyle::Hidden {
        return None;
    }
    let custom_tips = config.feature_tips.as_ref().map(|tips| {
        tips.iter()
            .map(String::as_str)
            .filter(|tip| !tip.trim().is_empty())
            .collect::<Vec<_>>()
    });
    let tips = custom_tips
        .as_deref()
        .filter(|tips| !tips.is_empty())
        .unwrap_or(FEATURE_TIPS);
    tips.get(rand::rng().random_range(0..tips.len()))
        .map(|tip| (*tip).to_string())
}

fn build_commit() -> String {
    let commit = option_env!("STABLE_GIT_COMMIT").unwrap_or("unknown");
    let (hash, suffix) = commit
        .split_once('+')
        .map_or((commit, ""), |(hash, suffix)| (hash, suffix));
    let hash = hash.chars().take(8).collect::<String>();
    if suffix.is_empty() {
        hash
    } else {
        format!("{hash}+{suffix}")
    }
}

fn format_directory(directory: &Path) -> String {
    if let Some(relative) = relativize_to_home(directory) {
        if relative.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~{}{}", std::path::MAIN_SEPARATOR, relative.display())
        }
    } else {
        directory.display().to_string()
    }
}

fn truncate_title(title: &str, max_width: usize) -> String {
    if display_width(title) <= max_width {
        title.to_string()
    } else {
        center_truncate_path(title, max_width)
    }
}
