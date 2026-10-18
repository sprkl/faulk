---
type: Spec
title: "Découpage du MVP"
description: "Split of the Faulk MVP (MIDI and audio tracks, automation, several instruments and effects, in both modes) into 96 tasks for a swarm of agents, with the founder's decisions of 2026-10-14: socle decisions, epics and tasks with their tags, phases, dependency graph, critical path, contracts to freeze before parallel work, and conflict hot spots."
status: draft
---

# Découpage du MVP

> **Décisions du fondateur du 2026-10-14 intégrées** (§ 7). Ce document reprend l'étude du 2026-10-13 sur le découpage du MVP pour un essaim d'agents, les ajustements de la synthèse (format dossier du [modèle de projet](../architecture/02-modele-projet.md), banc de charge QA-7), puis les quinze décisions du fondateur : les pistes audio et l'automation entrent dans le MVP, le moteur n'a plus de capacités fixes, le dossier réel devient le stockage par défaut. L'étude s'appuie sur le code de l'issue #2 (fusionnée dans `develop`, commit `03ee512`) ; le code reste la source de vérité. Le statut reste `draft` tant que le document n'a pas été vérifié contre le code.

## 0. Objectif du MVP

Objectif du fondateur : **arriver à la création d'un projet multi-pistes avec différents instruments, dans les différents modes.**

À la fin du MVP, un utilisateur (ou un agent MCP) peut :

