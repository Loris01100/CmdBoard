# Plan de conception : dashboard TUI en Rust (ratatui)

Application de lancement de raccourcis avec catégories, XP par session, niveaux et récompenses. Interface style terminal, pour Windows.

---

## 1. Principes directeurs

- **Un seul état central** (`App`), modifié uniquement par une fonction `update`.
- **Le rendu est pur** : `draw(frame, &app)` lit l'état et dessine, sans rien modifier.
- **Tout passe par `Command`** : touches, palette `:` et alias déclenchent le même chemin d'exécution.
- **Aucune logique métier dans `ui/`** : XP, récompenses et stockage vivent dans des modules testables sans terminal.

---

## 2. Arborescence

```
src/
├── main.rs              // init terminal, lance App::run()
├── app.rs               // struct App, boucle principale, update()
├── event.rs             // thread d'événements (clavier, tick, tracker)
├── command/
│   ├── mod.rs           // enum Command
│   ├── parser.rs        // texte -> Command
│   └── alias.rs         // chargement de commands.toml
├── core/
│   ├── xp.rs            // formules XP / niveaux
│   └── rewards.rs       // moteur de règles
├── storage/
│   ├── db.rs            // connexion SQLite, migrations
│   └── models.rs        // App, Category, Session, Reward
├── launcher/
│   ├── launch.rs        // lancement (exe, URI)
│   └── scan.rs          // import des .lnk
├── tracker.rs           // thread de suivi des sessions (sysinfo)
└── ui/
    ├── mod.rs           // draw() : routage selon l'écran
    ├── theme.rs         // palette de couleurs et styles
    ├── layout.rs        // découpage de l'écran
    ├── screens/
    │   ├── dashboard.rs
    │   ├── stats.rs
    │   ├── rewards.rs
    │   └── help.rs
    └── widgets/
        ├── category_list.rs
        ├── app_table.rs
        ├── profile_panel.rs
        ├── xp_bar.rs
        ├── command_line.rs
        ├── status_bar.rs
        └── popup.rs     // level-up, récompense, confirmation
```

---

## 3. Modèle de données (SQLite)

```
categories(id, name, color, icon)
apps(id, name, launch_target, watch_exe, icon, category_id, total_xp, level)
sessions(id, app_id, started_at, ended_at, duration_s, xp_gained)
rewards(id, app_id NULL, code, name, description, rule)
unlocked_rewards(reward_id, unlocked_at, session_id)
```

- `launch_target` : chemin ou URI de lancement (`steam://rungameid/...`).
- `watch_exe` : nom de l'exécutable réel à surveiller (utile pour les launchers).
- `app_id NULL` dans `rewards` : récompense globale. Sinon : récompense individuelle.

---

## 4. Modèle d'état

```rust
pub enum Screen { Dashboard, Stats, Rewards, Help }

pub enum Mode {
    Normal,
    Command,            // saisie après ':'
    Search,             // saisie après '/'
    Popup(PopupKind),   // LevelUp, RewardUnlocked, Confirm, Form
}

pub struct App {
    // navigation
    pub screen: Screen,
    pub mode: Mode,
    pub focus: Focus,                  // Categories | Apps
    pub cat_state: ListState,
    pub app_state: TableState,

    // données en cache (rechargées après chaque écriture)
    pub categories: Vec<Category>,
    pub apps: Vec<AppEntry>,
    pub profile: Profile,              // niveau global, XP, streak
    pub active_sessions: HashMap<i64, Instant>,

    // ligne de commande
    pub input: String,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub message: Option<(String, MsgKind)>,   // Info / Succès / Erreur

    // technique
    pub theme: Theme,
    pub db: Database,
    pub should_quit: bool,
}
```

---

## 5. Boucle d'événements

```rust
pub enum AppEvent {
    Key(KeyEvent),
    Tick,                                  // ~ toutes les 250 ms (animations, chrono)
    SessionStarted { app_id: i64 },
    SessionEnded { app_id: i64, secs: u64 },
}

fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
    while !self.should_quit {
        terminal.draw(|f| ui::draw(f, self))?;
        match self.events.recv()? {
            AppEvent::Key(k) if k.kind == KeyEventKind::Press => self.on_key(k),
            AppEvent::Tick => self.on_tick(),
            AppEvent::SessionEnded { app_id, secs } => self.on_session_end(app_id, secs),
            _ => {}
        }
    }
    Ok(())
}
```

Trois threads au total : l'UI (principal), les événements clavier et ticks, et le tracker. Ils communiquent par `mpsc`.

> **Windows** : crossterm envoie `Press` et `Release`. Le filtre `KeyEventKind::Press` est indispensable, sinon chaque touche compte double.

---

## 6. Système de commandes

### Enum typée

```rust
pub enum Command {
    Launch { app: String },
    Add { name: String, path: String, category: Option<String> },
    Move { app: String, category: String },
    Stats { app: Option<String> },
    Xp { app: String, amount: i32 },
    Theme { name: String },
    Help,
    Quit,
}
```

