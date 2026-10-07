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
├── i18n.rs              // textes de l'interface : t!(), langue courante
├── command/
│   ├── mod.rs           // enum Command
│   ├── parser.rs        // texte -> Command
│   └── alias.rs         // chargement de commands.toml
├── core/
│   ├── xp.rs            // formules XP / niveaux
│   └── rewards.rs       // moteur de règles
├── storage/
│   ├── db.rs            // connexion SQLite, migrations, contenu de départ
│   ├── queries.rs       // CRUD et agrégats (temps joué, profil, récompenses)
│   ├── backup.rs        // :export / :import en JSON
│   └── models.rs        // App, Category, Session, Reward
├── launcher/
│   ├── folders.rs       // explorateur de l'écran Stockage : liste, mesure, corbeille
│   ├── launch.rs        // lancement (exe, URI)
│   ├── programs.rs      // disques et programmes installés (registre), désinstallation
│   └── scan.rs          // import des .lnk et des bibliothèques Steam/Epic
├── popup.rs             // état des popups : confirmation, formulaires
├── text_input.rs        // champ texte éditable (ligne de commande, formulaires)
├── tracker.rs           // thread de suivi des sessions (sysinfo)
├── update.rs            // vérification et installation des mises à jour (self_update)
└── ui/
    ├── mod.rs           // draw() : routage selon l'écran
    ├── theme.rs         // chargement TOML, palette -> slots -> Theme
    ├── layout.rs        // découpage de l'écran
    ├── screens/
    │   ├── dashboard.rs
    │   ├── stats.rs
    │   ├── rewards.rs
    │   ├── storage.rs
    │   └── help.rs
    └── widgets/
        ├── category_list.rs
        ├── app_table.rs
        ├── profile_panel.rs
        ├── xp_bar.rs
        ├── command_line.rs
        ├── status_bar.rs
        └── popup.rs     // level-up, récompense, confirmation
themes/                  // thèmes intégrés au binaire (include_str!)
├── catppuccin-latte.toml
├── catppuccin-frappe.toml
├── catppuccin-macchiato.toml
├── catppuccin-mocha.toml
└── terminal.toml        // 16 couleurs ANSI, repli sans truecolor
locales/                 // textes de l'interface, intégrés au binaire (include_str!)
├── en.toml              // référence : toute autre langue a exactement ces clés
└── fr.toml
.github/workflows/release.yml   // généré par `dist init`
wix/main.wxs                    // installeur MSI, généré par `dist init`
```

---

## 3. Modèle de données (SQLite)

```
categories(id, name, color, icon)
apps(id, name, launch_target, watch_exe, icon, category_id, total_xp)
sessions(id, app_id, started_at, ended_at, duration_s, xp_gained)
rewards(id, app_id NULL, code, name, description, rule, scope)
unlocked_rewards(id, reward_id, app_id NULL, unlocked_at, session_id)
```

- `launch_target` : chemin ou URI de lancement (`steam://rungameid/...`).
- `watch_exe` : nom de l'exécutable réel à surveiller (utile pour les launchers).
- `app_id NULL` dans `rewards` : récompense commune à toutes les apps. Sinon : récompense propre à cette app.
- `scope` (migration v2) : `global` se débloque une seule fois en tout ; `app` se débloque une fois par app. Une récompense propre à une app (`app_id` renseigné) est traitée comme `app`.
- `unlocked_rewards.app_id` : l'app pour laquelle une récompense `app` a été débloquée (`NULL` pour une `global`). Index unique sur `(reward_id, IFNULL(app_id, 0))` : pas de double déblocage.
- Le niveau n'est pas stocké : il se déduit de `total_xp` (`core::xp::level_from_total`), ce qui évite toute incohérence. Le niveau global se déduit de la somme des `total_xp`.
- Temps total, dernière session et nombre de récompenses d'une app sont agrégés depuis `sessions` et `unlocked_rewards` à la lecture.
- Horodatages en secondes Unix (`INTEGER`). Les jours (XP du jour, streak) suivent le fuseau local via `date(..., 'unixepoch', 'localtime')`.
- Noms de catégories et d'apps uniques sans tenir compte de la casse. Une catégorie qui contient des apps ne peut pas être supprimée. Supprimer une app supprime ses sessions et récompenses.
- Une base neuve reçoit un contenu de départ (catégories Jeux, Dev, Outils et quelques apps Windows) pour avoir de quoi lancer dès le premier démarrage.
- **Migrations** : le schéma est versionné via `PRAGMA user_version`. Au démarrage, `storage/db.rs` applique dans l'ordre les migrations manquantes. Une mise à jour de l'app ne doit jamais perdre les données de `%APPDATA%` : on ne modifie jamais une migration déjà publiée, on en ajoute une nouvelle.

---

## 4. Modèle d'état

