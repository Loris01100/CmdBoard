# Plan de conception : dashboard TUI en Rust (ratatui)

Application de lancement de raccourcis avec catégories, XP par session, niveaux et récompenses. Interface style terminal, pour Windows.

---

## 1. Principes directeurs

- **Un seul état central** (`App`), modifié uniquement par une fonction `update`.
- **Le rendu est pur** : `draw(frame, &app)` lit l'état et dessine, sans rien modifier.
- **Tout passe par `Command`** : touches, palette `:` et alias déclenchent le même chemin d'exécution.
- **Aucune logique métier dans `ui/`** : XP, récompenses et stockage vivent dans des modules testables sans terminal.
- **Des couches qui ne dépendent que vers l'intérieur** (domaine ← infrastructure ← application ← présentation), vérifiées par `tests/architecture.rs` (section 2).

---

## 2. Arborescence

```
src/
├── main.rs              // init terminal, lance App::run()
├── app/                 // couche application : struct App et ce qui la modifie
│   ├── mod.rs           // App, Screen, Mode, boucle principale, routage des AppEvent
│   ├── keys.rs          // touches par mode -> Command
│   ├── commands.rs      // chemin d'exécution unique des Command, alias, complétion
│   ├── library.rs       // apps et catégories : sélection, ajout, édition, suppression
│   ├── forms.rs         // choix de l'app installée, formulaires
│   ├── sessions.rs      // sessions suivies, animations d'XP, popups de level-up
│   ├── goals.rs         // objectifs et limites : temps fait, annonces
│   ├── pins.rs          // favoris : :pin, p, Alt+1 à Alt+9
│   ├── background.rs    // passage des sessions au suivi en arrière-plan et retour
│   ├── settings.rs      // thème, tri, langue, alias, export/import, mises à jour
│   ├── storage.rs       // écran Stockage (StorageScreen) : disques, programmes
│   ├── folders.rs       // explorateur de dossiers de l'écran Stockage
│   ├── optimize.rs      // écran Optimisation (OptimizeScreen)
│   ├── sort.rs          // AppSort
│   └── tests/           // un fichier de tests par module
├── event.rs             // thread d'événements (clavier, tick, tracker)
├── i18n.rs              // textes de l'interface : t!(), langue courante
├── command/
│   ├── mod.rs           // enum Command
│   ├── parser.rs        // texte -> Command
│   └── alias.rs         // chargement de commands.toml
├── core/
│   ├── xp.rs            // formules XP / niveaux
│   ├── goals.rs         // objectifs et limites : durées, périodes, états
│   └── rewards.rs       // moteur de règles
├── storage/
│   ├── db.rs            // connexion SQLite, migrations, contenu de départ
│   ├── queries.rs       // CRUD et agrégats (temps joué, profil, récompenses)
│   ├── goals.rs         // objectifs et limites, temps du jour et de la semaine
│   ├── sessions.rs      // sessions : début, points de sauvegarde, fermeture + XP + récompenses
│   ├── backup.rs        // :export / :import en JSON
│   └── models.rs        // App, Category, Session, Reward
├── launcher/
│   ├── folders.rs       // explorateur de l'écran Stockage : liste, mesure, corbeille
│   ├── launch.rs        // lancement (exe, URI)
│   ├── programs.rs      // disques et programmes installés (registre), désinstallation
│   ├── registry.rs      // lecture du registre (API Unicode, sans reg.exe)
│   ├── scan.rs          // import des .lnk et des bibliothèques Steam/Epic
│   └── stores.rs        // GOG, Ubisoft Connect, apps du Store et jeux Xbox
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
├── fr.toml
└── pt.toml              // portugais du Brésil (Windows donne `pt` pour pt-BR comme pt-PT)
.github/workflows/release.yml   // généré par `dist init`
wix/main.wxs                    // installeur MSI, généré par `dist init`
tests/architecture.rs           // couches et tailles de fichiers, vérifiées en CI
clippy.toml                     // taille maximale d'une fonction
```

### Couches et limites

| Couche | Modules | Ne dépend pas de |
|---|---|---|
| Domaine | `core/` | tout le reste du crate, ni I/O (`std::fs`, threads, SQLite, terminal, Windows) |
| Infrastructure | `storage/`, `launcher/`, `config.rs`, `event.rs`, `instance.rs`, `optimize.rs`, `tracker.rs`, `update.rs` | `app`, `command`, `popup`, `ui`, ratatui |
| Application | `app/`, `command/`, `popup.rs` | SQLite, écrans et widgets de `ui/` |
| Présentation | `ui/` | `Database`, `config`, `tracker`, `update`, lancement, threads |
| Partagé | `i18n.rs`, `fuzzy.rs`, `text_input.rs`, `main.rs` (racine de composition) | — |

- `tests/architecture.rs` lit les sources (hors `#[cfg(test)] mod tests`) : tout fichier de `src/` doit appartenir à une couche, et aucune couche ne doit contenir les chemins qui lui sont interdits. Un nouveau module se classe dans ce fichier.
- Le champ `App::db` est privé : seul `app/` écrit dans la base, `ui/` ne peut pas l'atteindre.
- Taille : au plus 30 fonctions et 600 lignes par fichier, tests exclus (`tests/architecture.rs`), et 100 lignes par fonction (`clippy::too_many_lines`, seuil dans `clippy.toml`). Un test de table long peut s'en exempter avec `#[expect(clippy::too_many_lines, reason = "…")]`. Un fichier qui dépasse se découpe par fonctionnalité, comme `app/`.

---

## 3. Modèle de données (SQLite)

```
categories(id, name, color, icon)
apps(id, name, launch_target, watch_exe, icon, category_id, total_xp, launch_args, pin)
sessions(id, app_id, started_at, ended_at, duration_s, xp_gained, hidden, checkpoint_at)
rewards(id, app_id NULL, code, name, description, rule, scope)
unlocked_rewards(id, reward_id, app_id NULL, unlocked_at, session_id)
goals(id, kind, app_id NULL, category_id NULL, minutes, period)
```

