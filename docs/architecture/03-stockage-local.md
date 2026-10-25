---
type: Architecture
title: "Stockage local du projet et mémoire audio"
description: "Keeping a Faulk project and its audio on disk rather than in browser memory, decided on 2026-10-14: a real folder by default (Chrome, Edge), OPFS and archive export elsewhere, cross-origin isolation accepted, disk streaming through SharedArrayBuffer rings into the AudioWorklet, the Boost-mode impact, atomic writes and failure modes."
status: draft
---

# Stockage local du projet et mémoire audio

> **Décisions du fondateur du 2026-10-14 intégrées** (§ 6). Ce document reprend l'étude du 2026-10-13 sur le stockage local. Il suit le format de projet de [Modèle de projet et format sur disque](02-modele-projet.md) (dossier regroupé par piste, audio dans `blobs/<sha256>`, caches dans `.faulk/`), qui remplace la disposition à un seul document proposée par l'étude. Décidé : le dossier réel est le stockage par défaut (Chrome, Edge), l'OPFS avec export en archive le repli ; l'isolation cross-origin est acceptée, donc le streaming passe par `SharedArrayBuffer` ; les pistes audio (import et lecture) sont dans le MVP, l'enregistrement audio le suit immédiatement : les paragraphes sur l'enregistrement décrivent ce qui vient juste après. Le statut reste `draft` tant que le document n'a pas été vérifié contre le code.

La question : l'application web peut-elle travailler sur un projet stocké localement, avec ses fichiers audio, sans que tout soit dans la mémoire du navigateur ?

## Réponse courte

**Oui, le projet peut vivre sur disque, et non, tout n'a pas besoin d'être en mémoire.**

