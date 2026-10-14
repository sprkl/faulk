---
type: Architecture
title: "Architecture technique"
description: "One Rust engine compiled to WebAssembly for the browser and to native code for the helper; every action is a typed command exposed to the UI, to MCP and to ACP agents; the helper is the local bridge for audio, plugins and agents."
status: draft
---

# Architecture technique

Un seul moteur audio en Rust, compilé pour deux cibles, et une seule couche de commandes que l'interface, les serveurs MCP et les agents ACP utilisent de la même façon. On déplace le moteur, jamais le son.

```
     Interface web          Agent MCP              Agent ACP
     (un client parmi       (Claude Code,          (Claude Code, Codex,
      d'autres)              Cursor…)               Gemini CLI…)
          │                     │                       │ stdio
          │ commandes           │ MCP (HTTP local)      │
          ▼                     ▼                       ▼
   ┌─────────────────────────────────────────────────────────────┐
   │            Couche de commandes (crates/commands)            │
   │   chaque action = nom + schéma + description + annulation   │
   └──────────────┬──────────────────────────────┬───────────────┘
                  ▼                              ▼
       MODE WEB (défaut)                 MODE BOOST (helper = pont local)
       moteur → WASM                     moteur natif
       dans un AudioWorklet              • ASIO / WASAPI / CoreAudio (cpal)
       • modules Faust/WASM              • modules Faust/WASM (les mêmes)
       • WebMCP quand il sera stable     • plugins VST3 / CLAP tiers, MIDI
                                         • serveur MCP local
                                         • lancement des agents ACP
```

## Composants