- `launch_target` : chemin ou URI de lancement (`steam://rungameid/...`). Disponibilité (`launch::is_available`, pour les apps de départ) : chemin absolu existant, exe sur le `PATH`, ou URI dont le schéma (RFC 3986 : lettre puis lettres, chiffres, `+ - .`) a une clé `HKEY_CLASSES_ROOT\<schéma>`, lue par l'API registre sans lancer `reg.exe`.
- `watch_exe` : nom de l'exécutable réel à surveiller (utile pour les launchers).
- `launch_args` (migration v7) : arguments passés à la cible au lancement (`-dx12`, un dossier à ouvrir), `NULL` sans argument. Sans argument, la cible part par `opener` comme avant ; avec, par `ShellExecuteW` et ses paramètres (`programs::shell_execute`, partagé avec la désinstallation). La plupart des liens `steam://…` les ignorent. Deux apps peuvent viser la même cible avec des arguments différents (variantes).
- `pin` (migration v7) : emplacement de favori, de 1 à 9 (`CHECK`), lancé par `Alt+<n>`. Index unique partiel (`WHERE pin IS NOT NULL`) : un seul favori par emplacement ; `Database::set_pin` le prend à l'app qui l'avait, dans une transaction.
- `app_id NULL` dans `rewards` : récompense commune à toutes les apps. Sinon : récompense propre à cette app.
- `scope` (migration v2) : `global` se débloque une seule fois en tout ; `app` se débloque une fois par app. Une récompense propre à une app (`app_id` renseigné) est traitée comme `app`.
- `unlocked_rewards.app_id` : l'app pour laquelle une récompense `app` a été débloquée (`NULL` pour une `global`). Index unique sur `(reward_id, IFNULL(app_id, 0))` : pas de double déblocage.
- `sessions.checkpoint_at` (migration v5) : dernier enregistrement d'une session ouverte (début, puis chaque point de sauvegarde). `NULL` pour une session ouverte avant v5. Sert à distinguer une session passée à l'autre process il y a un instant d'une session laissée par un crash (section 12).
- `goals` (migration v6) : objectifs (`kind = 'goal'`, temps à atteindre) et limites (`'limit'`, temps à ne pas dépasser), en minutes par jour ou par semaine (`period = 'day' | 'week'`), sur une app (`app_id`), une catégorie (`category_id`) ou toutes les apps (les deux `NULL`). Index unique sur `(kind, IFNULL(app_id, 0), IFNULL(category_id, 0))` : au plus un objectif et une limite par cible. Supprimer l'app ou la catégorie les supprime (`ON DELETE CASCADE`).
- Le niveau n'est pas stocké : il se déduit de `total_xp` (`core::xp::level_from_total`), ce qui évite toute incohérence. Le niveau global se déduit de la somme des `total_xp`.
- Temps total, dernière session et nombre de récompenses d'une app sont agrégés depuis `sessions` et `unlocked_rewards` à la lecture.
- Horodatages en secondes Unix (`INTEGER`). Les jours (XP du jour, streak) suivent le fuseau local via `date(..., 'unixepoch', 'localtime')`.
- Noms de catégories et d'apps uniques sans tenir compte de la casse. Une catégorie qui contient des apps ne peut pas être supprimée. Supprimer une app supprime ses sessions et récompenses.
- Une base neuve reçoit un contenu de départ (catégories Jeux, Dev, Outils et quelques apps Windows) pour avoir de quoi lancer dès le premier démarrage.
- **Migrations** : le schéma est versionné via `PRAGMA user_version`. Au démarrage, `storage/db.rs` applique dans l'ordre les migrations manquantes. Une mise à jour de l'app ne doit jamais perdre les données de `%APPDATA%` : on ne modifie jamais une migration déjà publiée, on en ajoute une nouvelle.

---

## 4. Modèle d'état

```rust
pub enum Screen { Dashboard, Stats, Rewards, Storage, Optimize, Help }

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
    db: Database,                       // privé : seul app/ écrit dans la base

    // écrans avec un état propre
    pub storage: StorageScreen,         // disques, programmes, explorateur (app/storage.rs)
    pub optimize: OptimizeScreen,       // PC, tests, réglages de jeu (app/optimize.rs)
    pub should_quit: bool,
}
```

---

## 5. Boucle d'événements

```rust
pub enum AppEvent {
    Key(KeyEvent),
    Resize,                                // terminal redimensionné : redessiner
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
    BenchFinished { bench, heavy, result },   // écran Optimisation : un test terminé
}

fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>, events: &Receiver<AppEvent>) -> Result<()> {
    while !self.should_quit {
        if mem::take(&mut self.redraw) {
            terminal.draw(|f| ui::draw(f, self))?;
        }
        let first = events.recv()?;
        // L'événement reçu, puis tous ceux déjà en attente (256 au plus), avant de redessiner.
        for event in iter::once(first).chain(events.try_iter().take(MAX_BATCH)) {
            if matches!(event, AppEvent::Tick) && !self.redraw {
                self.redraw = ui::animates(self, taille_du_terminal);
            }
            self.handle(event);                    // Key déjà filtré sur Press par event.rs
            if self.should_quit { break; }
        }
    }
    self.end_all_sessions()                        // ferme les sessions en cours à la sortie
}
```