### Parser

```rust
pub fn parse(input: &str) -> Result<Command, String> {
    let parts = shell_words::split(input).map_err(|e| e.to_string())?;
    match parts.as_slice() {
        [c, app] if c == "launch" => Ok(Command::Launch { app: app.clone() }),
        [c, app, cat] if c == "move" => Ok(Command::Move { app: app.clone(), category: cat.clone() }),
        [c] if c == "help" => Ok(Command::Help),
        [c] if c == "quit" || c == "q" => Ok(Command::Quit),
        _ => Err(format!("Commande inconnue : {input}")),
    }
}
```

### Confort

- Historique avec flèches haut/bas.
- Autocomplétion avec Tab (commandes, apps, catégories) via `fuzzy-matcher`.
- Ligne de message : vert (succès), rouge (erreur).
- `:help` et `:help <commande>` générés à partir du parser.

### Alias personnalisés (`commands.toml`)

```toml
[alias]
gaming = "launch steam; launch discord"
focus  = "theme dark; launch vscode"
```

Découpage sur `;` puis exécution séquentielle. Variables `$1`, `$2` possibles pour des alias paramétrés. Scripting avancé (Rhai, mlua) : étape ultérieure.

---

## 7. Gestion des touches par mode

`on_key` dispatche selon `self.mode`, puis traduit la touche en `Command`.

| Mode | Touches | Effet |
|---|---|---|
| Normal | `↑↓` / `j k` | Navigation dans la liste |
| Normal | `←→` / `Tab` | Changer de panneau (catégories ↔ apps) |
| Normal | `Enter` | `Command::Launch` |
| Normal | `:` | Ouvre la ligne de commande |
| Normal | `/` | Recherche rapide |
| Normal | `1 2 3 4` | Changer d'écran |
| Normal | `a` / `d` / `m` | Ajouter / supprimer / déplacer |
| Command | `Enter` / `Esc` | Valider / annuler |
| Command | `↑↓` / `Tab` | Historique / autocomplétion |
| Popup | `Enter` / `Esc` | Fermer ou confirmer |

---

## 8. Écrans et layouts

### Dashboard (écran principal)

```
┌ Header : titre + horloge + session en cours ────────────────────┐
├ Catégories ─┬ Applications ─────────────────┬ Détails ──────────┤
│ > Jeux (12) │ Nom        Niv  XP       Temps│ Elden Ring        │
│   Dev   (5) │ Hades       7   ███░░    42h  │ Niveau 12         │
│   Créa  (3) │ Celeste     4   █████    18h  │ ████████░░ 80%    │
│             │                               │ Dernière: hier    │
│             │                               │ Récompenses: 🏆 3 │
├ Profil : Niv 23 ████████░░ | Streak 5j | XP du jour: +120 ──────┤
├ Dernières récompenses ──────────────────────────────────────────┤
├ Message / ligne de commande ────────────────────────────────────┤
└ Barre de statut : raccourcis contextuels ───────────────────────┘
```

```rust
let rows = Layout::vertical([
    Constraint::Length(1),   // header
    Constraint::Min(10),     // corps
    Constraint::Length(3),   // profil
    Constraint::Length(3),   // récompenses
    Constraint::Length(1),   // ligne de commande / message
    Constraint::Length(1),   // statut
]).split(area);

let cols = Layout::horizontal([
    Constraint::Length(20),      // catégories
    Constraint::Percentage(55),  // applications
    Constraint::Min(25),         // détails
]).split(rows[1]);
```

### Autres écrans

- **Stats** : historique des sessions (`Table`), temps par catégorie (`BarChart`), activité des 30 derniers jours (`Sparkline`).
- **Rewards** : grille des récompenses, débloquées en couleur et verrouillées en gris avec leur condition.
- **Help** : commandes et raccourcis, générés à partir du parser.

### Responsive

- Moins de ~90 colonnes : masquer le panneau Détails.
- Moins de ~60 colonnes : empiler les panneaux.

---

## 9. Widgets

| Widget | Basé sur | Rôle |
|---|---|---|
| `CategoryList` | `List` | Catégories avec compteur |
| `AppTable` | `Table` | Apps, niveau, mini-barre XP, temps total |
| `XpBar` | `Gauge` / `LineGauge` | Barre réutilisée partout |
| `ProfilePanel` | `Paragraph` + `XpBar` | Niveau global, streak |
| `CommandLine` | `Paragraph` + curseur | Saisie, message d'erreur |
| `StatusBar` | `Paragraph` | Raccourcis selon le mode |
| `Popup` | `Clear` + `Block` | Fenêtres centrées (level-up, formulaires) |

Chaque widget est une fonction `render(frame, area, &app)` ou une struct implémentant `Widget`. Pas d'état propre : tout est dans `App`.

---

## 10. Thème

