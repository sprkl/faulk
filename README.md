# Faulk

Faulk est un DAW dans la lignée d'Ableton Live qui s'ouvre dans le navigateur, et où l'on fabrique ses propres instruments et effets. Un helper natif optionnel apporte la latence pro et les plugins VST3 et CLAP.

Le projet démarre : rien n'est encore codé. La vision, l'architecture et la première étude technique sont dans [la documentation](docs/).

## Travailler sur le projet

Le projet se travaille avec Claude Code. Les règles pour les agents sont dans `CLAUDE.md`. À l'ouverture du dépôt, Claude Code propose d'installer les plugins déclarés dans `.claude/settings.json`, dont les skills d'ingénierie de Matt Pocock.

Prérequis prévus : Rust (`rustup`, cible `wasm32-unknown-unknown`), Node 24 et pnpm, le compilateur Faust.
