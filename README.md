# Faulk

Faulk est un DAW ouvert et portable qui s'ouvre dans le navigateur. Aujourd'hui, on y pose les bases : pistes, clips, piano roll, arrangement, mixeur, enregistrement MIDI, export audio. Un helper natif optionnel apporte la basse latence. À terme, chacun pourra y fabriquer ses propres instruments et effets, et le helper accueillera les plugins VST3 et CLAP.

Faulk est pensé pour l'IA dès le départ : chaque action passe par une commande, la même pour un humain dans l'interface et pour un agent. Ce que l'un peut faire, l'autre le peut aussi. Le but est de voir à quoi pourra ressembler, demain, une collaboration créative entre musiciens et agents.

## Une expérience de software factory

Faulk est aussi une expérience : voir jusqu'où une équipe d'agents peut construire un logiciel exigeant, presque sans intervention humaine. Je fixe la direction et tranche ce qui relève du produit ou du son. Le reste est fait par des agents Claude :

- un agent lead découpe le travail, lance jusqu'à quatre agents en parallèle, fusionne et publie une release chaque jour ;
- chaque tâche va de l'issue à la pull request : plan, code, tests, vérification dans un vrai navigateur ;
- Judge Dredd, un agent relecteur impartial, juge chaque livraison avant qu'elle soit fusionnée ;
- des garde-fous (hooks, CI ciblée, enregistrements de vérification) empêchent un agent de terminer une tâche sans avoir prouvé qu'elle marche.

## L'amélioration continue, en musique

Chaque jour, un agent product owner prend la dernière version, compose un morceau avec l'interface comme le ferait un musicien, et note tout ce qui l'a bloqué ou ralenti. Ces manques deviennent des issues, que les agents développent dans la foulée. Le morceau du jour est publié sur la page [Journey](https://www.faulk.app/journey) : on y entend, jour après jour, ce que l'outil permet de faire et comment il progresse.

## Pourquoi la musique

La musique est un sujet qui me passionne depuis longtemps. Ce qui m'intéresse ici, c'est la rencontre de deux choses qui semblent s'opposer : une création automatisée, faite de règles, de contraintes et de processus, et la part probabiliste de ce que produit un modèle, qui ne donne jamais deux fois la même chose. Entre les deux, il reste une question ouverte : ce qui en sort est-il de la musique, et à partir de quand devient-il une œuvre ? Faulk est l'endroit où je la pose, un morceau par jour.

La vision, l'architecture et les études techniques sont dans [la documentation](docs/).

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

## Licence et marque

Le code est publié sous licence MIT (voir `LICENSE`), copyright Sparkle. Le nom « Faulk », son logo et les domaines faulk.app ne sont pas couverts par cette licence : voir `TRADEMARKS.md`.
