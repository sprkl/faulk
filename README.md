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
cargo run --release -p helper   # affiche un code d'appairage à saisir dans la page
```

## Travailler sur le projet

Le projet se travaille avec Claude Code. Les règles pour les agents sont dans `CLAUDE.md`. À l'ouverture du dépôt, Claude Code propose d'installer les plugins déclarés dans `.claude/settings.json`, dont les skills d'ingénierie de Matt Pocock.

Prérequis : Rust (`rustup`, cible `wasm32-unknown-unknown`), Node 24 et pnpm ; sous Windows, les Visual Studio Build Tools (charge de travail C++) pour compiler le helper.