**Redessin seulement si l'écran change.** `App::redraw` est levé par tout événement autre que `Tick` (touches, `Resize`, sessions, threads temporaires). Un `Tick` ne le lève que si quelque chose bouge (`on_tick`) : barre d'XP en cours de remplissage (jusqu'à sa dernière image), popup de level-up ou de récompense qui clignote, chrono de session dont la seconde affichée change, changement de minute (les « il y a … »). Le défilement du fil d'activité dépend de la mise en page : `ui::animates` dit, sans dessiner, s'il déborde de son panneau (fonction pure de `App` et de la taille du terminal). Au repos, un tick ne dessine donc rien. Les événements en attente sont traités ensemble puis dessinés une fois : la mesure d'un dossier (un `FolderSized` par sous-dossier) ne coûte plus un dessin par événement. `run` est générique sur le `Backend` pour être testé avec `TestBackend`.

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
    Edit { app: String, name: String, target: String, category: String, watch_exe: Option<String> }, // idem, garde l'historique
    Help { command: Option<String> },
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
| `edit` | | `edit <app>` (le reste de la ligne ; ouvre le formulaire de modification) |
| `rm` | `delete` | `rm <app>` (confirmation, supprime aussi sessions et récompenses) |
| `rmcat` | | `rmcat <catégorie>` (catégorie vide uniquement, confirmation) |
| `clear` | | `clear sessions` (masque les sessions terminées de l'historique, colonne `sessions.hidden` (migration v4), les stats les comptent toujours) ou `clear stats` (supprime les sessions terminées : stats et série repartent de zéro) ; XP et récompenses conservées ; confirmation |
| `stats` | | `stats [app]` (sans argument : toutes les apps ; avec : filtre jusqu'au prochain `:stats`) |
| `theme` | | `theme [nom]` (sans nom : liste les thèmes et l'actuel ; nom complété par Tab) |
| `group` | | `group <nom> <app>, <app>…` (apps séparées par des virgules, sans guillemets ; écrit l'alias `<nom> = "launch A; launch B"` dans `commands.toml`, le remplace s'il existe, et le recharge aussitôt) |
| `goal` | | `goal [<durée>/day\|week\|off] [app\|catégorie]` (sans argument : liste les objectifs et le temps fait ; la durée d'abord pour que la cible, le reste de la ligne, se passe de guillemets ; sans cible : toutes les apps ; `off` retire ; voir section 11) |
| `pin` | | `pin [<1-9>\|off <app>]` (sans argument : liste les favoris ; l'emplacement d'abord pour qu'un nom commençant par un chiffre, `7 Days to Die`, reste entier ; un emplacement pris change de main ; `off` retire ; `p` sur le dashboard prend le premier libre ou retire) |
| `limit` | | `limit [<durée>/day\|week\|off] [app\|catégorie]` (même syntaxe, pour une limite) |
| `export` | | `export [fichier]` (JSON des apps et sessions terminées ; sans argument : `Documents\cmdboard-<aaaa-mm-jj>.json` ; fichier existant : confirmation avant de le remplacer, la commande confirmée porte le chemin résolu) |
| `import` | | `import <fichier>` (fusionne un export ; confirmation s'il ajoute des apps, voir ci-dessous) |
| `uninstall` | | `uninstall <programme>` (programme installé, complété par Tab ; confirmation, puis lance son propre désinstalleur, voir écran Stockage) |
| `help` | `h`, `?` | `help [commande]` |
| `quit` | `q` | `quit` |

Noms d'apps et de catégories insensibles à la casse. `:add` déduit `watch_exe` du nom de fichier quand la cible est un `.exe`, et refuse un chemin absolu inexistant. Après `:add`, `:move` ou `:edit`, la sélection suit l'app. `:edit` change nom, cible, catégorie et process d'une app sans toucher à son id (`Database::update_app`), donc sessions, XP et récompenses restent ; mêmes vérifications que `:add` (nom déjà pris par une autre app, mais changer la casse du sien est permis ; chemin absolu inexistant ; catégorie créée si absente ; process vide déduit de la cible). Un alias de `commands.toml` qui citait l'ancien nom n'est pas réécrit. Si le process change pendant une session, le tracker voit l'ancien disparaître : la session se ferme normalement.

`:export` / `:import` (`storage/backup.rs`) servent à la sauvegarde, à l'analyse externe et au changement de PC. Le fichier : `{ version: 1, exported_at, apps: [{ name, category, launch_target, watch_exe, launch_args, total_xp }], sessions: [{ app, started_at, ended_at, duration_s, xp_gained }] }` (sessions en cours exclues, horodatages Unix). L'import ne supprime rien, mais un fichier reçu de quelqu'un d'autre peut ajouter une app dont la cible lance n'importe quoi : s'il ajoute des apps, une confirmation les liste d'abord (`Database::import_preview`, « • nom → cible », 5 au plus puis « … et N de plus », caractères de contrôle remplacés par des espaces pour qu'un saut de ligne ne cache rien) ; sinon il s'applique directement. Il fusionne en une transaction : une app absente est créée avec sa catégorie et son `total_xp` ; une app déjà présente (même nom) garde sa cible et sa catégorie, et gagne l'XP de ses sessions nouvellement importées ; une session déjà présente (même app, même début) est ignorée, donc réimporter le même fichier ne change rien. Les récompenses ne sont pas exportées.

### Confort

- La saisie s'affiche dans un cadre « Commande » (bordure de focus) qui s'ouvre au-dessus de la barre de statut, sur tous les écrans, avec un texte d'exemple quand la ligne est vide et un défilement horizontal qui garde le curseur visible. Hors saisie, cette zone se réduit à une ligne de message.
- Historique avec flèches haut/bas (100 entrées, sans doublon consécutif, en mémoire seulement).
- Édition : `←→`, `Home`/`End`, `Backspace`/`Suppr`. `Backspace` sur une ligne vide ou `Esc` referment la ligne.
- Autocomplétion avec Tab (`command/complete.rs`, fonction pure) : nom de commande ou d'alias en premier mot (suivi d'un espace), puis selon la commande : app (`launch`, `rm`, `stats`, reste de la ligne), catégorie (`rmcat`, 2e argument de `move`, 3e de `add`), commande (`help`), `sessions`/`stats` (`clear`). Candidats classés par `fuzzy-matcher` (`src/fuzzy.rs`, partagé avec la recherche), mis entre guillemets s'ils contiennent un espace. `Tab` répété passe au suivant, `Shift-Tab` au précédent, toute autre touche repart de zéro. Les candidats s'affichent sur la bordure basse du cadre, le courant en surbrillance.
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
| Normal | `1 2 3 4 5` | Changer d'écran : Dashboard, Stats, Récompenses, Stockage, Optimisation |
| Normal | `0` / `?` | Aide |
| Normal | `a` / `e` / `m` | Formulaire d'ajout / de modification / de déplacement de l'app sélectionnée |
| Normal | `d` | Supprimer l'app sélectionnée, ou la catégorie si le focus y est (vide uniquement) |
| Normal | `s` | Tri suivant des apps (`Command::Sort`) : nom, XP, récent, temps |
| Normal (Dashboard) | `p` | Épingle l'app sélectionnée au premier favori libre, ou la retire si elle en est un (`Command::Pin`, `PinChange::Toggle`, touche seule) |
| Normal (tous les écrans) | `Alt+1` … `Alt+9` | Lance le favori de cet emplacement (`Command::LaunchPin`, touche seule), sinon dit comment le remplir. Lu avant les touches d'écran : `1` seul change d'écran, `Alt+1` lance. En AZERTY, les chiffres demandent `Maj` : la rangée sans `Maj` (`& é " ' ( - è _ ç`) compte aussi, et `Alt+Maj+1` arrive comme `1`. `AltGr` (`Ctrl+Alt`) est ignoré, il tape des caractères. Vérifier une disposition avec `cargo run --example keys`. |
| Normal (Stats) | `s` | Camembert par catégorie ↔ par app (`Command::ToggleStatsPie`) |
| Normal (Stockage) | `Tab` `→` `l` / `Shift-Tab` `←` `h` | Disque suivant / précédent, « tous les disques » avant le premier (`Command::CycleDisk`) |
| Normal (Stockage) | `s` | Plus gros ↔ plus petits d'abord (`Command::ToggleStorageOrder`) |
| Normal (Stockage) | `d` / `Suppr` | Désinstaller le programme sélectionné (`Command::Uninstall`, confirmation) |
| Normal (Stockage) | `f` | Explorateur de dossiers ↔ programmes (`Command::ToggleFolders`) |
| Normal (Stockage, deux vues) | `r` | Actualiser : relit disques et programmes, et le dossier affiché en remesurant ses tailles (`Command::RefreshStorage`, touche seule) |
| Normal (Stockage, dossiers) | `Entrée` `→` `l` / `Backspace` `←` `h` | Ouvrir le dossier (`Command::OpenFolder`) / remonter, puis revenir aux disques (`Command::ParentFolder`) |
| Normal (Stockage, dossiers) | `s` | Plus gros ↔ plus petits d'abord |
| Normal (Stockage, dossiers) | `d` / `Suppr` | Dossier d'un programme : `Command::Uninstall` ; sinon `Command::Trash` (corbeille, confirmation) |
| Normal (Optimisation) | `Tab` | Tests ↔ réglages Gaming (`Command::ToggleFocus`, état `gaming_focus`) |
| Normal (Optimisation) | `Entrée` | Lancer le test sélectionné (`Command::Bench`) / basculer le réglage (`Command::ToggleGaming`) |
| Normal (Optimisation) | `n` / `o` | Test rapide (3 s) ↔ complet (15 s) (`Command::ToggleBenchLevel`) / page Windows du réglage (`Command::OpenGamingPage`) |
| Normal (Optimisation) | `a` | Lance les quatre tests à la suite, au niveau courant (`Command::BenchAll`, file `queue`) |
| Command | `Enter` / `Esc` | Valider / annuler |
| Command | `↑↓` / `Tab` `Shift-Tab` | Historique / autocomplétion |
| Search | saisie | Filtre le panneau Applications (toutes catégories, meilleur résultat en tête et sélectionné) |
| Search | `↑↓` / `Tab` | Choisir parmi les résultats |
| Search | `Enter` / `Esc` | `Command::Select` (catégorie et app sélectionnées, focus sur les apps) / annuler et restaurer la sélection |
| Popup (confirmation) | `Enter` `o` `y` / `Esc` `n` | Confirmer / annuler |
| Popup (formulaire) | `Tab` `↓` / `Shift-Tab` `↑` | Champ suivant / précédent |
| Popup (formulaire) | `Enter` / `Esc` | Champ suivant, valider sur le dernier / annuler |

### Popups (`src/popup.rs`, rendu dans `ui/widgets/popup.rs`)

- **Confirmation** : toute commande destructive (`RemoveApp`, `RemoveCategory`, `Uninstall`, `Trash`, `ClearSessions`, `ClearStats`, et `Export` quand le fichier existe déjà) porte un champ `confirmed`. Non confirmée, son exécution ouvre une popup qui contient la même commande avec `confirmed: true`. Touche `d` et `:rm` passent donc par la même confirmation.
- **Formulaires** : `Form` = liste de champs (`TextInput`, partagé avec la ligne de commande) avec un champ focalisé. La validation produit une `Command` (`Add`, `Edit`, `Move`) exécutée par le chemin habituel. En cas d'erreur (champ requis, nom déjà pris, fichier introuvable), le formulaire reste ouvert et affiche l'erreur ; le premier champ requis vide reçoit le focus. Champs des formulaires d'ajout et de modification : nom, cible, catégorie, process, puis **arguments** (facultatif, en dernier : `launch_args`). `:add` en ligne de commande n'en prend pas : on les ajoute avec `e`.
- **Choix de l'app** (`Popup::Picker`) : `a` et `:add` sans argument ouvrent d'abord une liste filtrable (fuzzy) des apps installées, lue par `launcher/scan.rs` dans les bibliothèques Steam (`libraryfolders.vdf` puis `appmanifest_*.acf` complètement installés : cible `steam://rungameid/<id>`, process = le plus gros exe du dossier du jeu, jusqu'à trois niveaux) et Epic (manifests `.item` de `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`, hors DLC : cible `com.epicgames.launcher://apps/...?action=launch`, process = `LaunchExecutable`), puis (`launcher/stores.rs`) GOG (clés `HKLM\SOFTWARE\GOG.com\Games\<id>`, vue 32 bits, hors DLC et mods (`dependsOn`) et exe absents : cible = l'exe du jeu, qui se lance sans GOG Galaxy, process = son nom), Ubisoft Connect (`HKLM\SOFTWARE\Ubisoft\Launcher\Installs\<id>`, vue 32 bits : cible `uplay://launch/<id>/0`, nom = `DisplayName` de l'entrée de désinstallation `Uplay Install <id>`, sinon le nom du dossier, process deviné comme pour Steam), les apps du Store et les jeux Xbox (un Windows PowerShell caché, `CREATE_NO_WINDOW` pour ne pas toucher à la console de CmdBoard, environ une seconde : `Get-StartApps` pour les noms affichés et les AppID packagés (`famille!app`), `Get-AppxPackage` pour le dossier du paquet ; cible `shell:AppsFolder\<AppID>`, ouverte par `ShellExecuteW` ; process = `Executable` de `MicrosoftGame.config` pour un jeu Xbox, dont le manifeste ne nomme que `GameLaunchHelper.exe`, sinon l'attribut `Executable` de l'`<Application Id=…>` de `AppxManifest.xml`, commentaires XML ignorés), et enfin dans les raccourcis `.lnk` et `.url` du menu Démarrer et du Bureau. EA app et Battle.net n'ont pas de bibliothèque simple à lire (base protobuf, XML par jeu) : leurs jeux viennent des raccourcis qu'ils créent. À nom égal, l'entrée de bibliothèque l'emporte. Le scan tourne dans un thread temporaire (`AppEvent::ShortcutsScanned`) à chaque ouverture ; les six sources et la lecture des fichiers sont parallélisées avec rayon (ordre conservé, donc la priorité des bibliothèques aussi). Les apps déjà ajoutées sont masquées. `Entrée` ouvre le formulaire pré-rempli (cible = le `.lnk`, qui garde ses arguments ; process = l'exe pointé par le raccourci), focus sur la catégorie. `Tab`, ou `Entrée` sans résultat, ouvre le formulaire vide avec la recherche comme nom.
- Formulaire d'ajout : Nom*, Cible*, Catégorie* (pré-remplie avec la catégorie sélectionnée), Process (vide : déduit de la cible, affiché en grisé « auto : X.exe »). Une ligne d'aide sous les champs explique le champ actif, notamment à quoi sert Process (l'exe surveillé pour compter le temps de jeu).
- Formulaire de modification (`FormKind::Edit`, touche `e` ou `:edit <app>`) : mêmes champs, pré-remplis avec l'app, focus sur le nom. Vider Process le déduit à nouveau de la cible.
- **Level-up** : ouverte quand une app ou le profil gagne un niveau (fin de session). Bordure qui alterne de couleur à chaque `Tick`. `Entrée`, `Esc` ou `Espace` la ferment.
- **Récompense débloquée** : une popup par récompense, après celle de level-up, mêmes touches et même clignotement.
- **Objectif ou limite atteint** (`Popup::GoalReached`) : une popup la première fois qu'un objectif (bordure `success`) ou une limite (bordure `error`) est atteint dans sa période, mêmes touches que le level-up, sans clignotement.
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
├ Activité : ▶ Hades 1h (+90 XP) · hier   🏆 Marathon   ⚡ CPU… ◀──┤
├ Message / ligne de commande ────────────────────────────────────┤
└ Barre de statut : raccourcis contextuels ───────────────────────┘
```

```rust
let rows = Layout::vertical([
    Constraint::Length(1),   // header
    Constraint::Min(10),     // corps
    Constraint::Length(3),   // profil
    Constraint::Length(3),   // activité (bandeau défilant)
    Constraint::Length(1),   // ligne de commande / message
    Constraint::Length(1),   // statut
]).split(area);

let cols = Layout::horizontal([
    Constraint::Length(20),      // catégories
    Constraint::Percentage(55),  // applications
    Constraint::Min(25),         // détails
]).split(rows[1]);
```

La liste des catégories commence par une ligne **Récents** (`★ Récents (n)`, `apps.recent`) : les 8 apps dont la dernière session s'est terminée le plus tard (`App::recent_apps`, `RECENT_APPS`), de la plus récente à la plus ancienne, quel que soit le tri, toutes catégories confondues. Ce n'est pas une catégorie : `cat_state` vaut 0 sur cette ligne et les catégories sont décalées d'un rang (`App::selected_category` renvoie `None` sur « Récents », donc `d` et la catégorie par défaut de `a` n'y font rien ; `App::category_rows`). Au démarrage, la sélection est sur « Récents » si une app a déjà été jouée, sinon sur la première catégorie (`clamp_category_row`) : ouvrir CmdBoard puis `Entrée` `Entrée` relance le dernier jeu. Aller à une app par son nom (`/`, `:add`, `:edit`) sélectionne sa vraie catégorie. Dans le panneau Applications, un favori affiche son emplacement devant le nom (`1 Steam`, en `title`) ; les autres noms sont décalés d'autant pour rester alignés.

Le panneau **Détails** liste, sous les récompenses, les objectifs et limites que compte l'app sélectionnée (les siens, ceux de sa catégorie, ceux de toutes les apps), sur deux lignes chacun : « ◎ Limite · Jeux » puis « 1h52 / 2h aujourd'hui ». Couleur : objectif atteint en `success`, limite à 80 % en `warning`, atteinte en `error`. Le header affiche la limite la plus proche ou la plus dépassée, dès 80 % : « ⚠ Jeux 1h52/2h », même couleur, visible aussi quand le panneau Détails est masqué.

Le panneau **Activité** mélange les 10 derniers événements, du plus récent au plus ancien : sessions terminées non masquées (app, durée, XP, il y a combien de temps) et récompenses débloquées (`Database::activity`, rechargé à chaque `reload()`, une récompense avant la session qui l'a débloquée), puis le verdict du dernier résultat réussi de chaque benchmark de la session en cours (non enregistré). S'il dépasse la largeur, il défile de droite à gauche en boucle, d'une cellule par `Tick` (`frame_count`), sans état supplémentaire ; sinon il reste fixe.

### Autres écrans

- **Stats** : ligne de résumé (portée, nombre de sessions, temps total, plus longue session), temps par catégorie ou par app (un camembert en braille via `Canvas`, `s` bascule entre les deux via `Command::ToggleStatsPie`, état `App::stats_by_app` ; couleurs `xp_fill`/`info`/`warning`/`error`, au-delà le reste est regroupé en « Autres » en `muted`, légende avec durée et % ; masqué sous 60 colonnes), heatmap d'activité des 12 dernières semaines façon GitHub sur la moitié droite (une colonne par semaine, lundi en haut, aujourd'hui en bas à droite, jours de la semaine à gauche, date du lundi au-dessus des colonnes, un carré `■` par jour dont la couleur dit le temps joué (`Theme::heat` : rien en `muted`, < 22 min `success`, < 45 min `warning`, < 1 h `caution`, au-delà `error`), légende des durées par bloc dans la bordure du bas ; `Stats::today` donne le jour local en jours depuis 1970), historique des 200 dernières sessions (`Table`, sélection `stats_state`, `j`/`k`). `:stats <app>` filtre tout l'écran sur une app. Les données (`Database::stats`) sont rechargées avec le reste à chaque `reload()`.
- **Rewards** : tableau des récompenses (🏆 débloquées en couleur, 🔒 verrouillées en gris), titre « Récompenses (n/total) », sélection propre (`reward_state`, `j`/`k`). Un panneau Détail montre la portée, la condition (`rule`) et qui l'a débloquée, et quand.
- **Stockage** (`4`) : en haut, une jauge par disque (`LineGauge`, % utilisé ; `success`, `warning` dès 75 %, `error` dès 90 %) avec utilisé / total / libre ; le disque filtré est marqué `>`. En dessous, les programmes installés (nom, taille, disque, éditeur), triés par taille (plus gros d'abord par défaut, tailles inconnues toujours à la fin, égalités par nom), sur un disque ou tous. Sélection propre (`storage_state`, `j`/`k`), filtre `storage_disk`, ordre `storage_ascending`, non mémorisés. Les listes (programmes, éléments de l'explorateur) sont tenues dans l'ordre d'affichage quand elles changent (lecture, mesure, changement d'ordre), plutôt que retriées à chaque dessin et à chaque touche. Les données viennent de `launcher/programs.rs` : disques via `sysinfo::Disks`, programmes via les clés `HKLM\...\CurrentVersion\Uninstall` (vues 64 et 32 bits) et `HKCU` (ce que lit « Applications installées » de Windows), lues avec l'API registre de `windows-sys` (Unicode, sans lancer `reg`). Sont écartés : `SystemComponent = 1`, mises à jour (`ParentKeyName`, `ReleaseType` Update/Hotfix), entrées sans `DisplayName` ou `UninstallString` ; doublons par nom. Taille = `EstimatedSize` (déclarée par l'installeur, parfois absente ou approximative : pas de calcul de dossier pour l'instant). Disque = lettre de `InstallLocation`, sinon de `DisplayIcon` ; beaucoup de MSI n'en ont pas et n'apparaissent que sous « tous les disques ». Les apps du Store (MSIX) ne sont pas listées. Lecture dans un thread temporaire (`AppEvent::StorageScanned`) au démarrage (pour la complétion de `:uninstall`) et à chaque ouverture de l'écran. **Désinstallation** : `UninstallString` découpée en exe + arguments (exe entre guillemets, ou chemin non cité jusqu'à `.exe`), lancée par `ShellExecuteW` pour que Windows demande l'élévation (UAC) si besoin. La confirmation affiche la commande exacte : sous `HKCU`, n'importe quel programme de l'utilisateur peut la réécrire. CmdBoard n'attend pas la fin : `r` (ou rouvrir l'écran avec `4`) actualise la liste. Disques masqués sous 12 lignes de corps.
- **Stockage, explorateur** (`f`) : remplace la liste des programmes (état `App::folders`, `None` = vue programmes). Part du disque filtré, sinon de la liste des disques (taille = espace utilisé). Rien n'est mesuré à l'avance : ouvrir un dossier le lit dans un thread temporaire (`FolderListed` : fichiers avec leur taille, sous-dossiers à « … »), puis mesure chaque sous-dossier en parallèle (rayon ; `FolderProgress` affiche à la place de « … » le % de ses enfants directs déjà mesurés ; `FolderSized` un par un, l'élément mesuré est déplacé à sa place et la sélection le suit). Quitter le dossier arrête ses mesures (`AtomicBool` d'annulation). Les tailles mesurées restent en cache (`folder_sizes`) le temps de la session, pour remonter et redescendre sans tout recompter ; un envoi à la corbeille invalide l'élément et ses dossiers parents, et relit les disques. `r` relit le dossier affiché et oublie les tailles de ce dossier, de son contenu et de ses parents (toutes sur la liste des disques) : pour ce qui a changé hors de CmdBoard (désinstalleur terminé, fichier supprimé dans l'Explorateur, corbeille vidée). La liste des disques de l'explorateur suit chaque relecture des disques. Un élément envoyé à la corbeille occupe toujours le disque tant que la corbeille n'est pas vidée. Taille = somme des tailles logiques des fichiers (pas la taille sur disque) ; dossiers illisibles comptés vides ; liens symboliques et jonctions ignorés (certains bouclent, comme `Application Data`). Une colonne « Programme » signale le dossier d'installation d'un programme (`InstallLocation`) : `d` le désinstalle au lieu de le supprimer. Ailleurs, `d` envoie fichier ou dossier à la corbeille (`SHFileOperationW`, `FOF_ALLOWUNDO`, Windows prévient si l'élément est trop gros pour la corbeille), dans un thread temporaire (`Trashed`). Les disques eux-mêmes ne se suppriment pas. **Dossiers protégés** (`folders::is_protected`, vérifié avant la confirmation puis à nouveau dans `folders::trash`) : racine d'un disque, tout ce qui est dans Windows (`%SystemRoot%`) ou dans `%APPDATA%\CmdBoard` (la base), et tout dossier qui contient Program Files (x86 inclus), ProgramData, le dossier utilisateur, `%PUBLIC%` ou l'exe de CmdBoard. Les chemins sont résolus (`canonicalize`, minuscules) pour que noms courts, `..` et jonctions ne contournent pas la règle ; un dossier d'un jeu dans Program Files reste supprimable.
- **Optimisation** (`5`, `src/optimize.rs`) : rien n'est enregistré ni ne rapporte d'XP. En haut, le strict nécessaire pour lire les scores (processeur avec cœurs / threads, mémoire totale, via `sysinfo`, lu à la première ouverture ; masqué sous 14 lignes de corps). Puis deux tableaux, côte à côte dès 90 colonnes, sinon empilés : **Tests** (CPU 1 cœur et tous les cœurs : pas xorshift pendant 3 s / 15 s, en M op/s ; mémoire : copie d'un tampon de 256 Mo / 1 Go pendant 3 s / 15 s, allocation via `try_reserve_exact` pour échouer proprement ; disque : écriture puis lecture d'un fichier de 256 Mo / 2 Go dans `%TEMP%` sans cache Windows (`FILE_FLAG_NO_BUFFERING`, tampon aligné sur 4 Kio), refusé sous deux fois sa taille d'espace libre, fichier supprimé ensuite). Un test à la fois, dans un thread temporaire (`BenchFinished`) ; le dernier résultat de chaque test reste affiché le temps de la session. Le titre du panneau donne le mode (« Test rapide (3 s) » ou « Test complet (15 s) », `n` pour changer ; le disque dépend de sa taille, 256 Mo ou 2 Go, pas d'une durée) ; pas de colonne Niveau : un résultat mesuré dans l'autre mode porte une marque grise après le nom du test (« · complet » / « · rapide »). Chaque résultat se lit d'abord comme un verdict : une jauge de 10 cellules et un mot (Lent, Correct, Rapide, Très rapide ; pour le disque, le type probable : disque dur, SSD SATA, SSD NVMe, NVMe rapide), en couleur `error` / `warning` / `success` (`optimize::rate`, seuils dans `Bench::thresholds` : 400 / 550 / 750 M op/s sur un cœur, soit environ 160 × la fréquence en GHz ; 2 000 / 4 500 / 9 000 sur tous les cœurs ; 5 / 12 / 25 Go/s de copie mémoire ; 200 Mo/s / 1 / 2,5 Go/s en lecture disque ; un quart de jauge par palier, logarithmique à l'intérieur). La mesure brute suit en gris dans la colonne Mesure, masquée quand la largeur manque. **Tout tester** (`a`) : les quatre tests à la suite, le suivant lancé à chaque `BenchFinished` (`App::on_bench_finished`, même niveau que le premier même si `n` change entre-temps) ; les tests en file affichent « en attente ». Sous le tableau (si le panneau a 9 lignes intérieures), une phrase résume le PC dès que les quatre ont un résultat réussi (`optimize::summarize`) : palier de la moyenne des jauges (« PC un peu juste pour jouer », « PC correct pour jouer », « Bon PC de jeu », « Excellent PC de jeu », même couleur que les verdicts) et point faible = jauge la plus basse, omis si les quatre sont au même palier ; sinon, en gris, l'invitation à appuyer sur `a`. Les seuils CPU valent pour une build release : une build debug (`cargo run`) mesure environ 4 fois moins. **Gaming** : Mode Jeu (`HKCU\Software\Microsoft\GameBar\AutoGameModeEnabled`), enregistrement en arrière-plan (`HKCU\System\GameConfigStore\GameDVR_Enabled` et `...\CurrentVersion\GameDVR\AppCaptureEnabled`), lus à chaque ouverture (valeur absente = défaut Windows, activé) et basculés par `Entrée` via `RegSetKeyValueW`. La planification GPU (`HKLM\...\GraphicsDrivers\HwSchMode`, 2 = activée) n'est que lue : elle demande les droits admin et un redémarrage, `Entrée` ouvre donc sa page `ms-settings:`. GPU non testé (demanderait une dépendance graphique).
- **Help** (`0` ou `?`) : commandes et raccourcis, générés à partir du parser.

### Responsive

- Moins de ~90 colonnes : masquer le panneau Détails.
- Moins de 60 colonnes : catégories (5 lignes) au-dessus des apps.
- Moins de 24 lignes : masquer « Activité » ; moins de 18 : masquer aussi le profil. Les listes gardent la place.
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

**Icônes** : toutes dans `ui/icons.rs` (constantes : session ▶, trophée 🏆, cadenas 🔒, benchmark ⚡, étoile ★, puce ●, carré ■), jamais en dur dans les widgets ni dans les traductions (placeholder `{icon}`). Les emojis gardent leurs propres couleurs ; les symboles texte prennent celles du thème.

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
- À la fin d'une session gardée (≥ 60 s), `Database::close_session` ferme la ligne, enregistre `xp_gained`, l'ajoute à `apps.total_xp` et débloque les récompenses dans **une seule transaction** : une erreur ou un crash laisse la session ouverte, et la récupération des orphelines au démarrage la récompense. Une session n'est jamais fermée sans son XP. Les sessions fermées à la sortie et les orphelines (`Database::recover_orphan_sessions`, une transaction pour toutes) reçoivent aussi leur XP.
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

**Évaluation** : après chaque session gardée (fin normale, sortie de CmdBoard, orpheline au démarrage), une fois l'XP attribuée pour que les niveaux comptent. `storage/sessions.rs` calcule les faits (`session_facts_at`) et la liste des récompenses encore à débloquer pour l'app (`pending_rewards`), `core::rewards::evaluate` tranche, `storage` enregistre, dans la transaction de fermeture. `app/sessions.rs` n'affiche que le résultat (`SessionOutcome`) : animations, popups, message. Une règle cassée n'empêche pas les autres : l'erreur s'affiche dans la ligne de message.

---

### Objectifs et limites (`:goal`, `:limit`)

Temps de jeu par jour ou par semaine, sur une app, une catégorie ou toutes les apps : un **objectif** est atteint en jouant assez, une **limite** en jouant trop. Ils ne rapportent ni XP ni récompense, et n'empêchent rien de se lancer : ce sont des repères.

- **Syntaxe de la durée** (`core/goals.rs`, `parse_amount`) : `2h/day`, `1h30/week`, `90m/d`, `45/w` (minutes) ; refusée si nulle ou plus longue que la période (`25h/day`). L'affichage (`format_hm` : `1h05`, `45m`, `2h`) se relit avec la même syntaxe.
- **Temps compté** (`App::goal_secs`) : sessions terminées de la période (`Database::usage`, jour local et semaine du lundi au dimanche, une session compte le jour de sa fin, comme l'XP du jour) plus les sessions en cours, pour les apps de la cible. Relu à chaque `reload()` et chaque minute (changement de jour).
- **États** (`core::goals::status`) : en dessous, proche (80 %), atteint (100 %).
- **Annonce** (`App::check_goals`) : à chaque progression de session, fin de session et minute, un objectif ou une limite atteint pour la première fois dans sa période (`(id, jour ou lundi)` gardé en mémoire) ouvre une popup. Ce qui est déjà atteint au démarrage, à la reprise des sessions ou au moment où on le définit n'est pas annoncé. Le suivi en arrière-plan (section 12) ne montre rien : seuls le header et le panneau Détails le signalent au retour.

## 12. Détection des sessions

1. À l'ajout d'une app, enregistrer `watch_exe`. Après chaque `reload()`, l'UI envoie la liste `(app_id, watch_exe)` au tracker.
2. Le tracker interroge `sysinfo` toutes les 3 secondes, et tout de suite quand la liste change. Correspondance sur le nom de fichier de l'exe, sans tenir compte de la casse (`watch_exe` peut contenir un chemin complet). Une app tourne si au moins un de ses process tourne. Plusieurs apps peuvent surveiller le même exe.
3. Process détecté : `SessionStarted`. L'UI insère une ligne `sessions` avec `ended_at = NULL`. Process disparu : `SessionEnded { secs }`, mesuré par le tracker. L'UI appelle `Database::close_session`, qui ferme la ligne (`ended_at`, `duration_s`) et la récompense en une transaction (section 11).
4. **Temps réellement joué** : le tracker cumule lui-même le temps de chaque session, poll par poll, et l'envoie à chaque poll (`SessionProgress { played, idle }`) ; l'UI s'en sert pour le chrono du bandeau, les points de sauvegarde et la fermeture à la sortie. Un poll ne compte rien quand l'utilisateur est inactif : aucune entrée clavier / souris (`GetLastInputInfo`) ni manette XInput (changement de `dwPacketNumber`, Xbox et la plupart des autres via Steam Input) depuis `idle_minutes` minutes (`config.toml`, 10 par défaut, `0` désactive). Les entrées ne sont lues que si une session tourne. Jusqu'à `idle_minutes` d'inactivité comptent donc avant la pause. Un écart entre deux polls est plafonné à 10 s (`MAX_STEP`) : une mise en veille avec le jeu ouvert ne compte pas. En pause, le chrono s'arrête et affiche « en pause » (`session.idle`).
5. Les sessions de moins de 60 s (`MIN_SESSION_SECS`) sont supprimées au lieu d'être enregistrées.
6. Toutes les 60 s, `on_tick` enregistre `duration_s` des sessions en cours (point de sauvegarde).
7. Au démarrage (`App::recover_sessions`), une session ouverte est **reprise** si son app tourne encore (`tracker::running_now`, une lecture des process) et si son `checkpoint_at` date de moins de 5 min (`HANDOVER_WINDOW_SECS`) : c'est l'autre process qui vient de la passer (point 8). Elle garde sa ligne et son temps joué, et le tracker la poursuit sans nouveau `SessionStarted` (`tracker::spawn` reçoit le temps déjà joué par app). Les autres sont orphelines (crash, extinction du PC) : fermées à leur dernier point de sauvegarde, `ended_at = started_at + duration_s`, ou supprimées sous 60 s. Un crash perd donc au plus une minute.
8. **Suivi en arrière-plan** (`app/background.rs`, `config.toml` : `background = true` par défaut). En quittant, l'UI lance `cmdboard --background` (`instance::spawn_background` : `DETACHED_PROCESS`, sans console, sorti du job du terminal quand c'est permis, pour survivre à la fermeture de la fenêtre), lui passe ses sessions en cours (`hand_over_sessions` : point de sauvegarde, ligne laissée ouverte), puis affiche `background.started`. Ce process sans terminal attend que l'UI lâche l'instance (section 14), crée un `App` sur la base, reprend les sessions (point 7) et ne fait tourner que le tracker (`App::run_background`) : sessions, points de sauvegarde, XP et récompenses passent par le même code que dans l'UI, sans popups (le fil d'activité montre ensuite ce qui a été gagné). Pas de vérification de mise à jour ni de scan. Au démarrage, l'UI qui trouve l'instance prise demande l'arrêt (`instance::request_stop`, événement nommé `Local\CmdBoard.StopBackground` qui n'existe que tant que le suivi tourne) puis attend l'instance jusqu'à 10 s (`HANDOVER`) ; le suivi repasse ses sessions et quitte. `cmdboard --stop` fait de même sans ouvrir l'UI. Sans `background`, sans app à surveiller, ou si le lancement échoue, les sessions sont fermées normalement à la sortie. L'exe relancé est celui du démarrage (`current_exe` lu avant `:update`), qui contient la nouvelle version après une mise à jour. Pas de démarrage automatique avec Windows : après un redémarrage, le suivi reprend au prochain lancement de CmdBoard.
9. Seules les sessions fermées (`ended_at` non NULL) comptent dans le temps total, la streak et l'XP du jour.

Cas Steam / Epic / Battle.net : la commande de lancement (URI) et le process surveillé sont différents, d'où les deux champs séparés.

---

## 13. Animations (sobres)

Pilotées par `Tick` et un compteur `frame_count` dans `App`. Un tick ne redessine que si l'une d'elles est en cours (section 5) :

- Barre d'XP qui se remplit progressivement après une session : `App` garde une `XpAnim { from, to, start }` par app et une pour le profil. Le rendu en déduit le total affiché à partir de `frame_count` (8 ticks, soit 2 s, avec ralenti en fin de course), en repassant par `level_from_total`, donc le niveau affiché monte en même temps que la barre.
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
   - Steam : dossier lu dans `HKCU\Software\Valve\Steam\SteamPath` ; Epic : `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests` ; GOG : `HKLM\SOFTWARE\GOG.com\Games` ; Ubisoft Connect : `HKLM\SOFTWARE\Ubisoft\Launcher\Installs` (vue 32 bits pour les deux).
   - Les apps du Store et les jeux Xbox (MSIX) n'ont pas de `.lnk` : lues par `Get-StartApps` et `Get-AppxPackage`.
3. Icônes impossibles dans un terminal : glyphe Nerd Font ou emoji par catégorie.
4. Base de données dans `%APPDATA%` (crate `directories`).
5. Une seule instance par session Windows (`src/instance.rs`, mutex nommé `Local\CmdBoard.SingleInstance`, pris dans `main.rs` avant d'ouvrir la base et tenu jusqu'à la sortie) : deux CmdBoard suivraient les mêmes process, enregistreraient chaque session deux fois et se fermeraient mutuellement leurs sessions comme orphelines. La seconde affiche `error.already_running` et quitte avant de prendre le terminal, sauf si c'est le suivi en arrière-plan qui tient l'instance : elle lui demande alors de s'arrêter et la reprend (section 12, point 8). Windows libère le mutex même après un crash.
6. `config.toml` cassé : signalé au démarrage, et jamais réécrit. `config::save_value` refuse d'écrire dans un fichier illisible (comme `Aliases::save`), sinon la vérification de mise à jour quotidienne l'écraserait avec sa seule clé.

---

## 15. Tests

- **`core/`** : tests unitaires (XP, niveaux, règles), et propriétés avec `proptest` : le découpage de l'XP en niveaux redonne le total, un niveau ne baisse jamais, une règle saisie ne fait jamais paniquer.
- **`command/parser`** : tests de table (entrée → `Command`), et propriétés avec `proptest` : tout argument sans guillemet survit aux guillemets, un chemin Windows garde ses antislashs, aucune saisie ne fait paniquer.
- **`ui/`** : `TestBackend` de ratatui, et snapshots `insta` des écrans entiers (`src/ui/snapshots/`) qui ne dépendent ni de la date, ni des disques, ni de la machine. Après un changement voulu : `cargo insta review`.
- **`storage/`** : `Connection::open_in_memory()`. Un trigger qui échoue vérifie qu'une fermeture de session interrompue ne laisse rien d'écrit.
- **`app/`** : `App::with_defaults()` sur une base en mémoire, piloté par touches et lignes de commande ; un fichier de `app/tests/` par module.
- **Architecture** : `tests/architecture.rs` (couches, taille des fichiers) et `clippy::too_many_lines` (taille des fonctions), section 2.
- **Clippy** : le groupe `pedantic` est activé dans `Cargo.toml` (`[lints.clippy]`), donc bloquant en CI. On corrige l'avertissement ; sinon, `#[expect(clippy::…, reason = "…")]` au plus près du code. Seuls `cast_precision_loss` et `assert_is_empty` sont désactivés pour tout le projet. Les conversions qui réduisent passent par `try_from` : `storage::to_u64` et `to_i64` pour les entiers SQLite, `ui::layout::cells` pour les longueurs à l'écran.
- **Dépendances** : `cargo machete` en CI échoue sur une dépendance déclarée mais inutilisée. On la retire ; un faux positif (crate utilisée seulement par une macro) va dans `[package.metadata.cargo-machete] ignored`. `cargo deny check` (`deny.toml`, cible `x86_64-pc-windows-msvc` seulement) échoue sur une faille connue (base RustSec), une licence hors de la liste autorisée (licences permissives compatibles avec un binaire MIT) ou une source autre que crates.io. Une licence nouvelle s'ajoute comme exception pour une crate précise, après vérification. Les crates en double sont seulement signalées.
- **Clippy** : groupe `pedantic` activé dans `Cargo.toml` (`[lints.clippy]`), donc bloquant en CI. On corrige l’avertissement ; sinon `#[expect(clippy::…, reason = "…")]` au plus près du code. Seuls `cast_precision_loss` et `assert_is_empty` sont désactivés pour tout le projet. Les conversions passent par `try_from` (`storage::to_u64` et `to_i64` pour les entiers SQLite, `ui::layout::cells` pour les longueurs à l’écran).

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
3. La CI construit le `.zip`, le `.msi` et `cmdboard-installer.ps1`, puis crée la GitHub Release. Ses notes se terminent par la liste des commits depuis le tag `v*` précédent (tous pour la première version), rangés par préfixe (`feat :`/`feature :` → Features, `fix :` → Fixes, le reste → Other) et un lien de comparaison, ajoutés dans `release.yml` (job `host`, checkout complet `fetch-depth: 0`).

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

**winget** : identifiant `Loris01100.CmdBoard`, installeur = le MSI (`InstallerType: wix`, `Scope: machine`, commande `cmdboard`). La première version est soumise à la main dans [winget-pkgs](https://github.com/microsoft/winget-pkgs) (`wingetcreate submit` sur un manifeste validé par `winget validate`). Ensuite, le job `winget` de `release.yml` (action `winget-releaser`, épinglée par SHA) ouvre la PR de chaque nouvelle version après le job `host`. Il exige un fork de `winget-pkgs` sur le compte `Loris01100` et le secret `WINGET_TOKEN` (PAT classique, portée `public_repo`) ; sans le secret, ou pour une préversion, il ne fait rien.

### Points d'attention

- **SmartScreen** : sans signature de code, Windows affiche « Windows a protégé votre ordinateur » au premier lancement. Acceptable pour un projet perso. Sinon : Azure Trusted Signing.
- **Suivi en arrière-plan** : `cmdboard --background` garde l'exe ouvert après la sortie de l'UI, ce qui bloquerait un `winget upgrade` ou un MSI (fichier utilisé). Le message `update.use_winget` demande donc de lancer `cmdboard --stop` d'abord. `:update` (installation hors Program Files) n'est pas concerné : l'UI tourne, donc le suivi est arrêté.
- **Données** : la base vit dans `%APPDATA%`, hors du dossier d'installation. Elle survit aux mises à jour et aux désinstallations, et les migrations (section 3) font évoluer son schéma.

### Linux (hors périmètre)

CmdBoard est Windows uniquement (`.lnk`, `%APPDATA%`, URI des launchers). winget n'existe pas sous Linux. En cas de portage : `dist` produit aussi un installeur shell et une formule Homebrew, `self_update` fonctionne à l'identique, et des paquets natifs (AUR, `.deb` via `cargo-deb`) confieraient les mises à jour au gestionnaire de paquets de la distribution. Les `.lnk` seraient remplacés par les fichiers `.desktop` de `/usr/share/applications` et `~/.local/share/applications`.

---

## Prochaine étape

Les étapes 1 à 11 sont faites. Étape 12 en cours : `dist init` (`dist-workspace.toml`, `.github/workflows/release.yml`, `wix/main.wxs`), `:update` et la vérification passive quotidienne (`src/update.rs`, clés `update_check` et `last_update_check` dans `config.toml`) sont en place. `self_update` est en 1.x : il faut les features `github` et `ureq` en plus de celles de la section 17, et l'archive est choisie par son nom exact (`cmdboard-x86_64-pc-windows-msvc.zip`), sans quoi le `.msi` ou le `.sha256` pourraient correspondre. `v0.1.0` et `v0.2.0` sont publiées. Le manifeste winget de `v0.2.0` est validé et le job `winget` de `release.yml` automatise les versions suivantes. Reste : soumettre ce premier manifeste à `winget-pkgs`, créer le fork et le secret `WINGET_TOKEN`, puis attendre la fusion de la PR.
