//! Reward rules: a small condition language, evaluated against the facts of a finished
//! session. Rules are data (`rewards.rule`), never hardcoded.
//!
//! Grammar: `condition (("&&" | "||") condition)*`, where `condition` is
//! `variable operator number` and `&&` binds tighter than `||`.
//! Example: `session_minutes >= 180 || app_hours >= 10`.

/// What a rule can test, measured once the session is closed (so it counts).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    pub session_minutes: f64,
    /// Local hour the session started, 0 to 23.
    pub session_hour: f64,
    pub app_hours: f64,
    pub app_sessions: f64,
    pub app_level: f64,
    /// Global level.
    pub level: f64,
    pub streak_days: f64,
    pub total_hours: f64,
    pub total_sessions: f64,
    /// Different apps played during the last 7 days.
    pub apps_this_week: f64,
}

/// Every variable a rule can use, with what it measures.
pub const VARIABLES: &[(&str, &str)] = &[
    ("session_minutes", "durée de la session, en minutes"),
    ("session_hour", "heure locale de début de la session (0-23)"),
    ("app_hours", "temps total sur l'app, en heures"),
    ("app_sessions", "nombre de sessions sur l'app"),
    ("app_level", "niveau de l'app"),
    ("level", "niveau global"),
    ("streak_days", "jours actifs consécutifs"),
    ("total_hours", "temps total, toutes apps, en heures"),
    ("total_sessions", "nombre total de sessions"),
    ("apps_this_week", "apps différentes jouées sur 7 jours"),
];

impl Facts {
    pub fn get(&self, name: &str) -> Option<f64> {
        Some(match name {
            "session_minutes" => self.session_minutes,
            "session_hour" => self.session_hour,
            "app_hours" => self.app_hours,
            "app_sessions" => self.app_sessions,
            "app_level" => self.app_level,
            "level" => self.level,
            "streak_days" => self.streak_days,
            "total_hours" => self.total_hours,
            "total_sessions" => self.total_sessions,
            "apps_this_week" => self.apps_this_week,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
    Ne,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Var(String),
    Num(f64),
    Cmp(Op),
    And,
    Or,
}

/// Whether `rule` passes for `facts`. Every condition is checked, so a broken rule is
/// reported even when the result is already known.
pub fn evaluate(rule: &str, facts: &Facts) -> Result<bool, String> {
    let tokens = tokenize(rule)?;
    if tokens.is_empty() {
        return Err(t!("rule.empty"));
    }
    let mut any = false;
    for group in tokens.split(|t| *t == Token::Or) {
        let mut all = true;
        for condition in group.split(|t| *t == Token::And) {
            all &= check(condition, facts)?;
        }
        any |= all;
    }
    Ok(any)
}

#[expect(
    clippy::float_cmp,
    reason = "a rule like `level == 10` asks for that exact value"
)]
fn check(condition: &[Token], facts: &Facts) -> Result<bool, String> {
    let [Token::Var(name), Token::Cmp(op), Token::Num(value)] = condition else {
        return Err(t!("rule.condition_expected"));
    };
    let actual = facts.get(name).ok_or_else(|| {
        let known: Vec<_> = VARIABLES.iter().map(|(name, _)| *name).collect();
        t!("rule.unknown_variable", name, list = known.join(", "))
    })?;
    Ok(match op {
        Op::Ge => actual >= *value,
        Op::Gt => actual > *value,
        Op::Le => actual <= *value,
        Op::Lt => actual < *value,
        Op::Eq => actual == *value,
        Op::Ne => actual != *value,
    })
}

/// How far the facts are from passing a rule, for a locked reward: `2h10 / 3h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub current: u64,
    /// Above zero.
    pub target: u64,
    pub unit: Unit,
}

/// What `Progress` counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Play time, in seconds.
    Seconds,
    /// Sessions, levels, days or apps.
    Count,
}

impl Progress {
    /// 0 to 100, reached targets included.
    pub fn percent(&self) -> u64 {
        self.current.min(self.target) * 100 / self.target
    }
}

/// Progress toward `rule`, for the conditions that count up to a target (`>=`, `>`):
/// the furthest of its `||` alternatives, each as far as its least advanced `&&`
/// condition. An alternative with any other condition (`session_hour < 5`, `==`) has
/// none, nor has a broken rule.
pub fn progress(rule: &str, facts: &Facts) -> Option<Progress> {
    evaluate(rule, facts).ok()?; // broken: no progress either
    let tokens = tokenize(rule).ok()?;
    tokens
        .split(|t| *t == Token::Or)
        .filter_map(|group| {
            group
                .split(|t| *t == Token::And)
                .map(|condition| condition_progress(condition, facts))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .min_by_key(Progress::percent)
        })
        .max_by_key(Progress::percent)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "facts and targets are small and positive; a negative one saturates at 0"
)]
fn condition_progress(condition: &[Token], facts: &Facts) -> Option<Progress> {
    let [
        Token::Var(name),
        Token::Cmp(op @ (Op::Ge | Op::Gt)),
        Token::Num(value),
    ] = condition
    else {
        return None;
    };
    // An hour of the day is not something to build up.
    if name == "session_hour" {
        return None;
    }
    let actual = facts.get(name)?;
    let (unit, scale) = match name.as_str() {
        "session_minutes" => (Unit::Seconds, 60.0),
        "app_hours" | "total_hours" => (Unit::Seconds, 3600.0),
        _ => (Unit::Count, 1.0),
    };
    let target = match (unit, op) {
        // `total_sessions > 2` takes a third one.
        (Unit::Count, Op::Gt) => value.floor() + 1.0,
        (Unit::Count, _) => value.ceil(),
        _ => value * scale,
    } as u64;
    (target > 0).then_some(Progress {
        current: (actual * scale) as u64,
        target,
        unit,
    })
}

