---
type: Spike
title: "QA-7 — Banc de charge du format de projet"
description: "Load bench of the project folder format (#130): a deterministic synthetic project of 100 tracks, 200 000 notes and 6 million automation points, measured on open, save, memory, engine structure swaps and git, natively and in the browser (OPFS), with the proposed format thresholds and the CT-3 safety limits confronted with the numbers."
status: draft
---

# QA-7 — Banc de charge du format de projet

Cette étude mesure ce que coûte le format dossier du [modèle de projet](../architecture/02-modele-projet.md) sur un projet très lourd, et en tire les seuils du format (§ 2.1 et § 7 du modèle). Tous les chiffres viennent de mesures faites pour cette étude ; ce qui est calculé ou projeté est dit tel. Le statut reste `draft` : l'écrivain du format utilisé ici est un **prototype** que SV-1 reprend ou remplace.

## Machine et méthode

- **Machine** : Intel Core Ultra 7 268V (8 cœurs, 8 threads), 31,5 Gio de RAM, SSD NVMe KIOXIA KBG6AZNV 512 Go, Windows 11 Pro 10.0.26200, Defender (protection en temps réel) actif. Rust 1.99 (release, `lto`), git 2.50.0 et git-lfs 3.6.1 (configuration utilisateur et système écartées), Chromium sans tête 156 (Playwright).
- **La machine est partagée** avec d'autres sessions de compilation : la charge processeur mesurée avant chaque mesure native allait de 10 % à 97 % (elle est notée dans les journaux de mesure). Une mesure attend une machine calme au plus 40 s, puis tourne quand même. Les durées natives sont donc données en **intervalle** (meilleure médiane – pire médiane des exécutions faites à des charges différentes). Un chiffre à une décimale n'est pas plus précis que cet intervalle.
- **Cache de fichiers chaud** : les dossiers viennent d'être écrits ou relus ; une ouverture à froid (après redémarrage) n'est pas mesurée.
- **Générateur** (`crates/project/benches/load/synth.rs`) : graine **130**, SplitMix64 et un `IdGenerator` à horloge fixe, sans horloge système ni itération de `HashMap`. Profil lourd : 100 pistes (80 MIDI, 20 audio), 800 clips MIDI de 250 notes = **200 000 notes**, 840 couloirs d'automation = **6 000 000 points** (un point tous les 5 à 35 ticks, valeurs à 0,001, 15 % de paliers), 100 fichiers audio (en-tête WAV et bruit, stéréo 16 bits 48 kHz, incompressibles), 900 placements, 8 scènes, 3 devices par piste MIDI. Le projet passe `Project::validate()` et ses comptes sont vérifiés par les tests (`crates/project/tests/load_bench.rs`). Même graine, mêmes octets : l'empreinte du dossier de texte (audio de 64 Mio) est `ef8b56c432a23031abf792e2893ba82e42993ca9bb32c337602db1e90dd0bce0` dans deux processus distincts.
- **Écrivain et lecteur du prototype** (`benches/load/folder.rs`) : JSON à clés triées pour les squelettes, TSV pour les notes, points et placements, blobs binaires de 17 octets le point, écriture par fichier temporaire puis renommage, instantané binaire `.faulk/cache/project.snap` lié à une liste (chemin, taille, date) de 3 001 fichiers. 3 001 fichiers de texte, 112,6 Mio. Il réutilise les types et `validate` de `crates/project` ; rien n'entre dans la crate (pas de `project::file`). Les tests couvrent : déterminisme, aller-retour octet pour octet (texte et blobs), écriture paresseuse (rien changé, une note, première sauvegarde, modification extérieure invisible : limite documentée), sauvegarde d'un seul fichier, instantané (fichier changé, ajouté, supprimé, instantané plus vieux que le texte, tronqué, mauvais en-tête), bascule texte/blob dans les deux sens.
- **Audio** : 0,25, 0,5 et 1 Gio mesurés pour git ; les ouvertures et sauvegardes ne dépendent pas de l'audio (les blobs ne sont pas lus à l'ouverture). Une projection à 4 Gio est donnée comme **projection**, non mesurée.
- **Ce qui n'est pas mesuré** : le dossier réel du navigateur (File System Access, `showDirectoryPicker`) demande une boîte de dialogue native et un geste de l'utilisateur, que Playwright ne sait pas piloter : seul l'OPFS est mesuré. Git dans le navigateur (isomorphic-git) n'existe pas en phase 1 (modèle § 6.3) : git est mesuré en natif seulement. Une ouverture à froid et Firefox/Safari ne le sont pas non plus.

