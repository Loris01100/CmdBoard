//! Popup state: confirmations, forms and level-ups. Rendering lives in `ui/widgets/popup.rs`.

use crate::command::Command;
use crate::launcher::launch;
use crate::text_input::TextInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    /// Asks before running a destructive `command`.
    Confirm { message: String, command: Command },
    Form(Form),
    LevelUp(LevelUp),
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

    /// Move form for `app`, pre-filled with its current category.
    pub fn move_app(app: &str, category: &str) -> Self {
        Self::new(
            FormKind::MoveApp { app: app.to_string() },
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
        assert_eq!(form.placeholder(PROCESS).as_deref(), Some("auto : Hades.exe"));
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