fn tokenize(rule: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = rule.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Var(chars[start..i].iter().collect()));
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let value = text.parse().map_err(|_| t!("rule.bad_number", text))?;
            tokens.push(Token::Num(value));
            continue;
        }
        let (token, len) = match (c, next) {
            ('>', Some('=')) => (Token::Cmp(Op::Ge), 2),
            ('<', Some('=')) => (Token::Cmp(Op::Le), 2),
            ('=', Some('=')) => (Token::Cmp(Op::Eq), 2),
            ('!', Some('=')) => (Token::Cmp(Op::Ne), 2),
            ('&', Some('&')) => (Token::And, 2),
            ('|', Some('|')) => (Token::Or, 2),
            ('>', _) => (Token::Cmp(Op::Gt), 1),
            ('<', _) => (Token::Cmp(Op::Lt), 1),
            _ => return Err(t!("rule.unexpected_char", char = c)),
        };
        tokens.push(token);
        i += len;
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_counts_up_to_the_target() {
        let time = |current, target| {
            Some(Progress {
                current,
                target,
                unit: Unit::Seconds,
            })
        };
        let count = |current, target| {
            Some(Progress {
                current,
                target,
                unit: Unit::Count,
            })
        };
        let cases = [
            ("session_minutes >= 180", time(200 * 60, 180 * 60)),
            ("app_hours >= 50", time(45_000, 180_000)),
            ("level >= 10", count(3, 10)),
            ("total_sessions > 2", count(0, 3)),
            ("level >= 2.5", count(3, 3)),
            // Least advanced condition of an `&&`, furthest alternative of an `||`.
            ("level >= 10 && app_hours >= 25", count(3, 10)),
            ("level >= 30 || app_hours >= 25", time(45_000, 90_000)),
            // Unmeasurable: a time of day, another operator, an always-true target.
            ("session_hour < 5", None),
            ("session_hour >= 22", None),
            ("level == 10", None),
            ("level >= 6 && session_hour < 5", None),
            ("total_sessions >= 0", None),
            ("level >= 10 ||", None),
            ("nope >= 1", None),
        ];
        for (rule, expected) in cases {
            assert_eq!(progress(rule, &facts()), expected, "rule: {rule}");
        }
        assert_eq!(count(3, 10).unwrap().percent(), 30);
        assert_eq!(count(12, 10).unwrap().percent(), 100);
    }

    fn facts() -> Facts {
        Facts {
            session_minutes: 200.0,
            session_hour: 2.0,
            app_hours: 12.5,
            level: 3.0,
            ..Facts::default()
        }
    }

    #[test]
    fn evaluates_rules() {
        let cases = [
            ("session_minutes >= 180", true),
            ("session_minutes>=201", false),
            ("app_hours > 12.5", false),
            ("app_hours >= 12.5", true),
            ("session_hour < 5 && level == 3", true),
            ("session_hour < 5 && level != 3", false),
            ("level > 10 || app_hours >= 10", true),
            // && binds tighter: false || (true && true)
            ("level > 10 || session_hour < 5 && app_hours > 1", true),
            ("streak_days >= 1", false),
            ("level < 4", true),
            ("level <= 3", true),
        ];
        for (rule, expected) in cases {
            assert_eq!(evaluate(rule, &facts()), Ok(expected), "rule: {rule}");
        }
    }

    #[test]
    fn rejects_broken_rules() {
        let cases = [
            ("", "règle vide"),
            ("   ", "règle vide"),
            (
                "hours >= 3",
                "variable inconnue : hours (connues : session_minutes,",
            ),
            ("level >= 1 || hours >= 3", "variable inconnue : hours"), // checked even if true
            (
                "level 3",
                "condition attendue : <variable> <opérateur> <nombre>",
            ),
            (
                "3 <= level",
                "condition attendue : <variable> <opérateur> <nombre>",
            ),
            (
                "level >= 1 &&",
                "condition attendue : <variable> <opérateur> <nombre>",
            ),
            ("level = 3", "caractère inattendu : ="),
            ("level >= 1.2.3", "nombre invalide : 1.2.3"),
        ];
        for (rule, expected) in cases {
            let error = evaluate(rule, &facts()).unwrap_err();
            assert!(error.starts_with(expected), "rule: {rule}, error: {error}");
        }
    }

    #[test]
    fn every_variable_is_readable() {
        for (name, _) in VARIABLES {
            assert!(Facts::default().get(name).is_some(), "{name}");
        }
    }

    proptest::proptest! {
        /// A rule typed by the user is evaluated or rejected, never a panic.
        #[test]
        fn any_rule_text_is_handled(rule in r"[a-z_ 0-9.<>=!&|()]{0,40}|\PC{0,40}") {
            let _ = evaluate(&rule, &facts());
        }

        /// `x >= n` and `x < n` always disagree. Rules have no negative numbers: facts are >= 0.
        #[test]
        fn opposite_conditions_disagree(n in 0f64..1e6) {
            let ge = evaluate(&format!("level >= {n}"), &facts()).unwrap();
            let lt = evaluate(&format!("level < {n}"), &facts()).unwrap();
            proptest::prop_assert_ne!(ge, lt);
        }
    }
}
