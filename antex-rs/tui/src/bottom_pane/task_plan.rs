use std::cell::Cell;
use std::ops::Range;

use antex_protocol::plan_tool::PlanItemArg;
use antex_protocol::plan_tool::StepStatus;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::render::renderable::Renderable;

const MAX_VISIBLE_TASKS: usize = 5;

#[derive(Debug, Eq, PartialEq)]
struct VisibleWindow {
    range: Range<usize>,
    hidden_before: bool,
    hidden_after: bool,
}

impl VisibleWindow {
    fn height(&self) -> usize {
        self.range.len() + usize::from(self.hidden_before) + usize::from(self.hidden_after)
    }
}

#[derive(Debug)]
pub(super) struct TaskPlan {
    active: bool,
    plan: Vec<PlanItemArg>,
    terminal_height: Cell<u16>,
}

impl Default for TaskPlan {
    fn default() -> Self {
        Self {
            active: false,
            plan: Vec::new(),
            terminal_height: Cell::new(u16::MAX),
        }
    }
}

impl TaskPlan {
    pub(super) fn begin(&mut self) {
        self.active = true;
        self.plan = vec![PlanItemArg {
            step: "Preparing task plan".to_string(),
            status: StepStatus::InProgress,
        }];
    }

    pub(super) fn set(&mut self, plan: Vec<PlanItemArg>) {
        if !self.active {
            return;
        }
        self.plan = plan;
    }

    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn active_plan(&self) -> Option<&[PlanItemArg]> {
        self.active.then_some(self.plan.as_slice())
    }

    pub(super) fn set_terminal_height(&self, height: u16) {
        self.terminal_height.set(height);
    }

    fn visible_capacity(&self) -> usize {
        let capacity = match self.terminal_height.get() {
            0..=10 => 1,
            11..=13 => 2,
            14..=17 => 3,
            _ => MAX_VISIBLE_TASKS,
        };
        capacity.min(self.plan.len())
    }

    fn visible_range(&self, capacity: usize) -> Range<usize> {
        if capacity >= self.plan.len() {
            return 0..self.plan.len();
        }

        let focus = self
            .plan
            .iter()
            .position(|item| matches!(item.status, StepStatus::InProgress))
            .or_else(|| {
                self.plan
                    .iter()
                    .position(|item| matches!(item.status, StepStatus::Pending))
            })
            .unwrap_or_else(|| self.plan.len().saturating_sub(1));
        let start = focus
            .saturating_sub(capacity / 2)
            .min(self.plan.len().saturating_sub(capacity));
        start..start + capacity
    }

    fn visible_window(&self, capacity: usize) -> VisibleWindow {
        let range = self.visible_range(capacity);
        VisibleWindow {
            hidden_before: range.start > 0,
            hidden_after: range.end < self.plan.len(),
            range,
        }
    }

    fn visible_window_for_height(&self, height: usize) -> Option<VisibleWindow> {
        let max_capacity = self.visible_capacity().min(height);
        for capacity in (1..=max_capacity).rev() {
            let window = self.visible_window(capacity);
            if window.height() <= height {
                return Some(window);
            }
        }
        (max_capacity > 0).then(|| VisibleWindow {
            range: self.visible_range(1),
            hidden_before: false,
            hidden_after: false,
        })
    }

    fn line(item: &PlanItemArg, width: u16) -> Line<'static> {
        let line: Line<'static> = match &item.status {
            StepStatus::Completed => vec!["  ✓ ".green(), item.step.clone().dim()].into(),
            StepStatus::InProgress => vec!["  ● ".cyan().bold(), item.step.clone().bold()].into(),
            StepStatus::Pending => vec!["  ○ ".dim(), item.step.clone().dim()].into(),
        };
        truncate_line_with_ellipsis_if_overflow(line, usize::from(width))
    }

    fn continuation_line() -> Line<'static> {
        "  …".dim().into()
    }
}

impl Renderable for TaskPlan {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let Some(window) = self.visible_window_for_height(usize::from(area.height)) else {
            return;
        };
        let mut lines = Vec::with_capacity(window.height());
        if window.hidden_before {
            lines.push(Self::continuation_line());
        }
        lines.extend(
            self.plan[window.range]
                .iter()
                .map(|item| Self::line(item, area.width)),
        );
        if window.hidden_after {
            lines.push(Self::continuation_line());
        }
        Paragraph::new(lines).render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        let capacity = self.visible_capacity();
        if capacity == 0 {
            return 0;
        }
        u16::try_from(self.visible_window(capacity).height()).unwrap_or(u16::MAX)
    }
}

#[cfg(test)]
#[path = "task_plan_tests.rs"]
mod tests;