Un `Theme` unique, jamais de couleur codée en dur dans les widgets.

```rust
pub struct Theme {
    pub border: Style,
    pub border_focused: Style,
    pub title: Style,
    pub selected: Style,
    pub xp_fill: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub muted: Color,
}
```

- Chargement depuis `themes/*.toml`, changement via `:theme dracula`.
- Look de référence : bordures cyan, titres intégrés au cadre (`Block::title`), fond sombre ou transparent.
- Utiliser **Windows Terminal** avec une **Nerd Font** (ex. JetBrainsMono Nerd Font).

---

## 11. XP et récompenses

```rust
pub fn xp_for_session(duration_min: u32, streak_days: u32) -> u32 {
    if duration_min < 5 { return 0; }          // anti-abus
    let base = duration_min;                   // 1 XP / minute
    let streak_bonus = (streak_days.min(7) * 5) as u32;
    base + streak_bonus
}

pub fn xp_to_next_level(level: u32) -> u32 {
    (100.0 * (level as f32).powf(1.5)) as u32
}
```

Les récompenses sont définies **en données** (table ou JSON), pas en dur :

```json
{ "code": "marathon", "condition": "session_duration >= 180", "scope": "app" }
```

Après chaque session, on évalue les règles non débloquées et on enregistre celles qui passent.

Idées : seuils d'heures cumulées par app, longue session, première session du jour, streak, niveau atteint, nombre d'apps différentes dans la semaine.

---

## 12. Détection des sessions

1. À l'ajout d'une app, enregistrer `watch_exe`.
2. Un thread de fond interroge `sysinfo` toutes les 2 à 5 secondes.
3. Process détecté : début de session. Process disparu : fin, enregistrement.
4. Au démarrage, fermer les sessions orphelines (crash précédent).

Cas Steam / Epic / Battle.net : la commande de lancement (URI) et le process surveillé sont différents, d'où les deux champs séparés.

---

## 13. Animations (sobres)

Pilotées par `Tick` et un compteur `frame_count` dans `App` :

- Barre d'XP qui se remplit progressivement après une session.
- Popup de level-up qui clignote ou change de couleur.
- Spinner pendant l'import des raccourcis.
- Chrono de session en direct dans le header.

---

## 14. Pièges spécifiques à Windows

1. Événements clavier en double (`Press` / `Release`).
2. Raccourcis `.lnk` à scanner dans :
   - `%ProgramData%\Microsoft\Windows\Start Menu\Programs`
   - `%AppData%\Microsoft\Windows\Start Menu\Programs`
   - le Bureau
3. Icônes impossibles dans un terminal : glyphe Nerd Font ou emoji par catégorie.
4. Base de données dans `%APPDATA%` (crate `directories`).

---

## 15. Tests

- **`core/`** : tests unitaires (XP, niveaux, règles).
- **`command/parser`** : tests de table (entrée → `Command`).
- **`ui/`** : `TestBackend` de ratatui, ou snapshots avec `insta`.
- **`storage/`** : `Connection::open_in_memory()`.

---

## 16. Étapes de réalisation

| # | Étape | Livrable vérifiable |
|---|---|---|
| 1 | Init terminal, boucle, `q` pour quitter, hook de panic | Fenêtre vide propre |
| 2 | Layout complet, données factices, thème | Dashboard statique conforme à la maquette |
| 3 | Navigation, focus, sélection | Déplacement fluide |
| 4 | SQLite + CRUD + `Launch` | Vraies apps listées et lancées |
| 5 | Mode Command + parser + historique + message | `:add`, `:move`, `:launch` |
| 6 | Popups et formulaires | Ajout d'app sans quitter l'UI |
| 7 | Tracker + sessions en direct | Chrono qui tourne, session enregistrée |
| 8 | XP, niveaux, barres animées | Level-up fonctionnel |
| 9 | Récompenses + écran Rewards | Déblocage avec popup |
| 10 | Stats, recherche `/`, alias, autocomplétion | Version complète |
| 11 | Thèmes, responsive, finitions | Version personnalisable |

L'étape 1 doit inclure un **hook de panic** qui restaure le terminal. `ratatui::init()` le fait dans les versions récentes.

---

## 17. Dépendances

```toml
ratatui = "0.29"
crossterm = "0.28"
rusqlite = { version = "0.32", features = ["bundled"] }
sysinfo = "0.32"
opener = "0.7"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
chrono = "0.4"
anyhow = "1"
shell-words = "1"
directories = "5"
fuzzy-matcher = "0.3"
```

À vérifier avec `cargo add` au moment de créer le projet, pour obtenir les dernières versions. Pour lire les `.lnk`, ajouter `lnk` ou `parselnk`.

---

## Prochaine étape

Écrire le code des **étapes 1 à 3** : projet qui compile, boucle d'événements, thème et dashboard à 3 colonnes avec données factices, prêt à être branché sur SQLite.