Commandes : `cargo bench -p project --bench load -- <gen|open|save|mem|swap|stream|vcs|density|dirs> --dir <dossier>` (toujours en release) ; navigateur : `FAULK_BENCH_DIR=<dossier> FAULK_BENCH_WASM=<load_wasm.wasm> pnpm bench:project opfs` et `FAULK_BENCH_STREAM=<flux> pnpm bench:project engine-swap` (le lecteur wasm : `cargo build --release --target wasm32-unknown-unknown --manifest-path crates/project/benches/wasm/Cargo.toml`).

## Résultats natifs (projet lourd, format texte)

| Mesure | Résultat |
| --- | --- |
| Ouverture du dossier de texte, 1 thread (lecture + analyse) | 565 ms au mieux ; médianes 619 à 977 ms (lecture des fichiers ≈ 40 %, analyse ≈ 60 %) |
| Ouverture, 8 threads (une piste par tâche) | 144 ms au mieux ; médianes 161 à 223 ms |
| `validate` du projet | 35 à 57 ms |
| Ouverture par l'instantané (parcours des 3 001 fichiers pour vérifier + lecture + construction) | médianes 185 à 256 ms (le parcours seul : 28 à 40 ms) |
| Relire un seul fichier de notes (rafraîchir après l'instantané) | 0,07 à 0,15 ms |
| Sauvegarde du projet entier, dossier vide, 1 thread, écriture atomique | médianes 9 à 12 s ; 8 threads : 3,8 à 7,9 s ; avec `fsync` : 13 à 22 s (1 thread), 4,6 à 8,2 s (8 threads) |
| Écriture directe sans fichier temporaire ni renommage | 4,3 à 6,5 s (le fichier temporaire et le renommage doublent à peu près le coût) |
| Sérialiser tous les fichiers sans écrire (détecter ce qui a changé sans suivi des fichiers sales) | 0,56 à 0,77 s |
| Une note modifiée, sauvegarde du seul fichier du clip (250 notes) | 2,1 à 2,6 ms ; 2,8 à 3,7 ms avec `fsync` |
| Un point modifié dans un couloir de 7 143 points, sauvegarde du fichier | 3,0 à 3,4 ms ; 4,2 à 5,8 ms avec `fsync` |
| Tas du modèle après ouverture | 156,8 Mio (27,4 o par point, tout compris) ; `BTreeMap<Tick, AutomationPoint>` 26,2 o par point, `BTreeSet<Note>` 28,1 o par note ; pic de mémoire du processus 291 à 294 Mio |

Lecture : le coût d'une sauvegarde complète est celui des **fichiers**, pas des octets : 3 001 fichiers en 4,3 s au mieux, soit 1,4 ms le fichier avec Defender (la sérialisation de tout le projet ne pèse que 0,6 s). L'écriture d'**une note** ne coûte que 2 à 3 ms avec le suivi des fichiers sales : sans lui, il faut sérialiser et comparer tout le projet (0,6 s) pour n'écrire qu'un fichier.

### Échanges de structures du moteur (§ 2.3, natif)

L'instantané du projet par la couche de commandes (`commands::engine_msgs::snapshot`) passe message par message dans le vrai `Preparer` et le vrai `Engine`. Le compte est vérifié : 200 000 notes sur 200 000 et 6 000 000 points sur 6 000 000 arrivent au moteur (2 803 messages) ; **ne sont pas envoyés** les clips et placements audio, que la couche de commandes n'envoie pas encore (AU-3) : l'audio ne fait pas partie de la mesure.

| Mesure | Résultat |
| --- | --- |
| Instantané du projet en messages, puis encodage | 72 à 112 ms puis 82 à 158 ms, 78,5 Mio |
| `prepare` de tout le projet (hors chemin audio) | 32 à 39 ms au total ; un couloir de 7 143 points : médiane 6 à 12 µs, au plus 135 µs ; un clip de 250 notes : médiane 17 à 34 µs, au plus 850 µs |
| `apply` (chemin audio) : le plus long de tout le projet | 2,7 à 13,5 µs (budget de MO-2 : 500 µs) ; croissance de la table des pistes (100 pistes, 128 emplacements) : au plus 7,3 µs |
| Tas tenu par le moteur après tout le projet | 97,5 Mio (les messages seuls : 96,4 Mio) |
| Synchronisation par blocs de 128 images | 22 blocs (58,7 ms d'audio à 48 kHz) ; coût d'un bloc pendant la synchronisation : médiane 76 à 115 µs, au plus 194 µs ; bloc au repos du même moteur : médiane 141 à 189 µs |
| Limite de sûreté, notes : 17 clips de 986 895 notes = 16 777 215 | tas du moteur 387 Mio (24 o par note) ; pic 410 Mio ; `prepare` du dernier morceau d'un clip : jusqu'à 152 ms (médiane 0,47 ms) ; `apply` au plus 5,3 µs ; 2,6 s pour tout préparer |
| Limite de sûreté, points : un couloir de 16 777 216 points | tas du moteur 257 Mio (16 o par point), **pic 513 Mio** (les morceaux en attente et la table finale coexistent) ; `prepare` 340 µs par morceau de 65 536 points ; `apply` 1,3 µs |

### Git sur le dossier (git 2.50, blobs audio sous Git LFS sauf mention)

| Opération | Résultat |
| --- | --- |
| `git add -A` initial (3 103 fichiers suivis) | 10,8 s (256 Mio d'audio), 11,7 s (512 Mio), 21,8 s (1 Gio) ; sans LFS (256 Mio en objets git) : 23,1 s |
| `git commit` initial | 1,3 s, 1,6 s, 1,6 s ; sans LFS : 1,3 s |
| `git status`, rien de changé | 77 à 120 ms (186 ms au pire, sans LFS sous charge) |
| `git status`, une note changée | 95 à 156 ms |
| `git diff` d'une note : 11 lignes lisibles (`-0 840 66 99 0 100` / `+0 840 66 77 0 100`) | 65 à 80 ms ; un point d'automation : 13 lignes, 63 à 90 ms |
| `git add` + `git commit` d'une note | 0,12 s + 0,5 s (0,15 s sans LFS) |
| Fusion de deux branches sur des fichiers différents | 0,26 à 1,4 s |
| Fusion de deux lignes différentes du même fichier de notes | 0,23 à 1,3 s, sans conflit |
| Fusion de la même note changée des deux côtés | 0,17 à 0,44 s, conflit détecté (code 1) |
| `.git` après le premier commit | 293 Mio avec 256 Mio d'audio LFS, donc ≈ 37 Mio pour les 112,6 Mio de texte ; 1 061 Mio pour 1 Gio |

Le coût de `git add` croît avec l'audio de 10,8 s à 21,8 s de 0,25 à 1 Gio ; une **projection** linéaire de ces points donne de l'ordre d'une minute pour 4 Gio (non mesuré). Les mesures de `status`, `diff`, `commit` d'une note et de fusion ne dépendent pas de la taille de l'audio sous LFS.

### Densité d'automation : texte ou blob ?

Un seul couloir de N points (geste enregistré), en TSV (≈ 20 o le point) ou en blob de 17 o le point. Une machine à 43–46 % de charge, médianes.

| N | TSV Kio | blob Kio | lire + analyser TSV (ms) | idem blob (ms) | sauver 1 point TSV (ms) | idem blob (ms) | `git diff` (ms) | pack après `gc`, 2 versions (Kio) |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 1,8 | 1,7 | 0,05 | 0,04 | 1,1 | 3,1 | 26 | 2 |
| 1 000 | 17,6 | 16,6 | 0,14 | 0,08 | 2,2 | 3,7 | 25 | 6 |
| 10 000 | 185 | 166 | 0,97 | 0,51 | 2,7 | 4,1 | 28 | 50 |
| 100 000 | 1 949 | 1 660 | 9,3 | 4,3 | 14,6 | 6,6 | 49 | 493 |
| 1 000 000 | 20 459 | 16 602 | 86 | 38 | 124 | 25,9 | 430 | 4 918 |

- Après `git gc`, une version de plus d'un couloir de TSV coûte un **delta** de quelques octets (le pack de deux versions pèse une version compressée) et un diff de 13 lignes, quelle que soit la taille. Un blob sous LFS stocke une copie complète (17 o × N) par version.
- Le projet lourd en **disposition blob** (tous les couloirs de plus de 1 000 points, soit les 840) : ouverture 276 ms (1 thread) et 105 ms (8 threads) contre 619 à 977 ms et 161 à 223 ms en texte, mais **machine plus calme pour le blob (10 % contre 36 à 69 %)** : l'écart de 2 à 3 fois est de l'ordre du bruit de charge ; l'analyse seule passe de 379–549 ms à 106 ms (somme sur les threads). Détecter les changements : 125 à 142 ms contre 560 à 770 ms. En revanche le dossier passe de 3 001 à 3 841 fichiers (+28 %), la sauvegarde complète ne gagne rien (4,3 à 11 s), `git add` initial 15,8 s contre 10,8 s, et les diffs de couloirs deviennent binaires.
- Un couloir qui change de côté du seuil laisse un blob orphelin (nettoyage sur demande seulement, § 3.4) ; deux bascules sans modification ne créent aucun blob de plus (même contenu, même nom), mais une modification en crée un neuf (testé).

### Sous-division de `blobs/` (disque natif NTFS, fichiers de 64 octets, une exécution par taille, charge 45–67 %)

| Fichiers | à plat : création / liste / 1 000 lectures | 256 sous-dossiers : création / liste / 1 000 lectures |
| ---: | --- | --- |
| 1 000 | 367 ms / 1,1 ms / 50 ms | 642 ms / 16 ms / 54 ms |
| 10 000 | 6,8 s / 9,5 ms / 97 ms | 7,1 s / 33 ms / 87 ms |
| 100 000 | 85 s / 139 ms / 165 ms | 90 s / 103 ms / 145 ms |

Pas d'écart net entre les deux dispositions jusqu'à 100 000 fichiers (une seule exécution par ligne : l'écart de 10 à 15 % au plus à 100 000 est dans le bruit). Le coût est celui de la **création** (0,4 à 0,9 ms le fichier avec Defender).

## Résultats dans le navigateur (OPFS, Chromium 156 sans tête)

Le lecteur du prototype est compilé en wasm32 et tourne dans la page, alimenté par l'OPFS : c'est le code Rust, pas un analyseur de remplacement. Une exécution complète du banc ; les ouvertures sont la médiane de 3.

| Mesure | Résultat |
| --- | --- |
| Écrire les 3 001 fichiers (112,6 Mio) en OPFS, `createWritable` asynchrone | 8,8 s (1 à la fois), **2,5 s (8 à la fois)**, 2,8 s (32 à la fois) |
| Idem, un worker et des poignées d'accès synchrones | 5,6 s (séquentiel) |
| Lister tout l'arbre (`entries()`) | 248 ms |
| Lire les 3 001 fichiers, `getFile().arrayBuffer()` | 3,3 s (1 à la fois), 1,2 s (8), **1,0 s (32)** ; poignée synchrone dans un worker : 3,2 s |
| Sauver une note (fichier de 5,6 Kio réécrit) | 1,6 ms (médiane, p95 2,2 ms), asynchrone comme synchrone |
| Instantané de cache (102 Mio, un fichier) : écriture / lecture | 284 ms / 84 ms |
| **Ouvrir depuis les fichiers OPFS** | 2,45 s = lecture 0,90 s + copie dans la mémoire wasm 0,22 s + lecteur Rust 1,33 s |
| **Ouvrir depuis l'instantané**, sans vérifier sa validité (première exécution) | 1,56 s = lecture 0,09 s + copie 0,04 s + décodage 1,45 s |
| Seconde exécution du banc, **avec** la vérification de la taille et de la date de chacun des 3 001 fichiers (32 à la fois), machine plus chargée | fichiers : 3,93 s (lecture 2,18 s + copie 0,30 s + lecteur 1,63 s) ; instantané : 2,09 s = vérification 0,97 s + lecture 0,14 s + copie 0,05 s + décodage 1,23 s |
| `validate` en wasm | 24 ms |
| Sérialiser tout le projet sans écrire / les notes d'un clip | 0,79 s / 0,10 ms |
| Mémoire après ouverture | mémoire linéaire wasm 258 à 269 Mio (pic de 103 à 114 Mio à l'arrivée des octets, dont les copies), tas JS 347 Mio (les fichiers gardés par le banc) |
| `blobs/` en OPFS, fichiers de 64 octets, à plat / 256 dossiers | 1 000 fichiers : 1,6 s / 1,5 s ; 10 000 fichiers : 18,6 s / 20,2 s ; liste de 10 000 : 306 ms / 512 ms ; pas de gain à subdiviser |
| Blob audio en OPFS : 256 Mio | écriture 1,9 s (134 Mio/s), relecture 0,36 s, SHA-256 de tout le tampon (`SubtleCrypto`) 0,31 s |
| Projet aux limites de notes et de points (CT-3) construit en wasm32 | 16 777 215 notes et 16 777 215 points : 44,6 s, **1 047 Mio de mémoire linéaire sur 4 096**, `validate` 0,63 s |

**Échanges de structures dans le worklet livré** (flux du projet lourd avec les seuls devices du moteur web : 2 403 messages, 600 couloirs de 10 000 points en moyenne, 78,5 Mio ; `prepare` dans un Worker, `apply` dans la pompe du worklet) : quatre exécutions complètes (machine chargée) ont donné une pompe la plus longue de **268, 274, 411 et 499 µs** (budget de MO-2 : 500 µs ; la quatrième frôle le budget, sous charge, hors machine de référence), 0,19 à 0,25 µs par message, au plus 128 messages par bloc, aucune allocation ni libération côté rendu, aucun rejet ni saut, 100 pistes dans 128 emplacements, mémoire partagée de 18 à 116 Mio, tout appliqué en 0,3 à 0,8 s. Une exécution sur cinq (la troisième de la boucle) n'a pas fini avant le délai du test (10 min) et la suivante n'a pas tourné : **cause non analysée** (la boucle d'attente de la file de la harnais de MO-2 ne sort pas), à reprendre avant de s'appuyer sur ce banc en boucle. Les assertions du banc vérifient que la file s'est vidée et que le nombre de messages appliqués par le worklet égale celui du moteur (2 403, variable `FAULK_BENCH_EXPECT_APPLIED`) ; elles ont été ajoutées avant la quatrième exécution. Le médian de cinq exécutions sur une machine de référence au repos (`docs/agents/verification.md`) **n'a pas été fait** : ces quatre valeurs ne tranchent pas le budget.

## Seuils proposés pour le format

Reportés dans le [modèle de projet](../architecture/02-modele-projet.md) (§ 2.1 et § 7) comme **propositions de QA-7**, confirmées et appliquées par SV-1 (`project::file`, `Layout::QA7`).

1. **Automation : texte jusqu'à 100 000 points par couloir, blob au-delà** (au lieu de « environ 1 000 »). À 100 000 points, un couloir fait 1,9 Mio de TSV, se relit en 9 ms, se réécrit en 15 ms pour un point changé, et `git diff` reste à 49 ms ; à 1 000 000, la réécriture d'un point coûte 124 ms (hors du budget d'un geste) et le diff 430 ms. Sous 100 000, le blob n'apporte que 2 fois sur l'analyse, mais coûte un fichier de plus par couloir (mesuré : +28 % de fichiers, `git add` +46 %), des diffs illisibles et des copies complètes sous LFS. Avec **hystérésis** : passer en blob au-dessus de 100 000 points, revenir au texte sous 50 000, pour qu'un couloir au bord ne bascule pas à chaque sauvegarde (chaque bascule laisse un blob orphelin et un diff complet). Le projet lourd (7 143 points par couloir) reste en texte.
2. **Instantané de cache : oui, pour le navigateur, pas pour le natif.** En natif, lire les fichiers sur 8 threads (144 à 223 ms) vaut l'instantané (185 à 256 ms) ; en OPFS, l'instantané fait passer l'ouverture de 2,45 s à 1,56 s sans vérification, et de 3,93 s à 2,09 s **avec** la vérification de la taille et de la date des 3 001 fichiers (0,97 s, mesurée dans la seconde exécution, machine plus chargée) : un gain de 40 à 47 % dans les deux exécutions. L'essentiel du temps restant est la **construction du modèle** (1,3 à 1,45 s en wasm, contre 0,4 à 0,55 s en natif sur un thread), que l'instantané ne supprime pas : le gain à chercher est dans la représentation des couloirs (`BTreeMap<Tick, AutomationPoint>` coûte 26 o le point), pas dans le format. La validité de l'instantané se vérifie par la liste (chemin, taille, date) de **tous** les fichiers, ajouts et suppressions compris (testé) ; le parcours de 3 001 fichiers coûte 28 à 40 ms en natif, 0,97 s en OPFS (l'énumération seule : 248 ms). **Limite** : une réécriture de même taille dans la granularité de la date (2 s sur FAT et exFAT, 1 ms pour `lastModified` en OPFS) n'est pas vue (non testée : le test de péremption avance la date de 5 s) ; SV-1 doit ajouter l'empreinte du contenu pour les fichiers qui viennent d'être modifiés ou d'un autre outil (git).
3. **Pas de sous-division de `blobs/` en phase 1.** Le projet lourd n'a que 100 blobs audio (un par fichier) ; à plat ou en 256 dossiers, aucun écart net jusqu'à 100 000 fichiers en natif et 10 000 en OPFS. Si l'automation passe en blobs au-dessus de 100 000 points, le nombre de blobs reste de l'ordre du nombre de couloirs. À reconsidérer au-delà de 10 000 blobs (la création coûte 1,5 à 2 ms le fichier en OPFS, 18 s pour 10 000).
4. **Constat supplémentaire, hors des trois seuils demandés : le nombre de fichiers domine les coûts d'écriture.** 3 001 fichiers = 4,3 à 12 s d'écriture complète en natif (Defender), 2,5 s en OPFS, 10,8 s de `git add` ; le texte brut de 112,6 Mio ne pèse que 0,6 s à sérialiser. Le suivi des fichiers sales (§ 7) est donc indispensable, et SV-1 gagnerait à ne pas ajouter de fichiers (un clip = 2 fichiers, un couloir = 1 ou 2).

## Limites de sûreté de CT-3 confrontées aux mesures

| Limite (`engine::msg::limits`) | Mesure |
| --- | --- |
| 16 777 216 notes ; 1 048 576 par clip | engine : 24 o par note, 387 Mio pour 16 777 215 notes ; modèle de la page : 28,1 o par note |
| 16 777 216 points (aussi par couloir) | engine : 16 o par point, 257 Mio pour 16 777 216 points dans un couloir, pic 513 Mio pendant le montage ; modèle de la page : 26,2 o par point |
| Budget moteur 3 Gio − 64 Mio, PCM 1 Gio, plafond wasm32 4 Gio | les deux limites ensemble pèsent 387 + 257 = 644 Mio dans le moteur (mesurées séparément), soit au moins 3,3 fois de marge sur le budget (3 008 Mio contre la somme des pics mesurés, 410 + 513 Mio, qui eux ne sont pas simultanés) ; dans la page, le modèle aux deux limites tient dans 1 047 Mio de mémoire linéaire sur 4 096 (mesuré en wasm32) |
| Budget de la pompe : 500 µs la plus longue, 2 µs par message | heavy : natif `apply` au plus 13,5 µs ; worklet livré 268 à 499 µs (machine chargée), 0,2 à 0,33 µs par message |
| `prepare` « sans borne » dans le Worker | un clip de 986 895 notes : jusqu'à 152 ms pour son dernier morceau ; 2,6 s pour 16,7 millions de notes |
| 4 096 pistes, 65 536 couloirs, 1 048 576 clips | non mesurés ici (la mesure de MO-1 couvre les tables) |

Conclusion, pour la **mémoire** : les limites tiennent, avec une marge d'au moins 3,3 sur le budget du moteur. Pour la **pompe du worklet**, ces mesures ne concluent pas : la plus longue observée sur le projet lourd va de 268 à 499 µs sur une machine chargée, au bord du budget de 500 µs, alors que le natif reste 40 fois en dessous (13,5 µs) ; la règle du dépôt demande la médiane de cinq exécutions sur une machine de référence au repos, qui reste à faire. Rien ne demande de changer une limite, mais le budget de 500 µs du projet lourd n'est pas démontré. Le compteur d'occupation du PCM (`03-stockage-local.md` § 5.4) n'est pas mesuré ici : il dépend d'AU-3, pas encore livré.

## Décisions prises

- **Emplacement** : le banc est dans `crates/project/benches/load` (le fichier du ticket), sans crate ajoutée au workspace ; le lecteur wasm est une crate autonome (`benches/wasm`, son propre `[workspace]`, hors CI) pour ne pas ajouter de crate au workspace (le tableau de `CLAUDE.md` est une décision du fondateur). Conséquence : la CI ne la compile pas, et un changement de `folder.rs` ou `synth.rs` peut la casser sans alerte jusqu'à la prochaine exécution du banc navigateur. Les tests du banc sont dans `crates/project/tests/load_bench.rs`, qui compile les mêmes modules (un banc `harness = false` n'exécute aucun `#[test]`).
- **Projections** : le 4 Gio d'audio est une projection étiquetée, pas une mesure ; aucun autre chiffre n'est extrapolé.
- **Dossier réel du navigateur** : non mesuré, faute de pouvoir piloter la boîte de dialogue ; il passe par la même API de poignées que l'OPFS, mais avec le système de fichiers de l'utilisateur (et son antivirus) : son coût par fichier n'est pas connu : à mesurer à la main par SV-2 avant de dimensionner l'enregistrement.
- **Périmètre réduit, à valider par le fondateur** : le dossier réel du navigateur, git dans le navigateur et le compteur du PCM (`03-stockage-local.md` § 5.4, qui renvoie à QA-7) ne sont pas mesurés, avec les raisons ci-dessus ; l'agent n'a pas modifié ce renvoi.
- **Charge de la machine** : chaque mesure native a été lancée par un script (`crates/project/benches/load/run-native.sh`) qui attend une machine calme au plus 40 s et note la charge avant et après ; le script est livré, les journaux ne le sont pas (ils sont recopiés dans ce document).
- **Machine partagée** : les chiffres sont donnés en intervalles avec la charge ; une mesure plus propre demande une machine de référence au repos (`docs/agents/verification.md`).
- **Lanes sur le test effect** : le projet lourd utilise l'effet de test du moteur pour avoir 10 couloirs par piste MIDI (la bibliothèque n'a pas d'effet intégré) ; la variante web n'a que les devices intégrés (7 couloirs).
