//! Goals and limits (`:goal`, `:limit`, plan section 11): the time played toward each,
//! running sessions included, and a popup the first time one is reached in a period.

use anyhow::Context;

use super::{App, MsgKind, Outcome};
use crate::command::{CommandHelp, GoalChange};
use crate::core::goals::{self, GoalKind, Period, Status};
use crate::popup::{GoalReached, Popup};
use crate::storage::models::{AppEntry, Goal, GoalTarget};

impl App {
    /// `:goal` or `:limit`: lists them, or sets or removes the one of `target`.
    pub(super) fn goal_command(
        &mut self,
        kind: GoalKind,
        change: Option<GoalChange>,
        target: Option<&str>,
    ) -> Outcome {
        let Some(change) = change else {
            return Ok(Some((self.list_goals(kind), MsgKind::Info)));
        };
        let target = self.resolve_goal_target(target)?;
        let (kind_name, target_name) = (kind_label(kind), self.goal_target_name(target));
        let message = match change {
            GoalChange::Set { minutes, period } => {
                self.db.set_goal(kind, target, minutes, period)?;
                let total = goals::format_hm(u64::from(minutes) * 60);
                let period = per_period(period);
                let text = t!(
                    "goals.set",
                    kind = kind_name,
                    target = target_name,
                    total,
                    period
                );
                (text, MsgKind::Success)
            }
            GoalChange::Remove if self.db.remove_goal(kind, target)? => {
                let text = t!("goals.removed", kind = kind_name, target = target_name);
                (text, MsgKind::Success)
            }
            GoalChange::Remove => {
                let text = t!("goals.not_set", kind = kind_name, target = target_name);
                (text, MsgKind::Info)
            }
        };
        self.reload()?;
        // Already reached when set: nothing to announce.
        self.check_goals(false);
        Ok(Some(message))
    }

    /// Every goal or limit of `kind`, with the time played toward it.
    fn list_goals(&self, kind: GoalKind) -> String {
        let items: Vec<String> = self
            .goals
            .iter()
            .filter(|g| g.kind == kind)
            .map(|g| self.goal_line(g))
            .collect();
        if items.is_empty() {
            let usage = crate::command::find_help(kind.code()).map(CommandHelp::usage);
            return t!(
                "goals.empty",
                kind = kind_label(kind),
                usage = usage.unwrap_or_default()
            );
        }
        t!(
            "goals.list",
            kind = kind_label(kind),
            items = items.join(" · ")
        )
    }

    /// An app first, then a category; `None` counts every app.
    fn resolve_goal_target(&self, name: Option<&str>) -> anyhow::Result<GoalTarget> {
        let Some(name) = name else {
            return Ok(GoalTarget::All);
        };
        if let Some(app) = self.find_app(name) {
            return Ok(GoalTarget::App(app.id));
        }
        let lower = name.to_lowercase();
        self.categories
            .iter()
            .find(|c| c.name.to_lowercase() == lower)
            .map(|c| GoalTarget::Category(c.id))
            .with_context(|| t!("error.unknown_target", name))
    }

    pub fn goal_target_name(&self, target: GoalTarget) -> String {
        let name = match target {
            GoalTarget::All => None,
            GoalTarget::App(id) => self.app_name(id),
            GoalTarget::Category(id) => self
                .categories
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone()),
        };
        name.unwrap_or_else(|| t!("goals.all"))
    }

    /// Seconds played toward `goal` in its current period, running sessions included.
    pub fn goal_secs(&self, goal: &Goal) -> u64 {
        self.apps
            .iter()
            .filter(|a| counts_for(goal.target, a))
            .map(|a| {
                let (today, week) = self.usage.by_app.get(&a.id).copied().unwrap_or_default();
                let finished = match goal.period {
                    Period::Day => today,
                    Period::Week => week,
                };
                let running = self
                    .active_sessions
                    .get(&a.id)
                    .map_or(0, super::sessions::ActiveSession::shown_secs);
                finished + running
            })
            .sum()
    }

    pub fn goal_status(&self, goal: &Goal) -> Status {
        goals::status(self.goal_secs(goal), u64::from(goal.minutes) * 60)
    }

    /// The goals and limits `app` counts toward: its own, its category's, every app's.
    pub fn goals_for<'a>(&'a self, app: &'a AppEntry) -> impl Iterator<Item = &'a Goal> {
        self.goals.iter().filter(|g| counts_for(g.target, app))
    }

    /// The limit nearest to or furthest past its target, once at 80 % of it.
    pub fn worst_limit(&self) -> Option<(&Goal, Status)> {
        self.goals
            .iter()
            .filter(|g| g.kind == GoalKind::Limit)
            .map(|g| {
                let ratio = self.goal_secs(g) * 1000 / (u64::from(g.minutes) * 60);
                (g, ratio)
            })
            .max_by_key(|(_, ratio)| *ratio)
            .map(|(g, _)| (g, self.goal_status(g)))
            .filter(|(_, status)| *status >= Status::Near)
    }

    /// "Limit · Games: 1h52 / 2h today".
    pub fn goal_line(&self, goal: &Goal) -> String {
        t!(
            "goals.line",
            kind = kind_label(goal.kind),
            target = self.goal_target_name(goal.target),
            used = goals::format_hm(self.goal_secs(goal)),
            total = goals::format_hm(u64::from(goal.minutes) * 60),
            period = this_period(goal.period)
        )
    }

    /// Reloads the time played this day and week: the day may have changed.
    pub(super) fn refresh_usage(&mut self) {
        match self.db.usage() {
            Ok(usage) => self.usage = usage,
            Err(e) => self.message = Some((format!("{e:#}"), MsgKind::Error)),
        }
    }

    /// Notes the goals and limits reached in their current period and, with
    /// `announce`, queues a popup for each one reached since the last check.
    pub(super) fn check_goals(&mut self, announce: bool) {
        let reached: Vec<(Goal, u64)> = self
            .goals
            .iter()
            .filter(|g| self.goal_status(g) == Status::Reached)
            .map(|g| (g.clone(), self.goal_secs(g)))
            .collect();
        for (goal, secs) in reached {
            let period = match goal.period {
                Period::Day => self.usage.day,
                Period::Week => self.usage.week,
            };
            if !self.goals_reached.insert((goal.id, period)) || !announce {
                continue;
            }
            self.pending_popups
                .push_back(Popup::GoalReached(GoalReached {
                    kind: goal.kind,
                    target: self.goal_target_name(goal.target),
                    secs,
                    minutes: goal.minutes,
                    period: goal.period,
                }));
        }
        self.show_pending_popup();
    }
}

fn counts_for(target: GoalTarget, app: &AppEntry) -> bool {
    match target {
        GoalTarget::All => true,
        GoalTarget::App(id) => app.id == id,
        GoalTarget::Category(id) => app.category_id == id,
    }
}

pub fn kind_label(kind: GoalKind) -> String {
    match kind {
        GoalKind::Goal => t!("goals.goal"),
        GoalKind::Limit => t!("goals.limit"),
    }
}

/// "today", "this week".
pub fn this_period(period: Period) -> String {
    match period {
        Period::Day => t!("goals.today"),
        Period::Week => t!("goals.this_week"),
    }
}

/// "per day", "per week".
fn per_period(period: Period) -> String {
    match period {
        Period::Day => t!("goals.per_day"),
        Period::Week => t!("goals.per_week"),
    }
}
