//! Popup state: confirmations, forms, level-ups and unlocked rewards. Rendering lives in `ui/widgets/popup.rs`.

use crate::command::Command;
use crate::launcher::launch;
use crate::text_input::TextInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    /// Asks before running a destructive `command`.
    Confirm {
        message: String,
        command: Command,
    },
    /// First step of "add app": choose among the installed apps.
    Picker(Picker),
    Form(Form),
    LevelUp(LevelUp),
    RewardUnlocked(RewardUnlocked),
}

/// A reward a session just unlocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardUnlocked {
    pub name: String,
    pub description: String,
    /// App it was unlocked for, `None` for a global reward.
    pub app: Option<String>,
}

/// An app, the global profile, or both reached a new level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelUp {
    pub app: String,
    /// New level of the app, if it went up.
    pub app_level: Option<u32>,
    /// New global level, if it went up.
    pub global_level: Option<u32>,
    /// XP that caused it.
    pub gained: u32,
}

/// Filterable list of installed apps (`App::picker_matches`). Choosing one opens the
/// add form pre-filled; Tab opens it empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub query: TextInput,
    /// Index into the current matches.
    pub selected: usize,
    /// Category to pre-fill in the form.
    pub category: String,
}

impl Picker {
    pub fn new(category: &str) -> Self {
        Self {
            query: TextInput::default(),
            selected: 0,
            category: category.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormKind {
    AddApp,
    MoveApp { app: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub input: TextInput,
    pub required: bool,
}

impl Field {
    fn new(label: &'static str, value: &str, required: bool) -> Self {
        Self {
            label,
            input: TextInput::new(value),
            required,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub focused: usize,
    /// Validation or execution error, shown inside the form.
    pub error: Option<String>,
}

// Field order of the "add app" form.
const NAME: usize = 0;
const TARGET: usize = 1;
const CATEGORY: usize = 2;
const PROCESS: usize = 3;

impl Form {
    /// Add-app form; `category` pre-fills the category field.
    pub fn add_app(category: &str) -> Self {
        Self::new(
            FormKind::AddApp,
            vec![
                Field::new("Nom", "", true),
                Field::new("Cible", "", true),
                Field::new("Catégorie", category, true),
                Field::new("Process", "", false),
            ],
        )
    }

    /// Pre-filled add-app form, focused on the first empty field (the category when
    /// everything is filled: the one choice left).
    pub fn add_app_from(category: &str, name: &str, target: &str, watch_exe: Option<&str>) -> Self {
        let mut form = Self::add_app(category);
        form.fields[NAME].input.set(name);
        form.fields[TARGET].input.set(target);
        form.fields[PROCESS].input.set(watch_exe.unwrap_or(""));
        form.focused = [NAME, TARGET, CATEGORY]
            .into_iter()
            .find(|&i| form.fields[i].input.is_empty())
            .unwrap_or(CATEGORY);
        form
    }

    /// Move form for `app`, pre-filled with its current category.
    pub fn move_app(app: &str, category: &str) -> Self {
        Self::new(
            FormKind::MoveApp {
                app: app.to_string(),
            },
            vec![Field::new("Catégorie", category, true)],
        )
    }

    fn new(kind: FormKind, fields: Vec<Field>) -> Self {
        Self {
            kind,
            fields,
            focused: 0,
            error: None,
        }
    }

    pub fn title(&self) -> String {
        match &self.kind {
            FormKind::AddApp => "Ajouter une app".into(),
            FormKind::MoveApp { app } => format!("Déplacer « {app} »"),
        }
    }

    /// Greyed text shown in an empty field.
    pub fn placeholder(&self, index: usize) -> Option<String> {
        match (&self.kind, index) {
            (FormKind::AddApp, TARGET) => Some(r"C:\…\jeu.exe, notepad.exe ou steam://…".into()),
            (FormKind::AddApp, PROCESS) => {
                let target = self.fields[TARGET].input.text();
                Some(match launch::watch_exe_for(target) {
                    Some(exe) => format!("auto : {exe}"),
                    None => "exe à surveiller, ex. Hades.exe".into(),
                })
            }
            _ => None,
        }
    }

    /// What the focused field is for, shown under the fields.
    pub fn help(&self) -> Option<&'static str> {
        match (&self.kind, self.focused) {
            (FormKind::AddApp, NAME) => Some("Nom affiché dans CmdBoard."),
            (FormKind::AddApp, TARGET) => Some(
                "Ce qui est lancé : un .exe ou un raccourci .lnk, un nom sur le PATH, ou un lien steam://…",
            ),
            (FormKind::AddApp, CATEGORY) | (FormKind::MoveApp { .. }, _) => {
                Some("Une catégorie existante, ou une nouvelle qui sera créée.")
            }
            (FormKind::AddApp, PROCESS) => Some(
                "Exe surveillé pour compter le temps de jeu (XP). Vide : déduit de la cible. Pour un lien steam://…, l'exe du jeu (ex. Hades.exe).",
            ),
            _ => None,
        }
    }

    pub fn focused_input(&mut self) -> &mut TextInput {
        &mut self.fields[self.focused].input
    }

    pub fn is_last_field(&self) -> bool {
        self.focused + 1 == self.fields.len()
    }

    pub fn next_field(&mut self) {
        self.focused = (self.focused + 1) % self.fields.len();
    }

    pub fn prev_field(&mut self) {
        self.focused = (self.focused + self.fields.len() - 1) % self.fields.len();
    }

    /// Builds the command, or focuses the first empty required field.
    pub fn to_command(&mut self) -> Result<Command, String> {
        if let Some(i) = self
            .fields
            .iter()
            .position(|f| f.required && f.input.text().trim().is_empty())
        {
            self.focused = i;
            return Err(format!("{} : champ requis", self.fields[i].label));
        }
        let value = |i: usize| self.fields[i].input.text().trim().to_string();
        Ok(match &self.kind {
            FormKind::AddApp => Command::Add {
                name: value(NAME),
                target: value(TARGET),
                category: Some(value(CATEGORY)),
                watch_exe: Some(value(PROCESS)).filter(|p| !p.is_empty()),
            },
            FormKind::MoveApp { app } => Command::Move {
                app: app.clone(),
                category: value(0),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(form: &mut Form, index: usize, text: &str) {
        form.fields[index].input.set(text);
    }

    #[test]
    fn add_form_builds_command() {
        let mut form = Form::add_app("Jeux");
        fill(&mut form, NAME, " Hades ");
        fill(&mut form, TARGET, "steam://rungameid/1145360");
        assert_eq!(
            form.to_command(),
            Ok(Command::Add {
                name: "Hades".into(),
                target: "steam://rungameid/1145360".into(),
                category: Some("Jeux".into()),
                watch_exe: None,
            })
        );
        fill(&mut form, PROCESS, "Hades.exe");
        assert!(matches!(
            form.to_command(),
            Ok(Command::Add { watch_exe: Some(exe), .. }) if exe == "Hades.exe"
        ));
    }

    #[test]
    fn missing_required_field_is_focused() {
        let mut form = Form::add_app("Jeux");
        fill(&mut form, NAME, "Hades");
        form.focused = PROCESS;
        assert_eq!(form.to_command(), Err("Cible : champ requis".into()));
        assert_eq!(form.focused, TARGET);
    }

    #[test]
    fn process_placeholder_follows_target() {
        let mut form = Form::add_app("Jeux");
        fill(&mut form, TARGET, r"C:\Games\Hades\Hades.exe");
        assert_eq!(
            form.placeholder(PROCESS).as_deref(),
            Some("auto : Hades.exe")
        );
    }

    #[test]
    fn fields_cycle() {
        let mut form = Form::add_app("");
        form.prev_field();
        assert_eq!(form.focused, PROCESS);
        assert!(form.is_last_field());
        form.next_field();
        assert_eq!(form.focused, NAME);
    }
}