1. **Où vit le projet.** Dans Chrome et Edge (bureau), le projet peut être un **vrai dossier choisi par l'utilisateur** (File System Access, `showDirectoryPicker`) : visible dans l'explorateur de fichiers, versionnable avec git, sauvegardable. Firefox et Safari n'offrent pas ce dossier ; ils offrent l'**OPFS**, un système de fichiers privé au site, invisible pour l'utilisateur, soumis au quota et à l'éviction. Le dossier réel est donc un avantage Chromium, avec l'OPFS en repli et l'export `.faulkpack` pour les autres.
2. **Ce qui est en mémoire.** Seul ce qui joue ou va jouer doit être en mémoire, plus de petits caches (débuts de fichiers, aperçus de forme d'onde). `decodeAudioData` décode tout un fichier et ne convient pas aux longs enregistrements ; on peut lire des morceaux du fichier dans un Worker et les pousser vers l'`AudioWorklet` (streaming). Un projet de quelques dizaines de pistes tient même entièrement décodé (environ 23 Mo par minute de stéréo 48 kHz en float32) : **on peut charger les clips courts en mémoire et streamer les longs**.

## 1. Les options de stockage et leurs limites

### 1.1 File System Access (dossier réel)

| Point | État (octobre 2026) |
| --- | --- |
| `showDirectoryPicker`, `showOpenFilePicker`, `showSaveFilePicker` | Chrome et Edge bureau depuis la v86 (complet depuis la v105), Opera. **Pas Firefox** (position négative de Mozilla), **pas Safari**, **pas Chrome Android**. |
| Lecture | `FileSystemFileHandle.getFile()` renvoie un `File` (un instantané) ; `file.slice(a, b).arrayBuffer()` lit une plage sans charger le reste. Accès aléatoire, asynchrone. |
| Écriture | `createWritable()` écrit dans un fichier d'échange (`.crswap` dans Chrome) et remplace la cible à `close()`. Option `mode: "exclusive"` pour un seul écrivain. Chrome lance un contrôle Safe Browsing à la fermeture (`AbortError` possible). |
| Accès synchrone (`createSyncAccessHandle`) | **Réservé à l'OPFS.** |
| Mémoriser le dossier | Le `FileSystemDirectoryHandle` se range dans IndexedDB. Au retour : `queryPermission()` puis `requestPermission()` (geste de l'utilisateur obligatoire). |
| Permissions persistantes (Chrome 122+) | Invite à trois choix, dont « Autoriser à chaque visite ». Sans ce choix, l'accès dure jusqu'à la fermeture du dernier onglet du site et peut être **révoqué si l'onglet reste longtemps en arrière-plan**. Une PWA installée garde l'accès par défaut. |
| Dossiers interdits | Chrome refuse les dossiers système et sensibles (racine du disque, dossier du système, parfois tout le dossier de téléchargements) : l'interface gère le refus et propose un sous-dossier. |
| Changements externes | `FileSystemObserver` : livré dans Chrome et Edge 133 (bureau), observation récursive possible, sur ce que l'utilisateur a autorisé. Pas Firefox, pas Safari ; pas encore standardisé. |
| Quota | Un dossier de l'utilisateur **n'est pas compté dans le quota** et n'est **jamais évincé**. |

### 1.2 OPFS (Origin Private File System)

- `navigator.storage.getDirectory()` : disponible partout (Baseline).
- `createSyncAccessHandle()` dans un **Worker dédié** : lecture et écriture synchrones à une position, verrou exclusif (le mode `readwrite-unsafe` permet plusieurs poignées). Firefox 111+, Safari 15.2+, Chrome 102+.
- `createWritable()` sur l'OPFS : Chrome 86+, Firefox 111+, Safari seulement depuis la 26.0.
- **Invisible pour l'utilisateur** (pas de git, pas de sauvegarde système), **compté dans le quota**, **évincé avec toutes les données du site**.
- Le stockage le plus rapide du navigateur.

### 1.3 IndexedDB

Utile pour les métadonnées et pour **ranger les poignées de dossier**. Mauvais candidat pour l'audio : stockage par objets entiers, plus lent que l'OPFS, même quota, même éviction.

### 1.4 Quotas, persistance, éviction

| Navigateur | Mode « best-effort » (défaut) | Mode persistant (`navigator.storage.persist()`) |
| --- | --- | --- |
| Chrome / Edge | jusqu'à 60 % du disque | identique ; le navigateur entier plafonne à 80 % |
| Firefox | le plus petit de 10 % du disque ou 10 Gio | jusqu'à 50 % du disque, plafond 8 Tio |
| Safari (navigateur) | environ 60 % du disque par origine | non précisé |
| Safari (WebView) | environ 15 % | — |

- Le quota porte sur la taille totale du disque, pas l'espace libre : gérer `QuotaExceededError` partout. `estimate()` donne une estimation, pas un compte.
- `persist()` : Firefox affiche une invite ; Chrome et Safari décident sans invite selon l'engagement de l'utilisateur.
- **Éviction** : sous pression disque, le navigateur supprime en bloc les origines « best-effort » les moins récemment utilisées. **Safari** supprime toute donnée créée par script après 7 jours d'utilisation sans interaction avec l'origine (sauf application ajoutée à l'écran d'accueil ou au Dock).

**Conséquence** : l'OPFS est un bon cache et un repli acceptable, **jamais l'unique copie d'un projet** sans `persist()` accordé et sans export.

## 2. Le projet dans un vrai dossier

Faisable dans Chrome et Edge bureau, la cible prioritaire de Faulk.

- **Ouverture** : `showDirectoryPicker({ id: "faulk-project", mode: "readwrite" })`, poignée rangée dans IndexedDB ; la liste des projets récents est une liste de poignées. Au démarrage, `queryPermission({ mode: "readwrite" })` ; si la réponse est `prompt`, un bouton « Rouvrir le projet » déclenche `requestPermission()`.
- **Format** : le dossier de projet décrit dans [Modèle de projet et format sur disque](02-modele-projet.md). Les textes sont petits et se prêtent à git ; l'audio est dans `blobs/` (Git LFS côté utilisateur).
- **Écriture** : fichier par fichier, seulement les fichiers sales, chacun de façon atomique (§ 5.5).
- **Changements externes** (git checkout, éditeur de texte, autre outil) :
  - Chrome/Edge 133+ : `FileSystemObserver` récursif ; on recharge les fichiers dont le contenu a changé (comparer une empreinte pour ignorer ses propres écritures) ; on invalide le cache d'un blob touché.
  - Repli : relecture de `lastModified` et de la taille au retour du focus (`visibilitychange`) et avant toute lecture.
  - Un `File` obtenu par `getFile()` est un instantané : si le fichier change, la lecture lève `NotReadableError` ; le lecteur reprend `getFile()` et revalide l'empreinte.
  - Modification externe + modifications non enregistrées : décision produit (garder l'une ou l'autre version) ; la commande « recharger le projet » passe par `crates/commands` comme toute action.
- **Firefox et Safari** : pas de dossier réel. OPFS + export/import `.faulkpack` (ou `<input webkitdirectory>` pour importer un dossier en lecture seule, qui marche dans Firefox, mal dans Safari), ou le helper qui accède au vrai disque.

## 3. Stratégie mémoire pour l'audio

### 3.1 Ordres de grandeur

| Format | Octets/s | Par minute | 3 min | 1 h |
| --- | --- | --- | --- | --- |
| Stéréo 44,1 kHz float32 | 352 800 | 21,2 Mo | 63,5 Mo | 1,27 Go |
| Stéréo 48 kHz float32 | 384 000 | 23,0 Mo | 69,1 Mo | 1,38 Go |
| Stéréo 96 kHz float32 | 768 000 | 46,1 Mo | 138 Mo | 2,76 Go |
| Mono 48 kHz float32 | 192 000 | 11,5 Mo | 34,6 Mo | 0,69 Go |
| WAV stéréo 48 kHz 24 bits (disque) | 288 000 | 17,3 Mo | 51,8 Mo | 1,04 Go |
| WAV stéréo 48 kHz 16 bits (disque) | 192 000 | 11,5 Mo | 34,6 Mo | 0,69 Go |
| FLAC stéréo (≈ 55–60 % du WAV) | — | ≈ 7–10 Mo | — | — |

24 pistes stéréo de 4 min à 48 kHz entièrement décodées ≈ **2,2 Go**. Une session de 16 pistes mono d'une heure ≈ **11 Go** : impossible en mémoire. La mémoire WASM 32 bits plafonne à 4 Gio par instance, et un onglet souffre bien avant.

### 3.2 `decodeAudioData` : pratique mais tout en mémoire

- Décode **le fichier entier** en `AudioBuffer` float32 et le **rééchantillonne à la fréquence du contexte** ; le fichier compressé et le résultat coexistent pendant le décodage.
- Ni décodage partiel ni flux : correct pour les clips courts (< 1 min), mauvais au-delà.
- L'`AudioBuffer` vit côté JavaScript ; le moteur WASM en a besoin d'une copie dans sa mémoire linéaire : double occupation si on n'y prend pas garde.

### 3.3 Le streaming disque

Principe : seuls les **tampons de lecture anticipée** (quelques secondes par piste) sont en mémoire ; un thread disque les remplit en avance de la tête de lecture ; le thread audio lit ces tampons sans jamais attendre le disque.

```
 Disque (dossier utilisateur ou OPFS)
        │  lectures par plages (File.slice / SyncAccessHandle.read)
        ▼
 Worker « disque » (dédié)
   • ouvre les blobs, décode si besoin (WAV : direct ; FLAC/MP3/Opus : WebCodecs AudioDecoder ou décodeur WASM)
   • rééchantillonne vers la fréquence du contexte si besoin
   • tient un cache de blocs (LRU) et une lecture anticipée par piste
        │  blocs PCM float32
        ▼
 Anneau(x) lock-free en SharedArrayBuffer  ──►  AudioWorklet (moteur WASM)
   (ou, sans isolation : MessagePort + ArrayBuffer transférés)
```

- **Lecture anticipée** : 2 à 4 s par voix active. 48 pistes stéréo × 4 s ≈ **74 Mo**, quelle que soit la durée du projet.
- **Pré-chargement des débuts** : garder les 1 à 2 premières secondes de chaque clip pour démarrer instantanément sur un saut ou un lancement ; le streaming prend le relais.
- **Taille des lectures** : blocs de 64 à 256 Kio, alignés.
- **Débit** : 48 pistes WAV 24 bits ≈ 14 Mo/s, trivial pour un SSD. Aucune mesure publique fiable du débit OPFS ou File System Access : **à mesurer dans un spike**.
- **Sauts et boucles** : à chaque saut, le Worker vide et re-remplit les tampons ; en attendant, la voix lit le pré-chargement ou se tait, sans jamais bloquer le thread audio. Les boucles courtes restent en mémoire.
- **Enregistrement** : le chemin inverse. Le worklet pousse le PCM capté dans un anneau ; le Worker l'écrit par morceaux. Dans un dossier utilisateur, le fichier d'échange de Chrome ne remplace la cible qu'à `close()` : un plantage pendant la prise perdrait tout. On écrit donc en OPFS pendant la prise, puis on range le blob dans le projet à la fin.

### 3.4 Transport entre le Worker et l'AudioWorklet

Le worklet ne peut pas lire le disque (pas d'API de stockage dans `AudioWorkletGlobalScope`, et pas d'entrée-sortie sur le chemin audio).

| Option | Pour | Contre |
| --- | --- | --- |
| **`SharedArrayBuffer` + anneau lock-free** | Zéro allocation et zéro message dans le thread audio : conforme aux règles temps réel de Faulk. Le moteur WASM peut lire directement dans une mémoire partagée. | Exige l'**isolation cross-origin**. |
| `MessagePort` + `ArrayBuffer` transférés | Aucun en-tête particulier. | Chaque message crée des objets JS dans le thread audio (le `onmessage` du worklet tourne sur le thread de rendu) : allocation et ramasse-miettes dans le chemin audio. Acceptable pour un prototype, pas comme cible. |

**Isolation cross-origin** : `Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: require-corp` (ou `credentialless`).

- `require-corp` : toute ressource tierce doit être servie en CORS ou porter `Cross-Origin-Resource-Policy`. Le plus simple : **auto-héberger polices et bibliothèques** (Vite les empaquette de toute façon).
- `credentialless` : Chrome, Edge et Firefox, **pas Safari**.
- `COOP: same-origin` casse la relation avec les fenêtres d'autres origines (fenêtres d'authentification OAuth) : à prévoir si Faulk ajoute un compte en ligne.
- **Document-Isolation-Policy** (Chrome 137+) isole un document sans COOP/COEP sur tout le site ; Chromium seulement.

La [contrainte](../constraints/01-contraintes.md) rappelle que `SharedArrayBuffer` exige cette isolation. **Décidé le 2026-10-14** : l'isolation cross-origin est acceptée (tâche AU-0 du [découpage du MVP](../specs/02-decoupage-mvp.md), en phase 0) ; le streaming audio passe par des anneaux en `SharedArrayBuffer` (tâche AU-3), et le canal des commandes et des notes vers le worklet vise la même mémoire partagée (contrat CT-3). La voie `MessagePort`, où chaque message crée ses objets sur le thread de rendu, n'est plus qu'un repli à mesurer si la mémoire partagée se révélait impossible entre un Worker et le worklet.

### 3.5 Formats compressés : WebCodecs `AudioDecoder`

- Chrome/Edge 94+, Firefox 130+, Safari 26+, dans un Worker dédié et en contexte sécurisé.
- FLAC, Opus, MP3, AAC, Vorbis, PCM pris en charge dans environ 97 % des sessions mesurées ; vérifier avec `AudioDecoder.isConfigSupported()`.
- WebCodecs ne démultiplexe pas : il faut extraire soi-même les trames (en-têtes FLAC, trames MP3, pages Ogg, boîtes MP4).
- Le saut précis coûte plus cher dans un format compressé (FLAC a des tables de saut ; MP3 à débit variable non). D'où l'idée : **streamer du PCM non compressé**.
- Alternative : décodeur Rust (`symphonia`) compilé en WASM dans le Worker, identique dans le helper, cohérent avec « un seul moteur, deux cibles ».

### 3.6 Aperçus de forme d'onde

- Calculés une fois à l'import (dans le Worker) : min/max par bloc de 256 échantillons, plus des niveaux réduits (×4, ×16…) pour le zoom.
- Environ **45 Ko par minute** (int8) : se charge entièrement en mémoire.
- Cache régénérable dans `.faulk/cache/` (ou l'OPFS), exclu de git.

## 4. Ce que change le mode Boost

- Le helper lit et écrit **directement sur le disque** : pas de permission navigateur, pas de quota, streaming natif. Le son ne traverse jamais le lien page ↔ helper : seul le moteur du helper lit les fichiers audio.
- **La page et le helper doivent désigner le même dossier**, or un `FileSystemDirectoryHandle` ne révèle pas son chemin absolu. Options :
  1. l'utilisateur ouvre le projet par un sélecteur natif du helper (ou en collant un chemin) ; la page reçoit les données du projet par le WebSocket. Simple et fiable ;
  2. **rapprochement par identité** : `project.json` porte l'`id` stable du projet (ULID obligatoire, défini dans le [modèle de projet](02-modele-projet.md) § 1.3) ; la page l'envoie au helper, qui le cherche parmi ses projets récents. Deux dossiers de même `id` (une copie faite hors de Faulk) ne sont jamais choisis au hasard : le helper demande lequel ouvrir. Utile pour basculer à chaud.
- **Identité des fichiers audio** : l'empreinte SHA-256 du blob. La page (poignées) et le helper (chemins natifs) résolvent le même blob par `blobs/<sha256>`, chemins relatifs à la racine et en `/`.
- **Écritures concurrentes** : en mode Boost, une seule autorité écrit le dossier, le helper ; la page reste propriétaire du modèle (décision du 2026-10-14, [modèle de projet](02-modele-projet.md) § 5.4) et lui envoie les fichiers sales à écrire. C'est l'implémentation « helper » de `ProjectStore` (tâche AU-7 du [découpage du MVP](../specs/02-decoupage-mvp.md)) : seuls les fichiers texte du projet passent par le WebSocket ; ni le son, ni les blobs, ni le cache PCM ne traversent le lien (règle de `CLAUDE.md`), l'import (AU-1) et le streaming (AU-4) s'exécutant dans le helper en Boost.
- **Projets en OPFS** : le helper n'y a pas accès. Passer en Boost depuis un projet OPFS passe par un export `.faulkpack` que le helper ouvre comme dossier (tâche SV-3) : aucun fichier audio ne traverse le WebSocket.

## 5. Proposition pour le MVP

### 5.1 Emplacement

| Navigateur | Stockage du projet |
| --- | --- |
| Chrome / Edge bureau | **Dossier réel** choisi par l'utilisateur (défaut, décision du 2026-10-14), poignée en IndexedDB, permission « à chaque visite » encouragée. Caches en OPFS ou dans `.faulk/cache/`. Tâche SV-2. |
| Firefox / Safari, ou dossier refusé | **OPFS** + `navigator.storage.persist()` + export/import `.faulkpack` (archive zip du dossier). Avertissement clair : « vos projets sont dans le navigateur ; exportez-les ». Tâche SV-3. |
| Mode Boost (tous navigateurs) | Dossier réel, ouvert et écrit par le helper pour le compte de la page. Tâche AU-7. |

Une **seule abstraction de stockage** dans le Worker disque (`ProjectStore` : lister, lire une plage, écrire atomiquement, observer), avec trois implémentations : dossier File System Access, OPFS, helper. Le format du dossier est identique dans les trois cas : un projet OPFS exporté est un dossier de projet ordinaire.

### 5.2 Ce qui va dans `.faulk/`

Le dossier `.faulk/` (ignoré par git) complète l'arborescence du [modèle de projet](02-modele-projet.md) :

```
.faulk/
  cache/       # binary snapshot of the whole project, waveform peaks, decoded PCM: regenerable
  tmp/         # temp files for atomic writes, takes being recorded
  session.json # session state: solo, arming and monitoring per track, chosen MIDI input (restored on reopen, never versioned)
  view.json    # view state (zoom, scroll, selection)
  lock         # open lock (session, timestamp)
```

### 5.3 Format des fichiers audio

Les études divergent ; la proposition à trancher :

- **Le blob garde les octets d'origine** de l'import, pour que l'empreinte corresponde au fichier de l'utilisateur et que la déduplication fonctionne ([modèle de projet](02-modele-projet.md) § 3).
- **Fichiers compressés (MP3, AAC, Opus, Ogg, FLAC)** : pour streamer sans coût de décodage dans le chemin de lecture, deux options. (a) Décoder une fois à l'import vers un PCM rangé comme *second blob* référencé par le clip (double espace, mais versionné et identique sur toutes les machines). (b) Décoder vers un PCM dans `.faulk/cache/`, régénérable (pas de double espace dans git, mais un coût au premier chargement sur une nouvelle machine). **Décision à prendre.**
- **Rééchantillonnage** : **à la lecture** (tâche AU-4 du [découpage du MVP](../specs/02-decoupage-mvp.md)), dans le lecteur de clips du moteur, vers la fréquence du moteur (contexte audio en Web, périphérique en Boost, qui peut changer en cours de session) ; jamais à l'import, parce qu'une fréquence de projet fixe ne supprimerait pas ce besoin.

### 5.4 En mémoire et streamé

| En mémoire | Streamé depuis le disque |
| --- | --- |
| Le modèle du projet (structures Rust compactes), la pile d'annulation | Les clips longs (seuil proposé : > 30 s ou > 10 Mo décodés) |
| Les clips courts et échantillons (< 30 s), décodés une fois | Les enregistrements en cours (écriture) |
| Les 1 à 2 premières secondes de chaque clip long | |
| Les tampons de lecture anticipée (2 à 4 s par voix active) | |
| Les aperçus de forme d'onde (≈ 45 Ko/min) | |
| Les modules (WASM compilé) | |

Budget visé (24 pistes, 5 min) : moins de 300 Mo de mémoire audio, quelle que soit la durée totale des fichiers.

Le seuil par clip ne suffit pas à tenir ce budget : beaucoup de clips courts peuvent le dépasser à eux seuls. Le PCM décodé a donc un **budget global** (proposition : 300 Mo, réglable), tenu par le Worker disque :

- les clips courts, les débuts préchargés et les tampons de lecture anticipée puisent dans ce même budget ;
- au-delà, le Worker **évince** en priorité le PCM des clips qui ne jouent pas et ne sont pas proches de la tête de lecture (LRU), puis le préchargement des clips inactifs ; un clip évincé repasse en streaming ou est redécodé à la demande ;
- les tampons des voix en lecture ne sont jamais évincés ; si eux seuls dépassent le budget, l'interface l'affiche plutôt que de couper le son ;
- le compteur d'occupation est visible dans l'état du moteur et mesuré par le banc de charge (QA-7).

**Étape intermédiaire acceptable** : livrer d'abord « tout décodé en mémoire » avec une limite affichée (par exemple 1 Go de PCM par projet), puis le streaming. L'architecture (Worker disque + anneau vers le worklet) doit cependant être posée dès le départ, sinon la bascule touchera le moteur.

### 5.5 Écritures atomiques

- **Fichiers texte du projet** : seuls les fichiers sales sont écrits, après un court délai de regroupement (environ 1 s après la dernière commande, puisque toute action est une commande). Chacun est écrit dans `.faulk/tmp/` via `createWritable()` + `close()`, puis déplacé vers sa place par `move()` (disponible dans Chrome pour les fichiers locaux et dans l'OPFS ; **à valider dans le spike**, sinon écriture directe par `createWritable`, qui remplace déjà la cible à la fermeture).
- **Cohérence entre fichiers** : un enregistrement touche parfois plusieurs fichiers. L'instantané de cache n'est réécrit qu'après tous les fichiers ; à l'ouverture, la relecture des fichiers modifiés depuis l'instantané rattrape un enregistrement interrompu.
- `createWritable` ne modifie la cible qu'à `close()` : un onglet fermé pendant l'écriture laisse l'ancien fichier intact (le `.crswap` peut rester : nettoyé au démarrage suivant).
- **Enregistrement audio** : par morceaux dans l'OPFS (poignée synchrone, `flush()` toutes les quelques secondes, en-tête WAV corrigé à la fin), puis rangement dans `blobs/` à la fin de la prise. Au redémarrage, une prise orpheline est proposée en récupération.
- `beforeunload` : prévenir si une écriture est en cours, sans compter dessus.

### 5.6 Modes de défaillance

| Défaillance | Détection | Réponse |
| --- | --- | --- |
| Permission perdue (nouvelle session, onglet longtemps en arrière-plan, révocation) | `queryPermission()` ≠ `granted`, ou `NotAllowedError` | Pause du transport sur les pistes concernées, bandeau « Rétablir l'accès au dossier » (geste → `requestPermission()`) ; les modifications restent en mémoire et dans un journal OPFS en attendant. |
| Dossier déplacé ou renommé | `NotFoundError` sur la poignée | Proposer de rouvrir le dossier ; vérifier l'identifiant de projet dans `project.json`. |
| Blob absent ou modifié hors de Faulk | `NotFoundError`, `NotReadableError`, empreinte différente, `FileSystemObserver` | Clip « hors ligne » (silence, jamais de plantage), recherche par empreinte, ou demande à l'utilisateur. |
| Modification externe des fichiers texte (git checkout) | `FileSystemObserver` ou comparaison au retour du focus | Recharger si pas de modification locale ; sinon demander. |
| Quota dépassé (OPFS) | `QuotaExceededError` | Arrêter proprement l'écriture, garder la partie écrite, avertir avec `estimate()`, proposer l'export ou le nettoyage des caches. |
| Disque plein (dossier réel) | Erreur d'écriture | Même réponse ; l'enregistrement s'arrête sans corrompre la prise. |
| Éviction de l'OPFS (Firefox/Safari ; Safari 7 jours) | Projet absent au démarrage | Prévenir en amont : `persist()`, rappel d'export, « dernier export » affiché. Les caches évincés se régénèrent. |
| Onglet fermé pendant une écriture | Fichier d'échange orphelin, `.faulk/tmp/` non vide | Ancien fichier intact ; nettoyage au démarrage ; récupération des prises orphelines. |
| Deux onglets sur le même projet | Verrou `.faulk/lock` + Web Locks API ou `BroadcastChannel` | Deuxième onglet en lecture seule. |
| Lecture disque trop lente (sous-alimentation de l'anneau) | Compteur côté worklet | Silence sur la voix concernée, jamais d'attente ; compteur affiché ; lecture anticipée augmentée. |

### 5.7 Limites acceptées pour le MVP

- Dossier réel : Chrome et Edge bureau uniquement. Firefox et Safari : OPFS + export, sans dossier visible, sans git, sans surveillance des changements.
- Pas de mobile.
- Formats d'import : WAV et AIFF directs ; compressés selon la décision du § 5.3.
- Pas d'écriture concurrente page + helper : une seule autorité d'écriture.

### 5.8 Brouillons et projets du navigateur (SV-3, #96)

Ce qui est écrit dans `web/src/lib/storage/` (`opfs.ts`, `storage.svelte.ts`) et `crates/project-wasm/src/pack.rs`. Le code fait foi.

- **Où** : `faulk/browser/<uuid>/` dans l'OPFS du site, un répertoire par projet, au format de dossier ordinaire (le nom du répertoire n'est pas l'identité du projet : elle est dans `project.json`). Un répertoire de l'OPFS est un `FileSystemDirectoryHandle` : le même `ProjectStore` (Worker, mêmes écritures atomiques) l'écrit, il n'y a pas de seconde implémentation. Sans `createWritable` (Safari avant 26) l'OPFS n'est pas utilisé et le projet reste dans l'onglet, avec l'avertissement avant de partir.
- **Brouillon** : tant qu'aucun dossier n'est choisi, un projet dont le *projet* a changé (pas seulement la sélection ou un solo : le cœur distingue `project_dirty`) est écrit dans l'OPFS comme il le serait dans un dossier (1,5 s après la dernière modification). À la réouverture de la page, le plus récent des brouillons non ouverts ailleurs est proposé (bandeau « Rouvrir le brouillon », « Supprimer le brouillon »), sauf si le dernier dossier ouvert est plus récent. « Enregistrer sous » vers un dossier vide reprend le même projet (même identité, blobs copiés, verrou de l'identité repris) et supprime le brouillon une fois le dossier complet et retenu parmi les projets récents, avant que la page affiche « enregistré » (un rechargement après « enregistré » ne trouve donc pas le projet en double ; un rechargement pendant la suppression elle-même le peut encore, car supprimer avant de retenir le dossier risquerait de ne plus rien proposer ; si le dossier n'est pas confirmé parmi les récents, ou si la suppression échoue, le brouillon reste) ; « Enregistrer » et le dossier proposé à l'export attendent la fin du déplacement (copie, écriture, rattachement) ; un échec laisse le brouillon et ses verrous. Un brouillon n'est supprimé que par « Enregistrer sous » ou par un clic de l'utilisateur, après confirmation ; « Nouveau » ou « Ouvrir » le gardent (le menu les liste).
- **Sans dossier à choisir** (Firefox, Safari) : l'OPFS est le stockage normal. Le dernier projet rouvre seul ; « Enregistrer » le garde dans le navigateur ; `navigator.storage.persist()` est demandé à la première écriture ; l'avertissement « tes projets sont dans le navigateur ; exporte-les » est un bandeau (plus net si `persist()` a été refusé) et dans le menu.
- **Verrous (Web Locks)** : un par identité de projet (un seul onglet écrit un projet) et un par répertoire (`dir.<uuid>`), pris *avant* de créer le répertoire et tenus tant que l'onglet l'utilise. Au démarrage, un répertoire sans `project.json` dont personne ne tient le verrou (un onglet fermé en pleine première écriture ou en plein import) est supprimé ; un répertoire en cours de création dans un autre onglet est laissé.
- **Quota plein** (`QuotaExceededError`) : bandeau propre (exporter ou libérer de la place, réessayer), modifications gardées dans l'onglet, nouvel essai toutes les 10 s. Chaque fichier est remplacé en entier ou pas du tout ; un enregistrement interrompu entre deux fichiers laisse un dossier qui s'ouvre, car le fichier d'un clip n'est écrit qu'après ses notes (tests : un enregistrement qui ajoute, puis un qui renomme et supprime une piste, coupés à chaque fichier). Ce dossier peut mêler deux versions du projet (les pistes et clips sont trouvés en listant les répertoires, `project.json` vient en dernier, les suppressions après) : l'onglet, tant qu'il est ouvert, garde la version complète en mémoire et réessaie.
- **`.faulkpack`** : l'export reprend le projet tel qu'il est (modifications non écrites comprises) et les blobs du stockage, passe par `export_pack` (déterministe), et le rend en téléchargement ; l'import passe par `import_pack` (strict : seuls les chemins du format, longueurs comptées à la production, empreinte de chaque blob), est écrit en entier dans un nouveau répertoire (`project.json` en dernier) puis ouvert comme un projet du navigateur, avec l'identité du pack : un projet de même identité déjà ouvert ou gardé dans le navigateur n'est jamais remplacé. Limites de la page : l'import lit un fichier de 192 Mio au plus (le cœur tient le pack et ses fichiers en mémoire, en plusieurs copies) ; l'export refuse un projet de plus de 180 Mio, pour ne pas produire une sauvegarde que la page ne relirait pas. Au-delà, il faudra un traitement en flux (avec les pistes audio, AU-1).
- **Commandes des actions de stockage (#323)** : chaque bouton du bandeau et du menu envoie une commande, il n'a plus de chemin privé. `project.list_stored` (requête : la page relit ses listes, les met dans le contexte du cœur, `Context::stored`, et la commande répond avec les dossiers récents `{ id, name }` et les projets du navigateur `{ dir, name }`), `project.reopen_last` (bandeau « Rouvrir »), `project.open_folder`, `project.open_recent { id }`, `project.open_browser_project { dir }`, `project.delete_browser_project { dir }`, `project.keep_mine` et `project.load_folder_version` (conflit de SV-2), `project.allow_access` (« Autoriser »). Toutes sont `live`, non annulables ; le contrat réserve `x-destructive` aux éditions (`Meta::check`), donc **aucune n'est marquée destructive** et AG-4 (#100) ne les voit pas : c'est la page qui demande, **quel que soit l'appelant** (supprimer un projet du navigateur, remplacer les fichiers du dossier par le projet, remplacer le projet par le dossier ; ouvrir un autre projet demande déjà quand des modifications seraient perdues). Le résultat de la commande dit seulement qu'elle est acceptée, pas ce que l'utilisateur a répondu (le canal de retour des agents est celui d'AG-1/AG-4). Gardes de `ProjectStorage` : un nom (`dir`, `id`) qui n'est pas dans la liste que la page tient est refusé par un avis, jamais passé tel quel à l'OPFS ; `keep_mine` et `load_folder_version` ne font rien (avis) hors conflit, et revérifient le conflit après la réponse ; le sélecteur de dossier et l'invite d'autorisation ne s'ouvrent que depuis un clic (`navigator.userActivation`), sinon un avis le dit ; l'invite n'est demandée que si l'autorisation est perdue. Une question ouverte garde la file du stockage : les écritures du projet attendent la réponse (rien n'est perdu, elles partent après).
- **Pas vérifié sous Firefox et Safari** (aucun moteur de ces navigateurs dans les tests) : `move()` sur l'OPFS, la forme de `QuotaExceededError`, l'invite de `persist()`.

## 6. Décisions du fondateur (2026-10-14) et questions restantes

Décidé :

1. **Isolation cross-origin** (COOP/COEP, ou Document-Isolation-Policy dans Chrome) : acceptée, mise en place par la tâche AU-0 dès la phase 0, avec ressources auto-hébergées et prudence avec les fenêtres d'authentification d'autres origines.
4. **Firefox et Safari** : l'OPFS + export en archive suffit pour le MVP ; le helper reste le moyen d'avoir un dossier réel partout.

Choix techniques, tranchés dans les tâches du [découpage du MVP](../specs/02-decoupage-mvp.md) :

2. Fréquence d'échantillonnage : rééchantillonnage **à la lecture**, dans le moteur (§ 5.3, tâche AU-4) ; pas de fréquence de projet fixe.
3. PCM décodé d'un fichier compressé : second blob versionné, ou cache régénérable ? Proposition retenue par défaut : cache régénérable dans `.faulk/cache/`, le blob gardant les octets d'origine ; la PR d'AU-1 présente ce choix et son coût au fondateur.
5. En mode Boost, qui ouvre le projet : le sélecteur natif du helper ou le rapprochement par identifiant ? Tâche AU-7.
6. Débit réel de lecture par plages (File System Access et OPFS) et latence de démarrage d'une voix : **à mesurer**, tâches AU-3 et AU-8.
7. `FileSystemHandle.move()` sur un dossier choisi par l'utilisateur : comportement à vérifier dans Chrome stable, tâche SV-2.

## Sources

- MDN, Storage quotas and eviction criteria : https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria
- Chrome, Persistent permissions for the File System Access API : https://developer.chrome.com/blog/persistent-permissions-for-the-file-system-access-api
- web.dev, Open a directory : https://web.dev/articles/files/open-a-directory
- caniuse, File System Access API : https://caniuse.com/native-filesystem-api
- caniuse, `createWritable` : https://caniuse.com/mdn-api_filesystemfilehandle_createwritable
- caniuse, `FileSystemSyncAccessHandle` : https://caniuse.com/mdn-api_filesystemsyncaccesshandle
- caniuse, `FileSystemObserver` : https://caniuse.com/mdn-api_filesystemobserver
- caniuse, `AudioDecoder` : https://caniuse.com/mdn-api_audiodecoder
- MDN, `FileSystemSyncAccessHandle` : https://developer.mozilla.org/en-US/docs/Web/API/FileSystemSyncAccessHandle
- MDN, `createWritable()` : https://developer.mozilla.org/en-US/docs/Web/API/FileSystemFileHandle/createWritable
- MDN, `FileSystemObserver` : https://developer.mozilla.org/en-US/docs/Web/API/FileSystemObserver
- Chrome, File System Observer origin trial : https://developer.chrome.com/blog/file-system-observer
- blink-dev, Intent to Ship FileSystemObserver (Chrome 133, bureau) : https://groups.google.com/a/chromium.org/g/blink-dev/c/6oOaFmia2dc/m/gx0KgpQqBQAJ
- Position de Mozilla sur File System Access : https://github.com/mozilla/standards-positions/issues/154
- WebKit, bug File System : https://bugs.webkit.org/show_bug.cgi?id=231706
- MDN, Cross-Origin-Embedder-Policy : https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Cross-Origin-Embedder-Policy
- Chrome, Document Isolation Policy (Chrome 137) : https://developer.chrome.com/blog/document-isolation-policy
- web.dev, COOP et COEP : https://web.dev/articles/coop-coep
- StackBlitz, COEP credentialless : https://blog.stackblitz.com/posts/bringing-webcontainers-to-all-browsers/
- MDN, `AudioDecoder` : https://developer.mozilla.org/en-US/docs/Web/API/AudioDecoder
- WebCodecs Fundamentals, Codec Support Table : https://webcodecsfundamentals.org/datasets/codec-support-table/

Points non sourcés, à vérifier dans le spike : dossiers interdits par Chrome, fichier d'échange `.crswap`, `NotReadableError` sur un `File` modifié, comportement de `move()` sur un dossier utilisateur, plafond de 4 Gio de la mémoire WASM 32 bits, rééchantillonnage de `decodeAudioData` à la fréquence du contexte.
