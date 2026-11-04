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

Le schéma montre les points d'entrée, pas le lieu d'exécution : la couche de commandes et le projet vivent dans la page dans les deux modes. En Boost, le helper reçoit les messages moteur, écrit les fichiers texte du projet et relaie les appels MCP et ACP vers la page.

## Composants

| Composant | Rôle | Technologies envisagées |
| --- | --- | --- |
| `crates/engine` | Graphe audio, transport, horloge, DSP. Pur : aucune entrée-sortie, aucune allocation dans le chemin audio. Pas de capacités fixes : les structures grandissent hors chemin audio et sont échangées sans allocation temps réel (décision du 2026-10-14). | Rust |
| `crates/project` (décidé le 2026-10-14, squelette créé par #37, temps musical réexporté d'`engine::time` par #38) | Le document : projet, pistes MIDI et audio, devices, scènes, clips, notes, arrangement, couloirs d'automation, réglages du transport ; lecture, écriture et validation du [format sur disque](02-modele-projet.md). Pur, sans entrée-sortie. | Rust, `serde` |
| `crates/commands` | Toutes les actions de Faulk, typées : nom, schéma d'entrée et de sortie, description pour les agents, annulation. Elles modifient le projet, qui produit les messages pour le moteur. Source unique dont dérivent les outils MCP et les types TypeScript. | Rust, `serde`, `schemars` |
| `crates/project-wasm` (décidé le 2026-10-14, créé par #37, cœur projet depuis PR-4, #49) | Projet et commandes compilés pour le thread principal de la page : **la page possède le projet dans les deux modes** ; en Boost, le helper reçoit les messages moteur et les fichiers texte du projet à écrire (jamais les blobs ni le cache PCM, qu'il produit et lit lui-même), et relaie les appels des agents. Exports C simples (`pw_*`) : la page y écrit du JSON et lit en retour un rapport JSON (types générés depuis `commands::host`) et les octets moteur ; enveloppe TypeScript `web/src/lib/project/core.ts`. | Rust → WASM (ABI C, sans `wasm-bindgen`) |
| `crates/audio-import` (AU-1, #110) | L'import d'un fichier audio, écrit une fois : SHA-256 en flux (le nom du blob), décodage (`symphonia`, sans SIMD : le même PCM dans les deux cibles, au bit près sauf MP3 et AAC, à 3,3·10⁻⁷ près), métadonnées, disposition d'un WAV ou d'un AIFF lu directement, en-tête du cache PCM. Pur : il lit par une source que l'hôte lui donne ([stockage local](03-stockage-local.md) § 5.9). | Rust |
| `crates/audio-import-wasm` | Ce pipeline pour la page : un module à part (ABI C `ai_*`, sans `wasm-bindgen`), chargé au premier import par le Worker du magasin de projet, qui lui lit le fichier par plages (`FileReaderSync`) et écrit ce qu'il produit. | Rust → WASM |
| `crates/protocol` | Les messages entre l'interface et un moteur (web ou natif) : messages moteur, état, retours visuels, fichiers du projet en mode Boost. | Rust, types TypeScript générés |
| `crates/engine-wasm` | Le moteur dans un `AudioWorklet`. Exports C simples (`fk_*`), sans `wasm-bindgen` dont le code d'appui ne se charge pas dans un worklet. La page envoie les octets du `.wasm` au worklet, qui le compile lui-même. Depuis MO-2 (#55, [découpage du MVP](../specs/02-decoupage-mvp.md) § 2.3), un seul module pour `wasm32-wasip1-threads`, deux instances sur une `WebAssembly.Memory` partagée : un Worker reçoit les octets `ControlMsg` que la page encode en Rust (`project-wasm`) et prépare les structures, le worklet lit l'anneau lock-free au début de chaque bloc et écrit le rapport que la page lit dans la même mémoire ; aucun message ne traverse le thread audio une fois le moteur lancé. | Rust |
| `crates/helper` | Le pont local : le moteur natif, l'audio basse latence, le MIDI, les plugins tiers, le serveur MCP, le lancement des agents ACP, et un serveur WebSocket local vers la page. | Rust, `cpal`, `midir`, `clack`, SDK MCP Rust (`rmcp`), ACP |
| `web/` | Une seule application web : la page d'accueil publique sur `/` (`/en`, `/fr`) et le DAW sur `/app` (arrangement, clips, mixer, Atelier, panneau d'agent). | TypeScript, SvelteKit, Svelte 5, Vite |
| `modules/` | Les modules de Faulk. | Faust d'abord (compilation à la volée), Rust → WASM ensuite |

## Principes

1. **AI first : toute action est une commande.** L'interface n'a aucun accès privilégié : elle appelle les mêmes commandes qu'un agent. Une action disponible dans l'interface mais absente de `crates/commands` est un bug, détecté par un test qui compare la liste des commandes aux outils MCP exposés.
2. **Un seul moteur.** Toute fonctionnalité du moteur compile et se comporte pareil en WASM et en natif, vérifié par un rendu comparé.
3. **Le son ne traverse jamais le lien page ↔ helper.** Seuls passent les messages moteur (pas les commandes : la page les applique au projet qu'elle possède ; depuis BO-1, #102, la page envoie les octets `ControlMsg` que son cœur encode), les fichiers texte du projet à écrire en mode Boost, l'état et les retours visuels, et, pour les entrées MIDI du helper, les messages de ses périphériques (liste, choix de l'entrée, messages reçus, que la page joue par son projet). Depuis #598, la page envoie aussi au helper la table de son entrée live (`live.routing` : les emplacements des pistes sélectionnée et armées, dans l'époque du moteur) : le helper joue les notes de son entrée MIDI avec elle, sans attendre le fil principal de la page ; c'est un état moteur décidé par la page, pas une modification du projet, et la page reçoit toujours chaque message.
4. **Le helper est un pont, pas une application.** Il n'a pas d'interface propre ni de logique produit (sous Windows, une icône de zone de notification pilote le processus lui-même : état, code d'appairage, ouverture de la page, lancement au démarrage, arrêt ; une fois installé, le helper se met aussi à jour seul, voir [Distribution et mises à jour du helper](04-distribution-helper.md) ; ces réglages du helper ne sont pas des commandes du projet et restent hors de `crates/commands` ; une boîte d'erreur s'affiche si le démarrage échoue) : il exécute le moteur, expose les commandes en MCP et relaie les agents vers la page. La logique des commandes vit dans `crates/commands`, exécutée dans la page (`crates/project-wasm`) dans les deux modes, jamais dans le helper. **La page possède le projet dans les deux modes** (décision du 2026-10-14) ; règle : le helper relaie, il ne possède pas le projet : en mode Boost, le helper reçoit les messages moteur, lit les fichiers audio du dossier du projet pour le moteur natif et écrit le dossier pour le compte de la page.
5. **Des projets portables.** Un projet utilisant un plugin tiers reste ouvrable sans helper : la piste est gelée (rendue en audio) et rejouée telle quelle.
6. **Modules isolés.** Un module en WASM s'exécute isolé : un module bogué ne fait pas planter la session.
7. **Une protection de sortie, toujours active** (#402, décision du fondateur du 2026-10-29). Le dernier étage du moteur (`engine::limiter`) est un limiteur à −0,3 dBFS, invisible et non désactivable : attaque immédiate sur l'échantillon qui dépasse, sans anticipation donc sans latence ajoutée, relâchement exponentiel de constante de temps 80 ms (le gain revient à l'unité peu après, il n'y est pas 80 ms après la crête). Il est placé après le master, le métronome et le compteur de sortie : le mètre du master montre le niveau avant la protection (la surcharge reste visible), l'enceinte, le helper et l'export WAV reçoivent le signal protégé. Même code en Web et en Boost, sans allocation. L'export signale à l'utilisateur quand le mix a été poussé dans le limiteur (réduction de gain crête), sans jamais écrêter le fichier (décision du fondateur du 2026-10-31).

## Agents

### MCP : un agent pilote Faulk

| Contexte | Mécanisme | Statut (octobre 2026) |
| --- | --- | --- |
| Helper installé | Le helper expose un serveur MCP local (HTTP sur `127.0.0.1`, ou stdio quand l'agent le lance). Les outils sont générés depuis `crates/commands` et agissent sur la session ouverte dans la page. | Mature |
| Navigateur seul | WebMCP : la page déclare ses outils au navigateur (`document.modelContext`) et l'agent du navigateur les appelle. | Expérimental (phase de test dans Chrome, API encore renommée) : à brancher quand elle sera stable, sur la même couche de commandes |

#### Le serveur MCP du helper (AG-1, #97, protocole 26)

- **Outils** : un par commande, générés depuis le catalogue (`commands::catalog`) : description, schéma d'entrée et de sortie, indications `readOnlyHint` (requêtes) et `destructiveHint` (`x-destructive`). Le nom de l'outil est celui de la commande, `_` à la place de `.` (`track_create` pour `track.create`), car les API des modèles refusent le point dans un nom d'outil (`crates/helper/src/mcp.rs`).
- **Transports** : HTTP « Streamable HTTP » sur le port du helper, chemin `/mcp` (`http://127.0.0.1:47800/mcp`), et stdio par `faulk-helper mcp`, un pont vers ce même serveur HTTP (`crates/helper/src/mcp_stdio.rs`) : une seule logique MCP.
- **Sécurité** : chaque requête est vérifiée avant le serveur MCP : `Host` qui nomme le helper (contre le DNS rebinding), aucun en-tête `Origin` (aucune page web, celle de Faulk comprise, ne pilote le serveur MCP) et le jeton MCP (`Authorization: Bearer`), distinct du jeton de session de la page. Le helper ne crée jamais ce jeton : l'utilisateur le crée en lançant `faulk-helper mcp-token` (sa confirmation) ; jusque-là, toute requête MCP est refusée. Le helper relit le fichier à chaque requête : le supprimer ferme le serveur MCP aussitôt. Le fichier `mcp-token` du dossier de configuration du helper n'est lisible que par son propriétaire (Unix ; sous Windows, le dossier de l'utilisateur l'est déjà).
- **Relais** : le helper n'exécute aucune commande. Un appel d'outil devient `agent.call` (un `id`, le nom du client MCP, la commande en JSON) vers la page ; la page l'exécute par son répartiteur comme une action de l'utilisateur (auteur `agent:<nom>` dans l'historique, annulation, refus) et répond `agent.result` avec le même `id`. Sans page appairée ouverte, l'outil répond aussitôt l'erreur `page.absent`, qui dit d'ouvrir Faulk dans le navigateur ; une page fermée ou remplacée avant sa réponse donne `page.gone`, une page muette `page.timeout` (60 s) : dans ces deux derniers cas la commande a pu s'appliquer, l'agent relit l'état avant de recommencer. Les autres erreurs : `command.refused` (avec la `CommandError`), `command.invalid`, `page.unavailable` (pas de projet, page en bascule, ou commande qui ouvre un sélecteur ou une invite et exige donc un clic dans la page : `project.save_as`, `open_folder`, `allow_access`, `import_pack`, `export_pack`, `export_audio` ; `project.save` d'un projet sans dossier part vers « Enregistrer sous » et n'aboutit pas sans clic).
- **Lien de contrôle (#608, protocole 29)** : une page appairée ouvre, en plus de l'éventuelle session Boost, un lien de contrôle (`/control`, même jeton, même contrôle de `Host` et d'`Origin` que la session). Il ne porte que `agent.call`, `agent.result`, `agent.left` (depuis le protocole 32, voir ci-dessous) et `control.front`, ne prend jamais le rôle audio, ne remplace pas la session et s'ouvre que le helper ait une sortie audio ou non : un agent pilote donc la page en mode Web comme en Boost, et avec un helper sans sortie. Le helper envoie un appel à la session Boost si elle existe, sinon au lien de contrôle « au premier plan » : le plus récent, ou le dernier dont la page a signalé qu'elle passait au premier plan (`control.front`, envoyé quand la page est affichée ou prend le focus) ; avec plusieurs onglets, l'agent pilote donc celui que l'utilisateur a regardé en dernier. Le helper garde au plus 8 liens : un nouveau ferme le plus ancien avec le code de fermeture `CONTROL_PUSHED_OUT` (4001), et la page de ce lien ne revient pas d'elle-même (sinon les pages se chasseraient en boucle) mais quand l'utilisateur revient à elle (focus ou onglet affiché), à sa prochaine sonde, à son prochain appairage ou à son passage en Boost. La page ouvre le lien quand sa sonde trouve le helper, après un appairage et à l'ouverture de Boost ; un lien perdu (helper redémarré, mise à jour, veille) est retenté après 1, 2, 4, 8, 16 puis toutes les 30 s, sans fin, tant que la page reste appairée ; un lien refusé par un helper qui répond à la sonde fait oublier l'appairage, comme une session refusée, mais un lien trop lent à s'ouvrir (helper occupé) le garde et est retenté. Le helper ne sonde pas ses liens : un lien mort sans fermeture (veille de la machine) fait attendre un appel jusqu'à `page.timeout` (60 s), le temps que le navigateur le ferme.
- **Départ d'un agent (#641, protocole 32)** : un groupe d'annulation ouvert par un agent (`history_begin_group`) refuse l'annulation à tous jusqu'à son `history_end_group`. Le helper compte les agents par nom (l'auteur `agent:<nom>` de l'historique) : un nom est présent tant qu'une de ses sessions MCP est ouverte, ou qu'un de ses appels sans session (révision MCP sans sessions) date de moins de 5 minutes (`AGENT_IDLE`). Une session se termine quand le client la ferme (`DELETE` ; le pont stdio `faulk-helper mcp` en envoie un quand son entrée se ferme) ou après 5 minutes sans requête (un pont tué sans fermer son entrée finit ainsi). Quand le helper répond 404 à un pont stdio dont la session a ainsi pris fin (ou dont le helper a redémarré), le pont (#657) rejoue l'`initialize` qu'il a gardé (même `clientInfo`, donc même nom d'agent), puis `notifications/initialized`, puis le message en cours, une seule fois : l'agent n'en est pas informé. Pour la présence par nom, c'est le même agent qui revient. Ce que l'agent perd : à la fin de l'ancienne session la page a reçu `agent.left` (ou perdu son lien avec le helper) et fermé le groupe qu'il avait laissé ouvert, puis terminé son tour ; ses éditions suivantes s'exécutent hors de tout groupe, et son `history_end_group` n'a plus de groupe à fermer, sans que l'agent en soit averti ; un second 404 ou un 404 sans `initialize` connu du pont reste une erreur « restart the agent ». Quand le dernier de ces liens d'un nom se termine, le helper envoie `agent.left` (le nom) à la session Boost accueillie et à chaque lien de contrôle ; deux sessions du même nom (deux fenêtres d'un même client) ne partent qu'avec la dernière, et le départ d'un agent ne touche jamais le groupe d'un autre. La page ferme alors le groupe que cet agent a laissé ouvert, termine son tour (un agent du même nom qui revient commence un nouveau tour) et retire ses questions en attente (AG-4) : l'annulation de l'utilisateur fonctionne aussitôt. La page ferme aussi le groupe et le tour de tout agent quand elle perd à la fois sa session Boost et son lien de contrôle, quelle qu'en soit la cause (helper arrêté ou redémarré, lien chassé, nouvel appairage) : aucun agent ne peut plus l'atteindre, et un `agent.left` envoyé entre-temps serait perdu. Cette fermeture n'est pas une commande : le départ d'un agent n'est l'action de personne, c'est un événement de l'hôte, comme l'auteur ou l'horloge (`pw_agent_left`, `pw_agents_gone`).
- **Limites** : les commandes destructrices s'exécutent sans confirmation, annulables, jusqu'à AG-4 (accepté par le fondateur le 2026-11-03) ; un tour d'agent n'est pas encore un groupe d'annulation (AG-3).

Brancher Claude Code (le helper doit tourner ; `faulk-helper mcp-token` affiche le jeton et ces deux commandes) :

```bash
# stdio : Claude Code lance le pont, qui lit lui-même le jeton
claude mcp add faulk -- "<chemin de faulk-helper>" mcp
# ou HTTP, avec le jeton
claude mcp add --transport http faulk http://127.0.0.1:47800/mcp --header "Authorization: Bearer <jeton>"
```

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
