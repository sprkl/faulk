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
- le compteur d'occupation est visible dans l'état du moteur (`Feedback.pcm_kib`, AU-3) ; sa mesure revient à AU-3 (§ 5.11), QA-7 n'ayant pas pu la faire avant que le compteur existe.

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
- Formats d'import : WAV et AIFF directs ; compressés selon la décision du § 5.3 ; la liste décidée par AU-1 est au § 5.9 (Opus pas encore).
- Pas d'écriture concurrente page + helper : une seule autorité d'écriture.

### 5.8 Brouillons et projets du navigateur (SV-3, #96)

Ce qui est écrit dans `web/src/lib/storage/` (`opfs.ts`, `storage.svelte.ts`) et `crates/project-wasm/src/pack.rs`. Le code fait foi.

- **Où** : `faulk/browser/<uuid>/` dans l'OPFS du site, un répertoire par projet, au format de dossier ordinaire (le nom du répertoire n'est pas l'identité du projet : elle est dans `project.json`). Un répertoire de l'OPFS est un `FileSystemDirectoryHandle` : le même `ProjectStore` (Worker, mêmes écritures atomiques) l'écrit, il n'y a pas de seconde implémentation. Sans `createWritable` (Safari avant 26) l'OPFS n'est pas utilisé et le projet reste dans l'onglet, avec l'avertissement avant de partir.
- **Brouillon** : tant qu'aucun dossier n'est choisi, un projet dont le *projet* a changé (pas seulement la sélection ou un solo : le cœur distingue `project_dirty`) est écrit dans l'OPFS comme il le serait dans un dossier (1,5 s après la dernière modification). À la réouverture de la page, le plus récent des brouillons non ouverts ailleurs se rouvre de lui-même (#474 : le stockage du navigateur ne demande aucune permission, donc aucun bandeau ni clic), sauf si le dernier dossier ouvert est plus récent ; seul un dossier dont le navigateur réclame la permission garde un bandeau, avec un seul bouton principal « Rouvrir « nom » ». « Enregistrer sous » demande d’abord le nom du projet (champ prérempli, #627), puis le dossier : le projet est écrit dans un sous-dossier de ce nom (nettoyé pour le système de fichiers) créé dans le dossier choisi, ou déjà là et vide, un sous-dossier qui contient des fichiers est refusé, et le projet prend ce nom par `project.rename` ; vers un dossier vide, il reprend le même projet (même identité, blobs copiés, verrou de l'identité repris) et supprime le brouillon une fois le dossier complet et retenu parmi les projets récents, avant que la page affiche « enregistré » (un rechargement après « enregistré » ne trouve donc pas le projet en double ; un rechargement pendant la suppression elle-même le peut encore, car supprimer avant de retenir le dossier risquerait de ne plus rien proposer ; si le dossier n'est pas confirmé parmi les récents, ou si la suppression échoue, le brouillon reste) ; « Enregistrer » et le dossier proposé à l'export attendent la fin du déplacement (copie, écriture, rattachement) ; un échec laisse le brouillon et ses verrous. Un brouillon n'est supprimé que par « Enregistrer sous » ou par un clic de l'utilisateur, après confirmation ; « Nouveau » ou « Ouvrir » le gardent (le menu les liste).
- **Sans dossier à choisir** (Firefox, Safari) : l'OPFS est le stockage normal. Le dernier projet rouvre seul ; « Enregistrer » le garde dans le navigateur ; `navigator.storage.persist()` est demandé à la première écriture ; l'avertissement « tes projets sont dans le navigateur ; exporte-les » est un bandeau (plus net si `persist()` a été refusé) et dans le menu.
- **Verrous (Web Locks)** : un par identité de projet (un seul onglet écrit un projet) et un par répertoire (`dir.<uuid>`), pris *avant* de créer le répertoire et tenus tant que l'onglet l'utilise. Au démarrage, un répertoire sans `project.json` dont personne ne tient le verrou (un onglet fermé en pleine première écriture ou en plein import) est supprimé ; un répertoire en cours de création dans un autre onglet est laissé.
- **Quota plein** (`QuotaExceededError`) : bandeau propre (exporter ou libérer de la place, réessayer), modifications gardées dans l'onglet, nouvel essai toutes les 10 s. Chaque fichier est remplacé en entier ou pas du tout ; un enregistrement interrompu entre deux fichiers laisse un dossier qui s'ouvre, car le fichier d'un clip n'est écrit qu'après ses notes (tests : un enregistrement qui ajoute, puis un qui renomme et supprime une piste, coupés à chaque fichier). Ce dossier peut mêler deux versions du projet (les pistes et clips sont trouvés en listant les répertoires, `project.json` vient en dernier, les suppressions après) : l'onglet, tant qu'il est ouvert, garde la version complète en mémoire et réessaie.
- **`.faulkpack`** : l'export reprend le projet tel qu'il est (modifications non écrites comprises) et les blobs du stockage, passe par `export_pack` (déterministe), et le rend en téléchargement ; l'import passe par `import_pack` (strict : seuls les chemins du format, longueurs comptées à la production, empreinte de chaque blob), est écrit en entier dans un nouveau répertoire (`project.json` en dernier) puis ouvert comme un projet du navigateur, avec l'identité du pack : un projet de même identité déjà ouvert ou gardé dans le navigateur n'est jamais remplacé. Limites de la page : l'import lit un fichier de 192 Mio au plus (le cœur tient le pack et ses fichiers en mémoire, en plusieurs copies) ; l'export refuse un projet de plus de 180 Mio, pour ne pas produire une sauvegarde que la page ne relirait pas. Au-delà, il faudra un traitement en flux (avec les pistes audio, AU-1).
- **Commandes des actions de stockage (#323)** : chaque bouton du bandeau et du menu envoie une commande, il n'a plus de chemin privé. `project.list_stored` (requête : la page relit ses listes, les met dans le contexte du cœur, `Context::stored`, et la commande répond avec les dossiers récents `{ id, name }` et les projets du navigateur `{ dir, name }`), `project.reopen_last` (bandeau « Rouvrir »), `project.open_folder`, `project.open_recent { id }`, `project.open_browser_project { dir }`, `project.delete_browser_project { dir }`, `project.keep_mine` et `project.load_folder_version` (conflit de SV-2), `project.allow_access` (« Autoriser »). Toutes sont `live`, non annulables ; le contrat réserve `x-destructive` aux éditions (`Meta::check`), donc **aucune n'est marquée destructive** et AG-4 (#100) ne les voit pas : c'est la page qui demande, **quel que soit l'appelant** (supprimer un projet du navigateur, remplacer les fichiers du dossier par le projet, remplacer le projet par le dossier ; ouvrir un autre projet demande déjà quand des modifications seraient perdues). Le résultat de la commande dit seulement qu'elle est acceptée, pas ce que l'utilisateur a répondu (le canal de retour des agents est celui d'AG-1/AG-4). Gardes de `ProjectStorage` : un nom (`dir`, `id`) qui n'est pas dans la liste que la page tient est refusé par un avis, jamais passé tel quel à l'OPFS ; `keep_mine` et `load_folder_version` ne font rien (avis) hors conflit, et revérifient le conflit après la réponse ; le sélecteur de dossier et l'invite d'autorisation ne s'ouvrent que depuis un clic (`navigator.userActivation`), sinon un avis le dit ; l'invite n'est demandée que si l'autorisation est perdue. Une question ouverte garde la file du stockage : les écritures du projet attendent la réponse (rien n'est perdu, elles partent après).
- **Ouvrir un `.faulkpack` par son lien (#497)** : `project.open_url { url }` (commande « live », sans annulation : ouvrir n'est pas une édition ; on revient au projet d'avant par la liste des projets). Le cœur vérifie la forme du lien (`https`, ou `http` vers `localhost`, `127.0.0.1` ou `[::1]` seulement, sans identifiants, 2 048 caractères au plus) ; la page le relit avec `new URL` et refuse aussi `http` quand elle n'est pas elle-même servie depuis cet ordinateur (`web/src/lib/storage/open-url.ts`). Le téléchargement se fait sans cookies ni identifiants (`credentials: 'omit'`, `referrerPolicy: 'no-referrer'`), en CORS, hors de la file du stockage et un à la fois ; la taille est refusée avant lecture sur `Content-Length` puis pendant la lecture (le flux est annulé dès `MAX_PACK_BYTES` dépassé) ; une redirection vers un lien que la page ne prendrait pas, un hôte muet 30 s, une erreur réseau, CORS ou HTTP donnent « Impossible de télécharger ce projet depuis <hôte>. » avec « Importer un fichier… ». Les octets suivent ensuite le chemin d'un import de fichier (`import_pack` strict, nouveau répertoire, ouverture), sauf qu'un pack dont le navigateur garde déjà le projet rouvre cette copie au lieu d'être refusé. Le lien `/app?open=<url>` est lu au montage du shell et retiré aussitôt de l'adresse ; la page demande toujours avant de télécharger, dans une boîte qui nomme l'hôte et montre le lien entier, puis envoie la commande. Aucun contenu du pack n'est exécuté.
- **Pas vérifié sous Firefox et Safari** (aucun moteur de ces navigateurs dans les tests) : `move()` sur l'OPFS, la forme de `QuotaExceededError`, l'invite de `persist()`.

### 5.9 Import d'un fichier audio (AU-1, #110)

Ce qui est écrit dans `crates/audio-import`, `crates/audio-import-wasm` et `web/src/lib/audio/import/`. Le code fait foi.

- **Où.** Le pipeline est écrit une fois en Rust (`crates/audio-import` : SHA-256 en flux, décodage par `symphonia` 0.6 sans aucune fonction SIMD, pour que les deux cibles exécutent le même code scalaire, métadonnées, disposition des fichiers lus directement, en-tête du cache PCM). Dans la page, il tourne dans le Worker du magasin de projet (`storage/worker.ts`), le seul endroit qui écrit le dossier : un module WebAssembly à part (`audio-import-wasm`, ABI C `ai_*`, 1,5 Mo, chargé au premier import seulement), qui lit le fichier par plages par une seule importation synchrone (`env.ai_read`, servie par `FileReaderSync`) et décode paquet par paquet, de sorte que le Worker reprend la main entre deux étapes (écritures asynchrones, autres appels, annulation). Le Worker envoie sa progression (`{ id, progress }`), et chaque message de progression repousse le délai de 10 minutes du magasin : un import d'une heure n'est pas pris pour un Worker muet. Sans Worker (le magasin dans la page), l'import est refusé (`unavailable`).
- **Étapes.** (1) Hacher le fichier en le copiant dans `.faulk/tmp/import-<id>` ; (2) le décoder entièrement (un fichier abîmé est refusé à l'import, pas à la lecture), en écrivant le cache PCM dans un autre fichier temporaire s'il n'est pas lu directement ; (3) seulement alors, déplacer la copie dans `blobs/<sha256>` (ou la jeter si le dossier a déjà ce blob : un seul blob par fichier, quel que soit le nombre d'imports) et le cache dans `.faulk/cache/pcm/`. Un échec à n'importe quelle étape supprime les fichiers temporaires et ne range rien. Là où `move` est refusé, le fichier est écrit en place par `createWritable` (atomique à la fermeture) et supprimé si l'écriture échoue : un blob à moitié écrit ne reste jamais sous son empreinte. Deux imports du même fichier dans un onglet se suivent pour le rangement ; le nettoyage d'un rattachement (`cleanUp`) laisse les fichiers temporaires d'un import en cours (verrou Web Lock `faulk.import.<id>`). Le contrôleur range l'import dans sa file : un enregistrement, un « Enregistrer sous » ou une ouverture n'a jamais lieu au milieu d'un import (l'autosauvegarde attend la fin) ; il ajoute l'estampille du blob à celles de ses propres écritures, si bien que la surveillance du dossier ne prend pas l'import pour une modification extérieure. Un projet sans place reçoit d'abord son brouillon dans l'OPFS.
- **Formats** (décidé par AU-1) : WAV (PCM 8 à 32 bits, flottant 32 et 64, A-law, µ-law, ADPCM IMA et Microsoft), AIFF et AIFF-C, CAF, FLAC, MP3 (et MP1, MP2), AAC-LC en MP4, M4A ou ADTS, ALAC, Ogg Vorbis : WAV et tout ce que `decodeAudioData` de Chromium décode, **sauf Opus** (Ogg et WebM), que `symphonia` 0.6 ne décode pas : un fichier Opus est refusé par son nom (`unsupported_codec`), en attendant un décodeur (#609). Les retards d'encodeur sont retirés partout : étiquette LAME du MP3, positions des pages Ogg, liste d'éditions MP4 de l'AAC (lue par Faulk, `symphonia` ne l'applique pas ; la longueur gardée y est donnée dans l'échelle de temps du film, souvent 1 000 par seconde, donc à une demi-milliseconde près) ; les frames d'un fichier sont donc celles de sa musique, sans amorce ni remplissage. Limites : celles du modèle (8 à 768 kHz, 1 à 64 canaux, moins de 2^40 frames) ; aucune taille maximale propre au pipeline, qui travaille en flux (le disque ou le quota décident).
- **Lu directement ou mis en cache** (décision 3 du § 6, proposition par défaut retenue, présentée au fondateur par la PR) : un WAV ou un AIFF d'échantillons entiers de 16, 24 ou 32 bits ou flottants de 32 bits n'a **pas de cache** : AU-3 lira ses échantillons dans le blob, par plages, avec sa disposition (`PcmLayout` : position des données, codage, canaux, frames ; la conversion en `f32` est celle de `symphonia`, testée échantillon par échantillon). Tout autre fichier a un **cache PCM** `.faulk/cache/pcm/<sha256>.pcm` : en-tête de 32 octets (`FKPCM001`, fréquence, canaux, frames, version du décodeur), puis les frames en `f32` entrelacés, petit-boutiste. Il est nommé par le blob (partagé par tous les clips du même fichier), jamais versionné, valide seulement si son en-tête et sa taille sont exactement ceux du média et de la version du décodeur ; sinon il est refait. Coût : un décodage par machine (un MP3 de 3 minutes, environ 64 Mo de cache en stéréo 44,1 kHz) contre un second blob versionné qu'il faudrait copier partout.
- **Le cache absent se régénère à l'ouverture**, là où est le disque : dans la page, dans les deux modes. Après chaque rattachement d'un projet (ouverture, réouverture d'un brouillon, import d'un `.faulkpack`, « Enregistrer sous », qui copie `blobs/` mais pas `.faulk/`), le contrôleur demande en arrière-plan au Worker les caches qui manquent pour les médias des clips du projet (`ensureAudioCache`) ; un blob absent est compté (le clip sera « média absent », AU-5), un blob qui ne se décode plus comme le projet le dit est signalé et n'a pas de cache.
- **Commandes.** `clip.choose_audio { track }` (live) ouvre la fenêtre de fichiers de la page, seulement sur un clic (sinon un avis) ; `clip.cancel_audio_import` (live) arrête l'import en cours (le fichier en cours n'est pas gardé, les suivants ne sont pas importés) ; chaque fichier choisi passe par le pipeline, ses métadonnées sont mises dans le contexte du cœur (`pw_stage_media`, `Context::media`), puis `clip.import_audio { track, media }` (édition annulable) crée le clip dans la réserve de la piste audio : le fichier entier, 0 dB, sans boucle, nommé d'après le fichier. L'annulation est `clip.delete`, désormais valable pour un clip audio, dont l'inverse est `clip.restore_audio` ; le blob reste (nettoyage sur demande seulement). Un agent peut faire un clip d'un fichier que le projet utilise déjà, par son empreinte. Depuis AU-5 (#114), l'import peut aussi poser le clip dans une case ou dans l'arrangement, et un clip dont le blob manquait à l'ouverture est marqué « média absent » (`media_missing` de `clip.get`), jusqu'à ce que la page remette le fichier.
- **Parité page/helper.** Le helper exécutera le même crate en natif (AU-7). Pour le même blob, le PCM natif et le PCM WebAssembly sont identiques au bit près pour le WAV, l'AIFF, le FLAC, l'ALAC et le Vorbis ; pour le MP3 et l'AAC, qui passent par `powf` et `cos`, dont le dernier bit diffère entre la bibliothèque C native et celle de Rust en WebAssembly, ils diffèrent de 3,3·10⁻⁷ au plus (−130 dBFS), sous la tolérance de parité du banc de rendu (10⁻⁵).
- **Boost.** Depuis BO-1 le helper ne tient aucun projet (`disk: false`) : la page écrit le dossier dans les deux modes, donc l'import tourne dans la page en Boost aussi, et rien du fichier ne traverse le WebSocket (mesuré : un import d'une heure en Boost envoie 32 octets au helper, aucun échantillon). Ce que le helper lira pour jouer les clips audio en natif (blobs et caches par chemin natif) est la tâche AU-7, reconsidérée en conséquence (voir le [découpage du MVP](../specs/02-decoupage-mvp.md), AU-7).

### 5.10 La bibliothèque de presets (#191)

Les presets d'un projet sont des fichiers de son dossier (`presets/<id>.json`, [modèle de projet](02-modele-projet.md) § 2.8), écrits avec le reste du projet par le même chemin, fichier par fichier et de façon atomique. La bibliothèque de l'utilisateur, disponible dans tous les projets, vit ailleurs :

- **Où** : `faulk/presets/<id>.json` dans l'OPFS du site, un fichier par preset, au même format. Même origine en mode Web et en mode Boost, donc la même bibliothèque dans les deux.
- **Pourquoi l'OPFS** : la bibliothèque doit être joignable depuis tous les projets et tous les navigateurs sans clic. Un agent qui enregistre un preset n'a pas de geste de l'utilisateur, et une poignée de dossier réel perd sa permission à chaque rechargement (il faut un clic pour la rendre) ; Firefox et Safari n'ont pas de sélecteur de dossier. L'OPFS ne demande aucune permission, existe dans tous les navigateurs que Faulk sert, garde déjà les projets du navigateur (§ 5.8), et garde les fichiers au même format texte (pas IndexedDB) : exporter la bibliothèque plus tard est une copie.
- **Ce que ça coûte** : la bibliothèque n'est pas un dossier visible ; elle est propre à un profil de navigateur (absente sur une autre machine) ; un navigateur peut l'évincer si `persist()` est refusé (le même avertissement que pour les projets du navigateur) ; sans `createWritable` (Safari avant 26) elle ne vit que dans l'onglet. Une bibliothèque dans un vrai dossier (par un sélecteur, ou par le helper quand il écrira de nouveau sur disque) et son export restent à faire.
- **Dans le cœur** : la page lit les fichiers et les donne au cœur (`pw_stage_library`), qui les tient dans son état (`State::library`) pour que les commandes `preset.*` la changent comme une édition annulable ; un fichier illisible est laissé de côté, nommé dans le rapport, jamais supprimé. Un changement de la bibliothèque n'est pas un changement du projet (aucun enregistrement automatique du projet pour lui) : la page demande au cœur les octets canoniques du preset touché (`pw_preset_file`) et écrit ou supprime son fichier (`createWritable` ne remplace la cible qu'à `close()` : un onglet fermé pendant l'écriture laisse l'ancien fichier intact).
- **Dans la page** (`web/src/lib/storage/presets.ts`, `Dispatcher`) : la bibliothèque est lue quand le cœur est créé, quand la page revient au premier plan et quand un autre onglet annonce un changement (`BroadcastChannel` `faulk.presets`). Les écritures passent une à une dans une file ; une relecture n'est donnée au cœur que si aucune écriture n'a commencé ni fini pendant qu'elle lisait (sinon elle relit une fois les écritures faites) : le preset qu'une commande a enregistré ne disparaît pas. Une écriture refusée (quota, stockage indisponible) affiche un avis s'il n'y en a pas déjà un, le preset reste dans l'onglet et l'écriture est retentée au retour de la page ou au prochain changement annoncé ; l'avis s'efface quand elle réussit. La première écriture demande `navigator.storage.persist()`, pour que le navigateur garde la bibliothèque même à qui ne travaille que dans un vrai dossier. Seuls les fichiers nommés par un identifiant de preset sont lus ; les autres ne sont jamais touchés. Un fichier vide laissé par une écriture interrompue avant sa première fermeture est écarté à la lecture et remplacé par la prochaine écriture de ce preset.
- **L'en-tête du panneau d'un device** (`web/src/views/devices/PresetBar.svelte`) : précédent / suivant (les presets chargeables du type, usine puis projet puis bibliothèque), le nom du preset affiché (son nom actuel s'il est là, sinon celui que le device a retenu), une marque « modifié » (réponse de `preset.list`), et un menu : enregistrer dans le projet ou dans la bibliothèque (un champ de nom), copier dans la bibliothèque, renommer, supprimer. Chaque geste est une commande `preset.*`.

**Mode Boost** : depuis BO-1 (#102, protocole 23) le helper ne tient aucun projet et annonce `disk: false` ; c'est la page qui écrit le dossier du projet dans les deux modes. Le § 4 (« une seule autorité écrit le dossier, le helper ») et la ligne « Mode Boost » du tableau du § 5.1 décrivent l'architecture d'avant BO-1 : ils sont à reprendre quand le helper écrira de nouveau sur disque. Rien des presets ne traverse le lien page ↔ helper.

### 5.11 Lecture des fichiers audio depuis le disque (AU-3, #112)

Ce qui est écrit dans `crates/engine/src/audio_source.rs`, `crates/engine-wasm` (`fk_disk_*`, `fk_probe_*`) et `web/src/lib/audio/stream/`. Le code fait foi.

- **Sources.** Chaque média qu'un clip audio du moteur utilise est une source (`AudioSourceDeclare`, avec l'empreinte SHA-256 du blob). Le moteur ne lit rien lui-même : son côté préparation (le planificateur `Disk`) produit des **lectures par plages** (octets d'un fichier), l'hôte les fait et lui rend les octets. Il sonde d'abord la source : les 64 premiers Kio du blob, dont il tire la disposition (`PcmLayout`, désormais dans le moteur, la même conversion que celle de l'import) si elle correspond au média, sinon l'en-tête du cache PCM (`FKPCM001`, fréquence, canaux, frames, taille). Une source illisible (blob absent, cache pas encore refait, pas de disque) est **hors ligne** : comptée dans le `Feedback` (`offline_sources`) et sondée de nouveau toutes les 2 s ; sa voix se tait. Un fichier qui a changé sous la lecture (un cache refait) fait sonder la source à nouveau.
- **En mémoire.** Une source courte (≤ 30 s et ≤ 10 Mo décodés) est gardée entière ; sinon le début de chaque clip (1,5 s) et chaque boucle courte (≤ 10 s) le sont. Ces segments sont remplis entiers côté préparation, puis envoyés au moteur (`SegmentPut`) : le fil audio ne voit qu'un segment complet, et ce qu'il remplace revient par l'anneau des déchets. Tous les canaux de la source sont gardés (le choix du mixage des canaux revient à AU-4).
- **Anneaux.** Chaque piste a deux ports ; un port reçoit un anneau seulement quand une voix y lit une source qui n'est pas en mémoire, à la taille de 3 s de sa source (fréquence et canaux), et le perd après 10 s sans lecture. Le planificateur le garde 2,5 s en avance. Le protocole : la voix numérote ses demandes (génération) et les envoie par une file sans verrou ; le planificateur vide l'anneau, le marque de la génération (`ack`), puis le remplit en suivant le chemin de la demande (boucle comprise, donc sans trou au bouclage) ; la voix ne lit que les données de sa génération, et une lecture lancée pour une génération périmée est jetée à son retour.
- **Voix** (`Voice`, côté audio) : lit les segments qui couvrent sa position (départ instantané), passe à l'anneau au premier frame qu'ils ne couvrent pas (l'anneau a été demandé à partir de là au départ), garde son temps quand l'anneau manque de données (silence, puis les frames en retard sont jetés), et compte une **sous-alimentation** par bloc à court une fois l'anneau amorcé, ou après 250 ms d'attente du premier frame (un disque bloqué est compté, l'attente normale d'un saut ne l'est pas). Ni allocation, ni attente, ni panique (détecteur d'allocation dans les tests).
- **Budget.** Anneaux et segments puisent dans un budget de PCM (300 Mo par défaut, 1 Gio au plus, la marge que les limites du moteur réservent sous le plafond wasm32). Avant de charger un segment, le planificateur évince par LRU ce qui n'a pas servi depuis 5 s (les sources entières d'abord, puis les débuts de clips) ; un segment qui ne tient pas n'est pas chargé (le clip passe par l'anneau) ; un anneau n'est jamais refusé pour le budget (les tampons des voix qui jouent ne sont jamais coupés, § 5.4) : au-delà, le `Feedback` lève le drapeau de budget, et seul le plafond de 1 Gio refuse un anneau. Une voix arrêtée rend son anneau après 10 s ; une voix qui joue le garde (elle se signale chaque seconde) et en retrouve un, de la bonne forme, si elle l'a perdu. Une sous-alimentation n'est comptée que si le port a un anneau : une source hors ligne est comptée comme telle, pas comme un disque lent ; les échantillons NaN, infinis ou sous-normaux d'un fichier deviennent 0 à la conversion. L'occupation est dans le `Feedback` (`pcm_kib`).
- **Contre-pression.** Les messages du planificateur passent par la même file que ceux du projet, mais il n'en envoie plus quand cette file dépasse 64 messages (un moteur qui ne consomme pas, onglet en arrière-plan) : il ne prend jamais au projet la place qui lui rendrait `EngineBusy`.
- **Dans la page.** Le « Worker disque » est le Worker de préparation (`executor.ts`), écart à la tâche : le planificateur écrit dans la mémoire du moteur et y libère ce qui revient, et seul le fil qui tient l'instance de préparation peut le faire sans poignée de main entre deux fils ; les lectures sont asynchrones (`File.slice().arrayBuffer()`), la purge des 5 ms n'attend donc jamais le disque. Toutes les 10 ms, il demande ses lectures au planificateur, lit `blobs/<hash>` ou `.faulk/cache/pcm/<hash>.pcm` du dossier du projet (le dossier réel, ou le répertoire de l'OPFS d'un projet du navigateur), que la page lui donne à chaque rattachement. `SyncAccessHandle.read` n'est pas utilisé : il prend un verrou exclusif sur le fichier, dont le Worker du magasin a besoin quand il refait un cache ou importe.
- **Boost.** Depuis BO-1 le helper ne tient pas le dossier : personne ne fait les lectures de son planificateur, toutes ses sources restent hors ligne et aucun anneau n'est créé. La lecture native est AU-7.
- **Mesures** (#112, 2026-11-04, Windows 11, SSD, une machine de développement ; rien d'autre n'est affirmé) :
  - *Débit de lecture par plages*, Chromium 156 (Playwright, `web/scripts/bench-disk.mjs`), lectures de 256 Kio par `File.slice().arrayBuffer()` sur un fichier de 256 Mio : depuis l'OPFS, 430 à 600 Mo/s une lecture à la fois, 610 à 920 Mo/s à 8 en vol (la limite du planificateur), 0,5 à 1 ms par lecture à une position au hasard ; depuis un `File` du disque (un `<input type=file>`), 380 à 390 Mo/s une à la fois, 730 à 830 Mo/s à 8 en vol, 0,7 à 0,8 ms au hasard. Le fichier venait d'être écrit, donc probablement dans le cache du système : ce sont des chiffres à chaud. Le `File` d'un dossier choisi par le sélecteur n'a pas été mesuré (le sélecteur ne s'automatise pas) : le `File` d'un `<input>` est un fichier du disque, mais pas exactement le même chemin d'accès. Besoin à comparer : 48 voix stéréo 16 bits à 48 kHz lisent 9,2 Mo/s.
  - *Sous-alimentations sous charge*, natif (`crates/engine/tests/stream_load.rs`, à lancer à la main) : 48 voix sur 24 pistes, WAV 48 kHz stéréo 16 bits de 60 s, départs hors des segments résidents, blocs de 128 frames en temps réel, disque simulé qui répond à chaque lecture après 0 à 20 ms : **0 sous-alimentation sur 180 000 blocs de voix** en 10 s, avec ou sans un rendu qui occupe la moitié de chaque bloc ; premières données après 44 ms en moyenne, 83 ms au plus (les 48 voix partent ensemble). Avec un disque à 0 à 200 ms : 2 559 blocs comptés, ceux des départs qui attendent plus de 250 ms (premières données après 358 ms en moyenne, 707 ms au plus). Une première version, qui complétait chaque anneau par petites lectures, en affamait d'autres (139 425 blocs sur 180 000) : les anneaux sont désormais servis du plus vide au plus plein, par lectures d'au moins 0,5 s.
  - *Mémoire* : 55,3 Mo de PCM pour ces 48 voix (un anneau de 3 s par voix), compté dans le `Feedback` (`pcm_kib`).
  - *Latence ajoutée* : nulle pour un départ couvert par un segment résident (début de clip, source courte, boucle courte : premier frame dans le bloc du départ, testé). Un saut ailleurs attend un aller-retour du disque : 2 blocs dans le scénario déterministe (une lecture servie un tour plus tard), natif et WebAssembly identiques (même scénario, échantillons égaux au bit près), 44 ms en moyenne dans la mesure de charge ci-dessus. Mode Boost : sans objet tant que le helper ne lit pas le disque (AU-7). Non mesuré : dans le vrai worklet de Chromium, onglet chargé.

## 6. Décisions du fondateur (2026-10-14) et questions restantes

Décidé :

1. **Isolation cross-origin** (COOP/COEP, ou Document-Isolation-Policy dans Chrome) : acceptée, mise en place par la tâche AU-0 dès la phase 0, avec ressources auto-hébergées et prudence avec les fenêtres d'authentification d'autres origines.
4. **Firefox et Safari** : l'OPFS + export en archive suffit pour le MVP ; le helper reste le moyen d'avoir un dossier réel partout.

Choix techniques, tranchés dans les tâches du [découpage du MVP](../specs/02-decoupage-mvp.md) :

2. Fréquence d'échantillonnage : rééchantillonnage **à la lecture**, dans le moteur (§ 5.3, tâche AU-4) ; pas de fréquence de projet fixe.
3. PCM décodé d'un fichier compressé : second blob versionné, ou cache régénérable ? Proposition retenue par défaut : cache régénérable dans `.faulk/cache/`, le blob gardant les octets d'origine ; la PR d'AU-1 présente ce choix et son coût au fondateur. Mis en œuvre par AU-1 (#110, § 5.9) : `.faulk/cache/pcm/<sha256>.pcm`, sauf pour un WAV ou un AIFF lu directement, qui n'en a pas.
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