```rust
pub enum Screen { Dashboard, Stats, Rewards, Storage, Help }

pub enum Mode {
    Normal,
    Command,            // saisie après ':'
    Search,             // saisie après '/'
    Popup(Popup),       // Confirm, Form, LevelUp, RewardUnlocked
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
    pub active_sessions: HashMap<i64, ActiveSession>, // par app_id : id de la ligne sessions, Instant de début

    // ligne de commande
    pub command_line: CommandLine,      // saisie, curseur, historique (command/line.rs)
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
    UpdateFinished(Result<String, String>),   // voir section 18
    ShortcutsScanned(Vec<Shortcut>),          // choix de l'app (section 7)
    StorageScanned { disks, programs },       // écran Stockage (section 8)
    FolderListed { dir, entries },            // explorateur : contenu d'un dossier
    FolderProgress { path, percent },         // explorateur : % des enfants d'un sous-dossier mesurés
    FolderSized { path, size },               // explorateur : un sous-dossier mesuré
    Trashed { path, result },                 // explorateur : envoi à la corbeille
}

fn run(&mut self, terminal: &mut DefaultTerminal, events: &Receiver<AppEvent>) -> Result<()> {
    while !self.should_quit {
        terminal.draw(|f| ui::draw(f, self))?;
        match events.recv()? {
            AppEvent::Key(k) => self.on_key(k),          // déjà filtré sur Press par event.rs
            AppEvent::Tick => self.on_tick(),
            AppEvent::SessionStarted { app_id } => self.on_session_start(app_id),
            AppEvent::SessionEnded { app_id, secs } => self.on_session_end(app_id, secs),
        }
    }
    self.end_all_sessions()                        // ferme les sessions en cours à la sortie
}
```

Trois threads permanents : l'UI (principal), les événements clavier et ticks (`event.rs`), et le tracker (`tracker.rs`). Ils envoient leurs `AppEvent` à l'UI par un `mpsc` créé dans `main.rs`, qui passe le `Receiver` à `run`. En sens inverse, l'UI envoie au tracker la liste des `watch_exe` à surveiller par un second canal, à chaque `reload()`. Les tâches réseau ponctuelles (`:update`, section 18) tournent dans un thread temporaire qui renvoie son résultat par le canal des événements (`UpdateFinished`, ajouté à l'étape 12).

> **Windows** : crossterm envoie `Press` et `Release`. Le filtre `KeyEventKind::Press` est indispensable, sinon chaque touche compte double.

---

## 6. Système de commandes

### Enum typée

```rust
pub enum Command {
    // navigation (touches)
    Show(Screen),
    SelectNext,
    SelectPrev,
    FocusPanel(Focus),
    ToggleFocus,
    // actions (touches et ligne de commande)
    Launch { app: String },
    Add { name: String, target: String, category: Option<String> },  // catégorie créée si absente
    Move { app: String, category: String },                          // idem
    Help { command: Option<String> },
    Xp { app: String, amount: i64 },   // ajuste l'XP à la main (négatif : en retire)
    Stats { app: Option<String> },     // écran Stats, filtré sur une app ou non
    Select { app: String },            // touche seulement : Entrée dans la recherche `/`
    Quit,
    // à venir
    Theme { name: Option<String> },   // sans argument : liste les thèmes
    Update,
}
```

### Parser