1. créer un projet, plusieurs pistes MIDI et audio, chacune avec un instrument (ou un lecteur de clips audio) et une chaîne d'effets ;
2. jouer en direct sur la piste sélectionnée (MIDI, clavier de l'ordinateur), en mode Web comme en mode Boost ;
3. créer, éditer (piano roll) et enregistrer des clips MIDI ;
4. importer des fichiers audio (échantillons, boucles, stems) et les jouer comme clips, lus en flux depuis le disque ;
5. lancer des clips et des scènes dans une **grille de lancement**, avec quantification ;
6. placer des clips sur une **timeline d'arrangement** et la jouer en boucle ;
7. automatiser les paramètres des devices et du mixeur : dessiner et éditer des points, les entendre à l'échantillon près, et si c'est faisable enregistrer les mouvements de paramètres ;
8. piloter le transport (lecture, arrêt, boucle, tempo, métronome) ;
9. mixer (volume, panoramique, mute, solo, vu-mètres) ;
10. enregistrer le projet dans un dossier, le rouvrir, et obtenir le même rendu ;
11. faire tout cela par commandes typées, exposées en outils MCP.

Hors MVP (décision 5 du § 7) : l'**enregistrement audio** (il arrive juste après le MVP ; le format et le stockage lui réservent déjà sa place), le **recalage temporel** des clips audio (warping, audio élastique : hors MVP sauf s'il se révèle trivial, ce qui n'est pas attendu ; les clips audio sont non élastiques, voir [modèle de projet](../architecture/02-modele-projet.md) § 1.2), l'Atelier et la compilation Faust à la volée ([spec de l'Atelier](01-atelier-modules.md)), les sessions ACP (qui arrivent avec l'Atelier), les changements de tempo dans le temps, les envois et retours, les plugins tiers CLAP/VST3, WebMCP, les versions et variantes dans l'application, et avec elles la **fusion à trois points et le binaire `faulk` pilote de git** (`merge`, `diff`, `git setup`) que le [modèle de projet](../architecture/02-modele-projet.md) § 4 et § 8 proposait dans son MVP : aucune tâche de ce découpage ne les construit, le format v1 reste seulement *prêt pour git* ; le fondateur a confirmé ce périmètre le 2026-10-14.

## 1. Constat sur le code de l'issue #2

| Constat | Conséquence pour le MVP |
| --- | --- |
| `Command::apply(&self, synth: &mut Synth)` : les commandes agissent directement sur **un** synthé du moteur. | Il manque un **modèle de projet** distinct du moteur : les commandes modifient le projet, qui produit les messages moteur. |
| Un seul `enum Command` avec trois `match` (`name`, `apply`, `undo_kind`) dans `crates/commands/src/lib.rs`. | Point chaud de conflits dès que dix agents ajoutent des commandes. |
| Le mappage commande → moteur est écrit en TypeScript côté Web (`web/src/lib/engine/wasm-ops.ts`) et en Rust côté Boost (`helper/src/audio.rs::engine_event`). | Risque de divergence Web/Boost : il faut **un seul chemin**, en Rust, partagé par les deux cibles. |
| `Session.run` dans `web/src/lib/session.svelte.ts` est un `switch` par type de commande (depuis UI-1 : `Dispatcher.run`, `web/src/lib/stores/dispatcher.ts`). | À remplacer par un dispatcher générique. |
| Un export C `fk_*` par opération dans `engine-wasm`. | À remplacer par une boîte de réception binaire unique (`fk_push`) et une boîte d'envoi (`fk_feedback`). |
| `web/src/lib/i18n.ts` : un seul fichier de chaînes ; `App.svelte` : un seul écran. | Points chauds de conflits pour l'interface. |
| Pas de temps musical, de piste, de clip, d'identifiant stable. | Tout le socle est à construire avant les fonctions visibles. |
| Mode Boost écrit mais non vérifié. | À vérifier avant d'empiler le multi-pistes dessus. |
| Isolation cross-origin en place sur tout le site (AU-0, en-têtes dans `web/vercel.json`) ; contrat de synchronisation figé et voie WASM choisie par mesure (CT-3, `crates/engine/src/msg.rs`) ; un seul canal vers le moteur dans les deux cibles (MO-1, MO-2 : octets `ControlMsg` en mémoire partagée côté Web, même flux dans le helper). | Les pistes audio (AU-3) restent à construire. |

## 2. Décisions d'architecture du socle

Les décisions 1 à 4 et 7 à 8 du § 7 fixent ce socle ; les tâches de contrat de la phase 0 le figent en code. Ce qui reste à trancher l'est dans la tâche de contrat nommée.

### 2.1 Qui possède l'état du projet (décidé)

- Nouvelle crate **`crates/project`** : le document (projet, pistes MIDI et audio, devices, scènes, emplacements de clips, clips MIDI et audio, notes, arrangement, couloirs d'automation, réglages du transport), pur Rust, `serde`, sans I/O ; son format sur disque est celui du [modèle de projet](../architecture/02-modele-projet.md). Elle modifie la table « Stack and layout » de `CLAUDE.md` (fait par CT-1, qui crée les crates).
- `crates/commands` applique les commandes **au projet**. **Décidé par CT-4 (#40)** : modèle mutable à inverses explicites, `Command::apply(&self, state: &mut State, ctx: &mut Context) -> Result<Outcome, CommandError>`, où `State { project, session, history }` (l'état de session de la décision 10 est une cible de commande comme le projet ; PR-2 y ajoute l'historique d'annulation, que seuls `commands::run` et les commandes `history.*` modifient), `Context { author, group, ids }` porte l'auteur, le groupe d'historique et le générateur d'identifiants (ULID, semé par la page avec son horloge et son aléa : le projet reste pur), et `Outcome { output, undo, events, control }` contient la sortie typée, l'inverse (une commande ordinaire, appliquée par le même chemin, donc avec ses propres `ProjectEvent` et `ControlMsg`), les `ProjectEvent` (pour l'interface) et les `ControlMsg` (pour le moteur). Le modèle persistant du [modèle de projet](../architecture/02-modele-projet.md) § 5.2 n'est pas retenu : annuler un instantané exigerait un diff projet → moteur ou un rejeu complet ; les deux documents sont alignés sur ce choix.
- **La page possède le projet, dans les deux modes** (décision 1) : le cœur projet + commandes est compilé en WASM (`crates/project-wasm`, `wasm-bindgen` permis car il tourne sur le thread principal, pas dans le worklet). En mode Boost, la page envoie au helper les `ControlMsg`, pas les commandes ; le helper reste un pont sans logique produit, son serveur MCP relaie les appels d'outils vers la page, et il lit et écrit le dossier du projet pour le compte de la page (AU-7).
- Conséquence : basculer Web ↔ Boost = rejouer un **instantané** du projet vers l'autre moteur ; ouvrir un projet = même mécanisme.

### 2.2 Temps musical

- Positions et durées en **ticks entiers**, 960 par noire (`i64`) ; positions dans un fichier audio en **frames** entières.
- Tempo constant par projet (`f64` BPM, 20–999), signature rythmique ; pas de changement de tempo dans le temps au MVP.
- Le moteur convertit ticks ↔ échantillons avec une position `f64` ; les événements (notes, points d'automation, départs de clips audio) sont placés **à l'échantillon près** (découpage du bloc aux offsets).
- **Quantification de lancement** (décision 8) : globale au projet, **1 mesure par défaut**, réglable parmi : aucune, 1/16, 1 temps, 1, 2, 4, 8 mesures. Un réglage par clip viendra plus tard : champ optionnel dans le fichier du clip, absent = réglage global. Boucle du transport : région `[début, fin)` en ticks.

Figé par CT-2 (#38) dans `engine::time`, réexporté par `crates/project` :

- `Tick(i64)` et `Frame(u64)` sont des newtypes (un tick et une frame ne se mélangent pas), `PPQ = 960` ; `Tempo(f64)` fini dans `[20, 999]` ; `TimeSignature` (numérateur `1..=32`, dénominateur puissance de deux `1..=32`) ; `Quantize` est une énumération fermée (`None, Sixteenth, Beat, Bar, Bars2, Bars4, Bars8`, sur disque `none, sixteenth, beat, bar, bars_2, bars_4, bars_8`) ; `LoopRegion` refuse une région vide ou plus longue que `i64::MAX` ticks.
- Le **temps** est la valeur du dénominateur de la signature (en 6/8, une croche, 480 ticks) ; c'est ce que « 1 temps » et l'affichage mesure:temps:tick comptent. Choix à confirmer par le fondateur (l'alternative en mesure composée est la noire pointée).
- Grille ancrée au tick 0 (division euclidienne : les positions négatives du décompte sont sur la même grille). `next_boundary_at_or_after` rend le premier point de grille à partir de la position (un lancement demandé sur une frontière part tout de suite) ; `next_boundary_after` le premier strictement après. Les deux rendent `None` quand il n'y a pas de point (au-delà de `i64::MAX`) ; sans quantification, la première rend la position elle-même et la seconde `None`.
- Conversions sur `Tempo`, fonctions pures de la position absolue (jamais accumulées, donc sans dérive) : tick → secondes et tick → échantillon en `f64` (`tick × 60 × fréquence / (BPM × 960)`, à quelques ulp près ; le moteur garde cette position fractionnaire pour découper le bloc) ; secondes, échantillons et frames → tick **au plus proche, demi vers l'extérieur** (`f64::round`, identique sur toutes les cibles) ; entrée non finie → tick 0, entrée finie hors plage → saturation par signe. L'aller-retour tick → échantillon → tick est l'identité pour `|tick| ≤ 2^40` à 44,1, 48 et 96 kHz ; l'inverse ne l'est pas (à 120 BPM et 48 kHz un tick dure 25 échantillons).
- Fin en ticks d'un clip audio : `début + frames_to_ticks(frames)`, la durée arrondie au tick le plus proche puis ajoutée au début (à tempo constant ; une carte de tempo viendra après le MVP).

### 2.3 Synchronisation projet → moteur sans verrou et sans plafond visible

Un seul code pour les deux cibles :

1. **`ControlMsg`** (encodage binaire compact) : ce que le projet envoie au moteur (créer une piste, insérer un device, régler un paramètre, téléverser les notes d'un clip, téléverser un couloir d'automation, déclarer une source audio, lancer un clip, jouer…).
2. `Engine::prepare(ControlMsg) -> RtMsg` : **hors chemin audio**, peut allouer (construire un device, un tampon de delay, une table de notes, une table de pistes plus grande).
3. `Engine::apply(RtMsg)` : **sur le chemin audio**, sans allocation, en début de bloc ou à l'offset indiqué ; ce qui est remplacé (y compris une ancienne table de pistes) part dans une file de « déchets » libérée hors chemin audio.
4. **`Feedback`** (moteur → page) : tête de lecture, état des clips, vu-mètres par piste, notes enregistrées, charge processeur, compteurs de sous-alimentation disque, à environ 30 Hz.

**Pas de plafond visible** (décision 7). Le moteur n'a pas de capacités fixes : quand une structure est pleine, `prepare` en construit une plus grande hors chemin audio et `apply` l'échange en un bloc, sans allocation en temps réel. Il n'existe qu'une **limite de sûreté très haute** (chiffrée par CT-3, par exemple 4 096 pistes et 16 777 216 notes) que le projet refuse par une erreur typée avant que la commande n'atteigne le moteur : le moteur n'échoue jamais en temps réel. Les événements des clips et de l'automation ne passent par aucune file bornée : le moteur les lit dans leurs tables (construites par `prepare`) au moment de rendre le bloc ; seule l'entrée jouée en direct (MIDI, clavier) passe par une file bornée, dont la taille est hors d'atteinte d'un humain et dont le débordement est compté. Le banc de charge (QA-7) mesure le coût de ces échanges sur de très gros projets.

Par cible :

- **Natif (helper)** : `prepare` sur un thread de contrôle, `RtMsg` vers le callback audio par `rtrb`, déchets en retour par un second `rtrb`.
- **WASM** : le `onmessage` d'un `AudioWorklet` s'exécute **sur le thread de rendu**, comme `process` ; y appeler un `prepare` qui alloue violerait la règle « hors chemin audio ». L'isolation cross-origin étant acceptée (décision 4, tâche AU-0), **CT-3 tranche par prototype et mesure entre deux voies** :
  - (b) **mémoire partagée** : `prepare` tourne dans un Worker dédié qui partage la mémoire du moteur (`WebAssembly.Memory` partagée) et le worklet ne lit qu'un anneau lock-free, comme dans le helper. Ce que cette voie implique, et que le prototype doit régler : un build WASM avec atomics (aujourd'hui la bibliothèque standard précompilée de la cible `wasm32-unknown-unknown` sur la chaîne `stable` épinglée par `rust-toolchain.toml` n'en a pas ; passer par `build-std` sur `nightly` est un choix pour tout le dépôt, présenté au fondateur dans la PR de CT-3), deux instances du même module sur une mémoire (pile et stockage local de thread distincts, segments de données initialisés une seule fois, sans `wasm-bindgen`), et un allocateur partagé qu'aucune libération ne doit verrouiller sur le thread de rendu ;
  - (a) **sans mémoire partagée** : `onmessage` copie des octets bornés dans une boîte de réception préallouée du worklet, et une structure qui grandit est construite dans le worklet **entre deux quanta de rendu, hors de `process`**, par un message dédié ; le coût sur le thread de rendu (objets `MessageEvent` du navigateur, allocation entre deux quanta) est mesuré et doit tenir sous un budget que CT-3 fixe.
  Dans les deux cas, `process` n'alloue jamais et le critère de MO-2 s'énonce par rapport à la voie retenue. **Mémoire linéaire** : le module réserve une mémoire initiale au démarrage et **grandit hors chemin audio** (`memory.grow` appelé par `prepare`, dans le Worker ou entre deux quanta, jamais dans `process`) ; après une croissance, les vues typées du worklet sur la mémoire sont réobtenues en début de bloc (comparaison de `memory.buffer`, coût constant) ; un `memory.grow` refusé (plafond de 4 Gio de wasm32, mémoire de la machine) est une erreur typée renvoyée au projet et affichée, jamais un plantage ni un silence. Les limites de sûreté de CT-3 tiennent sous ce plafond, budget PCM (AU-3) compris.
- Le mappage commande → messages vit en Rust (`crates/commands`), jamais en TypeScript.

**Décision CT-3 (#39, 2026-10-14) : contrat figé dans `crates/engine/src/msg.rs`, voie (b) retenue.** Les types (`ControlMsg`, `RtMsg`, `Feedback`, `Table`, limites, en-tête) sont le contrat ; MO-1 écrit les codecs des charges utiles, `prepare` et `apply`, MO-2 le canal de chaque cible. Ce que le code fige :

- **Format binaire de `ControlMsg`** : petit-boutiste partout ; en-tête de 16 octets `version u8` (= 1), `flags u8` (0), `tag u16` (une valeur par variante, stable), `epoch u16`, `réservé u16`, `len u32` (≤ 16 Mio, au-delà l'envoi est découpé en morceaux `first`/`total`), `seq u32` ; le message suivant commence au multiple de 4 suivant. Un `tag` inconnu est sauté grâce à `len` et compté (`Feedback.rejected`), le flux continue ; version, bits réservés ou `len` hors limite : flux corrompu, erreur typée. Les structures sont nommées par des **emplacements** (`TrackSlot`, `ClipSlot`…, des `u32` attribués par le projet, qui garde la correspondance avec ses identifiants stables). Le PCM des pistes audio ne passe jamais par un message (AU-3 le diffuse dans des tampons que `AudioSourceDeclare` déclare).
- **Époque** : seule `Reset { epoch }` porte une nouvelle époque, que le moteur adopte si elle est plus récente (comparaison avec enroulement sur 16 bits : un `Reset` d'une époque passée, dupliqué ou rejoué, est jeté et ne ramène jamais le moteur en arrière) en jetant tout (y compris les morceaux d'envoi en cours) ; tout autre message d'une autre époque est jeté et compté (égalité stricte). À chaque bascule de moteur (web ↔ helper, ou retour sur un moteur suspendu), la page incrémente l'époque et renvoie tout le projet après un `Reset` : un envoi découpé coupé par la bascule ne se termine jamais dans l'ancienne époque.
- **`RtMsg`** : une valeur Rust, jamais sérialisée, déplacée dans un anneau à un producteur et un consommateur ; ≤ 64 octets (testé) ; **un `ControlMsg` donne au plus un `RtMsg`**, et un `RtMsg` au plus un déchet (`Garbage`). Seul le côté `prepare` écrit dans l'anneau ; l'entrée live a sa propre file. Anneau plein : le côté `prepare` garde le message dans sa file ordonnée (hors chemin audio, il peut allouer) et le pousse dès qu'une place se libère ; rien n'est perdu ni réordonné. Cette file est bornée elle aussi (4 096) : au-delà, le moteur ne consomme pas (contexte audio suspendu, périphérique perdu) et le projet reçoit `EngineBusy`, arrête d'envoyer et se resynchronise par un `Reset` quand le moteur repart ; en mode web la page sait son `AudioContext` suspendu et s'arrête avant.
- **Croissance** : deux sortes de tables. Les **tables de contenu** (notes d'un clip, points d'un couloir, placements d'une piste) sont des instantanés immuables construits entièrement par `prepare` à partir des données du projet et échangés par pointeur : leur taille ne coûte rien au thread de rendu (déplacer un million de notes dans `apply` coûterait 3,5 ms en natif, mesuré). Les **tables d'exécution** (pistes, sources audio : l'état que le thread audio modifie) sont des tableaux d'emplacements : chaque emplacement existe (`None` s'il est vide) et un message l'adresse directement ; quand un message nomme un emplacement au-delà de la capacité, `prepare` construit d'abord la suivante, vide, deux fois plus grande, `apply` y déplace les entrées (un pointeur chacune, borné par la limite de la table : 3,4–5 µs pour 4 096 pistes en natif, moins de 350 µs dans la pompe du worklet pour 4 096 puis 8 192 emplacements déplacés) et l'ancienne part aux déchets ; un emplacement hors table dans `apply` est sauté et compté, jamais une panique. Toute allocation de `prepare` est **faillible** (`Table::try_with_capacity`, `try_box`, jamais `Box::new`) : un `memory.grow` refusé est `MsgError::OutOfMemory`, jamais un piège. L'anneau des déchets a une place de plus que l'anneau `RtMsg` et `prepare` le vide avant chaque envoi : après une vidange, au plus 4 096 messages sont en vol et le côté audio peut en appliquer un (un déchet) avant que l'envoi qui suit la vidange n'arrive, d'où au plus 4 097 déchets ; `apply` a toujours une place et ne libère jamais rien.
- **Limites de sûreté** (`msg::limits`) : 4 096 pistes, 1 024 devices par piste, 1 048 576 clips, 1 048 576 placements (131 072 par piste), 16 777 216 notes (1 048 576 par clip), 65 536 couloirs, 16 777 216 points (par couloir aussi), 4 096 sources audio, 4 096 scènes (alignées avec `crates/project/src/limits.rs`, qui prend les siennes dans `engine::msg::limits` et vérifie à la compilation qu'aucune ne dépasse celles du moteur) ; file live 4 096 ; anneaux 4 096 ; 128 messages appliqués par bloc au plus ; mémoire initiale 16 Mio ; budget PCM 1 Gio (AU-3) ; budget moteur 3 Gio − 64 Mio pour les tables et les devices, dont la taille déclarée par le descripteur (CT-5) est additionnée par le projet qui refuse un insert au-delà. Le pire cas des tables (toutes à leur limite, doublées pendant une croissance, plus la plus grande une fois pour la fragmentation d'une mémoire linéaire qui ne rétrécit pas, plus les anneaux et le `Feedback`) tient sous le budget moteur (test `limits_fit_under_the_wasm32_cap`).
- **Entrée live** : seule file bornée ; en débordement l'événement est jeté et un compteur atomique (`LiveOverflow`) incrémenté ; le côté audio vide d'abord la file live et applique ses événements, puis lit le compteur en début de chaque bloc et, s'il a bougé, relâche toutes les notes live (l'ordre du helper aujourd'hui : un note-on en file avant le note-off perdu est appliqué avant la relâche, jamais après) : aucune note bloquée même si plus rien n'est envoyé.
- **`Feedback`** : en-tête fixe de 56 octets (version, transport, époque, `seq` en verrou de séquence, impair pendant l'écriture, pour une lecture sans arrêt du moteur, tête de lecture en ticks et en frames, charge, compteurs : débordements live, rejets, sous-alimentations disque), puis les vu-mètres par piste, les clips actifs seulement (au plus deux par piste : un joué, un en attente) et les notes enregistrées depuis le dernier rapport (au plus 1 024 par rapport, le reste au suivant). La plus grande session tient sous 256 Kio par rapport.

**Mesures** (prototype `web/src/lib/engine/proto/`, suite `web/tests/e2e/engine-channel.spec.ts` ; la vraie synthèse joue une note pendant toute la mesure ; horloge : un Worker qui incrémente un compteur partagé, calibré sur `performance.now()`, car un `AudioWorkletGlobalScope` n'a pas de `performance` ; variation de l'étalonnage ≤ 10 % ; une lecture nulle signifie que le Worker-horloge a été préempté pendant la mesure et elle est écartée ; le compteur est un entier 32 bits lu comme un nombre, sans `BigInt`, pour que le worklet n'alloue rien). Chromium sans tête, Windows 11, 48 kHz ; les mêmes ordres de grandeur dans Chrome avec le périphérique audio réel :

| Mesure | (a) sans mémoire partagée | (b) mémoire partagée | Natif (helper, `rtrb`) |
| --- | --- | --- | --- |
| Par message sur le thread de rendu | corps du gestionnaire 0,8–1,9 µs ; 3,3–20 µs par message en rafale, `MessageEvent` compris | 0,2–1,0 µs par message pompé (lectures d'horloge comprises), 1,4–1,7 µs par pompe, vide ou non | push 2–3 ns, pop + apply 2–3 ns |
| Par croissance (1 M notes + tableau de pistes doublé, 4 096 puis 8 192 emplacements déplacés) | **5–18,6 ms sur le thread de rendu** (`prepare` entre deux quanta), au-delà d'un quantum (2,67 ms) et d'un rappel de périphérique de 10 ms ; préparer 4 096 pistes y coûte 1,2–1,8 ms de plus | `prepare` dans le Worker 4,4–18 ms, hors thread de rendu ; `apply` dans la pompe : pompe la plus longue 36–340 µs, déplacement des 4 096 puis 8 192 emplacements et préemption compris | `apply` 0,2 µs (64 entrées), 1,0–1,2 µs (1 024), 3,4–5,0 µs (4 096) ; libération hors thread ≤ 1,7 µs |
| Allocations sur le thread de rendu (mise en place exclue) | 4 103 allocations et 6 libérations (`prepare` y tourne : 4 096 pistes, les tables) | **0 et 0** (compteur par instance) ; côté `prepare` 4 113 et 6 | 0 (détecteur d'allocation sur `apply`) |
| `memory.grow` pendant que le worklet tient des vues | l'ancien tampon est détaché (longueur 0) : vues reprises au bloc suivant, aucun bloc silencieux | l'ancien tampon reste lisible, vues reprises (`memory.buffer` comparé), aucun bloc silencieux | sans objet |
| `memory.grow` refusé (mémoire à `maximum` 20 Mio, doublure du plafond de 4 Gio) | code typé, l'instance reste vivante | code typé, l'instance reste vivante | `try_reserve` → `OutOfMemory` (test) |

Le compteur de blocs en retard du worklet ne distingue pas les deux voies : Chrome rend quatre quanta par rappel de 10 ms, au repos comme en charge (écart moyen 2,5–2,7 ms, maximum 11 ms dans les deux cas) ; c'est la durée du travail sur le thread de rendu qui tranche, et une croissance de (a) dépasse le rappel.

**Voie retenue : (b)**, sur la cible **`wasm32-wasip1-threads`** de la chaîne `stable` épinglée (bibliothèque standard précompilée avec atomics et mémoire partagée importée ; `--max-memory` porté à 4 Gio ; Chrome accepte un `maximum` de 65 536 pages). **Pas de passage à `nightly`** : l'option `build-std` sur `wasm32-unknown-unknown` donnerait le même module sans imports WASI, au prix d'une chaîne instable pour tout le dépôt ; elle est présentée au fondateur dans la PR de CT-3 avec ces chiffres. Ce que (b) impose, réglé par le prototype : deux instances du même module sur une mémoire (la seconde reçoit sa propre pile, 1 Mio du tas partagé, par le global `__stack_pointer` exporté ; les segments de données ne sont initialisés qu'une fois) ; cinq imports WASI remplacés par des bouchons (`environ_get`, `environ_sizes_get`, `fd_write`, `proc_exit`, `sched_yield`) ; un stockage local de thread distinct par instance (vérifié par un `thread_local!` qui garde une valeur différente dans chaque instance ; le module du prototype n'a pas de segment TLS à initialiser, MO-2 revérifie les exports du vrai module et initialise un bloc TLS par instance si `__wasm_init_tls` apparaît) ; un allocateur partagé (celui de la libc WASI, verrouillé) que le thread de rendu ne touche jamais, prouvé par le compteur d'allocations par instance. Effet sur la chaîne : la cible ajoutée à `rust-toolchain.toml` et à la CI ; MO-2 fait basculer `engine-wasm`.

**Budget que MO-2 doit tenir** (thread de rendu, mesuré par la suite e2e) : ≤ 2 µs par `RtMsg` pompé, ≤ 500 µs pour la pompe qui échange une table d'exécution à sa limite (4 096 puis 8 192 emplacements déplacés, mesuré 36–340 µs dans Chromium, 3,4–5 µs en natif), la suite e2e ne tenant en CI qu'un plafond dix fois plus haut (20 µs par message, 5 000 µs pour la pompe), car un coureur partagé dépasse parfois 500 µs sans régression (#220 : lectures de 512 à 1 201 µs sur le prototype, jamais plus de 169 µs sur le worklet livré), et ces bornes elles-mêmes sur la médiane de cinq mesures avec `FAULK_E2E_STRICT_TIMING=1`, sur une machine de référence (`docs/agents/verification.md`), ≤ 128 messages par bloc (`MAX_APPLY_PER_BLOCK`), soit moins de 10 % d'un quantum de 128 frames à 48 kHz pour une vidange complète ; aucune allocation ni libération sur l'instance du worklet (compteur à zéro) ; `prepare` sans borne, dans le Worker.

**Mise en œuvre MO-1 (#54, 2026-10-14) : `crates/engine/src/engine.rs` et `crates/engine/src/msg/codec.rs`.** Ce que le code ajoute au contrat de CT-3, et où il s'en écarte :

- **Codecs** : chaque variante de `ControlMsg` a sa charge utile (champs dans l'ordre de déclaration, largeurs naturelles, petit-boutiste ; disposition dans la documentation du module `codec`). Le décodage lit exactement `len` octets, refuse une valeur hors domaine (booléen autre que 0 ou 1, flottant non fini, hauteur MIDI au-delà de 127, tempo, signature ou boucle invalides) par `MsgError::BadPayload`, message sauté et compté, et vérifie le nombre d'éléments d'une liste avant toute allocation. L'état initial d'un device (`DeviceInsert::state`, laissé à MO-1 par CT-5) est une suite de paires `(identifiant de paramètre u32, valeur f32)`.
- **Un `ControlMsg` donne au plus un `RtMsg`, croissance comprise** : le tableau de CT-3 envoyait `Swap(Tracks)` puis `Swap(Track)` pour une piste qui fait grandir la table, en contradiction avec sa propre règle. La table agrandie voyage désormais dans le même message que l'élément (`Put`) ; `apply` déplace les entrées, échange, place l'élément, et renvoie un seul déchet. Les types opaques de CT-3 (`Swap`, `Prepared`, `TableKind`, `Opaque`) deviennent des types concrets (`State`, `Track`, `Clip`, `Lane`, `Source`, `DeviceEntry`) ; MO-3 les étend. Le prototype de CT-3 (`web/src/lib/engine/proto/crate`) garde une copie des anciens types pour rester la référence de ses mesures.
- **Tables paginées** : les clips (1 048 576) et les couloirs (65 536) seraient 256 et 16 fois au-delà du cas mesuré par CT-3 (4 096 emplacements) s'ils grandissaient emplacement par emplacement. Ils sont rangés en pages de 1 024 emplacements derrière un répertoire : une croissance déplace au plus 1 024 pointeurs. Les pistes, les sources, et par piste les devices et les scènes restent des tableaux d'emplacements (au plus 4 096).
- **`prepare` est atomique** : toutes ses allocations précèdent tout changement de son état (l'ombre des capacités, les envois en cours, l'époque d'un `Reset`) ; un `OutOfMemory` ou un `EngineBusy` laisse le côté `prepare` tel quel, et le flux peut être renvoyé à partir de ce message (`Preparer::stopped_at` en donne la position). Un retrait d'emplacement que le moteur n'a jamais reçu n'envoie rien. Un message qui nomme une piste ou un device absents est refusé par `prepare` (`NoSuchTarget`), compté.
- **Effet en début de bloc** : `RtMsg::ParamSet` n'a plus d'`offset` ; un réglage par la main ou par un agent prend effet au début du bloc qui l'applique (`Device::set_param`, CT-5). Les événements à l'échantillon près sont ceux de MO-4.
- **Coût mesuré** (natif, release, `msg_cost`) : préparer et pousser un `ParamSet` 25 ns, le dépiler et l'appliquer 21 ns ; croissance de la table des pistes à sa limite (2 048 pistes déplacées) 3,1 µs, du répertoire des clips à sa limite (512 pages) 1,0 µs, d'une rangée de 512 devices 0,7 µs.
- **WASM** : le test de MO-1 tourne sur `wasm32-unknown-unknown` dans un interpréteur, une instance qui porte les deux côtés : la croissance de la mémoire linéaire par `prepare` pendant la lecture (9 Mio) ne laisse aucun bloc silencieux, et le rendu est identique au natif ; un `memory.grow` refusé rend `OutOfMemory` et l'instance continue. Le canal à deux instances sur mémoire partagée (`wasm32-wasip1-threads`) est celui de MO-2, qui reprend ce test sur sa cible.
- **Changement de fréquence d'échantillonnage** : les devices sont construits pour le `DeviceSetup` du canal ; à un changement de fréquence, l'hôte (MO-2) construit un nouveau canal et renvoie le projet après un `Reset`.

**Mise en œuvre MO-3 (#56, 2026-10-14) : `crates/engine/src/graph.rs`.** `Engine::render` (et `Engine::process`, qui le fait précéder de `begin_block`) rend le graphe :

- **Pistes et chaînes** : chaque piste, dans l'ordre de ses emplacements, fait tourner sa chaîne dans l'ordre de ses emplacements (le projet garde l'ordre de CT-5) : un effet MIDI transforme le flux de notes en sa propre sortie, réduite aux notes ; le premier instrument activé reçoit ce flux et écrit le son ; un effet audio le transforme sur place ; un device désactivé est contourné. Les notes jouées en direct arrivent à l'offset 0 (le routage à l'échantillon près est celui de MO-4). Une seule paire de tampons stéréo, allouée par `channel` hors chemin audio, sert à toutes les pistes, rendues l'une après l'autre.
- **Mixage** : par canal, `audible × gain × balance(pan)` ; `audible` est faux pour une piste muette, ou non soliste quand une autre l'est (solo cumulatif) ; balance à gain unité au centre (`gauche = min(1, 1 − pan)`, `droite = min(1, 1 + pan)`), loi provisoire jusqu'à MX-1. Chaque niveau, master compris, suit une rampe linéaire de 5 ms (la règle de lissage de CT-5).
- **Fondus** : une piste nouvelle part de 0 et monte en 5 ms. Une piste qui quitte son emplacement (`TrackRemove`, un `TrackPut` sur un emplacement occupé, toutes les pistes à un `Reset`) n'est plus un déchet d'`apply` : elle rejoint une liste chaînée par ses propres pointeurs (aucune allocation, aucune borne), descend à 0 en 5 ms, puis passe dans un **anneau de retraite** (4 096 places) que `Preparer::collect` vide. Anneau plein : la piste reste dans la liste, muette et non rendue, et repasse au bloc suivant. Cet anneau est distinct de celui des déchets, dont la borne (un déchet par message) ne couvre pas ce qu'un rendu produit.
- **Effet MIDI ou instrument changé** : insérer, retirer, activer ou désactiver un effet MIDI ou un instrument relâche toutes les notes de sa piste au prochain bloc rendu (avant les notes jouées de ce bloc), car l'instrument ne recevrait jamais les note-off des voix nommées par l'ancien flux, ni ceux arrivés pendant qu'il était contourné. Un échantillon non fini (NaN, infini) ou dénormal ne sort jamais du graphe : il vaut 0. Retirer ou désactiver un device n'est pas fondu (seules les pistes le sont). La renumérotation des voix entre effets MIDI et le dimensionnement des sorties MIDI enchaînées restent à MO-4.
- **Coût mesuré** (natif, release, `graph_cost`, bloc de 128 frames à 48 kHz, budget 2 667 µs) : une piste vide (sans device activé) coûte 14 à 17 ns par bloc, 4 095 pistes vides 2,2 à 2,7 % du budget ; une piste qui joue un son de test, 1,5 à 2 µs.

**Mise en œuvre MO-4 (#57, 2026-10-15) : `crates/engine/src/events.rs`, `crates/engine/src/graph.rs`.** Ce que le code ajoute, et ce qu'il laisse aux tâches qui lisent les tables :

- **Un bloc d'horloge par bloc du graphe** : `Engine::render` découpe un rendu en blocs d'au plus `max_block_frames` ; chacun est un bloc de l'horloge de TR-1, dont les segments (coupés à chaque bouclage) sont relevés une fois dans une table préallouée (`max_frames + 2` places, la borne de TR-1) que lisent le graphe et le métronome (`Clicker::render` reçoit les segments au lieu de l'horloge).
- **Sous-blocs** : chaque piste fait tourner sa chaîne une fois par segment, et coupe encore le segment quand ses événements remplissent sa réserve. Un sous-bloc ne traverse donc jamais un bouclage ni un déplacement, et chaque device reçoit dans `BlockCtx` la longueur du sous-bloc, l'état de l'horloge (`playing`) et la position en ticks fractionnaires de sa première frame (TR-2 la laissait à 0) ; à l'arrêt, un seul sous-bloc à la position figée.
- **Routage** : les événements d'un sous-bloc sont rangés en deux suites, les notes (relâche d'une chaîne changée, notes live à l'offset 0 du bloc, relâche après un débordement live, puis les notes de la table) et les valeurs de paramètres avec le device qu'elles visent. Chaque device reçoit la fusion par offset de ses propres valeurs (en premier à offset égal) et, s'il est instrument ou effet MIDI, du flux de notes ; un effet audio ne reçoit que ses valeurs. Les notes live restent à l'offset 0 : un `LiveEvent` ne porte pas d'heure.
- **Table d'événements datés** (`Schedule`) : une table de contenu triée par tick, construite hors chemin audio par allocations faillibles (`Schedule::try_new` : une note donne un note-on à son début et un note-off à `début + max(durée, 1)` ; à tick égal, les valeurs, puis les note-off, puis les note-on ; identifiants de voix `premier + index`, à partir de 129 pour ne jamais croiser les voix live 1 à 128 ; accord fin porté jusqu'au device). Le thread audio la lit avec un curseur, **sans file bornée** : les événements sont pris par groupes de même offset ; quand un groupe ne tient pas dans la réserve du sous-bloc (1 024 notes au-delà du bloc live, 1 024 valeurs), le sous-bloc s'arrête avant lui et la lecture reprend au même index. Un groupe plus grand que toute la réserve, sur un seul échantillon, est livré en partie, le reste à la première frame du sous-bloc suivant, même après un bouclage ou un déplacement : aucun événement n'est perdu (test : 10 000 notes et 2 000 valeurs dans un bloc ; 10 000 note-on sur un échantillon ; 6 000 sur la dernière frame d'un bloc suivi d'un déplacement).
- **Sauts et arrêt** : un segment qui ne commence pas là où la lecture s'était arrêtée (lecture, déplacement, bouclage) relâche les notes que la table tient, **chacune par son propre note-off** (les notes live de la piste ne sont pas touchées), puis repositionne le curseur par recherche dichotomique ; le note-off d'une note dont le note-on n'a pas été livré est ignoré. Un bloc à l'arrêt relâche de même et oublie le curseur.
- **Ce qui ne remplit pas encore la table** : aucun message ne la porte ; le lecteur de clips (CL-2), l'arrangement (AR-2) et l'automation (AM-2) y compileront leurs tables ou la remplaceront, et les tests la posent hors chemin audio (`TrackGraph::set_schedule`). Le rattrapage d'une valeur d'automation après un déplacement (AM-2), les cibles du mixeur (MX-1, AM-2) et le départ d'un clip audio (AU-4, qui choisira comment le lecteur de clips reçoit son départ) n'y sont pas.
- **Effets MIDI** : un effet MIDI est dimensionné à partir de la réserve d'un sous-bloc, ou de la sortie du plus grand effet MIDI placé avant lui (l'ombre de `prepare` garde la capacité de sortie de chaque emplacement) ; un effet inséré plus tard devant un autre ne redimensionne pas celui-ci, le filet de sécurité de CT-5 (un `AllNotesOff` dans la place réservée) couvre ce cas. Un device qui reçoit des valeurs de paramètres derrière une sortie MIDI plus grande que le tampon de fusion les reçoit par `set_param` à la première frame du sous-bloc : aucune note n'est perdue. Les identifiants de voix ne sont pas renumérotés : le graphe ne fusionne jamais deux flux de notes (la sortie d'un effet MIDI remplace le flux), donc aucun identifiant ne peut en croiser un autre ; le contrat de `device.rs` le dit désormais (la renumérotation annoncée par CT-5 est sans objet).
- **Débordement live** : déjà compté (`LiveOverflow`) et écrit dans le `Feedback` (mot 28, MO-2), inchangé.
- **Coût mesuré** (natif, release, `events_cost`) : 10 000 notes (20 000 événements) dans un bloc de 2 048 frames à 48 kHz, médiane 0,78 ms pour un budget de 42,7 ms.
- **Parité** : `engine::events::testing::scenario` (instrument et effet de test, notes avec accord fin et valeurs pour les deux devices, boucle, déplacement, arrêt, blocs de tailles impaires) rendu en natif et par le module `.wasm` (`crates/engine-wasm/tests/events_parity.rs`), écart ≤ 1e-5 et silences aux mêmes échantillons.

**Mise en œuvre MO-2 (#55, 2026-10-14).** Ce que le code ajoute, et ce qui reste provisoire :

- **Un seul module, deux instances** : `engine-wasm` est livré pour `wasm32-wasip1-threads` (arguments d'édition de liens dans `.cargo/config.toml`). Le Worker de `prepare` (`web/src/lib/engine/prepare-worker.ts`) crée le canal (`fk_channel_new`) et alloue, pour l'instance du worklet, sa pile (1 Mio) et son bloc de stockage local de thread : le module réel a un segment TLS (192 octets) et une seconde instance non initialisée l'aurait placé à l'adresse 0 ; le worklet appelle `__wasm_init_tls` avant tout. Exports : `fk_inbox` puis `fk_push(canal, ptr, len)` (le pointeur de canal s'ajoute à la signature `fk_push(ptr, len)` du ticket, une page pouvant tenir plusieurs canaux), `fk_flush` (toutes les 5 ms), `fk_render` (= `fk_pump` puis `fk_render_block`), `fk_left`, `fk_right`, `fk_feedback`. Les anciens exports `fk_new`, `fk_note_on`… sont supprimés ; le prototype de CT-3 en garde une copie figée.
- **Mappage en Rust seulement** : `commands::engine_msgs` traduit les commandes en `ControlMsg` pour le seul synthé (piste 0, device 0) en attendant la table identifiant → emplacement de PR-3 ; la page encode par `crates/project-wasm` (ABI C provisoire, que PR-4 remplace), le helper encode les commandes qu'il reçoit et soumet les mêmes octets à son côté `prepare` (thread de contrôle toutes les 5 ms). Les commandes, et non les octets, traversent encore la WebSocket : c'est BO-1. `wasm-ops.ts` et `engine_event` sont supprimés.
- **Rendu et synthé** (`crates/engine/src/host.rs`) : l'hôte rend le graphe de MO-3 (`Engine::render`) ; il construit les devices par le registre des devices intégrés (`engine::devices`) ; l'adaptateur provisoire `SynthDevice` de MO-2 a été retiré par DV-1, le synthé étant lui-même un `Device`.
- **Rapport** (MO-5, version 2 de la disposition) : le thread de rendu écrit le `Feedback` environ 30 fois par seconde dans une zone fixe de la mémoire partagée (taille maximale, 160 Kio), sous verrou de séquence ; la page la lit sans message. Écrits : version, époque, état du transport et tête de lecture (`Engine::report_transport`), débordements live, rejets, un vu-mètre crête et RMS par emplacement de piste jusqu'à la dernière piste (après le fader de la piste ; zéros pour un emplacement vide) puis celui de la sortie (après le master et le métronome), le plus grand nombre de voix des devices des pistes sur la période (`Device::voices`, mot 52), la charge processeur. La charge est mesurée par l'appelant (`Host::add_render_time`) : le worklet web n'a pas d'horloge et le helper ne chronomètre pas son rendu tant que la règle « aucun appel au système sur le fil audio » n'a pas d'exception décidée par le fondateur ; sans mesure, le mot vaut −1 (« non mesuré »), jamais 0. Clips actifs, notes enregistrées et sous-alimentations disque restent à zéro (LA, RE, AU-3). En mode Boost, le helper envoie les mêmes octets en trames binaires WebSocket (protocole 3, une trame au plus en attente par session) : la page les décode avec le même décodeur (`web/src/lib/engine/feedback.ts`). L'association emplacement → `TrackId` dans la page est de la vue mixeur (MX-3) ; en Boost les emplacements sont ceux du projet à un synthé du helper jusqu'à BO-1.
- **Erreurs** : un flux arrêté (`OutOfMemory`, `EngineBusy`) est renvoyé à la page avec son code et sa position, et journalisé ; son affichage et la resynchronisation (renvoi après `Reset`) sont de PR-3 / PR-5. Les époques des deux cibles commencent à 1 (un `Reset` d'époque 0 serait refusé).
- **Mesures** (Chromium sans tête, Windows 11, 48 kHz, `web/tests/e2e/engine-mo2.spec.ts`, charge enregistrée en Rust) : trois passages : 1 000 `ParamSet` à 0,10–0,16 µs par message pompé, pompe la plus longue 12,5–47,6 µs ; 4 095 pistes (dernière croissance du tableau : 2 048 entrées déplacées) à 0,03–0,07 µs par message, pompe la plus longue 12,3–19,8 µs ; pompe à vide 1,1–1,5 µs ; aucune allocation ni libération sur l'instance du worklet, aucun bloc silencieux. Budget tenu (≤ 2 µs par message, ≤ 500 µs par pompe). Croissance de la mémoire partagée par un autre thread pendant le rendu, `memory.grow` refusé (`-8`, le canal continue) et parité avec le rendu natif : `web/src/lib/engine/channel.test.ts` (Node, module livré) et `crates/engine-wasm/tests/parity.rs` (flux enregistré `tests/fixtures/session.stream`).

### 2.4 Contrat de commande v2

Figé en code par CT-4 (#40) ; PR-2 (#47) l'a promu à la racine de `crates/commands` (le contrat v1 n'existe plus) et y a migré les commandes v1 : `live.note_on`, `live.note_off`, `live.all_notes_off`, `device.set_param`, `midi.list_inputs`, `midi.select_input`.

- Nom `domaine.verbe` (`track.create`, `clip.notes.add`, `transport.play`). Seize domaines, un fichier chacun : `project`, `track`, `device`, `clip`, `note`, `scene`, `launch`, `arrangement`, `automation`, `transport`, `mixer`, `record`, `midi`, `live`, `history`, `view` ; l'enum racine compte une ligne par domaine et le désérialiseur aiguille sur le préfixe (un domaine inconnu ou un verbe inconnu est nommé dans l'erreur). Les réglages du métronome sont des réglages du transport : `transport.set_metronome`, pas de domaine `metronome`.
- Entrée **et sortie** typées (`schemars`) : chaque commande a une variante `Output` du même nom ; la sortie donne les identifiants créés. Le schéma exporté (clé `command` du bundle : `input`, `output`, `catalog`) porte l'entrée, la sortie et le **catalogue** des outils MCP, dérivé du schéma et comparé un pour un aux commandes déclarées par un test ; le serveur MCP (AG-1) expose ce catalogue, jamais une liste écrite à la main.
- **Identifiants stables** (`TrackId`, `ClipId`, `SceneId`, `DeviceId`, `LaneId`, `PlacementId`, dans `project::ids`) : ULID de 26 caractères Crockford base32, générés par le projet via le `IdGenerator` du `Context`, jamais des index ; le projet tient la correspondance identifiant → emplacement moteur. **Pas de `NoteId`** : une note est une valeur (`Note { start, pitch, length, velocity }`), les commandes l'adressent par sa valeur entière (`clip.notes.remove { clip, notes: [Note] }`, une modification = retrait et ajout dans une seule commande), et un inverse est construit depuis ce qui a réellement changé, jamais depuis les seuls arguments (ajouter une note déjà présente ne change rien, son inverse ne retire rien). Le `note_id` de 24 bits de l'ABI de l'Atelier est un identifiant de voix alloué par le moteur parmi les notes tenues ([spec de l'Atelier](01-atelier-modules.md) § 1.3, voie (a)), jamais écrit dans le projet ni porté par une commande.
- Métadonnées de schéma : `x-kind` (`edit` entre dans l'historique ; `live` éphémère comme une note jouée, la lecture, le solo ou la sélection : touche la session ou le moteur, jamais le projet ; `query` lecture seule), `x-undo` (`inverse` : l'inverse est dans l'`Outcome` ; `none`), `x-destructive` (irréversible, confirmation obligatoire). `host` et `snapshot` ne sont pas retenus : la page possède le projet et l'état de session, rien ne s'annule hors du modèle. Règles de cohérence testées sur chaque commande : `live` et `query` ⇒ `none` et non destructive ; `edit` ⇒ `inverse`, ou `none` seulement si destructive ; un `edit` change l'état et émet au moins un `ProjectEvent` ; chaque cas de refus d'une commande est testé par `check_refusal`, qui vérifie que l'état reste intact.
- Historique : pile annuler/rétablir, **groupes** (`GroupId` : un glisser de souris, un tour d'agent = une annulation), **auteur** sur chaque entrée (`Author`, sérialisé `user` ou `agent:<nom>`). La pile elle-même et `history.undo` / `history.redo` sont de PR-2 : l'hôte passe chaque commande par `commands::run`, qui applique puis enregistre l'inverse d'une édition (une édition irréversible vide les deux piles) ; `history.undo` a pour inverse `history.redo` et réciproquement ; `history.begin_group` / `history.end_group` ouvrent et ferment un groupe qui appartient à son auteur (seules ses éditions le rejoignent, lui seul le ferme, annuler est refusé tant qu'il est ouvert ; l'hôte ferme celui d'un auteur parti sans le fermer, `History::abandon_group`) ; le `Context::group` d'un tour d'agent l'emporte sur le groupe ouvert. L'historique reste dans l'ordre d'application : une édition d'un autre auteur au milieu d'un groupe le coupe en plusieurs entrées. L'inverse d'une suppression est une commande de restauration publique qui porte l'entité avec ses identifiants, refusée s'ils entrent en collision.
- Disposition anti-conflits : un module par domaine (§ 5) ; une commande est un bloc `domain!` dans le fichier de son domaine, d'où dérivent nom, métadonnées, schéma et outil MCP.
- Règle de test, en code (`commands::testing`) : chaque commande a une description, un schéma d'entrée et de sortie, et son undo est testé par le harnais (`apply`, puis `apply(inverse)` restaure l'état à l'identique, puis le rétablir le ramène) ; une commande déclarée sans exemple fait échouer les tests, donc une commande n'est déclarée que lorsqu'elle est implémentée.
- État de vue, tranché : le zoom, le défilement et la vue affichée ne sont pas des commandes (la page les écrit dans le fichier de session, sous sa propre clé `view`, à côté du `SessionState` du modèle : un seul écrivain, le magasin de session de la page) ; la sélection de piste est une commande, `view.select_track { track }`, `x-kind: live`, parce qu'elle décide où va le MIDI live et qu'un agent doit pouvoir la lire et la poser.

### 2.5 Contrat de device (instrument, effet, lecteur de clips)

- Trait `Device` dans `crates/engine` : `process(&mut self, ctx: &BlockCtx, events: &[Event], io: &mut AudioIo)`, `set_param(id, value)`, `reset()`. Sans allocation, sans panique. Les événements portent un offset dans le bloc ; ils incluent les notes **et les valeurs de paramètres** (`ParamValue { offset, id, value }`), pour que l'automation (AM-2) tombe à l'échantillon près.
- Descripteur statique par type : `kind` (`"faulk.osc_synth"`), nom affiché, rôle, paramètres (id stable, clé de nom, unité, min, max, défaut, échelle, pas, **temps de lissage**, valeurs énumérées). Exporté dans le schéma : l'interface **génère** les panneaux de réglage et les agents connaissent les paramètres.
- Le nom affiché d'un device est un seul nom anglais, jamais traduit, le même dans les deux langues de l'interface (décision du fondateur du 2026-10-14) ; les libellés et descriptions de ses paramètres sont traduits (EN, FR). Le nom vient du descripteur ; les fichiers de traduction (par exemple `web/src/lib/i18n/devices.ts` de DV-9) ne portent que les libellés et descriptions, et toute clé de nom de device qui y entre est listée avec les autres (`DEVICE_NAME_KEYS`, `LANDING_DEVICE_NAME_KEYS`), dont un test vérifie qu'elles sont identiques en EN et en FR.
- Un fichier par device (`crates/engine/src/devices/<kind>.rs`) et un registre d'une ligne par device. Le lecteur de clips audio (`faulk.clip_player`, AU-4) est un device de rôle instrument : une piste audio est une piste dont l'instrument lit des clips audio.
- Les futurs modules de l'Atelier implémenteront ce même contrat.

**Décision CT-5 (#41, 2026-10-14) : contrat figé dans `crates/engine/src/device.rs`, rôle « effet MIDI » compris (#170).** Ce que le code fige :

- **Rôles** (`DeviceRole`) : instrument (notes en entrée, stéréo en sortie, au plus un par piste), effet audio (stéréo transformée sur place), **effet MIDI** (notes en entrée, notes en sortie, `EventBuffer`). Ordre de la chaîne : effets MIDI, instrument, effets audio ; effets MIDI sur les pistes MIDI seulement. Le modèle de projet ne connaît pas encore ce rôle et exige l'instrument en tête de chaîne (`InstrumentNotFirst`) : son amendement est une tâche à part.
- **Événements** (`Event`, 24 octets au plus, testé), chacun avec son offset dans le bloc : `NoteOn { voice, key, velocity 0–1, tune_cents }`, `NoteOff { voice, key }`, `AllNotesOff`, `ParamValue { id, value }`. La hauteur d'une note est `key + tune_cents / 100` demi-tons (accord fin, ±1 200 cents, pour #167). Une note est désignée par son `VoiceId` (24 bits, l'identifiant de voix de l'Atelier § 1.3) : un `NoteOff` termine la note de même `VoiceId`, `key` n'est qu'informatif. Les identifiants qu'écrit un effet MIDI sont son propre espace ; l'hôte les remplace par des identifiants neufs avant le device suivant.
- **Garanties de l'hôte** (MO-4) : événements triés par offset, offset < `frames` ; à un même offset, les valeurs de paramètres, puis tous les événements de notes dans l'ordre où ils ont été émis (`Event::rank`, fusion stable) : une note démarre avec les valeurs posées à son offset, un note-off puis un note-on est un redéclenchement, un note-on puis un note-off de la même voix (une frappe plus courte qu'un bloc) donne une note qui s'arrête aussitôt et jamais une note bloquée, un note-on émis avant une relâche est relâché (débordement live de CT-3), un note-on émis après un arrêt joue ; la sortie d'un effet MIDI, triée par offset dans l'ordre où il l'écrit, n'a pas à être retriée ; un device ne reçoit que ses paramètres, et des notes s'il est instrument ou effet MIDI ; un bloc ne traverse jamais un bouclage ni un déplacement. Un device ne panique jamais si une garantie manque : offset ramené à la dernière frame ou au « maintenant », identifiant inconnu ignoré, NaN ignoré, valeur bornée et arrondie au pas.
- **Lissage** : un paramètre à lissage nul s'applique à l'offset exact ; sinon, avec `n = round(lissage × fréquence)`, la valeur à la frame `k + i` est `départ + (cible − départ) · min(i, n) / n` ; une nouvelle valeur pendant une rampe repart de la valeur courante ; un paramètre à pas ou énuméré n'est jamais lissé. `set_param` vaut un `ParamValue` à l'offset 0 du bloc suivant ; `reset` vide l'état DSP et amène les rampes à leur cible.
- **Trait** : `Device { descriptor, process(ctx, events, io), set_param, reset }`, sans allocation, verrou ni panique ; construit dans `prepare` par une `Factory` à allocation faillible (`MsgError::OutOfMemory`) ; l'état initial d'un device intégré est la liste de ses valeurs, appliquée par `set_param` puis `reset` avant le thread audio (l'encodage est de MO-1). `BlockCtx` porte la fréquence, la taille du bloc et le transport (lecture, tempo, signature, position en ticks fractionnaires) : l'horloge qu'un effet MIDI demande.
- **Sortie d'un effet MIDI** : l'hôte dimensionne l'`EventBuffer` dans `prepare` à partir des bornes que le descripteur déclare (`MidiOutput { per_input, per_block }`) et du nombre d'événements qu'il peut lui donner dans un bloc, et le reconstruit plus grand quand ses tables grandissent (décision 7) : il ne se remplit pas. Le débordement reste un filet de sécurité : une place réservée reçoit un `AllNotesOff` après le dernier événement, pour qu'aucune note ne reste bloquée.
- **Descripteur** (`DeviceDescriptor`, statique) : `kind` (`faulk.osc_synth`), `kind_id` (le numéro de `ControlMsg::DeviceInsert`, jamais réattribué), `version`, nom anglais jamais traduit, rôle, mémoire maximale (à 192 kHz, additionnée par le projet sous le budget moteur de CT-3), bornes de sortie MIDI, paramètres : `id` numérique stable, `key` texte stable (clé de la carte des paramètres du projet et clé de traduction), unité, min, max, défaut, échelle, pas, temps de lissage, valeurs énumérées. `validate` vérifie chaque règle (clés au format du projet, au plus 10 000 paramètres…). Un module de l'Atelier, dont les paramètres ne sont connus qu'à la compilation, obtient son descripteur statique par une fuite bornée par le nombre de compilations.
- **Export** : le bundle de schémas (`crates/protocol`) porte `devices` (les descripteurs intégrés, aujourd'hui `faulk.osc_synth`, « Oscillator », six paramètres, et `faulk.drums`, « Drums », seize paramètres) et `device_descriptor` (leur schéma). Le synthé implémente le trait et suit le lissage linéaire déclaré depuis DV-1 (paragraphe suivant) ; AM-3 écrit le lisseur commun.
- **DV-1 (#63, 2026-10-14)** : le synthé est le device `faulk.osc_synth` (`crates/engine/src/devices/osc_synth.rs`), au registre `engine::devices::REGISTRY` (une ligne par device, résolu par `engine::builtin_factory` et exporté par `builtin_descriptors`). Ses notes sont désignées par leur `VoiceId`, ses événements tombent à l'échantillon près. Décision sur le lissage : le gain et le sustain suivent les rampes linéaires déclarées (10 ms et 5 ms) au lieu du filtre à un pôle et de la pente fixe. À gain et sustain constants, le rendu est identique à l'échantillon près à celui de l'ancien synthé (rendu de référence, `crates/engine/tests/osc_synth.rs`) ; quand ils bougent, l'écart reste sous l'amplitude du geste pendant la rampe, puis sous 1e-5 (l'ancien filtre s'arrêtait à quelques millionièmes de sa cible). Un sustain changé pendant le déclin laisse un écart de moins de 2e-3 jusqu'à ce que la voix atteigne le sustain. Le panneau de l'interface vient du descripteur (`DEVICES` du protocole généré) : nom, paramètres, bornes, échelle, unité.
- **DV-6 (#68, 2026-10-18)** : la batterie est le device `faulk.drums` (`kind_id` 2, `crates/engine/src/devices/drums.rs`), une ligne de plus au registre. Quatre sons synthétisés, sans échantillon, sur les notes d'une table de batterie : grosse caisse 36, caisse claire 38, clap 39, charleston fermé 42 et ouvert 46 (un seul son : une frappe fermée coupe une ouverte). Les autres notes sont ignorées ; un `NoteOff` aussi (frappe sans durée, le son s'éteint seul) ; `AllNotesOff` éteint tout en 3 ms ; une frappe sur un son qui sonne encore le fait descendre en 3 ms avant de repartir. Seize paramètres, par son (`kick_tune`, `snare_decay`, `clap_tone`, `hat_open_decay`…) et un `gain` général : réglage, déclin (durée de chute de 60 dB), timbre, niveau. Réglage, déclin et timbre sont lus à la frappe (jamais lissés) ; les niveaux et le gain suivent la rampe de 10 ms déclarée. Le bruit vient d'un générateur entier qui repart du même état au `reset` : un rendu est identique à chaque fois, en natif comme en WASM (écart mesuré 9e-8). Le choix des sons et de leurs valeurs par défaut est une décision sonore du fondateur, à valider à l'écoute (`.qa/renders/drums-*.wav`).
- **Vérification** : trois devices de test (fonctionnalité `testing` du moteur : un instrument, un effet, un effet MIDI) et une passe de conformité (`device::testing::conformance`) que DV-1 et les tâches suivantes font passer à chaque device du registre ; rendus hors ligne à l'échantillon près pour des blocs de 1 à 2 048 frames, sans allocation, et parité native / WASM du même scénario.

### 2.6 Interface

- `Session` est découpée en : **dispatcher** (`run(command)` générique vers `project-wasm`), **store projet** (miroir réactif alimenté par les `ProjectEvent`), **store moteur** (statut, mode, latence, tête de lecture, vu-mètres, alimenté par le `Feedback`).
- Les vues lisent les stores et n'appellent que `run`.
- Composants partagés à figer tôt : `TimeGrid` (règle, zoom, aimantation, conversion ticks ↔ pixels), `ParamControl` (bouton généré depuis un descripteur), `TrackHeader` (nom, couleur, mute, solo, armement), `Waveform` (aperçu de forme d'onde depuis le cache de crêtes).

## 3. Épopées et tâches

Chaque tâche tient en une pull request pour un agent et donne : **But**, **Critères**, **Touche** (dossiers), **Dépend de**, **Phase**, **Étiquettes**. Les critères s'ajoutent à la Definition of done de `CLAUDE.md` (fmt, clippy, tests, build WASM, parité, Playwright pour le web, schéma et undo testés pour les commandes).

Étiquettes :

- `hardware: oui` : la tâche exige le matériel audio ou MIDI du fondateur, ou une vérification manuelle sous Windows ; elle tourne en local (décision 15) et porte `ready-for-human`.
- `founder: oui` : la tâche attend une décision du fondateur (comportement produit à valider dans la PR) ou livre des textes visibles par l'utilisateur (EN et FR) qu'il valide dans la pull request (règle de `CLAUDE.md`) ; une tâche qui attend une décision ne porte pas `ready-for-agent`. Une tâche dont seuls les textes attendent sa validation reçoit `ready-for-agent` une fois débloquée (décision du fondateur du 2026-10-14) : le travail ne l'attend pas, seule la fusion de sa PR l'attend.

Préfixes : `CT` contrats, `PR` projet et commandes, `MO` moteur, `TR` transport, `DV` devices, `CL` clips, `GR` grille de lancement, `AR` arrangement, `PN` piano roll, `RE` enregistrement MIDI, `MX` mixeur, `SV` sauvegarde, `AG` agents, `BO` mode Boost, `UI` interface, `AU` pistes audio, `AM` automation, `QA` qualité, `DOC` documentation. La numérotation de l'étude est conservée : il n'y a pas de DV-7 ; BO-4 est l'issue #10 déjà ouverte.

### Épopée 0 — Contrats et décisions (CT)

Chaque contrat est figé **en code** (types, traits, schémas) dans la pull request de sa tâche ; la décision et ses raisons sont écrites dans le plan de la tâche, la description de la pull request et une section de spec. Conformément à [domain docs](../agents/domain.md), l'ADR ne s'écrit qu'**après** le code qui la met en œuvre, jamais en préalable.

**CT-1 — Appliquer la décision 1 : crates `project` et `project-wasm`, la page possède le projet**
- But : appliquer § 2.1 (décidé le 2026-10-14) : créer les crates et mettre la documentation d'architecture en accord.
- Critères : `CLAUDE.md` (table Stack and layout) et l'[architecture](../architecture/01-architecture.md) mis à jour ; squelettes vides de `crates/project` et `crates/project-wasm` dans le workspace (compilent, aucun code métier, `project-wasm` construit en `wasm32-unknown-unknown`) ; la règle « le helper relaie, ne possède pas le projet » écrite dans le plan et la PR.
- Touche : `docs/architecture/`, `CLAUDE.md`, `Cargo.toml`, `crates/project*`.
- Dépend de : aucune.
- Phase : 0.
- Étiquettes : hardware: non, founder: non.

**CT-2 — Contrat : modèle de temps musical**
- But : figer § 2.2.
- Critères : types `Tick`, `Frame`, `Tempo`, `TimeSignature`, `Quantize` (aucune, 1/16, 1 temps, 1, 2, 4, 8 mesures ; défaut 1 mesure), `LoopRegion` avec conversions et tests (aller-retour ticks ↔ échantillons à 44,1/48/96 kHz, arrondis documentés ; prochaine frontière de quantification depuis une position quelconque, testée pour chaque valeur).
- Touche : module `time` (probablement dans `engine`, réexporté, puisque moteur et projet en ont besoin ; à décider dans le contrat).
- Dépend de : CT-1.
- Phase : 0.
- Étiquettes : hardware: non, founder: non.

**CT-3 — Contrat : synchronisation projet → moteur (prepare/apply, croissance sans plafond, feedback)**
- But : figer § 2.3.
- Critères : énumérations `ControlMsg`, `RtMsg`, `Feedback` déclarées (variantes du MVP listées, même vides, y compris automation et sources audio), format binaire décrit (en-tête, version, endianness) ; mécanisme de croissance des structures écrit (table pleine → `prepare` construit la suivante → `apply` échange → ancienne aux déchets) et limites de sûreté chiffrées ; voie WASM choisie par un prototype mesuré des deux voies du § 2.3 (pour (b) : chaîne d'outils avec atomics, deux instances sur une mémoire, allocateur partagé ; pour (a) : coût par message et par croissance sur le thread de rendu), avec le budget que MO-2 devra tenir ; si la voie retenue impose de quitter la chaîne `stable`, ce choix est présenté au fondateur dans la PR ; attaque écrite : file d'entrée live pleine, message inconnu, limite de sûreté atteinte, croissance pendant la lecture, `memory.grow` pendant que le worklet tient des vues, `memory.grow` refusé, plafond de 4 Gio atteint, bascule de moteur en cours de message, libération sur le thread de rendu.
- Touche : `crates/engine/src/msg.rs` (types seulement), `web/src/lib/engine/` (prototype).
- Dépend de : CT-1, AU-0.
- Phase : 0.
- Étiquettes : hardware: non, founder: oui (chaîne d'outils : si la voie retenue impose de quitter `stable`, le fondateur valide ce choix dans la PR).

**CT-4 — Contrat : contrat de commande v2**
- But : figer § 2.4 ; choisir entre le modèle mutable à inverses et le modèle persistant à instantanés (§ 2.1) ; trancher l'état de vue, l'existence et la **largeur** du `NoteId` (l'ABI des modules de l'Atelier n'a que 24 bits pour l'identifiant de note, voir la [spec de l'Atelier](01-atelier-modules.md) § 1.3).
- Critères : signature `apply` figée en code ; domaines du MVP figés (`project`, `track`, `device`, `clip`, `note`, `scene`, `launch`, `arrangement`, `automation`, `transport`, `mixer`, `record`, `midi`, `live`, `history`, `view`) ; règle de test « chaque commande a description, schéma d'entrée et de sortie, undo testé ».
- Touche : `crates/commands` (squelettes des modules).
- Dépend de : CT-1.
- Phase : 0.
- Étiquettes : hardware: non, founder: non.

**CT-5 — Contrat : contrat de device**
- But : figer § 2.5, événements de paramètres datés et lissage déclaré compris.
- Critères : trait et descripteurs écrits dans `crates/engine/src/device.rs` ; `Event` porte notes et `ParamValue` avec offset ; le descripteur déclare pas et temps de lissage ; le schéma exporté contient les descripteurs (exemple avec le synthé actuel).
- Touche : `crates/engine`.
- Dépend de : CT-3.
- Phase : 0.
- Étiquettes : hardware: non, founder: non.

**CT-6 — Glossaire du domaine**
- But : créer `GLOSSARY.md` à la racine (projet, piste MIDI, piste audio, device, instrument, effet, lecteur de clips, scène, emplacement de clip, clip, note, arrangement, placement, lancement, quantification, armement, tête de lecture, prise, couloir d'automation, point, **version**, **variante** (décision 9), blob, session).
- Critères : chaque terme a sa définition et son identifiant de code ; aucun nom d'autre logiciel.
- Touche : `GLOSSARY.md`.
- Dépend de : aucune.
- Phase : 0.
- Étiquettes : hardware: non, founder: non.

**UI-0 — Anti-conflits côté web et fichiers générés**
- But : préparer le dépôt au travail parallèle (§ 5).
- Critères : i18n découpée par fonctionnalité (`web/src/lib/i18n/<feature>.ts`, assemblage typé, test de complétude EN/FR) ; registre de vues (`web/src/views/registry.ts`, une ligne par vue) ; fichiers générés (`web/src/lib/generated/*`) produits au `prebuild`/`pretest` et vérifiés en CI au lieu d'être fusionnés à la main (ou retirés de git : décision dans la PR) ; CI découpée en workflows par domaine.
- Touche : `web/src/lib/i18n*`, `web/src/views/`, `web/scripts/`, `.github/workflows/`, `.gitattributes`.
- Dépend de : aucune.
- Phase : 0.
- Étiquettes : hardware: non, founder: non.

**AU-0 — Isolation cross-origin et ressources auto-hébergées**
- But : appliquer la décision 4 : la page de l'application est isolée cross-origin (`Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: require-corp` ou `credentialless`, ou `Document-Isolation-Policy` là où elle existe), pour que `SharedArrayBuffer` et la mémoire WASM partagée soient disponibles au canal moteur (CT-3, MO-2) et au streaming audio (AU-3).
- Critères : `crossOriginIsolated === true` vérifié par un test Playwright en dev (`pnpm dev`) et sur le build de production ; polices et bibliothèques auto-hébergées (aucune ressource tierce sans CORS ni `Cross-Origin-Resource-Policy`, test par énumération des requêtes réseau) ; les WebSockets vers le helper (`web/src/lib/engine/helper-client.ts`, `native-engine.ts`) et le Web MIDI simulé fonctionnent toujours sous isolation (tests Playwright) ; la page d'accueil du site, si elle partage l'origine, n'est pas affectée (ou l'est sans casse, vérifié) ; conséquences notées dans les [contraintes](../constraints/01-contraintes.md) (fenêtres d'autres origines, Safari sans `credentialless`).
- Touche : `web/vite.config.ts`, configuration de l'hébergement, `web/public/`, `web/tests/e2e/`, `docs/constraints/`.
- Dépend de : aucune.
- Phase : 0.
- Étiquettes : hardware: non, founder: non.

**BO-0 — Vérifier le mode Boost existant**
- But : vérifier sur la machine du fondateur le code Boost de l'issue #2 avant d'empiler dessus.
- Critères : appairage, son, MIDI natif, bascule aller-retour vérifiés ; latence **mesurée** (méthode notée) ; défauts ouverts en issues `Bug`.
- Touche : `crates/helper` (corrections seulement), [S1](../spikes/S1-moteur-hybride.md).
- Dépend de : aucune (l'issue #2 est fusionnée).
- Phase : 0.
- Étiquettes : hardware: oui, founder: non.

### Épopée 1 — Projet et couche de commandes (PR)

**PR-1 — Modèle de projet**
- But : implémenter `crates/project` selon le [modèle de projet](../architecture/02-modele-projet.md) : projet, pistes (MIDI, audio), devices, scènes, emplacements de clips, clips MIDI et audio (référence de blob, plage en frames, boucle), notes, placements, couloirs d'automation, réglages du transport (quantification globale), identifiants stables, clés d'ordre fractionnaires, limites de sûreté ; plus l'**état de session** (`SessionState` : solo par piste, entrée MIDI choisie), distinct du document et jamais sérialisé avec lui (décision 10).
- Critères : invariants vérifiés par `validate()` (identifiants uniques, références valides, notes dans la longueur du clip, au plus un point par tick, limites de sûreté) ; sérialisation aller-retour ; tests de propriétés sur la génération d'identifiants et de clés d'ordre.
- Touche : `crates/project`.
- Dépend de : CT-1, CT-2, CT-4.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**PR-2 — Couche de commandes v2 sur le projet**
- But : refondre `crates/commands` en modules par domaine, `apply` selon le contrat CT-4 → `Outcome { output, undo, events, control }`, historique (annuler, rétablir, groupes, auteur) ; migrer les commandes existantes (`note.on` devient `live.note_on { track }`, `synth.set_param` devient `device.set_param`, `midi.*` inchangées).
- Critères : un test parcourt **toutes** les commandes (énumération générée, pas une liste écrite à la main) et vérifie description, schémas, `x-undo`, `x-kind` ; aller-retour apply → undo → état identique pour chaque commande `edit` ; `history.undo`, `history.redo`, `history.begin_group`, `history.end_group`.
- Touche : `crates/commands`, `crates/protocol` (export du schéma).
- Dépend de : PR-1.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**PR-3 — Compilateur projet → moteur et instantané**
- But : chaque `Outcome` porte les `ControlMsg` du changement ; `snapshot(&Project) -> Vec<ControlMsg>` rejoue un projet entier sur un moteur vide (bascule de mode, ouverture).
- Critères : « appliquer N commandes en direct » et « instantané du projet final » produisent le **même rendu** hors ligne (harnais QA-1) ; table identifiant → emplacement moteur tenue par le projet, réutilisation des emplacements libérés et croissance de la table (§ 2.3) testées.
- Touche : `crates/commands`, `crates/project`.
- Dépend de : PR-2, MO-1, QA-1.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**Mise en œuvre PR-3 (#48, 2026-10-15) : `crates/commands/src/engine_msgs.rs`, `crates/project/src/slots.rs`.** Ce que le code ajoute, et ce qu'il laisse :

- **Compilation** : `contract::ControlMsg` est le type du moteur. `Command::apply` (la racine) appelle `engine_msgs::compile` après la commande : les notes jouées, la lecture, l'arrêt et le déplacement sont traduits d'après la commande, tout le reste d'après ses `ProjectEvent` lus sur l'état d'après (piste créée : la piste entière ; piste supprimée : la piste, ses clips et ses couloirs retirés ; clip créé, supprimé ou changé ; paramètre ; mixeur ; solo ; sélection ; tempo, signature, boucle, métronome). `history.*` ne compile rien lui-même : annuler et rétablir appliquent leurs commandes par `Command::apply`, une seule fois chacune.
- **Instantané** : `snapshot(&Project, &SessionState)` (la session donne le solo et la sélection, que le moteur rend) renvoie les messages et la table qu'ils attribuent ; `resync` y ajoute le `Reset` et adopte la table. Les envois de notes, de placements et de points sont découpés en morceaux de 65 536.
- **Table des emplacements** (`EngineSlots`) : dans `Context`, comme le générateur d'identifiants, car elle dépend de l'ordre des créations et suppressions et appartient à une connexion moteur ; un hôte garde un seul `Context` par moteur et change l'auteur et le groupe par appel. Plus petit emplacement libre, croissance sans plafond, emplacement d'un device = sa place dans la chaîne. Un groupe annulé qui échoue rend aussi la table. Une entité sans emplacement (un moteur jamais synchronisé) ne reçoit aucun message.
- **Limites** : le projet avait des limites par piste seulement pour les clips, les placements et les couloirs ; les emplacements du moteur sont globaux. `project::limits` ajoute `MAX_CLIPS`, `MAX_PLACEMENTS`, `MAX_LANES` (ceux du moteur), vérifiés par `validate()` et refusés par `clip.create`, `clip.duplicate`, `clip.restore` et `track.duplicate` (`limit`).
- **Hôtes** : `Synced` (état, contexte, flux) remplace le mappage sans état de MO-2 dans `project-wasm` (nouveau code `ERR_REFUSED = -101`), le helper et le harnais QA-1 ; la page et le helper tiennent chacun une copie du projet à un synthé jusqu'à PR-4, PR-5 et BO-1. Les commandes `transport.play`, `stop` et `seek` produisent leurs messages : `play` part de la dernière position donnée par `seek` (0 au début), aussi après un arrêt, et n'envoie rien pendant la lecture (le `Play` du moteur positionne toujours, et la tête de lecture n'est pas encore remontée) ; choix à valider par le fondateur.
- **Flux refusé** : la page le range dans `EngineStore.streamError`, arrête le moteur Web, affiche le message (PR-5) ; « Démarrer l'audio » relance un moteur neuf qui repart d'un instantané (`Synced::resync`).
- **Pas encore envoyé** : les clips audio et leurs placements (AU-3), les devices qui ne sont pas des devices intégrés du registre (lecteur de clips, modules) ; le clip du moteur n'a pas de région de boucle (`looped` seulement) et ses notes n'ont ni accord fin, ni chance, ni répétition (CL-2).
- **Un instantané envoyé d'un bloc** au-delà d'environ 8 192 messages préparés (anneau et file d'attente) est arrêté par `EngineBusy` : l'envoi rythmé d'un gros projet revient à l'hôte (#245).

**PR-4 — `project-wasm` : le cœur projet dans la page**
- But : compiler `project` + `commands` en WASM pour le thread principal ; API `run(json) -> json` (sortie, événements, octets moteur), `snapshot()`, `load(files)`, `save() -> fichiers sales`.
- Critères : test Vitest qui crée un projet, des pistes et des notes par commandes ; types TypeScript générés depuis le schéma ; taille du `.wasm` mesurée et notée.
- Touche : `crates/project-wasm`, `web/scripts/`, `web/src/lib/project/`.
- Dépend de : PR-2, PR-3.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**Mise en œuvre PR-4 (#49, 2026-10-18) : `crates/project-wasm`, `crates/commands/src/host.rs`, `web/src/lib/project/core.ts`.** Ce que le code ajoute, et ce qu'il laisse :

- **ABI** : exports C simples (`pw_*`), sans `wasm-bindgen` (son outil en ligne de commande serait une dépendance de plus à épingler pour une dizaine d'appels). La page écrit du JSON dans la boîte de réception, appelle un export, lit le JSON rendu et, à part, les octets moteur ; chaque appel remplace ces octets (vides quand il n'en produit pas), jamais ceux d'un appel précédent.
- **Un cœur** (`Core`) : le projet, sa session et son historique, le contexte des commandes et le flux du moteur (`Synced`), plus ce qui a été écrit en dernier. Création vide ou à un synthé ; les identifiants sont semés par l'horloge et 80 bits d'aléa de la page (`pageSeed`).
- **Appels** : `run(commande)` → `RunReport` (`applied` avec sortie, événements et `stream_error`, `refused` avec l'erreur typée, `invalid`) et octets moteur ; `setup()` (époque courante) et `snapshot()` (époque suivante) → `SnapshotReport` et octets ; `project()` → `ProjectView` (projet et session, ce que la page relit après `track_created` ou `clip_created`) ; `load(fichiers)` → `LoadReport` (historique vidé, instantané dans l'époque suivante) ; `save()` → `SaveReport` (`ProjectChanges` : fichiers à écrire et à supprimer) ; `saved(changements)` enregistre ce que l'hôte a écrit. Types générés dans `web/src/lib/generated/protocol.ts`.
- **Enregistrement en deux temps** : `save()` ne marque rien ; seul `saved()` fixe la référence, avec le texte écrit. Une écriture qui échoue laisse le fichier sale, une modification faite entre `save()` et `saved()` aussi. Un projet chargé est propre.
- **Fichiers provisoires** jusqu'au format dossier de SV-1 (#93) : `project.json` contient le `Project` sérialisé entier et `.faulk/session.json` la session. Rien ne les écrit encore sur disque (SV-2). Un fichier de session illisible n'empêche pas d'ouvrir le projet : la session par défaut le remplace (`session_reset`) et le fichier reste tel quel tant que la session ne change pas.
- **Taille** (build release, 2026-10-18) : `project.wasm` pèse 1 748 910 octets, dont 322 159 de section `name`, et 442 877 octets compressé (`gzip -9`).
- **Pas encore** : la copie du projet dans le helper reste jusqu'à BO-1 (le cœur unique de la page, le store projet et la bascule par instantané sont de PR-5, ci-dessous).

**PR-5 — Dispatcher web générique et stores**
- But : remplacer le `switch` de `Dispatcher.run` (`web/src/lib/stores/dispatcher.ts`) par `run(command)` → `project-wasm` → octets moteur vers le moteur actif ; stores projet et moteur (`wasm-ops.ts` est déjà supprimé par MO-2, qui a posé un premier encodeur `project-wasm` provisoire).
- Critères : aucune référence à un type de commande dans le dispatcher ; l'écran actuel (synthé, clavier, MIDI) fonctionne via les nouvelles commandes ; test Playwright : note jouée, son mesuré non nul, aucune erreur console.
- Touche : `web/src/lib/stores/`, `web/src/lib/engine/`.
- Dépend de : PR-4, MO-2, UI-1.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**Mise en œuvre PR-5 (#50, 2026-10-18) : `web/src/lib/stores/dispatcher.ts`.** Ce que le code fait :

- **Un seul cœur, celui du dispatcher** : `ProjectCore` créé au premier `start()` (projet à un synthé, époque 1) et gardé tant que la page vit ; `WebEngine` n'a plus de cœur. `run(commande)` : le cœur applique, le moteur actif reçoit ce que la commande lui veut (`Engine.send(commande, octets)` : le moteur Web prend les octets encodés, le helper prend la commande, sa copie provisoire ignorant les octets), le store projet reçoit les événements. Une commande refusée par le projet ne part nulle part. Sans moteur (audio pas démarré, ou arrêté sur un flux refusé), `run` ne fait rien.
- **Store projet** : miroir (`view`, drapeaux annuler/rétablir) nourri par les `ProjectEvent` ; `device_param_changed` met à jour le miroir en place, tout autre événement relit le projet du cœur.
- **Effets propres à la page** (`effects.ts`) : le dispatcher ne nomme aucune commande ; l'entrée MIDI Web et les notes « qui sonnent » du clavier, qui ne sont ni dans le projet ni dans un événement, sont une table commande → effet. Les commandes que la page émet d'elle-même (relâcher les notes, demander les entrées MIDI du helper) sont dans `housekeeping.ts`. La mise à niveau de la copie du helper à l'entrée en Boost est dans `engine/helper-copy.ts` (BO-1 la supprime).
- **Bascule** : retour au mode Web = instantané du cœur (`Reset` à l'époque suivante) poussé au moteur Web ; entrée en Boost = les octets du cœur ne vont plus nulle part, le helper reçoit les commandes.
- **Flux refusé** : le moteur Web est arrêté, `streamError` posé et le message affiché (EN « The audio engine stopped taking changes. Click "Start audio" to continue. », FR « Le moteur audio n'accepte plus les changements. Clique sur « Démarrer l'audio » pour continuer. », texte validé par le fondateur le 2026-10-18) ; le bouton « Démarrer l'audio » relance un moteur neuf depuis un instantané, le projet est conservé. Un événement tardif d'un moteur arrêté est ignoré (génération du moteur Web).
- **Pas encore** : un contexte audio suspendu par le navigateur en mode Web ne suspend pas l'envoi (voir « file d'attente » ci-dessus) ; la bascule Boost → Web n'est vérifiée dans un vrai navigateur que contre un helper de substitution.

**PR-6 — Commandes de requête pour les agents**
- But : `project.get`, `track.list`, `track.get`, `clip.get`, `clip.notes.list`, `scene.list`, `device.describe`, `automation.lane.list`, `transport.get` (`x-kind: query`).
- Critères : sorties typées et documentées ; aucune requête ne modifie le projet (égalité avant/après) ; pagination des listes de notes et de points.
- Touche : `crates/commands`.
- Dépend de : PR-2.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**PR-7 — Annuler, rétablir et historique dans l'interface**
- But : boutons et raccourcis annuler/rétablir, panneau d'historique (libellé, auteur, heure).
- Critères : un glisser de note = une entrée ; l'historique affiche l'auteur ; test Playwright.
- Touche : `web/src/views/history/`, `web/src/lib/i18n/history.ts`.
- Dépend de : PR-5.
- Phase : 3.
- Étiquettes : hardware: non, founder: oui.

**PR-8 — Commandes de pistes**
- But : `track.create { kind: midi | audio, instrument?, name?, color? }`, `track.delete` (destructive), `track.rename`, `track.set_color`, `track.move`, `track.duplicate`, `view.select_track`.
- Critères : la limite de sûreté (§ 2.3) est refusée par une erreur typée, et en dessous la création ne connaît aucun plafond (test à 1 000 pistes sur le projet) ; `track.delete` puis undo restaure piste, devices et clips à l'identique.
- Touche : `crates/commands/src/track.rs`, `crates/project`.
- Dépend de : PR-2, DV-1.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

### Épopée 2 — Moteur : graphe et synchronisation (MO)

**MO-1 — Messages moteur et cycle prepare/apply**
- But : `ControlMsg` (encodage et décodage binaires), `Engine::prepare`, `Engine::apply`, file de déchets, croissance des structures hors chemin audio (§ 2.3).
- Critères : décodage de n'importe quels octets sans panique (fuzz ou proptest) ; `apply` sous détecteur d'allocation, y compris pendant l'échange d'une table pleine contre une plus grande ; en WASM, une croissance de la mémoire linéaire déclenchée par `prepare` pendant la lecture ne produit ni trou ni plantage (test), et un `memory.grow` refusé remonte en erreur typée ; même comportement en natif et en WASM.
- Touche : `crates/engine/src/msg.rs`, `crates/engine/src/engine.rs`.
- Dépend de : CT-3.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**MO-2 — Un seul canal vers le moteur, dans les deux cibles**
- But : `engine-wasm` expose `fk_push(ptr, len)` et `fk_feedback()` ; le worklet devient générique (octets in, octets out) par la voie que CT-3 a retenue (anneau en mémoire partagée écrit par le Worker de `prepare`, ou boîte de réception préallouée alimentée entre deux quanta) ; le helper consomme le même flux (thread de contrôle + `rtrb`).
- Critères : `worklet.ts` ne connaît aucun type de commande ; test de parité natif/WASM sur un flux de messages enregistré ; anciens exports `fk_note_on`… supprimés ; aucune allocation dans `process` ; le coût par message sur le thread de rendu est mesuré et tient dans le budget fixé par CT-3 ; le test de croissance de MO-1 (`crates/engine-wasm/tests/cycle_parity.rs`) repris sur la cible retenue : une croissance de la mémoire partagée déclenchée par le Worker de `prepare` pendant que le worklet rend ne produit ni bloc silencieux ni plantage (vues reprises), et un `memory.grow` refusé remonte en erreur typée.
- Touche : `crates/engine-wasm`, `web/src/lib/engine/`, `crates/helper/src/audio.rs`.
- Dépend de : MO-1, AU-0.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**MO-3 — Graphe de pistes et chaînes de devices**
- But : pistes allouées hors chemin audio et échangées dans `apply`, chacune instrument + effets en série, somme vers le master, tampons préparés hors chemin audio.
- Critères : rendu de 8 pistes avec le synthé ; ajouter ou supprimer une piste pendant la lecture sans clic (fondu court) ni allocation dans `process`, y compris quand l'ajout fait grandir la table des pistes ; une piste vide ne coûte pas de calcul notable (mesuré).
- Touche : `crates/engine/src/graph.rs`.
- Dépend de : MO-1, CT-5.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**MO-4 — Événements horodatés à l'échantillon près**
- But : les événements (notes live, notes de clips, valeurs de paramètres, départs de clips audio) portent un offset dans le bloc ; le moteur découpe le bloc et route vers le device de la piste cible.
- Critères : une note programmée à l'échantillon *k* démarre exactement à *k* (détection d'onset) pour des blocs de 64, 128, 512, 2048 ; les événements des clips et de l'automation sont lus dans leurs tables sans file bornée (test : 10 000 événements dans un bloc, aucun perdu) ; seule l'entrée live passe par une file bornée, débordement compté et signalé dans le `Feedback`.
- Touche : `crates/engine/src/events.rs`, `crates/engine/src/graph.rs`.
- Dépend de : MO-3.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**MO-5 — Retour visuel multi-pistes**
- But : vu-mètres crête et RMS par piste et master, tête de lecture, charge processeur, compteurs de débordement et de sous-alimentation, à environ 30 Hz.
- Critères : aucune allocation sur le chemin audio (côté worklet, un tampon partagé réutilisé, jamais un `postMessage` d'objet) ; décodage côté page testé.
- Touche : `crates/engine`, `crates/engine-wasm`, `web/src/lib/engine/`, `crates/helper`.
- Dépend de : MO-3.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

### Épopée 3 — Transport et temps musical (TR)

**TR-1 — Horloge musicale du moteur**
- But : lecture, arrêt, déplacement de la tête, tempo, signature, boucle, conversion ticks ↔ échantillons.
- Critères : dérive nulle sur 10 minutes à 120 BPM ; boucle sans trou ni doublon d'événement à la jointure ; changement de tempo en lecture sans saut.
- Touche : `crates/engine/src/transport.rs`.
- Dépend de : MO-1, CT-2.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**TR-2 — Métronome**
- But : clic synthétisé (accent sur le premier temps), activation, volume, décompte optionnel avant enregistrement.
- Critères : clics à l'échantillon près (test d'onset) ; silence quand désactivé ; sortie sur le master hors vu-mètres des pistes.
- Touche : `crates/engine/src/metronome.rs`.
- Dépend de : TR-1, MO-3.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**TR-3 — Commandes de transport**
- But : `transport.play`, `transport.stop`, `transport.seek`, `transport.set_tempo`, `transport.set_time_signature`, `transport.set_loop`, `transport.toggle_loop`, `transport.set_metronome` (le domaine `metronome` n'existe pas : CT-4).
- Critères : tempo, signature, boucle et métronome sont des `edit` (annulables, enregistrés) ; lecture et arrêt sont `live` ; bornes de tempo refusées proprement.
- Touche : `crates/commands/src/transport.rs`.
- Dépend de : PR-2, TR-1.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**TR-4 — Barre de transport**
- But : lecture/arrêt, position en mesures:temps:ticks et minutes:secondes, tempo éditable, boucle, métronome, quantification de lancement globale (décision 8), enregistrement (désactivé jusqu'à RE-5), état du moteur (mode, latence, charge) toujours visible.
- Critères : chaque contrôle passe par une commande ; glyphes d'état du DS-0.1 ; test Playwright lecture/arrêt et changement de tempo.
- Touche : `web/src/views/transport/`, `web/src/lib/i18n/transport.ts`.
- Dépend de : PR-5, TR-3, UI-1.
- Phase : 2.
- Étiquettes : hardware: non, founder: oui.

### Épopée 4 — Instruments et effets (DV)

**DV-1 — Registre de devices et synthé porté**
- Nom affiché : Oscillator. Le panneau prend ce nom dans le descripteur ; la clé `synth.title` du catalogue web (et son entrée dans `DEVICE_NAME_KEYS`) disparaît alors, pour garder une seule source.
- But : registre des devices ; le synthé oscillateur actuel devient le device `faulk.osc_synth` avec son descripteur ; descripteurs exportés dans le schéma.
- Critères : rendu identique à l'ancien synthé (rendu de référence) ; aucun identifiant de paramètre écrit en dur côté web.
- Touche : `crates/engine/src/devices/`, `crates/protocol`.
- Dépend de : CT-5, MO-3.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**DV-2 — Filtre multimode**
- Nom affiché : Filter.
- But : passe-bas, passe-haut, passe-bande, résonance ; lissage des paramètres par le mécanisme commun (AM-3).
- Critères : atténuation mesurée à une octave de la coupure ; stable à résonance maximale ; ni NaN ni dénormaux.
- Touche : `crates/engine/src/devices/filter.rs`.
- Dépend de : DV-1, AM-3.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**DV-3 — Delay synchronisable au tempo**
- Nom affiché : Echo (le même device que l'extension Echo du catalogue, #161).
- But : temps libre ou en division, retour, mélange.
- Critères : écho mesuré au bon nombre d'échantillons ; tampon alloué dans `prepare` ; changement de tempo sans craquement.
- Touche : `crates/engine/src/devices/delay.rs`.
- Dépend de : DV-1, TR-1.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

**DV-4 — Réverbération**
- Nom affiché : Reverb.
- But : réverbération algorithmique (taille, amortissement, mélange).
- Critères : queue qui décroît vers le silence ; coût processeur mesuré.
- Touche : `crates/engine/src/devices/reverb.rs`.
- Dépend de : DV-1.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

**DV-5 — Compresseur**
- Nom affiché : Compressor (le même device que son extension du catalogue, #157).
- But : seuil, ratio, attaque, relâchement, gain de compensation.
- Critères : courbe statique mesurée conforme au ratio ; aucun dépassement non borné.
- Touche : `crates/engine/src/devices/compressor.rs`.
- Dépend de : DV-1.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

**DV-6 — Batterie synthétisée**
- Nom affiché : Drums.
- But : un deuxième instrument de nature différente : grosse caisse, caisse claire, charleston, clap, mappés sur des notes.
- Critères : chaque son déclenché par sa note, non silencieux et sans NaN ; paramètres par son dans le descripteur.
- Touche : `crates/engine/src/devices/drums.rs`.
- Dépend de : DV-1.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**DV-8 — Commandes de devices**
- But : `device.insert { track, kind, index }`, `device.remove` (destructive), `device.move`, `device.set_param`, `device.set_bypass`, `track.set_instrument`. L'index de la commande est traduit en clé d'ordre fractionnaire.
- Critères : `device.set_param` valide l'identifiant contre le descripteur et arrondit au pas déclaré ; l'undo de `device.remove` restaure les réglages.
- Touche : `crates/commands/src/device.rs`.
- Dépend de : PR-2, DV-1.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**DV-9 — Panneau de chaîne de devices**
- But : panneau du bas avec l'instrument et les effets de la piste sélectionnée, réglages générés (`ParamControl`), ajout depuis un navigateur d'instruments et d'effets, suppression, réordonnancement, contournement.
- Critères : aucun panneau écrit à la main par device ; un nouveau device apparaît sans changer l'interface ; test Playwright.
- Touche : `web/src/views/devices/`, `web/src/lib/components/ParamControl.svelte`, `web/src/lib/i18n/devices.ts`.
- Dépend de : PR-5, DV-8, UI-1.
- Phase : 2.
- Étiquettes : hardware: non, founder: oui.

### Épopée 5 — Clips (CL)

**CL-1 — Modèle et commandes de clips MIDI**
- But : `clip.create { slot | arrangement }`, `clip.delete`, `clip.duplicate`, `clip.rename`, `clip.set_length`, `clip.set_loop`, `clip.notes.add` (lot), `clip.notes.remove`, `clip.notes.update` (position, durée, hauteur, vélocité, en lot).
- Critères : un lot = une entrée d'historique ; notes hors longueur refusées ou tronquées selon le contrat ; aucun plafond de notes en dessous de la limite de sûreté (test à 100 000 notes).
- Touche : `crates/commands/src/clip.rs`, `crates/project`.
- Dépend de : PR-2.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**CL-2 — Lecteur de clips dans le moteur**
- But : tables de notes construites dans `prepare` et échangées dans `apply`, téléversement par fragments, lecture d'un clip en boucle synchronisée au transport.
- Critères : aucune note coincée à l'arrêt, au bouclage, à la suppression d'un clip en lecture ; note qui chevauche la fin de boucle coupée proprement ; parité natif/WASM.
- Touche : `crates/engine/src/clip_player.rs`.
- Dépend de : MO-4, TR-1.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**CL-3 — Édition d'un clip pendant sa lecture**
- But : remplacer atomiquement la version d'un clip en lecture (nouvelle table préparée, échange dans `apply`, ancienne table aux déchets).
- Critères : 1 000 éditions pendant la lecture : aucune allocation sur le chemin audio, aucune note coincée, pas de trou audible.
- Touche : `crates/engine`, `crates/commands`.
- Dépend de : CL-1, CL-2, PR-3.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

### Épopée 6 — Grille de lancement (GR)

**GR-1 — Lancement quantifié dans le moteur**
- But : lancer ou arrêter un clip (MIDI ou audio) ou une scène à la prochaine frontière de la quantification globale ; un clip à la fois par piste ; états (armé, en attente, en lecture, arrêt en attente) dans le `Feedback`.
- Critères : départ à l'échantillon de la frontière pour chaque valeur de quantification (aucune, 1/16, 1 temps, 1, 2, 4, 8 mesures) ; lancer un autre clip sur la même piste remplace le premier à la frontière sans note coincée.
- Touche : `crates/engine/src/launcher.rs`.
- Dépend de : CL-2.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**GR-2 — Commandes de scènes et de lancement**
- But : `scene.create`, `scene.delete`, `scene.rename`, `scene.move`, `launch.clip`, `launch.scene`, `launch.stop_track`, `launch.stop_all`, `transport.set_launch_quantization` (valeurs de la décision 8, défaut 1 mesure, enregistrée dans `project.json`).
- Critères : `launch.*` sont `live` ; les scènes et la quantification sont `edit` ; supprimer une scène qui contient des clips est destructive et confirmée.
- Touche : `crates/commands/src/scene.rs`, `crates/commands/src/launch.rs`.
- Dépend de : CL-1, GR-1.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**GR-3 — Vue grille de lancement**
- But : pistes en colonnes, scènes en lignes ; lancement par clic, état visible ; clip vide par double-clic ; ouverture du piano roll.
- Critères : fluide avec 32 pistes × 64 scènes (cible de mesure, pas un plafond) ; chaque état a un glyphe en plus de sa couleur ; test Playwright de lancement de scène.
- Touche : `web/src/views/grid/`, `web/src/lib/i18n/grid.ts`.
- Dépend de : GR-2, UI-2.
- Phase : 3.
- Étiquettes : hardware: non, founder: oui.

### Épopée 7 — Timeline d'arrangement (AR)

**AR-1 — Modèle et commandes d'arrangement**
- But : placements par piste (début, durée, décalage dans la source, boucle), écrits dans l'`arrangement.tsv` de chaque piste ; `arrangement.place`, `arrangement.move`, `arrangement.resize`, `arrangement.split`, `arrangement.delete`, `arrangement.duplicate { linked: bool }` (copie indépendante par défaut, clip lié sur option : décision 11).
- Critères : chevauchements résolus selon une règle écrite dans le contrat (le dernier posé gagne, l'autre est rogné) et testés ; dupliquer sans option crée un nouveau clip, avec `linked` un second placement du même clip.
- Touche : `crates/commands/src/arrangement.rs`, `crates/project`.
- Dépend de : CL-1.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

**AR-2 — Lecture de l'arrangement et priorité grille / arrangement**
- But : le moteur joue la timeline ; par piste, un clip lancé depuis la grille prend la main jusqu'à `launch.back_to_arrangement`.
- Critères : seek au milieu d'une note longue sans démarrage en retard ni en double (règle écrite) ; boucle de région ; parité natif/WASM.
- Touche : `crates/engine/src/arrangement.rs`, `crates/commands/src/launch.rs`.
- Dépend de : CL-2, TR-1, GR-1.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

**AR-3 — Composant `TimeGrid` partagé**
- But : règle (mesures, temps), zoom, défilement, aimantation, conversion ticks ↔ pixels, rendu canvas ; utilisé par la timeline, le piano roll et les couloirs d'automation.
- Critères : tests unitaires des conversions et de l'aimantation ; 60 images/s au défilement sur 200 mesures (mesuré dans Playwright).
- Touche : `web/src/lib/components/timegrid/`.
- Dépend de : UI-1, CT-2.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**AR-4 — Vue timeline d'arrangement**
- But : pistes, clips placés, glisser, redimensionner, couper, dupliquer (indépendant par défaut, lié par un modificateur), tête de lecture, région de boucle.
- Critères : chaque geste = une commande, un glisser = un groupe d'annulation ; test Playwright.
- Touche : `web/src/views/arrangement/`, `web/src/lib/i18n/arrangement.ts`.
- Dépend de : AR-1, AR-3, UI-2.
- Phase : 4.
- Étiquettes : hardware: non, founder: oui.

**AR-5 — De la grille vers l'arrangement**
- But : glisser un clip de la grille vers la timeline ; commande `arrangement.place_from_slot { linked: bool }` (copie indépendante par défaut, décision 11).
- Critères : la copie est indépendante de l'original (éditer l'un ne change pas l'autre) ; avec `linked`, le placement référence le clip de la grille ; tests des deux cas.
- Touche : `crates/commands`, `web/src/views/arrangement/`.
- Dépend de : GR-3, AR-4.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

### Épopée 8 — Piano roll (PN)

**PN-1 — Éditeur piano roll**
- But : clavier vertical, grille `TimeGrid`, dessin, sélection (clic, rectangle), déplacement, redimensionnement, suppression, ligne de vélocité, écoute de la note éditée, longueur et boucle du clip.
- Critères : chaque geste = une commande, un glisser = un groupe ; fluide avec 2 000 notes (mesuré) ; test Playwright dessin + annulation.
- Touche : `web/src/views/piano-roll/`, `web/src/lib/i18n/piano-roll.ts`.
- Dépend de : CL-1, AR-3, PR-5.
- Phase : 3.
- Étiquettes : hardware: non, founder: oui.

**PN-2 — Édition avancée**
- But : quantification (avec force), transposition, duplication de sélection, copier/coller, raccourcis.
- Critères : `clip.notes.quantize` testée sur des cas limites (note à cheval sur la fin du clip).
- Touche : `crates/commands/src/clip.rs`, `web/src/views/piano-roll/`.
- Dépend de : PN-1.
- Phase : 3.
- Étiquettes : hardware: non, founder: oui.

**PN-3 — Piano roll ouvert depuis la timeline**
- But : ouvrir le clip d'un placement, avec la tête de lecture dans le contexte de la timeline.
- Critères : la tête de lecture du piano roll suit la position dans le clip pour les deux origines (grille, timeline).
- Touche : `web/src/views/piano-roll/`, `web/src/views/arrangement/`.
- Dépend de : PN-1, AR-4.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

### Épopée 9 — Enregistrement MIDI (RE)

**RE-1 — Armement, monitoring et MIDI live vers les pistes**
- But : `track.arm`, `track.set_monitor` ; le MIDI live (Web MIDI, MIDI natif, clavier de l'ordinateur) va à la piste sélectionnée et aux pistes armées ; l'entrée MIDI choisie est écrite dans le `SessionState` (PR-1), que SV-1 enregistre ; les messages de contrôle (CC) sont décodés et disponibles pour l'automation (AM-5).
- Critères : changer de piste pendant une note tenue ne coince pas la note ; fonctionne en Web et en Boost (MIDI simulé dans les tests).
- Touche : `crates/commands/src/record.rs`, `crates/engine`, `web/src/lib/midi/`, `crates/helper/src/midi.rs`.
- Dépend de : MO-4, PR-8.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**RE-2 — Capture dans le moteur**
- But : horodater les entrées au tick, tampon d'enregistrement préparé hors chemin audio, notes capturées renvoyées dans le `Feedback` ; compensation de la latence de sortie.
- Critères : une note jouée sur un temps (MIDI simulé à horodatage connu) est enregistrée sur ce temps à ± 1 ms en Web, méthode notée ; tampon plein signalé, jamais d'allocation.
- Touche : `crates/engine/src/recorder.rs`, `web/src/lib/midi/`.
- Dépend de : RE-1, TR-1.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**RE-3 — Enregistrer dans un emplacement de la grille**
- But : enregistrer une prise dans un emplacement vide ; longueur arrondie à la quantification ; la prise devient un clip par **une** commande annulable (`record.commit_take`).
- Critères : annuler retire toute la prise ; enregistrement en boucle (overdub) sur un clip existant.
- Touche : `crates/commands/src/record.rs`, `crates/engine`.
- Dépend de : RE-2, GR-1.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**RE-4 — Enregistrer dans l'arrangement**
- But : enregistrer sur la timeline à la tête de lecture, avec décompte, et en boucle de région (prises superposées fusionnées).
- Critères : la prise est placée là où elle a été jouée ; une entrée d'historique par prise.
- Touche : `crates/commands/src/record.rs`, `crates/engine`.
- Dépend de : RE-2, AR-2.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

**RE-5 — Interface d'enregistrement**
- But : bouton d'enregistrement global, armement par piste, décompte, notes en cours d'enregistrement visibles.
- Critères : test Playwright avec Web MIDI simulé : enregistrer 4 notes, les voir dans le piano roll, annuler.
- Touche : `web/src/views/transport/`, `web/src/views/grid/`, `web/src/lib/components/TrackHeader.svelte`.
- Dépend de : RE-3, TR-4.
- Phase : 3.
- Étiquettes : hardware: non, founder: oui.

### Épopée 10 — Mixeur (MX)

**MX-1 — Mixage dans le moteur**
- But : volume et panoramique par piste et master (lissés), mute, solo (exclusif ou cumulatif selon le contrat), loi de panoramique documentée ; volume, panoramique et mute du master et des pistes sont des paramètres adressables par l'automation (AM-2) avec le même mécanisme que ceux des devices.
- Critères : −6 dB demandé = −6 dB mesuré ; mute et solo sans clic ; vu-mètres après fader ; une rampe de volume reçue par événements datés est suivie à l'échantillon près.
- Touche : `crates/engine/src/mixer.rs`.
- Dépend de : MO-3, MO-5.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**MX-2 — Commandes de mixage**
- But : `mixer.set_volume`, `mixer.set_pan`, `mixer.set_mute`, `mixer.set_master_volume` (`edit`, versionnées) et `mixer.set_solo` (décision 10 : état de session, `x-kind: live`, écrit dans le `SessionState` de PR-1 ; SV-1 l'écrit dans `.faulk/session.json` et SV-4 le restaure).
- Critères : un geste de fader = un groupe d'annulation ; valeurs hors bornes refusées ou bornées selon le contrat ; le solo n'apparaît ni dans l'historique ni dans la sérialisation du projet (test) ; le mute est versionné.
- Touche : `crates/commands/src/mixer.rs`, `crates/project` (état de session).
- Dépend de : PR-2.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**MX-3 — Vue mixeur**
- But : tranches de pistes (fader, pan, mute, solo, armement, vu-mètre, nom, couleur), tranche master.
- Critères : vu-mètres à 30 Hz sans fuite mémoire (mesuré sur 10 minutes) ; test Playwright.
- Touche : `web/src/views/mixer/`, `web/src/lib/i18n/mixer.ts`.
- Dépend de : MX-1, MX-2, UI-2.
- Phase : 4.
- Étiquettes : hardware: non, founder: oui.

### Épopée 11 — Sauvegarde et chargement (SV)

**SV-1 — Format de projet sur disque**
- But : implémenter le format dossier du [modèle de projet](../architecture/02-modele-projet.md) (décision 2) : écrivain et lecteur canoniques (JSON à clés triées, TSV pour notes, points d'automation et placements), `blobs/<sha256>` (décision 12), version de format et migrations, validation à l'ouverture, refus explicite d'un projet invalide ou plus récent, suivi des fichiers sales, instantané de cache dans `.faulk/cache/`, état de session dans `.faulk/session.json`, `.gitattributes` et `.gitignore`, export et import `.faulkpack` (archive zip du dossier).
- Critères : projets de référence relus et réécrits octet pour octet ; écrire → relire → réécrire identique sur des projets générés ; éditer une note ne réécrit qu'un fichier ; fichier corrompu → erreur typée, jamais de panique (fuzz) ; seuils issus de QA-7 appliqués (densité d'automation qui bascule en blob).
- Touche : `crates/project/src/file/`.
- Dépend de : PR-1, QA-7.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**SV-4 — Chargement d'un projet dans le moteur**
- But : `project.open` remplace le projet : arrêt, réinitialisation du moteur, instantané rejoué, état de session (`.faulk/session.json` : solo, entrée MIDI) restauré.
- Critères : rendu d'un projet enregistré puis rouvert identique au rendu d'origine ; aucune note coincée de l'ancien projet ; le solo (`mixer.set_solo`, MX-2) survit à une réouverture sans apparaître dans les fichiers versionnés (test).
- Touche : `crates/commands/src/project.rs`, `web/src/lib/stores/`.
- Dépend de : PR-3, SV-1, PR-5, MX-2.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**SV-2 — Magasin de projet et enregistrement automatique dans un dossier réel**
- But : appliquer la décision 3 : interface `ProjectStore` dans un Worker (lister, lire une plage, écrire atomiquement, observer), avec l'implémentation **dossier réel** (File System Access, Chrome et Edge) : `project.new`, `project.save`, `project.save_as`, choix du dossier (`showDirectoryPicker`), poignée rangée en IndexedDB, liste des projets récents, `queryPermission` / `requestPermission` avec bandeau, enregistrement automatique des fichiers sales après un court délai, écriture atomique (`.faulk/tmp/` puis `move()`, ou `createWritable` seul si `move()` ne convient pas, vérifié), relecture après modification externe (`FileSystemObserver`, repli au retour du focus) ; voir [stockage local](../architecture/03-stockage-local.md).
- Critères : test Playwright avec un `showDirectoryPicker` simulé qui rend un dossier OPFS (même interface `FileSystemDirectoryHandle`) : modifier, recharger la page, retrouver le projet ; onglet fermé pendant une écriture → ancien fichier intact ; permission perdue → bandeau, geste, reprise ; `project.new` sur un projet modifié non enregistré demande confirmation (destructive) ; dossier refusé par le navigateur géré.
- Touche : `crates/commands/src/project.rs`, `web/src/lib/storage/`, `web/src/views/projects/`.
- Dépend de : SV-1, SV-4.
- Phase : 3.
- Étiquettes : hardware: non, founder: oui.

**SV-3 — Repli OPFS et export `.faulkpack`**
- But : appliquer la décision 3 hors Chrome et Edge (Firefox, Safari, ou dossier refusé) : implémentation OPFS de `ProjectStore`, `navigator.storage.persist()`, avertissement « vos projets sont dans le navigateur ; exportez-les », export et import `.faulkpack` (archive zip du dossier, lisible par l'implémentation dossier réel), quota dépassé géré. Un projet OPFS passe en Boost par **export `.faulkpack` puis ouverture du dossier par le helper** (AU-7) : aucun fichier de projet, et surtout aucun blob, ne traverse le WebSocket.
- Critères : test Playwright du repli (navigateur sans `showDirectoryPicker`) : modifier, recharger, retrouver, exporter, importer dans un dossier réel simulé et obtenir le même projet octet pour octet ; quota plein signalé sans corruption ; un `.faulkpack` exporté depuis l'OPFS s'ouvre par le helper (test avec le helper en CI) et rend le même projet.
- Touche : `web/src/lib/storage/`, `web/src/views/projects/`, `crates/helper/src/disk.rs` (ouverture d'un `.faulkpack`).
- Dépend de : SV-2, AU-7.
- Phase : 6.
- Étiquettes : hardware: non, founder: oui.

### Épopée 12 — Agents (AG)

**AG-1 — Serveur MCP dans le helper, outils générés**
- But : serveur MCP local (`rmcp`, HTTP sur `127.0.0.1` avec jeton, et stdio) ; un outil par commande, généré depuis le schéma ; le helper relaie l'appel à la page et renvoie la sortie.
- Critères : protocole v2 avec requêtes et réponses corrélées (`id`) ; jeton, `Origin` et `Host` vérifiés (tests d'attaque) ; page absente → erreur typée claire.
- Touche : `crates/helper/src/mcp.rs`, `crates/protocol`, `web/src/lib/agents/`.
- Dépend de : PR-2, PR-5, BO-1.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**AG-2 — Test de parité commandes ↔ outils MCP**
- But : test CI qui compare l'énumération des commandes aux outils exposés (nom, schéma, description).
- Critères : une commande sans outil fait échouer le test (vérifié par un test qui l'enlève).
- Touche : `crates/helper/tests/`.
- Dépend de : AG-1.
- Phase : 3.
- Étiquettes : hardware: non, founder: non.

**AG-3 — Auteur et tour d'agent**
- But : chaque appel MCP porte l'auteur ; un tour d'agent = un groupe d'annulation ; l'historique l'affiche.
- Critères : annuler une fois défait tout un tour ; un geste de l'utilisateur pendant le tour n'est pas avalé dans le groupe (règle écrite et testée).
- Touche : `crates/commands`, `web/src/views/history/`.
- Dépend de : PR-7, AG-1.
- Phase : 6.
- Étiquettes : hardware: non, founder: non.

**AG-4 — Confirmation des commandes destructrices**
- But : toute commande `x-destructive` demande une confirmation dans la page, quel que soit l'appelant ; refus → erreur typée renvoyée à l'agent.
- Critères : avec un client MCP de test, `track.delete` attend la confirmation ; refus → aucun changement.
- Touche : `web/src/lib/agents/`, `web/src/views/confirm/`.
- Dépend de : AG-1.
- Phase : 6.
- Étiquettes : hardware: non, founder: oui.

**AG-5 — Scénario agent de bout en bout**
- But : un client MCP scripté crée un projet : 3 pistes (synthé, batterie, synthé avec delay) et une piste audio avec un clip importé, 2 scènes, des clips, un couloir d'automation sur le filtre, lance une scène, place des clips sur la timeline.
- Critères : le scénario passe en CI (helper sans interface + page Playwright) ; rendu non silencieux et sans NaN ; vérification manuelle avec Claude Code notée.
- Touche : `tests/e2e/agents/`.
- Dépend de : AG-1, GR-2, DV-8, AR-1, AU-5, AM-1.
- Phase : 6.
- Étiquettes : hardware: non, founder: non.

### Épopée 13 — Parité du mode Boost (BO)

**BO-1 — Le helper parle le protocole v2**
- But : le helper reçoit les octets `ControlMsg`, renvoie le `Feedback` multi-pistes ; `Hello` annonce ce que le helper sait faire (version du protocole, MIDI, accès au disque).
- Critères : même flux de messages → même rendu natif et WASM ; un message invalide ne coupe pas la session.
- Touche : `crates/helper`, `crates/protocol`.
- Dépend de : MO-2, BO-0.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**BO-2 — Bascule à chaud avec un projet**
- But : passer Web ↔ Boost en cours de session : arrêt des notes, instantané rejoué, position et état de lecture conservés.
- Critères : projet de 8 pistes, bascule pendant la lecture, rendus équivalents après la bascule ; perte du helper → retour Web sans perte du projet ; vérifié sur la machine du fondateur.
- Touche : `web/src/lib/stores/`, `crates/helper`.
- Dépend de : BO-1, PR-3, PR-5.
- Phase : 2.
- Étiquettes : hardware: oui, founder: non.

**BO-3 — MIDI natif horodaté pour l'enregistrement**
- But : horodatage `midir` converti en position d'échantillon du callback.
- Critères : même test d'exactitude que RE-2, en Boost, avec un contrôleur réel.
- Touche : `crates/helper/src/midi.rs`, `crates/helper/src/audio.rs`.
- Dépend de : RE-2, BO-1.
- Phase : 4.
- Étiquettes : hardware: oui, founder: non.

**BO-4 — Mesure de latence dans les deux modes** (issue #10, déjà ouverte)
- But : mesure aller-retour documentée (méthode, matériel, valeurs) ; l'interface distingue une latence mesurée d'une latence estimée.
- Critères : ceux de l'issue #10 ; valeurs mesurées notées dans `docs/` ; aucune latence annoncée sans mesure.
- Touche : `docs/spikes/`, `crates/helper`, `web/`.
- Dépend de : BO-1, MO-5.
- Phase : 6.
- Étiquettes : hardware: oui, founder: non.

### Épopée 14 — Interface (UI)

**UI-1 — Coquille d'application**
- But : mise en page (barre du haut avec transport et état moteur, zone principale avec vues grille, timeline, mixeur, panneau du bas pour devices et piano roll, liste des pistes) ; navigation par le registre ; `Session` découpée en stores (§ 2.6).
- Critères : l'écran actuel reste accessible ; thème DS-0.1 ; sans débordement à 1 280 et 1 920 px ; test Playwright de navigation.
- Touche : `web/src/App.svelte`, `web/src/views/shell/`, `web/src/lib/stores/`.
- Dépend de : UI-0.
- Phase : 1.
- Étiquettes : hardware: non, founder: oui.

**UI-2 — En-têtes et liste de pistes**
- But : composant `TrackHeader` (nom, couleur, type de piste, instrument, mute, solo, armement, sélection) partagé par la grille, la timeline et le mixeur ; créer une piste MIDI (en choisissant son instrument) ou audio.
- Critères : chaque action = une commande ; renommage au clavier ; test Playwright.
- Touche : `web/src/lib/components/TrackHeader.svelte`, `web/src/views/tracks/`.
- Dépend de : UI-1, PR-5, PR-8, MX-2.
- Phase : 2.
- Étiquettes : hardware: non, founder: oui.

**UI-3 — Clavier de l'ordinateur et MIDI vers la piste sélectionnée**
- But : adapter le clavier musical et le MIDI existants à la piste sélectionnée.
- Critères : aucun raccourci ne se déclenche pendant le jeu au clavier (règle écrite) ; test Playwright.
- Touche : `web/src/lib/keyboard.ts`, `web/src/lib/midi/`.
- Dépend de : UI-2, RE-1.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**UI-4 — Palette de commandes**
- But : palette générée depuis le schéma (nom, description, paramètres), recherche, exécution.
- Critères : toute commande `edit` ou `live` est trouvable (test par énumération du schéma).
- Touche : `web/src/views/palette/`.
- Dépend de : PR-5.
- Phase : 6.
- Étiquettes : hardware: non, founder: oui.

**UI-5 — Registre de raccourcis**
- But : raccourcis centralisés (lecture, enregistrement, annuler, dupliquer, vues), sans conflit avec le clavier musical, affichés dans la palette.
- Critères : un test détecte deux raccourcis identiques.
- Touche : `web/src/lib/shortcuts/`.
- Dépend de : UI-1, UI-4.
- Phase : 6.
- Étiquettes : hardware: non, founder: oui.

### Épopée 16 — Pistes audio (AU)

Décision 5 : import et lecture de fichiers audio (échantillons, boucles, stems), lus en flux depuis le disque selon le [stockage local](../architecture/03-stockage-local.md) ; **pas d'enregistrement audio dans le MVP** (il suit immédiatement) ; **pas de recalage temporel** (warping), sauf s'il se révèle trivial, ce qui n'est pas attendu. Une piste audio est une piste dont l'instrument est le lecteur de clips (`faulk.clip_player`) ; elle a sa chaîne d'effets, son mixage et son automation comme une piste MIDI. AU-0 est en phase 0 (§ Épopée 0).

**AU-1 — Import d'un fichier audio : empreinte, blob, PCM**
- But : pipeline d'import écrit **une fois en Rust** (`crates/audio-import` : SHA-256 en flux, décodage, métadonnées, crêtes pour AU-2) et exécuté **là où sont les octets** : dans un Worker de la page en mode Web (WASM), **dans le helper en mode Boost** (natif), parce que le son et les fichiers audio ne traversent jamais le lien page ↔ helper (`CLAUDE.md`, [architecture](../architecture/01-architecture.md) principe 3). En Web, la page lit le fichier ; en Boost, la page demande au helper d'importer un chemin natif (sélecteur de fichiers du helper, ou chemin déposé quand le navigateur le fournit) et ne reçoit que des métadonnées et des crêtes. Le pipeline range les octets d'origine dans `blobs/<sha256>` par le `ProjectStore` (décision 12), écrit les métadonnées (fréquence, canaux, frames, nom d'origine) dans le JSON du clip, et décode en PCM float32 dans `.faulk/cache/` (régénérable, jamais versionné : c'est la proposition par défaut du [stockage local](../architecture/03-stockage-local.md) § 6, que la PR présente au fondateur avec son coût, un décodage par machine, contre celui d'un second blob versionné). WAV et AIFF décodés directement ; formats compressés (FLAC, MP3, Opus, AAC) par le même décodeur Rust (`symphonia`) dans les deux cibles, pour que les deux modes produisent le même PCM (WebCodecs n'est qu'un repli, dont la parité serait à tolérance) ; le rééchantillonnage est fait à la lecture (AU-4), jamais à l'import, puisque la fréquence du moteur dépend du périphérique.
- Critères : importer deux fois le même fichier ne crée qu'un blob ; l'empreinte est celle de `sha256sum` sur le fichier d'origine ; décodage de fichiers de référence comparé à un décodage de référence (tolérance notée) et identique entre page et helper pour le même blob compressé (test de parité, décalages d'amorce compris) ; en Boost, aucun octet de fichier audio ne traverse le WebSocket (test sur un import d'une heure : seuls métadonnées et crêtes passent) ; fichier corrompu ou format inconnu → erreur typée, jamais de plantage ; progression visible pour un fichier d'une heure ; le cache absent se régénère à l'ouverture, dans la cible qui tient le disque.
- Touche : `crates/audio-import`, `web/src/lib/audio/import/`, `crates/helper/src/import.rs`, `crates/project` (blobs), `crates/protocol`, `web/src/lib/storage/`.
- Dépend de : SV-1, SV-2, BO-1.
- Phase : 5.
- Étiquettes : hardware: non, founder: oui.

**AU-2 — Cache d'aperçus de forme d'onde**
- But : à l'import, calcul des crêtes (min/max par bloc de 256 échantillons, niveaux réduits ×4, ×16, ×64) par le pipeline AU-1 (Worker en Web, helper en Boost, qui les envoie à la page : ce sont des données visuelles), rangées dans `.faulk/cache/` ; composant `Waveform` qui dessine depuis ce cache au niveau adapté au zoom.
- Critères : environ 45 Ko par minute au niveau le plus fin (mesuré) ; cache absent → régénéré sans bloquer l'interface ; rendu d'une heure d'audio à 60 images/s au défilement (mesuré dans Playwright) ; test de conformité des crêtes sur un signal connu ; mêmes crêtes en Web et en Boost pour le même blob (test).
- Touche : `crates/audio-import` (crêtes), `web/src/lib/audio/peaks/`, `web/src/lib/components/Waveform.svelte`.
- Dépend de : AU-1.
- Phase : 5.
- Étiquettes : hardware: non, founder: non.

**AU-3 — Worker disque et anneau partagé vers le worklet**
- But : le Worker disque lit le PCM par plages (`File.slice`, `SyncAccessHandle.read`, sur le blob WAV/AIFF ou sur le cache PCM d'un fichier compressé), tient la lecture anticipée par voix (2 à 4 s), le pré-chargement des débuts de clips (1 à 2 s) et le budget global de PCM décodé avec éviction LRU ([stockage local](../architecture/03-stockage-local.md) § 5.4), et pousse des blocs PCM dans des anneaux lock-free en `SharedArrayBuffer` lus par le moteur dans le worklet ; sauts et boucles vident et remplissent les anneaux sans jamais bloquer le thread audio.
- Critères : aucune allocation ni attente sur le chemin audio (détecteur) ; sous-alimentation comptée dans le `Feedback` et traduite en silence sur la voix, jamais en blocage ; test de saut et de boucle courte sans trou (rendu comparé) ; budget respecté sous un test qui demande plus que le budget ; débit de lecture par plages mesuré en OPFS et en dossier réel (méthode et chiffres notés).
- Touche : `web/src/lib/audio/stream/`, `crates/engine/src/audio_source.rs`, `crates/engine-wasm`.
- Dépend de : AU-0, MO-2, SV-2.
- Phase : 5.
- Étiquettes : hardware: non, founder: non.

**AU-4 — Lecteur de clips audio dans le moteur**
- But : device `faulk.clip_player` (rôle instrument) : lit une source audio (PCM en mémoire pour les clips courts, anneau de streaming pour les longs) à partir d'une position en frames, départ et arrêt à l'échantillon près (MO-4), gain du clip, fondus courts aux bords et au lancement, boucle d'un clip, **rééchantillonnage à la lecture** dès que la fréquence de la source diffère de celle du moteur (celle du contexte audio en Web, du périphérique en Boost, qui peut changer en cours de session) ; le helper alimente le même device depuis le disque natif.
- Critères : rendu d'un clip de référence identique à la source au gain près (tolérance notée) ; un clip à 44,1 kHz rendu par un moteur à 48 kHz (et l'inverse) garde sa hauteur et sa durée (mesurées) ; départ à l'échantillon *k* (onset) ; aucun clic au lancement, à l'arrêt, au bouclage (mesuré) ; parité natif/WASM ; aucune allocation dans `process`.
- Touche : `crates/engine/src/devices/clip_player.rs`, `crates/engine/src/audio_source.rs`, `crates/helper/src/disk.rs`.
- Dépend de : MO-4, AU-3, DV-1.
- Phase : 5.
- Étiquettes : hardware: non, founder: non.

**AU-5 — Modèle et commandes de pistes et de clips audio**
- But : `track.create { kind: audio }` crée une piste dont l'instrument est le lecteur de clips ; `clip.import_audio { track, file, slot | arrangement }` (passe par le pipeline AU-1), `clip.audio.set_range { start, frames }`, `clip.audio.set_gain`, `clip.audio.set_loop` ; `ClipContent::Audio` et `Span::Frames` du [modèle de projet](../architecture/02-modele-projet.md) écrits et relus ; un clip dont le blob manque reste ouvrable et marqué « média absent ».
- Critères : un clip audio dans un placement ou un emplacement de grille se sérialise, se relit et se rejoue à l'identique ; blob absent → projet valide, clip silencieux, état visible dans le `Feedback` ; undo de l'import retire le clip mais pas le blob (nettoyage sur demande seulement).
- Touche : `crates/commands/src/clip.rs`, `crates/commands/src/track.rs`, `crates/project`.
- Dépend de : CL-1, AU-1, PR-8.
- Phase : 5.
- Étiquettes : hardware: non, founder: non.

**AU-6 — Clips audio dans la grille et la timeline**
- But : glisser un fichier audio sur une piste (grille ou timeline) pour l'importer et le placer ; forme d'onde (`Waveform`) dans la case et dans le placement ; lancement quantifié d'un clip audio depuis la grille ; redimensionner et déplacer un placement audio (plage source en frames) ; gain du clip éditable.
- Critères : chaque geste = une commande ; test Playwright : importer un fichier de référence, le lancer, mesurer un niveau non nul, le placer sur la timeline, recharger, le retrouver.
- Touche : `web/src/views/grid/`, `web/src/views/arrangement/`, `web/src/lib/i18n/audio.ts`.
- Dépend de : AU-4, AU-5, AU-2, GR-3, AR-4.
- Phase : 5.
- Étiquettes : hardware: non, founder: oui.

**AU-7 — Le helper et le dossier du projet en mode Boost**
- But : en Boost, la page reste propriétaire du projet (décision 1) et le helper est la seule autorité qui écrit le dossier : implémentation `ProjectStore` « helper » pour les **fichiers texte du projet seulement** (JSON, TSV, état de session, lus et écrits par le WebSocket) ; les blobs et le cache PCM ne traversent jamais le lien, ils sont écrits et lus par le helper lui-même (import natif AU-1, streaming natif AU-4) ; rapprochement du dossier par l'identifiant du projet ou choix par un sélecteur natif (tranché dans le plan de la tâche), résolution des blobs et du cache PCM par chemin natif pour le streaming du helper. Les messages de fichiers sont **confinés à la racine du projet** et soumis au même jeton d'appairage que le reste de la session ([contraintes](../constraints/01-contraintes.md), sécurité du pont local) : chemins relatifs normalisés, `..`, chemins absolus, lettres de lecteur et chemins UNC refusés, liens symboliques qui sortent du dossier refusés, écriture atomique dans `.faulk/tmp/` puis déplacement.
- Critères : ouvrir un projet dans la page puis basculer en Boost : le helper joue les clips audio depuis le disque sans que l'audio traverse le lien (test : le WebSocket ne porte ni trame PCM ni octet de blob, en lecture comme à l'import) ; un message de fichier qui vise `blobs/` ou `.faulk/cache/` est refusé ; une modification faite en Boost est écrite dans le dossier et relue en Web ; deux dossiers de même identifiant → le helper demande, jamais de choix silencieux ; tests d'attaque : lecture et écriture avec `..`, chemin absolu, lecteur, UNC, lien symbolique vers l'extérieur, et sans jeton, toutes refusées par une erreur typée sans toucher au disque.
- Touche : `crates/helper/src/disk.rs`, `crates/helper/src/server.rs`, `crates/helper/src/security.rs`, `crates/protocol`, `web/src/lib/storage/`.
- Dépend de : BO-1, SV-2, AU-1, AU-4.
- Phase : 5.
- Étiquettes : hardware: non, founder: oui.

**AU-8 — Scénario audio de bout en bout et parité**
- But : le parcours complet : importer trois fichiers (court, long d'une heure, compressé), les placer dans la grille et la timeline, lancer, boucler, sauter, enregistrer le projet, le rouvrir, obtenir le même rendu ; en Web (CI) et en Boost (machine du fondateur) ; projets de référence avec audio ajoutés au harnais QA-1.
- Critères : rendu Web et Boost comparés à tolérance fixée ; sous-alimentation nulle sur le fichier d'une heure en lecture continue (compteur) ; mémoire audio sous le budget pendant tout le parcours (mesurée) ; écoute du rendu par le fondateur notée.
- Touche : `web/tests/e2e/audio/`, `crates/engine/tests/fixtures/`.
- Dépend de : AU-6, AU-7, SV-4, QA-1, QA-3.
- Phase : 5.
- Étiquettes : hardware: oui, founder: non.

### Épopée 17 — Automation (AM)

Décision 5 : couloirs d'automation sur les paramètres des devices et du mixeur, dessin et édition de points, lecture à l'échantillon près, et si c'est faisable l'enregistrement des mouvements de paramètres depuis l'interface ou le MIDI CC. Automation d'arrangement seulement (par piste) ; les enveloppes propres aux clips viennent plus tard ([modèle de projet](../architecture/02-modele-projet.md) § 1.2).

**AM-1 — Modèle, format et commandes des couloirs d'automation**
- But : `AutomationLane { target, points }` du [modèle de projet](../architecture/02-modele-projet.md) (cible : paramètre d'un device ou du mixeur ; au plus un point par tick ; courbe `linear` ou `step` par point), fichier `automation/<lane>.tsv` et bascule en blob au-delà du seuil de densité de QA-7 ; commandes `automation.lane.create { track, target }`, `automation.lane.delete`, `automation.points.add` (lot), `automation.points.remove`, `automation.points.update`, `automation.lane.clear`.
- Critères : la cible est validée contre le descripteur du device ou la liste des paramètres du mixeur ; un lot = une entrée d'historique ; valeurs arrondies au pas déclaré ; couloir dense écrit en blob et relu comme table ; supprimer le device supprime ses couloirs (undo les restaure).
- Touche : `crates/commands/src/automation.rs`, `crates/project`.
- Dépend de : PR-2, DV-8, MX-2.
- Phase : 5.
- Étiquettes : hardware: non, founder: non.

**AM-2 — Lecture de l'automation à l'échantillon près**
- But : le moteur reçoit les couloirs par `prepare` (tables échangées dans `apply`, comme les notes), évalue chaque couloir par bloc, interpole entre les points et émet des `ParamValue` datés vers le device ou le mixeur ; parité. Comportement proposé, **à valider par le fondateur dans la PR** (comportement produit) : pendant la lecture, le couloir a la main et un geste manuel sur un paramètre automatisé est appliqué jusqu'au point suivant puis repris par le couloir ; à l'arrêt, le dernier geste manuel reste.
- Critères : automation de volume de −∞ à 0 dB sur une mesure → rampe mesurée à l'échantillon près (rendu hors ligne) ; seek au milieu d'une rampe → valeur correcte au premier bloc ; 100 couloirs sur 1 000 points sans allocation dans `process` ; parité natif/WASM ; la règle geste manuel / couloir est écrite et testée.
- Touche : `crates/engine/src/automation.rs`, `crates/commands` (compilateur).
- Dépend de : AM-1, AM-3, MO-4, TR-1, PR-3, MX-1.
- Phase : 5.
- Étiquettes : hardware: non, founder: oui.

**AM-3 — Lissage des paramètres**
- But : mécanisme commun de lissage dans la couche device : chaque paramètre déclare un temps de lissage (CT-5) ; un `ParamValue` daté fixe une cible, la valeur suit une rampe par échantillon (ou par bloc pour les paramètres sans audibilité) sans clic ; utilisé par le synthé, les effets et le mixeur.
- Critères : saut de gain de −60 à 0 dB → aucun clic (spectre mesuré) ; la rampe atteint la cible dans le temps déclaré ± un bloc ; aucune allocation ; un paramètre à lissage nul est appliqué à l'offset exact.
- Touche : `crates/engine/src/param.rs`, `crates/engine/src/devices/`.
- Dépend de : DV-1, CT-5.
- Phase : 2.
- Étiquettes : hardware: non, founder: non.

**AM-4 — Édition des points d'automation dans la timeline**
- But : couloirs dépliables sous chaque piste de la timeline (choix du paramètre parmi les descripteurs), dessin de points (clic), déplacement, suppression, sélection multiple, courbe par point, ligne de valeur affichée en unité naturelle ; `TimeGrid` partagée.
- Critères : chaque geste = une commande, un glisser = un groupe ; fluide avec 10 000 points affichés (mesuré) ; test Playwright : dessiner trois points, lire, voir le paramètre bouger dans le panneau de devices, annuler.
- Touche : `web/src/views/arrangement/automation/`, `web/src/lib/i18n/automation.ts`.
- Dépend de : AM-1, AM-2, AR-4, DV-9.
- Phase : 5.
- Étiquettes : hardware: non, founder: oui.

**AM-5 — Enregistrement des mouvements de paramètres**
- But : si c'est faisable dans le temps du MVP : en enregistrement, un bouton tourné dans le panneau de devices ou un MIDI CC assigné à un paramètre écrit des points dans le couloir ; deux modes proposés, « touche » (le couloir reprend la main quand le geste s'arrête) et « verrou » (la dernière valeur reste écrite jusqu'à l'arrêt), le choix des modes livrés étant **validé par le fondateur dans la PR** ; les points sont quantifiés au pas et simplifiés (points colinéaires supprimés) ; apprentissage CC → paramètre (`midi.map_cc`).
- Critères : un geste simulé (suite de valeurs horodatées) produit un couloir dont la relecture reproduit le geste à ± un pas, dans chaque mode livré ; une prise = une entrée d'historique ; faisabilité tranchée dans le plan : si elle est reportée, la raison est écrite et l'issue renvoyée au fondateur.
- Touche : `crates/commands/src/automation.rs`, `crates/engine/src/recorder.rs`, `web/src/views/devices/`, `web/src/lib/midi/`.
- Dépend de : AM-2, DV-9, RE-1.
- Phase : 6.
- Étiquettes : hardware: non, founder: oui.

**AM-6 — Scénario d'automation de bout en bout**
- But : un projet de référence avec des couloirs sur un device et sur le mixeur, dense (blob) et clairsemé (TSV) : lu, enregistré, rouvert, rendu ; comparé en natif et en WASM ; ajouté au harnais QA-1.
- Critères : rendu identique après réouverture ; parité natif/WASM ; le niveau mesuré suit le couloir de volume ; test Playwright du parcours complet dans l'interface.
- Touche : `web/tests/e2e/automation/`, `crates/engine/tests/fixtures/`.
- Dépend de : AM-2, AM-4, QA-1, SV-4.
- Phase : 5.
- Étiquettes : hardware: non, founder: non.

### Épopée 15 — Qualité et outillage de test (QA)

**QA-1 — Harnais de rendu hors ligne de projet**
- But : construire un projet par commandes, le rendre N secondes en natif et en WASM ; assertions (niveau, silence, spectre, onset, NaN, dénormaux) ; comparaison natif/WASM à tolérance fixée.
- Critères : utilisé par au moins un test de chaque crate moteur ; projets de référence dans `tests/fixtures/`.
- Touche : `crates/engine/tests/`, `crates/engine-wasm/tests/`, `crates/test-support` si besoin.
- Dépend de : MO-2, PR-2.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**QA-2 — Détecteur d'allocation généralisé**
- But : `assert_no_alloc` sur `apply` + `process` pour tout le moteur et chaque device, en debug.
- Critères : un test qui alloue volontairement sur le chemin audio échoue (test du garde-fou lui-même).
- Touche : `crates/engine`, `crates/test-support`.
- Dépend de : MO-1.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**QA-3 — Harnais Playwright**
- But : démarrage de l'audio, Web MIDI simulé, capture des erreurs console, lecture du niveau de sortie, chargement de projets de référence, `showDirectoryPicker` simulé sur l'OPFS.
- Critères : un test d'exemple pour chaque point ; tourne en CI.
- Touche : `web/tests/e2e/`, `.github/workflows/`.
- Dépend de : UI-1.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**QA-4 — Tests de propriétés des commandes**
- But : séquences aléatoires de commandes : jamais de panique, `validate()` toujours vrai, tout annuler ramène au projet vide, limites de sûreté respectées.
- Critères : tourne en CI avec la graine affichée en cas d'échec.
- Touche : `crates/commands/tests/`.
- Dépend de : PR-2.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**QA-5 — Banc de performance audio**
- But : coût processeur par bloc avec 16 pistes × (instrument + 3 effets), en natif et en WASM ; budget fixé ; suivi en CI (non bloquant au début). Instruments : synthé (DV-1) et batterie (DV-6) en alternance ; les trois effets mesurés sont le delay (DV-3), la réverbération (DV-4) et le compresseur (DV-5).
- Critères : résultats publiés dans la sortie CI ; budget noté dans `docs/constraints/`.
- Touche : `crates/engine/benches/`, `.github/workflows/`.
- Dépend de : MO-3, DV-6, DV-3, DV-4, DV-5.
- Phase : 4.
- Étiquettes : hardware: non, founder: non.

**QA-6 — Scénario MVP de bout en bout**
- But : le parcours complet dans l'interface : créer un projet, 4 pistes de 2 instruments et une piste audio, effets, clips dans la grille, enregistrement MIDI, scène lancée, arrangement, un couloir d'automation, mixage, enregistrement dans un dossier, réouverture, même rendu ; dans les deux modes quand le helper est disponible.
- Critères : test Playwright en CI (mode Web) ; parcours Boost vérifié et noté. Ce sont les fonctions qu'exerce le scénario et ses harnais (le graphe en déduit le reste par transitivité) ; le scénario ne requiert pas les autres tâches des phases 1 à 5, qui ont leurs propres tests.
- Touche : `web/tests/e2e/mvp/`.
- Dépend de : RE-4, RE-5, AR-5, MX-3, SV-3, BO-2, DV-9, UI-3, PN-1, AU-8, AM-6, QA-1, QA-3.
- Phase : 6.
- Étiquettes : hardware: oui, founder: non.

**QA-7 — Banc de charge du format de projet** (ajout de la synthèse)
- But : générer un projet synthétique très lourd (100 pistes, 200 000 notes, 6 millions de points d'automation, quelques Go d'audio) au format dossier, et mesurer l'ouverture, l'enregistrement (une note modifiée, le projet entier), la mémoire, le coût des échanges de structures du moteur (§ 2.3) et les opérations git (status, commit, diff, fusion), dans le navigateur (OPFS et dossier réel) et en natif.
- Critères : générateur déterministe (graine notée) ; chiffres publiés avec la méthode et la machine ; seuils du format proposés à partir des mesures (densité d'automation qui bascule en blob, instantané de cache, éventuelle subdivision de `blobs/`) et reportés dans le [modèle de projet](../architecture/02-modele-projet.md) ; limites de sûreté de CT-3 confrontées aux mesures ; aucun chiffre annoncé sans mesure. L'écrivain du format utilisé ici est un prototype que SV-1 reprend ou remplace.
- Touche : `crates/project/benches/`, `web/tests/bench/`, `docs/architecture/02-modele-projet.md`.
- Dépend de : PR-1.
- Phase : 1.
- Étiquettes : hardware: non, founder: non.

**DOC-1 — Documentation du MVP**
- But : mettre à jour `docs/architecture/`, `docs/product/` (statut des décisions) et `docs/specs/` pour les fonctions livrées.
- Critères : chaque document vérifié contre le code change de `status` ; liens valides ; entrée de journal en fragment `docs/log.d/`.
- Touche : `docs/`.
- Dépend de : QA-6.
- Phase : 6.
- Étiquettes : hardware: non, founder: non.

## 4. Chemin critique et colonne vertébrale

### 4.1 La colonne vertébrale (en série, d'abord)

Deux branches se rejoignent en QA-1, puis le socle est en série jusqu'à PR-5 :

- branche projet : **CT-1 → CT-2 et CT-4 (les deux, en parallèle) → PR-1 → PR-2** ;
- branche moteur : **CT-1 et AU-0 → CT-3 → MO-1 → MO-2** (AU-0, sans prérequis, met l'isolation en place pour le prototype de CT-3 et le canal de MO-2) ;
- puis **QA-1 → PR-3 → PR-4 → PR-5**.

Après PR-5 (« la page envoie n'importe quelle commande, le projet produit les messages, les deux moteurs jouent N pistes »), les domaines deviennent des feuilles parallèles. Les autres tâches du socle (CT-5, MO-3, DV-1, TR-1, MO-4…) ont de la marge.

### 4.2 Chemin critique jusqu'au MVP

Méthode : un script lit les lignes `Dépend de` de chaque tâche du § 3 (une arête par prérequis), vérifie que le graphe est sans cycle et qu'aucune tâche ne dépend d'une tâche d'une phase postérieure, puis calcule pour chaque tâche sa **profondeur** (plus long chemin depuis une tâche sans prérequis, en nombre de tâches) et sa **hauteur** (plus long chemin jusqu'à une tâche terminale). La longueur du chemin critique est la profondeur maximale ; une tâche est critique (marge nulle) si profondeur + hauteur − 1 égale cette longueur. Le même script engendre le graphe Mermaid du § 4.3 depuis ces arêtes, ce qui garantit l'égalité des deux ; au dernier calcul (2026-10-14) : 96 tâches, 211 arêtes. (BO-4 est l'issue #10 ; ses prérequis BO-1 et MO-5 sont dans le graphe.)

<!-- critical-path:start -->
**Longueur : 16 tâches en série.** Plusieurs chemins ont cette longueur ; les tâches sans marge, par profondeur :

```
1  CT-1 | AU-0
2  CT-2 | CT-3 | CT-4
3  PR-1 | MO-1
4  PR-2 | MO-2
5  QA-1
6  PR-3
7  PR-4
8  PR-5
9  SV-4
10 SV-2
11 AU-1 | AU-3
12 AU-2 | AU-4 | AU-5
13 AU-6 | AU-7
14 SV-3 | AU-8
15 QA-6
16 DOC-1
```
<!-- critical-path:end -->

Les chemins critiques passent donc par le socle, puis par la sauvegarde et les pistes audio : CT-1 ou AU-0 → (CT-3 → MO-1 → MO-2, ou CT-2/CT-4 → PR-1 → PR-2) → QA-1 → PR-3 → PR-4 → PR-5 → SV-4 → SV-2 → (AU-1 → AU-2 ou AU-5 → AU-6, ou AU-3 → AU-4 → AU-6 ou AU-7) → (AU-8, ou SV-3 après AU-7) → QA-6 → DOC-1. L'étude du 2026-10-13 comptait 13 tâches en série : les pistes audio en ajoutent trois, parce qu'elles attendent le dossier réel (SV-2) et que leur scénario (AU-8) précède celui du MVP. Toute autre tâche a au moins une tâche de marge : l'automation (AM-1 à AM-6), l'enregistrement MIDI, la grille et la timeline tournent en parallèle sans allonger le MVP, tant qu'elles ne prennent pas de retard. Ce chiffre est à recalculer à chaque changement de dépendance.

### 4.3 Graphe des dépendances

<!-- graph:start -->
```mermaid
graph LR
  %% Phase 0
  CT1["CT-1 crates project"]
  CT1 --> CT2["CT-2 temps"]
  CT1 & AU0 --> CT3["CT-3 sync moteur"]
  CT1 --> CT4["CT-4 commande v2"]
  CT3 --> CT5["CT-5 device"]
  CT6["CT-6 glossaire"]
  UI0["UI-0 anti-conflits"]
  AU0["AU-0 isolation cross-origin"]
  BO0["BO-0 vérif Boost"]
  %% Phase 1
  CT1 & CT2 & CT4 --> PR1["PR-1 modèle projet"]
  PR1 --> PR2["PR-2 commandes v2"]
  PR2 & MO1 & QA1 --> PR3["PR-3 compilateur + instantané"]
  PR2 & PR3 --> PR4["PR-4 project-wasm"]
  CT3 --> MO1["MO-1 prepare/apply"]
  MO1 & AU0 --> MO2["MO-2 canal unique"]
  MO1 & CT5 --> MO3["MO-3 graphe pistes"]
  MO1 & CT2 --> TR1["TR-1 horloge"]
  CT5 & MO3 --> DV1["DV-1 registre + synthé"]
  MO2 & BO0 --> BO1["BO-1 helper v2"]
  UI0 --> UI1["UI-1 coquille"]
  MO2 & PR2 --> QA1["QA-1 rendu hors ligne"]
  MO1 --> QA2["QA-2 détecteur alloc"]
  UI1 --> QA3["QA-3 Playwright"]
  PR2 --> QA4["QA-4 propriétés"]
  PR1 --> QA7["QA-7 banc de charge"]
  %% Phase 2
  PR4 & MO2 & UI1 --> PR5["PR-5 dispatcher"]
  PR2 & DV1 --> PR8["PR-8 pistes"]
  MO3 --> MO4["MO-4 événements horodatés"]
  MO3 --> MO5["MO-5 feedback"]
  TR1 & MO3 --> TR2["TR-2 métronome"]
  PR2 & TR1 --> TR3["TR-3 cmd transport"]
  PR5 & TR3 & UI1 --> TR4["TR-4 barre transport"]
  DV1 & AM3 --> DV2["DV-2 filtre"]
  DV1 --> DV6["DV-6 batterie"]
  PR2 & DV1 --> DV8["DV-8 cmd devices"]
  PR5 & DV8 & UI1 --> DV9["DV-9 panneau devices"]
  MO4 & PR8 --> RE1["RE-1 armement + MIDI live"]
  MO3 & MO5 --> MX1["MX-1 mixage moteur"]
  PR2 --> MX2["MX-2 cmd mixage"]
  BO1 & PR3 & PR5 --> BO2["BO-2 bascule à chaud"]
  UI1 & PR5 & PR8 & MX2 --> UI2["UI-2 en-têtes pistes"]
  UI2 & RE1 --> UI3["UI-3 clavier + MIDI"]
  DV1 & CT5 --> AM3["AM-3 lissage"]
  %% Phase 3
  PR2 --> PR6["PR-6 requêtes"]
  PR5 --> PR7["PR-7 historique UI"]
  PR2 --> CL1["CL-1 cmd clips"]
  MO4 & TR1 --> CL2["CL-2 lecteur clips"]
  CL1 & CL2 & PR3 --> CL3["CL-3 édition en lecture"]
  CL2 --> GR1["GR-1 lancement quantifié"]
  CL1 & GR1 --> GR2["GR-2 cmd scènes"]
  GR2 & UI2 --> GR3["GR-3 vue grille"]
  UI1 & CT2 --> AR3["AR-3 TimeGrid"]
  CL1 & AR3 & PR5 --> PN1["PN-1 piano roll"]
  PN1 --> PN2["PN-2 édition avancée"]
  RE1 & TR1 --> RE2["RE-2 capture"]
  RE2 & GR1 --> RE3["RE-3 prise en grille"]
  RE3 & TR4 --> RE5["RE-5 UI enregistrement"]
  PR1 & QA7 --> SV1["SV-1 format projet"]
  PR3 & SV1 & PR5 & MX2 --> SV4["SV-4 chargement"]
  SV1 & SV4 --> SV2["SV-2 dossier réel + autosave"]
  PR2 & PR5 & BO1 --> AG1["AG-1 serveur MCP"]
  AG1 --> AG2["AG-2 parité MCP"]
  %% Phase 4
  DV1 & TR1 --> DV3["DV-3 delay"]
  DV1 --> DV4["DV-4 réverb"]
  DV1 --> DV5["DV-5 compresseur"]
  CL1 --> AR1["AR-1 cmd arrangement"]
  CL2 & TR1 & GR1 --> AR2["AR-2 lecture arrangement"]
  AR1 & AR3 & UI2 --> AR4["AR-4 vue timeline"]
  GR3 & AR4 --> AR5["AR-5 grille vers timeline"]
  PN1 & AR4 --> PN3["PN-3 piano roll depuis timeline"]
  RE2 & AR2 --> RE4["RE-4 prise en timeline"]
  MX1 & MX2 & UI2 --> MX3["MX-3 vue mixeur"]
  RE2 & BO1 --> BO3["BO-3 MIDI natif horodaté"]
  MO3 & DV6 & DV3 & DV4 & DV5 --> QA5["QA-5 perf"]
  %% Phase 5
  SV1 & SV2 & BO1 --> AU1["AU-1 import audio"]
  AU1 --> AU2["AU-2 forme d'onde"]
  AU0 & MO2 & SV2 --> AU3["AU-3 streaming disque"]
  MO4 & AU3 & DV1 --> AU4["AU-4 lecteur clips audio"]
  CL1 & AU1 & PR8 --> AU5["AU-5 cmd pistes audio"]
  AU4 & AU5 & AU2 & GR3 & AR4 --> AU6["AU-6 clips audio UI"]
  BO1 & SV2 & AU1 & AU4 --> AU7["AU-7 helper + dossier"]
  AU6 & AU7 & SV4 & QA1 & QA3 --> AU8["AU-8 scénario audio"]
  PR2 & DV8 & MX2 --> AM1["AM-1 cmd automation"]
  AM1 & AM3 & MO4 & TR1 & PR3 & MX1 --> AM2["AM-2 lecture automation"]
  AM1 & AM2 & AR4 & DV9 --> AM4["AM-4 édition points"]
  AM2 & AM4 & QA1 & SV4 --> AM6["AM-6 scénario automation"]
  %% Phase 6
  SV2 & AU7 --> SV3["SV-3 repli OPFS + export"]
  PR7 & AG1 --> AG3["AG-3 tour d'agent"]
  AG1 --> AG4["AG-4 confirmation"]
  AG1 & GR2 & DV8 & AR1 & AU5 & AM1 --> AG5["AG-5 scénario agent"]
  BO1 & MO5 --> BO4["BO-4 latence (#10)"]
  PR5 --> UI4["UI-4 palette"]
  UI1 & UI4 --> UI5["UI-5 raccourcis"]
  AM2 & DV9 & RE1 --> AM5["AM-5 enregistrement mouvements"]
  RE4 & RE5 & AR5 & MX3 & SV3 & BO2 & DV9 & UI3 & PN1 & AU8 & AM6 & QA1 & QA3 --> QA6["QA-6 scénario MVP"]
  QA6 --> DOC1["DOC-1 docs MVP"]
```
<!-- graph:end -->

## 5. Contrats à figer tôt et points chauds de conflits

### 5.1 Contrats à figer avant d'ouvrir le parallélisme

| Contrat | Où | Figé par | Qui en dépend |
| --- | --- | --- | --- |
| Propriété de l'état (la page possède le projet) | Décision 1, CT-1 | Phase 0 | Tout |
| Types de temps (`Tick`, `Frame`, `Tempo`, `Quantize`, `LoopRegion`) | `engine::time` | CT-2 | Transport, clips, grille, timeline, piano roll, enregistrement, automation, audio |
| `ControlMsg` / `RtMsg` / `Feedback`, croissance sans plafond, encodage binaire, mémoire partagée | `engine::msg` | CT-3, MO-1 | Moteur, helper, worklet, compilateur projet, streaming audio |
| Isolation cross-origin | Hébergement, `web/` | AU-0 | Canal moteur (MO-2), streaming (AU-3) |
| Signature `apply`, `Outcome`, `x-undo` / `x-destructive` / `x-kind`, domaines | `commands` | CT-4, PR-2 | Toutes les commandes, MCP, palette |
| Identifiants stables et clés d'ordre | `project` | PR-1 | Commandes, MCP, interface, format |
| Trait `Device`, descripteur de paramètres, événements `ParamValue` datés, lissage | `engine::device` | CT-5, DV-1, AM-3 | Devices, panneau généré, agents, automation, futurs modules de l'Atelier |
| `ProjectEvent` (miroir de l'interface) | `commands` | PR-2 | Stores web, vues |
| Format de projet v1 (arborescence par piste, règle texte / blobs, seuils, état de session) | `project::file` | QA-7, SV-1 | Enregistrement, tests de référence, import audio (et, après le MVP, le pilote de fusion git) |
| `ProjectStore` (dossier réel, OPFS, helper) | `web/src/lib/storage/` | SV-2 | Sauvegarde, import audio, streaming, Boost |
| Protocole page ↔ helper v2 (octets moteur, requêtes MCP corrélées, fichiers du projet) | `protocol` | BO-1, AG-1, AU-7 | Helper, page |
| Stores web (`project`, `engine`) et `run()` | `web/src/lib/stores/` | PR-5 | Toutes les vues |
| Composants `TimeGrid`, `ParamControl`, `TrackHeader`, `Waveform` | `web/src/lib/components/` | AR-3, DV-9, UI-2, AU-2 | Timeline, piano roll, grille, mixeur, automation, clips audio |

Règle pour l'essaim : **une tâche feuille ne modifie jamais un contrat figé.** Si elle en a besoin, elle s'arrête, et une tâche « amendement de contrat » est ouverte et livrée seule, par le lead.

### 5.2 Fichiers à risque de conflit et parades

| Fichier | Pourquoi | Parade |
| --- | --- | --- |
| `crates/commands/src/lib.rs` (`enum Command` + trois `match`) | Chaque commande y ajoute 3 à 4 lignes | Un module par domaine avec son enum et son `apply` ; l'enum racine ne compte qu'**une ligne par domaine**, figée en PR-2 ; `name()` et `undo_kind()` dérivés par macro ou par le schéma. |
| `web/src/lib/stores/dispatcher.ts` (`switch` sur les commandes) | Chaque commande y passerait | Dispatcher générique (PR-5). |
| `web/src/lib/engine/wasm-ops.ts`, `crates/helper/src/audio.rs::engine_event` | Deux mappages commande → moteur | Supprimés par MO-2 : le mappage vit en Rust. |
| `web/src/lib/i18n.ts` | Toutes les chaînes dans un fichier | Un fichier par fonctionnalité (UI-0), test de complétude EN/FR. |
| `web/src/App.svelte` | Un seul écran | Coquille + registre de vues (UI-0, UI-1) : une vue = un dossier + une ligne au registre. |
| `web/src/lib/generated/protocol.ts`, `schema.json` | Régénérés par chaque PR qui touche une commande | Générés au build et vérifiés en CI ; en cas de conflit, régénérer, jamais fusionner à la main (ou les retirer de git : décision en UI-0). |
| `crates/protocol/src/lib.rs` (`ServerMessage`) | Chaque retour moteur y ajoutait une variante | Le `Feedback` passe en octets opaques ; `ServerMessage` ne bouge plus qu'au changement de version. |
| `crates/engine/src/lib.rs`, registre des devices | Réexports, une ligne par device | Une ligne par device, triée ; un fichier par device. |
| `crates/engine/src/msg.rs` (`ControlMsg`) | Chaque fonction moteur ajoute des messages | Toutes les variantes du MVP déclarées d'avance en CT-3 / MO-1, regroupées par domaine. |
| `Cargo.toml`, `Cargo.lock`, `web/pnpm-lock.yaml` | Ajouts de crates et de dépendances | Crates du MVP créées en phase 0 ou 1 ; une tâche feuille annonce toute dépendance dans son plan ; en cas de conflit, rebaser et régénérer le verrou. |
| `.github/workflows/ci.yml` | Chaque domaine ajoute des étapes | Un workflow par domaine (UI-0). |
| `docs/*/index.md` | Ajouts en fin de fichier par chaque PR | Une ligne par PR, ajoutée en dernier au rebase ; conflits résolus en gardant les deux entrées. |
| `docs/log.d/` | Un fragment par PR qui modifie `docs/` | Fichier `<issue>-<slug>.md` propre à la PR, jamais de modification de `docs/log.md` ; le lead replie les fragments à la préparation d'une release. |
| `CLAUDE.md` | Stack et règles | Modifié seulement par les tâches de contrat (CT-1) et DOC-1. |

Le format de projet suit la même logique pour les musiciens : un fichier par device, un arrangement par piste, une note par ligne ([modèle de projet](../architecture/02-modele-projet.md) § 4).

### 5.3 Règles de travail de l'essaim

- Un agent = une tâche = une branche (`feat/<issue>-<slug>`) dans son propre worktree, rebasée sur `develop` avant la PR.
- Chaque tâche liste dans son plan les fichiers qu'elle possède ; deux tâches qui possèdent le même fichier ne tournent pas en même temps (lien de blocage GitHub).
- Les tâches moteur livrent toujours le test de parité natif/WASM et le détecteur d'allocation : la parité Boost est une condition de chaque PR, pas une phase.
- Les textes visibles par l'utilisateur (EN et FR) sont proposés dans la PR et validés par le fondateur (règle de `CLAUDE.md`) ; ces tâches portent `founder: oui`.
- **Fusion des PR** (décision 14) : le lead fusionne lui-même une PR prête (CI verte, revues réglées : Judge Dredd selon la tâche, Copilot facultatif si le quota le permet), qu'elle soit feuille, contrat ou colonne vertébrale ; seule une PR qui attend une décision du fondateur ou la validation d'un texte attend le fondateur (`CLAUDE.md`, [issue tracker](../agents/issue-tracker.md)).
- **Où tournent les agents** (décision 15) : dans le cloud pour l'essaim ; en local, sur la machine du fondateur, pour les tâches `hardware: oui` (audio ou MIDI réels, vérifications manuelles sous Windows).

## 6. Phases (jalons)

| Phase | But | Tâches | Nombre |
| --- | --- | --- | --- |
| **Phase 0 — Contrats** | Les décisions qui permettent le parallélisme sont écrites en code ; l'isolation cross-origin est en place ; le Boost existant est vérifié ; le dépôt est prêt pour le travail parallèle. | CT-1, CT-2, CT-3, CT-4, CT-5, CT-6, UI-0, AU-0, BO-0 | 9 |
| **Phase 1 — Socle** | Un projet de N pistes construit par commandes rend le même signal en natif et en WASM ; la coquille d'interface et les harnais de test existent ; le format de projet est mesuré sous forte charge. | PR-1, PR-2, PR-3, PR-4, MO-1, MO-2, MO-3, DV-1, TR-1, BO-1, QA-1, QA-2, QA-3, QA-4, QA-7, UI-1 | 16 |
| **Phase 2 — Premier projet multi-pistes jouable** | Pistes avec des instruments différents, jeu en direct sur la piste sélectionnée, transport et métronome, effets avec lissage des paramètres, volume/mute/solo, dans les deux modes avec bascule à chaud. | PR-5, PR-8, MO-4, MO-5, TR-2, TR-3, TR-4, DV-2, DV-6, DV-8, DV-9, AM-3, MX-1, MX-2, UI-2, UI-3, RE-1, BO-2 | 18 |
| **Phase 3 — Clips, grille de lancement, piano roll, sauvegarde** | Créer, éditer et enregistrer des clips, les lancer par scènes avec quantification ; le projet vit dans un dossier réel et survit à un rechargement ; un agent MCP pilote Faulk. | CL-1, CL-2, CL-3, GR-1, GR-2, GR-3, AR-3, PN-1, PN-2, RE-2, RE-3, RE-5, PR-6, PR-7, SV-1, SV-4, SV-2, AG-1, AG-2 | 19 |
| **Phase 4 — Arrangement, mixeur, effets** | Un morceau sur la timeline, enregistrement dedans, vue mixeur, delay, réverbération et compresseur ; budget de performance connu. | AR-1, AR-2, AR-4, AR-5, PN-3, RE-4, MX-3, DV-3, DV-4, DV-5, QA-5, BO-3 | 12 |
| **Phase 5 — Pistes audio et automation** | Importer des fichiers audio et les jouer en flux depuis le disque dans la grille et la timeline, en Web et en Boost ; automatiser les paramètres des devices et du mixeur à l'échantillon près. | AU-1, AU-2, AU-3, AU-4, AU-5, AU-6, AU-7, AU-8, AM-1, AM-2, AM-4, AM-6 | 12 |
| **Phase 6 — Fichiers, agents, finition** | Repli OPFS et export, garde-fous des agents, palette et raccourcis, enregistrement des mouvements de paramètres, latence mesurée, scénario MVP complet, documentation vérifiée. | SV-3, AG-3, AG-4, AG-5, BO-4 (#10), UI-4, UI-5, AM-5, QA-6, DOC-1 | 10 |
| **Total** | | | **96** |

<!-- phases:start -->
Parallélisme attendu (largeur maximale du front de tâches d'une même phase à une même profondeur, calculée par le script du § 4.2) : phase 0 ≈ 5 ; phase 1 ≈ 6 ; phase 2 ≈ 5 ; phase 3 ≈ 5 ; phase 4 ≈ 4 ; phase 5 ≈ 4 ; phase 6 ≈ 4.
<!-- phases:end -->

Après le MVP : l'enregistrement audio (le chemin inverse du streaming, [stockage local](../architecture/03-stockage-local.md) § 3.3 et § 5.5), l'Atelier (modules Faust compilés à la volée, contrat `Device` réutilisé), les sessions ACP dans l'Atelier, le recalage temporel des clips audio, les enveloppes de clip, les envois et retours, les plugins CLAP puis VST3 en Boost avec gel des pistes, WebMCP, les versions et variantes dans l'application, la quantification de lancement par clip.

## 7. Décisions du fondateur (2026-10-14)

Les quinze décisions, et ce qu'elles changent dans ce plan :

| # | Décision | Effet |
| --- | --- | --- |
| 1 | Créer `crates/project` et `crates/project-wasm` ; la page possède le projet dans les deux modes ; le helper relaie. | § 2.1 ; CT-1 applique ; AU-7 pour le dossier en Boost. |
| 2 | Format de projet : dossier regroupé par piste, texte pour la structure, blobs adressés par contenu pour les données massives. | SV-1 ; [modèle de projet](../architecture/02-modele-projet.md). |
| 3 | Stockage par défaut : dossier réel (Chrome, Edge) ; OPFS + export zip ailleurs. | SV-2 (dossier réel, phase 3), SV-3 (repli, phase 6) ; [stockage local](../architecture/03-stockage-local.md) § 5.1. |
| 4 | Isolation cross-origin (COOP/COEP ou Document-Isolation-Policy, polices et bibliothèques auto-hébergées). | AU-0 (phase 0) ; chemin à mémoire partagée de CT-3 et MO-2 ; AU-3. |
| 5 | Périmètre du MVP : pistes MIDI, **pistes audio** (import et lecture, streaming disque, pas d'enregistrement audio), **automation** ; tempo constant ; pas d'envois et retours ; ACP avec l'Atelier plus tard. | § 0 ; épopées 16 et 17 ; phase 5. |
| 6 | Instruments et effets : synthé oscillateur, batterie synthétisée, filtre, delay, réverbération, compresseur, plus le lecteur de clips des pistes audio. | DV-1 à DV-6, AU-4. |
| 7 | Pas de plafond visible : structures préparées hors chemin audio et échangées sans allocation temps réel ; seule une limite de sûreté très haute existe. | § 2.3 ; CT-3, MO-1, MO-3, PR-1, PR-8, CL-1, CL-2, GR-3, QA-4, QA-7 adaptés. |
| 8 | Quantification de lancement globale, 1 mesure par défaut, réglable (aucune, 1/16, 1 temps, 1, 2, 4, 8 mesures) ; réglage par clip plus tard (champ optionnel du fichier de clip, absent = global). | § 2.2 ; CT-2, GR-1, GR-2, TR-4. |
| 9 | Vocabulaire de l'interface : « Version » (commit) et « Variante » (branche). | CT-6 ; [modèle de projet](../architecture/02-modele-projet.md) § 4.6. |
| 10 | Solo : état de session dans `.faulk/` (ignoré par git, restauré à la réouverture), jamais versionné ; le mute reste versionné. | MX-2, SV-1, SV-4. |
| 11 | Copie d'un clip dans la timeline : copie indépendante par défaut, lien en option. | AR-1, AR-4, AR-5. |
| 12 | Stockage audio versionné : fichiers nommés par SHA-256, compatibles Git LFS ; LFS ou magasin séparé décidé quand le besoin viendra. | SV-1, AU-1. |
| 13 | faustwasm (LGPL-3.0) : acceptable si Faulk est open source ; à revoir sinon. | [Spec de l'Atelier](01-atelier-modules.md), hors MVP. |
| 14 | Fusion des PR de l'essaim : le lead fusionne les PR prêtes (CI, revues réglées ; Copilot facultatif si le quota le permet), feuilles comme contrats et colonne vertébrale. | § 5.3. |
| 15 | Où tournent les agents : dans le cloud pour l'essaim ; en local pour les tâches qui touchent l'audio ou le MIDI réels. | § 3 (étiquette `hardware`), § 5.3. |

Décision ajoutée le même jour (#172) : le nom d'un device n'est jamais traduit ; un seul nom anglais, le même dans les deux langues de l'interface, les libellés et descriptions des paramètres restant traduits. Effet : § 2.5 ; noms affichés de DV-1 à DV-6 ; [spec de l'Atelier](01-atelier-modules.md) § 1.4 (nom d'un module).

Tranché par CT-4 (#40, § 2.4) : l'état de vue (sélection de piste en commande `live`, zoom et défilement hors commandes), l'absence de `NoteId` (notes adressées par valeur, `note_id` de voix côté moteur), le modèle mutable à inverses. Tranché par CT-3 (#39, § 2.3) : les chiffres des limites de sûreté et la voie WASM. Restent à trancher, dans les tâches de contrat (choix techniques, pas des décisions du fondateur) : le cache PCM régénérable plutôt qu'un second blob (AU-1, proposition par défaut présentée au fondateur) ; le choix du dossier en Boost, sélecteur natif ou rapprochement par identifiant (AU-7). Le format des prises audio (FLAC ou WAV) sera décidé avec l'enregistrement audio, après le MVP.
