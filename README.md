# Faulk

Faulk est un DAW ouvert et portable : il s'ouvre dans le navigateur, et on y fabrique ses propres instruments et effets. En attendant que les navigateurs offrent un accès audio de niveau professionnel, un helper natif optionnel apporte la basse latence et les plugins VST3 et CLAP.

Le projet démarre. La vision, l'architecture et la première étude technique sont dans [la documentation](docs/).

## Lancer en local

```bash
cd web
pnpm install
pnpm dev          # compile le moteur en WebAssembly puis sert http://localhost:5173
```

L'application web a deux parties : la page d'accueil publique sur `/` (en anglais, `/fr` en français, `/en` en anglais) et le DAW sur `/app`. La page d'accueil choisit le français si le navigateur est en français ; le choix fait avec le bouton EN / FR est mémorisé et partagé avec le DAW.

Pour vérifier la version statique, telle qu'elle sera publiée :

```bash
pnpm build        # écrit le site dans web/dist (index.html, en.html, fr.html, app.html)
pnpm preview      # sert ce build sur http://localhost:4173
```

Pour le mode Boost (helper natif, latence basse, MIDI natif) :

```bash
cargo run -p helper             # debug : console, le code d'appairage s'y affiche
cargo run --release -p helper   # release : sans console, icône dans la zone de notification
```

Sous Windows, le helper vit dans la zone de notification : son état, le code d'appairage (un clic le copie), « Ouvrir Faulk » qui ouvre la page et l'appaire sans saisie, le lancement au démarrage de Windows et « Quitter ». Le journal est dans `%APPDATA%\faulk\logs\helper.log`. Sous macOS et Linux, il n'y a pas encore d'icône : le helper tourne dans son terminal et y affiche le code. Une seule instance par port : relancer le helper ouvre simplement Faulk. L'option `--app-url` change la page ouverte : par défaut `http://localhost:5173/app` (le DAW du serveur de développement) pour un build de debug, `https://www.faulk.app/app` pour un build de release, `cargo run --release` compris (ajouter `-- --app-url http://localhost:5173/app` pour rester en local). Le helper accepte par défaut le site de production et les serveurs de développement et de prévisualisation ; `--allow-origin` remplace cette liste.

## Travailler sur le projet

Le projet se travaille avec Claude Code. Les règles pour les agents sont dans `CLAUDE.md`. À l'ouverture du dépôt, Claude Code propose d'installer les plugins déclarés dans `.claude/settings.json`, dont les skills d'ingénierie de Matt Pocock.

Prérequis : Rust (`rustup`, cible `wasm32-unknown-unknown`), Node 24 et pnpm ; sous Windows, les Visual Studio Build Tools (charge de travail C++) pour compiler le helper.