`command/parser.rs`. Découpage maison plutôt que `shell-words` : les espaces séparent les arguments, les guillemets doubles les regroupent, et les antislashs restent tels quels (sinon `C:\Program Files\...` serait mangé comme une séquence d'échappement POSIX). Les apostrophes ne sont pas des délimiteurs (`Assassin's Creed`).

La table `COMMANDS` (`command/mod.rs`) décrit chaque commande (nom, alias, usage, résumé). Elle sert à la fois au parser (résolution des alias, message `Usage : ...` si les arguments ne collent pas), à `:help <commande>` et à l'écran d'aide.

| Commande | Alias | Usage |
|---|---|---|
| `launch` | `l` | `launch <app>` (le reste de la ligne, guillemets inutiles) |
| `add` | | `add [<nom> <cible> [catégorie]]` (sans catégorie : la sélectionnée ; sans argument : formulaire) |
| `move` | `mv` | `move <app> [catégorie]` (sans catégorie : formulaire) |
| `rm` | `delete` | `rm <app>` (confirmation, supprime aussi sessions et récompenses) |
| `rmcat` | | `rmcat <catégorie>` (catégorie vide uniquement, confirmation) |
| `xp` | | `xp <app> <montant>` (montant en dernier, signé ; l'XP ne descend pas sous 0) |
| `stats` | | `stats [app]` (sans argument : toutes les apps ; avec : filtre jusqu'au prochain `:stats`) |
| `theme` | | `theme [nom]` (sans nom : liste les thèmes et l'actuel ; nom complété par Tab) |
| `group` | | `group <nom> <app>, <app>…` (apps séparées par des virgules, sans guillemets ; écrit l'alias `<nom> = "launch A; launch B"` dans `commands.toml`, le remplace s'il existe, et le recharge aussitôt) |
| `export` | | `export [fichier]` (JSON des apps et sessions terminées ; sans argument : `Documents\cmdboard-<aaaa-mm-jj>.json`) |
| `import` | | `import <fichier>` (fusionne un export, voir ci-dessous) |
| `uninstall` | | `uninstall <programme>` (programme installé, complété par Tab ; confirmation, puis lance son propre désinstalleur, voir écran Stockage) |
| `help` | `h`, `?` | `help [commande]` |
| `quit` | `q` | `quit` |

Noms d'apps et de catégories insensibles à la casse. `:add` déduit `watch_exe` du nom de fichier quand la cible est un `.exe`, et refuse un chemin absolu inexistant. Après `:add` ou `:move`, la sélection suit l'app.

`:export` / `:import` (`storage/backup.rs`) servent à la sauvegarde, à l'analyse externe et au changement de PC. Le fichier : `{ version: 1, exported_at, apps: [{ name, category, launch_target, watch_exe, total_xp }], sessions: [{ app, started_at, ended_at, duration_s, xp_gained }] }` (sessions en cours exclues, horodatages Unix). L'import fusionne en une transaction, sans confirmation puisqu'il ne supprime rien : une app absente est créée avec sa catégorie et son `total_xp` ; une app déjà présente (même nom) garde sa cible et sa catégorie, et gagne l'XP de ses sessions nouvellement importées ; une session déjà présente (même app, même début) est ignorée, donc réimporter le même fichier ne change rien. Les récompenses ne sont pas exportées.

### Confort

- La saisie s'affiche dans un cadre « Commande » (bordure de focus) qui s'ouvre au-dessus de la barre de statut, sur tous les écrans, avec un texte d'exemple quand la ligne est vide et un défilement horizontal qui garde le curseur visible. Hors saisie, cette zone se réduit à une ligne de message.
- Historique avec flèches haut/bas (100 entrées, sans doublon consécutif, en mémoire seulement).
- Édition : `←→`, `Home`/`End`, `Backspace`/`Suppr`. `Backspace` sur une ligne vide ou `Esc` referment la ligne.
- Autocomplétion avec Tab (`command/complete.rs`, fonction pure) : nom de commande ou d'alias en premier mot (suivi d'un espace), puis selon la commande : app (`launch`, `rm`, `stats`, `xp`, reste de la ligne), catégorie (`rmcat`, 2e argument de `move`, 3e de `add`), commande (`help`). Candidats classés par `fuzzy-matcher` (`src/fuzzy.rs`, partagé avec la recherche), mis entre guillemets s'ils contiennent un espace. `Tab` répété passe au suivant, `Shift-Tab` au précédent, toute autre touche repart de zéro. Les candidats s'affichent sur la bordure basse du cadre, le courant en surbrillance.
- Ligne de message : cyan (info), vert (succès), rouge (erreur). Effacée à la touche suivante.
- `:help` ouvre l'écran d'aide, `:help <commande>` affiche l'usage dans la ligne de message.

### Alias personnalisés (`commands.toml`)

```toml
[alias]
gaming = "launch steam; launch discord"
focus  = "theme dark; launch vscode"
```

Fichier `%APPDATA%\CmdBoard\commands.toml`, lu au démarrage (`command/alias.rs`). Absent : aucun alias. Cassé : message d'erreur, CmdBoard démarre quand même. Un alias qui porte le nom (ou l'alias) d'une commande intégrée est ignoré et signalé.

Découpage sur `;` puis exécution séquentielle, chaque ligne passant par le parser puis `run_command`. `$1`…`$9` sont remplacés par les arguments (remis entre guillemets s'ils contiennent un espace), `$*` par tous ; un argument manquant est une erreur. L'exécution s'arrête à la première erreur (message `<ligne> : <erreur>`) ou dès qu'une popup s'ouvre (confirmation, formulaire). Un seul niveau : un alias ne peut pas en appeler un autre. `:help <alias>` affiche sa définition, et l'écran d'aide liste les alias après les commandes. Scripting avancé (Rhai, mlua) : étape ultérieure.

---

## 7. Gestion des touches par mode

`on_key` dispatche selon `self.mode`, puis traduit la touche en `Command`.

| Mode | Touches | Effet |
|---|---|---|
| Normal | `↑↓` / `j k` | Navigation dans la liste |
| Normal | `←→` / `Tab` | Changer de panneau (catégories ↔ apps) |
| Normal | `Enter` | `Command::Launch` (sur les catégories : passe au panneau apps) |
| Normal | `:` | Ouvre la ligne de commande |
| Normal | `/` | Recherche floue parmi toutes les apps (`Mode::Search`) |
| Normal | `1 2 3 4 5` | Changer d'écran : Dashboard, Stats, Récompenses, Stockage, Aide |
| Normal | `?` | Aide |
| Normal | `a` / `m` | Formulaire d'ajout / de déplacement de l'app sélectionnée |
| Normal | `d` | Supprimer l'app sélectionnée, ou la catégorie si le focus y est (vide uniquement) |
| Normal | `s` | Tri suivant des apps (`Command::Sort`) : nom, XP, récent, temps |
| Normal (Stats) | `s` | Camembert par catégorie ↔ par app (`Command::ToggleStatsPie`) |
| Normal (Stockage) | `Tab` `→` `l` / `Shift-Tab` `←` `h` | Disque suivant / précédent, « tous les disques » avant le premier (`Command::CycleDisk`) |
| Normal (Stockage) | `s` | Plus gros ↔ plus petits d'abord (`Command::ToggleStorageOrder`) |
| Normal (Stockage) | `d` / `Suppr` | Désinstaller le programme sélectionné (`Command::Uninstall`, confirmation) |
| Normal (Stockage) | `f` | Explorateur de dossiers ↔ programmes (`Command::ToggleFolders`) |
| Normal (Stockage, dossiers) | `Entrée` `→` `l` / `Backspace` `←` `h` | Ouvrir le dossier (`Command::OpenFolder`) / remonter, puis revenir aux disques (`Command::ParentFolder`) |
| Normal (Stockage, dossiers) | `s` | Plus gros ↔ plus petits d'abord |
| Normal (Stockage, dossiers) | `d` / `Suppr` | Dossier d'un programme : `Command::Uninstall` ; sinon `Command::Trash` (corbeille, confirmation) |
| Command | `Enter` / `Esc` | Valider / annuler |
| Command | `↑↓` / `Tab` `Shift-Tab` | Historique / autocomplétion |
| Search | saisie | Filtre le panneau Applications (toutes catégories, meilleur résultat en tête et sélectionné) |
| Search | `↑↓` / `Tab` | Choisir parmi les résultats |
| Search | `Enter` / `Esc` | `Command::Select` (catégorie et app sélectionnées, focus sur les apps) / annuler et restaurer la sélection |
| Popup (confirmation) | `Enter` `o` `y` / `Esc` `n` | Confirmer / annuler |
| Popup (formulaire) | `Tab` `↓` / `Shift-Tab` `↑` | Champ suivant / précédent |
| Popup (formulaire) | `Enter` / `Esc` | Champ suivant, valider sur le dernier / annuler |

### Popups (`src/popup.rs`, rendu dans `ui/widgets/popup.rs`)

- **Confirmation** : toute commande destructive (`RemoveApp`, `RemoveCategory`, `Uninstall`, `Trash`) porte un champ `confirmed`. Non confirmée, son exécution ouvre une popup qui contient la même commande avec `confirmed: true`. Touche `d` et `:rm` passent donc par la même confirmation.
- **Formulaires** : `Form` = liste de champs (`TextInput`, partagé avec la ligne de commande) avec un champ focalisé. La validation produit une `Command` (`Add`, `Move`) exécutée par le chemin habituel. En cas d'erreur (champ requis, nom déjà pris, fichier introuvable), le formulaire reste ouvert et affiche l'erreur ; le premier champ requis vide reçoit le focus.
- **Choix de l'app** (`Popup::Picker`) : `a` et `:add` sans argument ouvrent d'abord une liste filtrable (fuzzy) des apps installées, lue par `launcher/scan.rs` dans les bibliothèques Steam (`libraryfolders.vdf` puis `appmanifest_*.acf` complètement installés : cible `steam://rungameid/<id>`, process = le plus gros exe du dossier du jeu, jusqu'à trois niveaux) et Epic (manifests `.item` de `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`, hors DLC : cible `com.epicgames.launcher://apps/...?action=launch`, process = `LaunchExecutable`), puis dans les raccourcis `.lnk` et `.url` du menu Démarrer et du Bureau. À nom égal, l'entrée de bibliothèque l'emporte. Le scan tourne dans un thread temporaire (`AppEvent::ShortcutsScanned`) à chaque ouverture ; les trois sources et la lecture des fichiers sont parallélisées avec rayon (ordre conservé, donc la priorité des bibliothèques aussi). Les apps déjà ajoutées sont masquées. `Entrée` ouvre le formulaire pré-rempli (cible = le `.lnk`, qui garde ses arguments ; process = l'exe pointé par le raccourci), focus sur la catégorie. `Tab`, ou `Entrée` sans résultat, ouvre le formulaire vide avec la recherche comme nom.
- Formulaire d'ajout : Nom*, Cible*, Catégorie* (pré-remplie avec la catégorie sélectionnée), Process (vide : déduit de la cible, affiché en grisé « auto : X.exe »). Une ligne d'aide sous les champs explique le champ actif, notamment à quoi sert Process (l'exe surveillé pour compter le temps de jeu).
- **Level-up** : ouverte quand une app ou le profil gagne un niveau (fin de session ou `:xp`). Bordure qui alterne de couleur à chaque `Tick`. `Entrée`, `Esc` ou `Espace` la ferment.
- **Récompense débloquée** : une popup par récompense, après celle de level-up, mêmes touches et même clignotement.
- Une popup déclenchée par un événement (level-up, récompense) n'interrompt pas une saisie : elle attend dans une file (`pending_popups`) que l'utilisateur revienne en mode Normal.
- Les popups se dessinent par-dessus l'écran courant (`Clear` puis cadre centré).

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

- **Stats** : ligne de résumé (portée, nombre de sessions, temps total, plus longue session), temps par catégorie ou par app (un camembert en braille via `Canvas`, `s` bascule entre les deux via `Command::ToggleStatsPie`, état `App::stats_by_app` ; couleurs `xp_fill`/`info`/`warning`/`error`, au-delà le reste est regroupé en « Autres » en `muted`, légende avec durée et % ; masqué sous 60 colonnes), heatmap d'activité des 12 dernières semaines façon GitHub sur la moitié droite (une colonne par semaine, lundi en haut, aujourd'hui en bas à droite, jours de la semaine à gauche, date du lundi au-dessus des colonnes, un carré `■` par jour dont la couleur dit le temps joué (`Theme::heat` : rien en `muted`, < 22 min `success`, < 45 min `warning`, < 1 h `caution`, au-delà `error`), légende des durées par bloc dans la bordure du bas ; `Stats::today` donne le jour local en jours depuis 1970), historique des 200 dernières sessions (`Table`, sélection `stats_state`, `j`/`k`). `:stats <app>` filtre tout l'écran sur une app. Les données (`Database::stats`) sont rechargées avec le reste à chaque `reload()`.
- **Rewards** : tableau des récompenses (🏆 débloquées en couleur, 🔒 verrouillées en gris), titre « Récompenses (n/total) », sélection propre (`reward_state`, `j`/`k`). Un panneau Détail montre la portée, la condition (`rule`) et qui l'a débloquée, et quand.
- **Stockage** (`4`) : en haut, une jauge par disque (`LineGauge`, % utilisé ; `success`, `warning` dès 75 %, `error` dès 90 %) avec utilisé / total / libre ; le disque filtré est marqué `>`. En dessous, les programmes installés (nom, taille, disque, éditeur), triés par taille (plus gros d'abord par défaut, tailles inconnues toujours à la fin, égalités par nom), sur un disque ou tous. Sélection propre (`storage_state`, `j`/`k`), filtre `storage_disk`, ordre `storage_ascending`, non mémorisés. Les données viennent de `launcher/programs.rs` : disques via `sysinfo::Disks`, programmes via les clés `HKLM\...\CurrentVersion\Uninstall` (vues 64 et 32 bits) et `HKCU` (ce que lit « Applications installées » de Windows), lues avec l'API registre de `windows-sys` (Unicode, sans lancer `reg`). Sont écartés : `SystemComponent = 1`, mises à jour (`ParentKeyName`, `ReleaseType` Update/Hotfix), entrées sans `DisplayName` ou `UninstallString` ; doublons par nom. Taille = `EstimatedSize` (déclarée par l'installeur, parfois absente ou approximative : pas de calcul de dossier pour l'instant). Disque = lettre de `InstallLocation`, sinon de `DisplayIcon` ; beaucoup de MSI n'en ont pas et n'apparaissent que sous « tous les disques ». Les apps du Store (MSIX) ne sont pas listées. Lecture dans un thread temporaire (`AppEvent::StorageScanned`) au démarrage (pour la complétion de `:uninstall`) et à chaque ouverture de l'écran. **Désinstallation** : `UninstallString` découpée en exe + arguments (exe entre guillemets, ou chemin non cité jusqu'à `.exe`), lancée par `ShellExecuteW` pour que Windows demande l'élévation (UAC) si besoin. CmdBoard n'attend pas la fin : rouvrir l'écran (`4`) actualise la liste. Disques masqués sous 12 lignes de corps.
- **Stockage, explorateur** (`f`) : remplace la liste des programmes (état `App::folders`, `None` = vue programmes). Part du disque filtré, sinon de la liste des disques (taille = espace utilisé). Rien n'est mesuré à l'avance : ouvrir un dossier le lit dans un thread temporaire (`FolderListed` : fichiers avec leur taille, sous-dossiers à « … »), puis mesure chaque sous-dossier en parallèle (rayon ; `FolderProgress` affiche à la place de « … » le % de ses enfants directs déjà mesurés ; `FolderSized` un par un, la liste se retrie et la sélection suit son élément). Quitter le dossier arrête ses mesures (`AtomicBool` d'annulation). Les tailles mesurées restent en cache (`folder_sizes`) le temps de la session, pour remonter et redescendre sans tout recompter ; un envoi à la corbeille invalide l'élément et ses dossiers parents. Taille = somme des tailles logiques des fichiers (pas la taille sur disque) ; dossiers illisibles comptés vides ; liens symboliques et jonctions ignorés (certains bouclent, comme `Application Data`). Une colonne « Programme » signale le dossier d'installation d'un programme (`InstallLocation`) : `d` le désinstalle au lieu de le supprimer. Ailleurs, `d` envoie fichier ou dossier à la corbeille (`SHFileOperationW`, `FOF_ALLOWUNDO`, Windows prévient si l'élément est trop gros pour la corbeille), dans un thread temporaire (`Trashed`). Les disques eux-mêmes ne se suppriment pas.
- **Help** : commandes et raccourcis, générés à partir du parser.

### Responsive

- Moins de ~90 colonnes : masquer le panneau Détails.
- Moins de 60 colonnes : catégories (5 lignes) au-dessus des apps.
- Moins de 24 lignes : masquer « Dernières récompenses » ; moins de 18 : masquer aussi le profil. Les listes gardent la place.
- Stats : graphiques masqués sous 18 lignes de corps, temps par catégorie masqué sous 60 colonnes. Rewards : panneau Détail masqué sous 12 lignes.

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

Un `Theme` unique, jamais de couleur codée en dur dans les widgets. Thèmes de référence : **[Catppuccin](https://catppuccin.com/palette)** (Latte, Frappé, Macchiato, Mocha).

### Deux couches dans chaque fichier TOML

1. **Palette** : les couleurs brutes, copiées telles quelles depuis la palette officielle (Catppuccin, base16…).
2. **Slots sémantiques** : le rôle de chaque couleur dans l'UI. Un slot référence un nom de la palette, une couleur `#rrggbb` ou une couleur ANSI (`"cyan"`). Ces slots correspondent un à un aux champs de `Theme`.

```toml
# themes/catppuccin-mocha.toml
name = "Catppuccin Mocha"

[palette]
base     = "#1e1e2e"
text     = "#cdd6f4"
overlay0 = "#6c7086"
overlay1 = "#7f849c"
mauve    = "#cba6f7"
sapphire = "#74c7ec"
green    = "#a6e3a1"
yellow   = "#f9e2af"
red      = "#f38ba8"
# … le reste de la palette officielle

[slots]
border             = "overlay0"
border_focused     = "sapphire"
title              = { fg = "mauve", bold = true }
selected           = { fg = "base", bg = "mauve", bold = true }
selected_unfocused = { bg = "surface0", bold = true }
xp_fill            = "green"
info               = "sky"
success            = "green"
warning            = "yellow"
caution            = "peach"
error              = "red"
muted              = "overlay1"
```

Ajouter un thème revient à coller une palette et à remplir les slots. Les noms de palette sont libres : le code ne lit que les slots. Un slot de style accepte une couleur (premier plan) ou une table `{ fg, bg, bold, italic, underlined, dim }` ; un slot de couleur (`xp_fill`…`muted`) n'accepte qu'une couleur. Une couleur est un nom de la palette, sinon tout ce que `ratatui::Color` sait lire (`#rrggbb`, noms ANSI comme `darkgray`, index). Tous les slots sont obligatoires sauf `caution` (orange, `warning` s'il manque, pour ne pas casser les thèmes utilisateur existants), un slot inconnu est une erreur (faute de frappe).

```rust
// Résolu au chargement : la palette n'est pas conservée.
pub struct Theme {
    pub name: String,
    pub border: Style,
    pub border_focused: Style,
    pub title: Style,
    pub selected: Style,
    pub selected_unfocused: Style,
    pub xp_fill: Color,
    pub info: Color,      // messages d'information
    pub success: Color,
    pub warning: Color,   // clignotement des popups level-up et récompense (avec success)
    pub caution: Color,   // orange, entre warning et error (heatmap)
    pub error: Color,
    pub muted: Color,
}
```

### Chargement

- **Intégrés** : les 4 saveurs Catppuccin et `terminal`, embarqués via `include_str!`. Pas de dépendance au crate `catppuccin` : thèmes intégrés et thèmes utilisateur passent par le même parseur.
- **Utilisateur** : `%APPDATA%\CmdBoard\themes\*.toml`. Le nom d'un thème est celui du fichier sans extension, en minuscules (`name` dans le fichier est le nom affiché). À nom égal, le fichier utilisateur remplace le thème intégré.
- **Erreurs** : un slot manquant ou une référence inconnue affiche une erreur claire dans la ligne de message, et le thème courant est conservé.
- `:sort [name|xp|recent|time]` trie le panneau Applications (nom croissant, sinon le plus grand ou le plus récent d'abord, égalités par nom). Mémorisé dans `config.toml` (`sort = "..."`), affiché dans le titre du panneau. La recherche `/` garde son propre ordre (meilleur résultat d'abord).
- `:theme` liste les thèmes, `:theme catppuccin-latte` en change. Le choix est mémorisé dans `%APPDATA%\CmdBoard\config.toml` (`theme = "..."`, les autres clés du fichier sont conservées). Au démarrage, un thème configuré introuvable ou cassé affiche l'erreur et bascule sur le thème par défaut. Les erreurs de `config.toml`, du thème et de `commands.toml` sont réunies dans la ligne de message.

### Langues

- Aucun texte d'interface en dur : `t!("clé")` ou `t!("clé", nom = valeur)` lit `locales/<langue>.toml` (tables imbriquées = clés pointées, `{nom}` remplacé). Une clé absente retombe sur l'anglais, puis sur la clé elle-même. Des tests vérifient que chaque langue a toutes les clés et les mêmes `{…}` que `en.toml`, et que chaque `t!("…")` du code existe.
- Langue courante globale (`i18n::set`), pas dans `App` : erreurs du parser, du stockage et du thread de mise à jour sont traduites aussi. Exception assumée à l'état unique.
- `:lang` liste les langues, `:lang en` en change ; mémorisé dans `config.toml` (`lang = "..."`). Sans `lang`, la langue d'affichage de Windows si elle existe, sinon l'anglais. Choisie avant d'ouvrir la base : le contenu de départ (catégories, apps) est créé dans cette langue.
- Les récompenses de départ sont stockées en français (migrations publiées) ; `starter_rewards.<code>` les affiche dans la langue courante. Une récompense ajoutée à la main garde son texte.
- Ajouter une langue : un fichier dans `locales/` et une ligne dans `i18n::LANGS`.

### Truecolor et repli

Les couleurs `#rrggbb` exigent un terminal truecolor. Windows Terminal le gère (variable `WT_SESSION` présente), l'ancienne console `conhost` non. Sans truecolor détecté (`WT_SESSION` absent et `COLORTERM` différent de `truecolor`/`24bit`), le thème par défaut est `terminal` : il n'utilise que les 16 couleurs ANSI et suit donc le schéma du terminal. Sinon, le défaut est `catppuccin-mocha`.

### Rendu

- Titres intégrés au cadre (`Block::title`), bordures arrondies. Le fond n'est pas peint : le terminal garde le sien.
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

- `streak_days` compte les jours actifs consécutifs, aujourd'hui inclus : il est calculé une fois la session fermée.
- À la fin d'une session gardée (≥ 60 s), `xp_gained` est enregistré dans `sessions` et ajouté à `apps.total_xp`, dans une même transaction. Les sessions fermées à la sortie et les orphelines fermées au démarrage reçoivent aussi leur XP.
- Le niveau d'une app vient de son `total_xp`, le niveau global de la somme des `total_xp`. Un level-up est détecté en comparant les niveaux avant et après.

Les récompenses sont définies **en données** (table `rewards`), pas en dur. La migration v2 insère un jeu de départ : Premiers pas, Marathon, Noctambule, Habitué, Passionné, Vétéran, Régulier, Assidu, Touche-à-tout, Centurion, Expert. La migration v3 ajoute Inarrêtable (30 jours de suite) et Légende (365 jours de suite).

```
code = "marathon", scope = "app", rule = "session_minutes >= 180"
```

**Langage des règles** (`core/rewards.rs`, testable sans base) : `variable opérateur nombre`, combinées par `&&` et `||` (`&&` prioritaire). Opérateurs `>= > <= < == !=`. Toutes les conditions sont vérifiées, même quand le résultat est déjà connu, pour signaler une règle cassée.

| Variable | Mesure |
|---|---|
| `session_minutes` | durée de la session |
| `session_hour` | heure locale de début (0-23) |
| `app_hours`, `app_sessions`, `app_level` | cumul, nombre de sessions et niveau de l'app |
| `level`, `streak_days` | niveau global, jours consécutifs |
| `total_hours`, `total_sessions` | cumul toutes apps |
| `apps_this_week` | apps différentes sur 7 jours |

**Évaluation** : après chaque session gardée (fin normale, sortie de CmdBoard, orpheline au démarrage), une fois l'XP attribuée pour que les niveaux comptent. `storage` calcule les faits (`session_facts`) et la liste des récompenses encore à débloquer pour l'app (`pending_rewards`), `core::rewards::evaluate` tranche, `storage` enregistre. Une règle cassée n'empêche pas les autres : l'erreur s'affiche dans la ligne de message. `:xp` ne déclenche pas d'évaluation.

---

## 12. Détection des sessions

1. À l'ajout d'une app, enregistrer `watch_exe`. Après chaque `reload()`, l'UI envoie la liste `(app_id, watch_exe)` au tracker.
2. Le tracker interroge `sysinfo` toutes les 3 secondes, et tout de suite quand la liste change. Correspondance sur le nom de fichier de l'exe, sans tenir compte de la casse (`watch_exe` peut contenir un chemin complet). Une app tourne si au moins un de ses process tourne. Plusieurs apps peuvent surveiller le même exe.
3. Process détecté : `SessionStarted`. L'UI insère une ligne `sessions` avec `ended_at = NULL`. Process disparu : `SessionEnded { secs }`, mesuré par le tracker. L'UI ferme la ligne (`ended_at`, `duration_s`). Le calcul de `xp_gained` suit la section 11.
4. Les sessions de moins de 60 s (`MIN_SESSION_SECS`) sont supprimées au lieu d'être enregistrées.
5. Toutes les 60 s, `on_tick` enregistre `duration_s` des sessions en cours (point de sauvegarde).
6. Au démarrage, fermer les sessions orphelines (crash précédent) à leur dernier point de sauvegarde : `ended_at = started_at + duration_s`, ou suppression sous 60 s. Un crash perd donc au plus une minute.
7. À la sortie de CmdBoard, les sessions en cours sont fermées normalement. Une app qui continue de tourner n'est plus suivie.
8. Seules les sessions fermées (`ended_at` non NULL) comptent dans le temps total, la streak et l'XP du jour.

Cas Steam / Epic / Battle.net : la commande de lancement (URI) et le process surveillé sont différents, d'où les deux champs séparés.

---

## 13. Animations (sobres)

Pilotées par `Tick` et un compteur `frame_count` dans `App` :

- Barre d'XP qui se remplit progressivement après une session (ou `:xp`) : `App` garde une `XpAnim { from, to, start }` par app et une pour le profil. Le rendu en déduit le total affiché à partir de `frame_count` (8 ticks, soit 2 s, avec ralenti en fin de course), en repassant par `level_from_total`, donc le niveau affiché monte en même temps que la barre.
- Popups de level-up et de récompense dont la bordure et le titre alternent entre `warning` et `success` à chaque `Tick`.
- Spinner pendant l'import des raccourcis.
- Chrono de session en direct dans le header.

---

## 14. Pièges spécifiques à Windows

1. Événements clavier en double (`Press` / `Release`).
2. Raccourcis `.lnk` à scanner dans :
   - `%ProgramData%\Microsoft\Windows\Start Menu\Programs`
   - `%AppData%\Microsoft\Windows\Start Menu\Programs`
   - le Bureau
   - Steam : dossier lu dans `HKCU\Software\Valve\Steam\SteamPath` ; Epic : `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`.
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
| 11 | Thèmes Catppuccin + TOML, responsive, finitions | Version personnalisable |
| 12 | Distribution : `dist`, release GitHub, `:update`, winget | `v0.1.0` installable (MSI, PowerShell) et mise à jour depuis l'app |

Les migrations (section 3) sont mises en place dès l'étape 4, car les mises à jour de l'étape 12 en dépendent. Le squelette de release (`dist init`) peut être posé plus tôt pour publier des préversions.

L'étape 1 doit inclure un **hook de panic** qui restaure le terminal. `ratatui::init()` le fait dans les versions récentes.

---

## 17. Dépendances

```toml
ratatui = "0.30"
crossterm = "0.29"
rusqlite = { version = "0.40", features = ["bundled"] }
sysinfo = "0.39"
opener = "0.9"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
chrono = "0.4"
anyhow = "1"
directories = "6"
fuzzy-matcher = "0.3"
self_update = { version = "1.3", default-features = false, features = ["github", "ureq", "rustls", "archive-zip", "compression-zip-deflate"] }
windows-sys = { version = "0.61", features = ["Win32_Globalization", "Win32_System_Registry", "Win32_UI_Shell", "Win32_UI_WindowsAndMessaging"] }  # langue de Windows ; registre et ShellExecute (écran Stockage)
```

À vérifier avec `cargo add` au moment de créer le projet, pour obtenir les dernières versions. Pour lire les `.lnk`, ajouter `lnk` ou `parselnk`.

---

## 18. Distribution et mises à jour

### Build et release : `dist`

[`dist`](https://github.com/axodotdev/cargo-dist) (ex-cargo-dist) génère le workflow GitHub Actions et les installeurs.

```sh
cargo install cargo-dist
dist init        # cible x86_64-pc-windows-msvc, installeurs "powershell" et "msi"
dist plan        # affiche ce qui sera publié
```

Configuration attendue (dans `dist-workspace.toml`) :

```toml
[dist]
targets = ["x86_64-pc-windows-msvc"]
installers = ["powershell", "msi"]
install-updater = false   # on utilise self_update, pas axoupdater
```

Publier une version :

1. Monter `version` dans `Cargo.toml`.
2. `git tag v0.2.0` puis `git push --tags`.
3. La CI construit le `.zip`, le `.msi` et `cmdboard-installer.ps1`, puis crée la GitHub Release.

Installation par l'utilisateur :

```powershell
irm https://github.com/Loris01100/CmdBoard/releases/latest/download/cmdboard-installer.ps1 | iex
```

ou en téléchargeant le `.msi` depuis la page Releases.

**À ne jamais changer** : les GUID `upgrade-guid` et `path-guid` que `dist init` écrit dans `Cargo.toml` (`[package.metadata.wix]`). S'ils changent, le MSI n'est plus reconnu comme une mise à jour et installe une seconde copie.

### Mises à jour

Deux canaux, selon le mode d'installation :

| Installé via | Mise à jour |
|---|---|
| Installeur PowerShell ou `.zip` | `:update` dans l'app (crate `self_update`) |
| MSI ou winget | `winget upgrade CmdBoard`, ou nouveau MSI |

**`:update`** (`Command::Update`, logique dans `src/update.rs`) :

1. Interroge la dernière GitHub Release et la compare à `env!("CARGO_PKG_VERSION")`.
2. Si l'exécutable est sous `Program Files` (MSI ou winget), ne remplace rien et affiche `winget upgrade CmdBoard`. Remplacer l'exe exigerait les droits admin et désynchroniserait winget.
3. Sinon, télécharge l'archive Windows et remplace l'exe en cours. Windows verrouille un exe en cours d'exécution : `self_update` contourne ce verrou via `self_replace`. Un message demande ensuite de relancer l'app.

Le téléchargement tourne dans un thread temporaire qui renvoie `AppEvent::UpdateFinished` : l'UI ne bloque jamais. Une vérification passive au démarrage (au plus une fois par jour, désactivable dans `config.toml`) affiche « vX.Y disponible » dans la barre de statut, sans rien installer.

**winget** : publier le MSI dans [winget-pkgs](https://github.com/microsoft/winget-pkgs) avec `wingetcreate`, puis automatiser chaque release avec l'action GitHub `winget-releaser`. winget voit alors passer les nouvelles versions sans intervention.

### Points d'attention

- **SmartScreen** : sans signature de code, Windows affiche « Windows a protégé votre ordinateur » au premier lancement. Acceptable pour un projet perso. Sinon : Azure Trusted Signing.
- **Données** : la base vit dans `%APPDATA%`, hors du dossier d'installation. Elle survit aux mises à jour et aux désinstallations, et les migrations (section 3) font évoluer son schéma.

### Linux (hors périmètre)

CmdBoard est Windows uniquement (`.lnk`, `%APPDATA%`, URI des launchers). winget n'existe pas sous Linux. En cas de portage : `dist` produit aussi un installeur shell et une formule Homebrew, `self_update` fonctionne à l'identique, et des paquets natifs (AUR, `.deb` via `cargo-deb`) confieraient les mises à jour au gestionnaire de paquets de la distribution. Les `.lnk` seraient remplacés par les fichiers `.desktop` de `/usr/share/applications` et `~/.local/share/applications`.

---

## Prochaine étape

Les étapes 1 à 11 sont faites. Étape 12 en cours : `dist init` (`dist-workspace.toml`, `.github/workflows/release.yml`, `wix/main.wxs`), `:update` et la vérification passive quotidienne (`src/update.rs`, clés `update_check` et `last_update_check` dans `config.toml`) sont en place. `self_update` est en 1.x : il faut les features `github` et `ureq` en plus de celles de la section 17, et l'archive est choisie par son nom exact (`cmdboard-x86_64-pc-windows-msvc.zip`), sans quoi le `.msi` ou le `.sha256` pourraient correspondre. Reste : publier `v0.1.0`, puis le manifeste winget.
