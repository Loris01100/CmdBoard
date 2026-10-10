use super::*;
use crate::core::goals::{GoalKind, Period, Status};
use crate::popup::GoalReached;
use crate::storage::models::GoalTarget;

fn goal_popup(app: &App) -> Option<&GoalReached> {
    match &app.mode {
        Mode::Popup(Popup::GoalReached(goal)) => Some(goal),
        _ => None,
    }
}

#[test]
fn goals_and_limits_are_set_listed_and_removed() {
    let mut app = App::with_defaults();
    run(&mut app, "limit");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Limite : — (usage : limit [<durée>/day|week|off] [app|catégorie])"
    );

    run(&mut app, "limit 2h/day jeux");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Enregistré : Limite · Jeux, 2h par jour"
    );
    run(&mut app, "goal 10h/week Steam");
    run(&mut app, "limit 3h30/day");
    let targets: Vec<_> = app.goals.iter().map(|g| (g.kind, g.target)).collect();
    let jeux = app.categories.iter().find(|c| c.name == "Jeux").unwrap().id;
    assert_eq!(
        targets,
        [
            (GoalKind::Limit, GoalTarget::Category(jeux)),
            (GoalKind::Goal, GoalTarget::App(steam_id(&app))),
            (GoalKind::Limit, GoalTarget::All),
        ]
    );
    run(&mut app, "limit");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Limite : Limite · Jeux : 0m / 2h aujourd'hui · Limite · toutes les apps : 0m / 3h30 aujourd'hui"
    );

    run(&mut app, "limit off Jeux");
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    run(&mut app, "limit off Jeux");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Rien à supprimer : Limite · Jeux"
    );
    run(&mut app, "goal 1h/day Nowhere");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "app ou catégorie inconnue : Nowhere"
    );
    assert_eq!(app.goals.len(), 2);
}

#[test]
fn time_counts_finished_and_running_sessions_of_the_target() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    run(&mut app, "goal 1h/week Jeux");
    run(&mut app, "limit 30m/day Steam");
    play(&mut app, steam, 20 * 60);
    app.on_session_start(steam);
    app.on_session_progress(steam, Duration::from_mins(5), false);

    let goal = app.goals[0].clone();
    let limit = app.goals[1].clone();
    let secs = app.goal_secs(&goal);
    assert!((25 * 60..25 * 60 + 5).contains(&secs), "{secs}");
    assert_eq!(app.goal_status(&goal), Status::Under);
    assert_eq!(app.goal_status(&limit), Status::Near); // 25 of 30 minutes
    assert_eq!(app.worst_limit().map(|(g, _)| g.id), Some(limit.id));
    let entry = app.find_app("Steam").unwrap().clone();
    assert_eq!(app.goals_for(&entry).count(), 2);
    let notepad = app.apps.iter().find(|a| a.id != steam).unwrap().clone();
    assert_eq!(app.goals_for(&notepad).count(), 0); // another category
}

#[test]
fn reaching_a_limit_opens_one_popup_per_period() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    run(&mut app, "limit 1m/day Steam");
    app.on_session_start(steam);
    app.on_session_progress(steam, Duration::from_secs(30), false);
    assert!(goal_popup(&app).is_none());

    app.on_session_progress(steam, Duration::from_secs(70), false);
    let popup = goal_popup(&app).unwrap();
    assert_eq!(
        (popup.kind, popup.target.as_str(), popup.period),
        (GoalKind::Limit, "Steam", Period::Day)
    );
    press(&mut app, KeyCode::Esc);
    app.on_session_progress(steam, Duration::from_secs(90), false);
    assert_eq!(app.mode, Mode::Normal); // already announced today
}

#[test]
fn a_target_already_reached_when_set_is_not_announced() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    play(&mut app, steam, 10 * 60);
    run(&mut app, "goal 5m/day Steam");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.goal_status(&app.goals[0]), Status::Reached);
    // Nor at the next start.
    app.check_goals(true);
    assert_eq!(app.mode, Mode::Normal);
}