| Composant | Rôle | Technologies envisagées |
| --- | --- | --- |
| `crates/engine` | Graphe audio, transport, horloge, DSP. Pur : aucune entrée-sortie, aucune allocation dans le chemin audio. Pas de capacités fixes : les structures grandissent hors chemin audio et sont échangées sans allocation temps réel (décision du 2026-10-14). | Rust |
| `crates/project` (décidé le 2026-10-14, à créer) | Le document : projet, pistes MIDI et audio, devices, scènes, clips, notes, arrangement, couloirs d'automation, réglages du transport ; lecture, écriture et validation du [format sur disque](02-modele-projet.md). Pur, sans entrée-sortie. | Rust, `serde` |
| `crates/commands` | Toutes les actions de Faulk, typées : nom, schéma d'entrée et de sortie, description pour les agents, annulation. Elles modifient le projet, qui produit les messages pour le moteur. Source unique dont dérivent les outils MCP et les types TypeScript. | Rust, `serde`, `schemars` |
| `crates/project-wasm` (décidé le 2026-10-14, à créer) | Projet et commandes compilés pour le thread principal de la page : **la page possède le projet dans les deux modes** ; en Boost, le helper reçoit les messages moteur et les fichiers texte du projet à écrire (jamais les blobs ni le cache PCM, qu'il produit et lit lui-même), et relaie les appels des agents. | Rust → WASM (`wasm-bindgen`) |
| `crates/protocol` | Les messages entre l'interface et un moteur (web ou natif) : messages moteur, état, retours visuels, fichiers du projet en mode Boost. | Rust, types TypeScript générés |
| `crates/engine-wasm` | Le moteur dans un `AudioWorklet`. Exports C simples (`fk_*`), sans `wasm-bindgen` dont le code d'appui ne se charge pas dans un worklet. La page envoie les octets du `.wasm` au worklet, qui le compile lui-même. Aujourd'hui les commandes arrivent par `postMessage` ; la cible (décision du 2026-10-14, isolation cross-origin acceptée) est une mémoire partagée : un Worker prépare les structures, le worklet lit un anneau lock-free. | Rust |
| `crates/helper` | Le pont local : le moteur natif, l'audio basse latence, le MIDI, les plugins tiers, le serveur MCP, le lancement des agents ACP, et un serveur WebSocket local vers la page. | Rust, `cpal`, `midir`, `clack`, SDK MCP Rust (`rmcp`), ACP |
| `web/` | L'interface : arrangement, clips, mixer, Atelier, panneau d'agent. | TypeScript, SvelteKit, Svelte 5, Vite |
| `modules/` | Les modules de Faulk. | Faust d'abord (compilation à la volée), Rust → WASM ensuite |

## Principes

1. **AI first : toute action est une commande.** L'interface n'a aucun accès privilégié : elle appelle les mêmes commandes qu'un agent. Une action disponible dans l'interface mais absente de `crates/commands` est un bug, détecté par un test qui compare la liste des commandes aux outils MCP exposés.
2. **Un seul moteur.** Toute fonctionnalité du moteur compile et se comporte pareil en WASM et en natif, vérifié par un rendu comparé.
3. **Le son ne traverse jamais le lien page ↔ helper.** Seuls passent les commandes, l'état et les retours visuels.
4. **Le helper est un pont, pas une application.** Il n'a pas d'interface propre ni de logique produit (sous Windows, une icône de zone de notification pilote le processus lui-même : état, code d'appairage, ouverture de la page, lancement au démarrage, arrêt ; ces réglages du helper ne sont pas des commandes du projet et restent hors de `crates/commands` ; une boîte d'erreur s'affiche si le démarrage échoue) : il exécute le moteur, expose les commandes en MCP et relaie les agents vers la page. La logique des commandes vit dans `crates/commands`, partagée avec le build web. **La page possède le projet dans les deux modes** (décision du 2026-10-14) : en mode Boost, le helper reçoit les messages moteur, lit les fichiers audio du dossier du projet pour le moteur natif et écrit le dossier pour le compte de la page.
5. **Des projets portables.** Un projet utilisant un plugin tiers reste ouvrable sans helper : la piste est gelée (rendue en audio) et rejouée telle quelle.
6. **Modules isolés.** Un module en WASM s'exécute isolé : un module bogué ne fait pas planter la session.

## Agents

### MCP : un agent pilote Faulk

| Contexte | Mécanisme | Statut (octobre 2026) |
| --- | --- | --- |
| Helper installé | Le helper expose un serveur MCP local (HTTP sur `127.0.0.1`, ou stdio quand l'agent le lance). Les outils sont générés depuis `crates/commands` et agissent sur la session ouverte dans la page. | Mature |
| Navigateur seul | WebMCP : la page déclare ses outils au navigateur (`document.modelContext`) et l'agent du navigateur les appelle. | Expérimental (phase de test dans Chrome, API encore renommée) : à brancher quand elle sera stable, sur la même couche de commandes |

### ACP : connecter son propre agent

Faulk joue le rôle du client ACP (l'« éditeur ») ; l'agent choisi par l'utilisateur est un processus local qui dialogue en JSON-RPC sur stdio. La page ne peut pas lancer de processus : le helper lance l'agent et relaie la conversation vers la page par WebSocket, sur le modèle des outils qui servent une interface web depuis un serveur local.

À l'ouverture d'une session ACP, Faulk fournit à l'agent son propre serveur MCP : l'agent avec qui l'utilisateur discute dans l'Atelier agit directement sur le projet.

Les agents distants (transport ACP par HTTP et WebSocket) sont une proposition encore en cours de spécification : à reprendre quand elle sera stable.

### Garde-fous

- Chaque action d'un agent apparaît dans l'historique avec son auteur, et les actions d'un même tour d'agent s'annulent en une fois.
- Les commandes destructrices (supprimer une piste, écraser un module, vider un projet) sont marquées comme telles dans leur schéma et demandent une confirmation de l'utilisateur.
- Sécurité du pont local : `../constraints/01-contraintes.md`.

## Points à valider (spike S1)

- Latence réelle du mode Web avec `renderSizeHint` (Chrome 153+) sur la machine du fondateur.
- Un seul code moteur réellement partagé entre les deux cibles.
- Détection du helper depuis la page, appairage et bascule à chaud.
- Une commande définie une fois, appelable depuis l'interface, depuis un agent MCP et par un agent ACP lancé par le helper.
- Compilation Faust à la volée dans les deux cibles.
