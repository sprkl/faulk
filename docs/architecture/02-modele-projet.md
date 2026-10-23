---
type: Architecture
title: "Modèle de projet et format sur disque"
description: "The Faulk project model and its on-disk format, decided on 2026-10-14: a folder grouped by track with canonical text for human-scale data and content-addressed blobs for machine-scale data, the page owning the project in both modes, a semantic three-way merge shared by git, the browser and the helper, in-app versions and variants, and the strategy for very large projects."
status: draft
---

# Modèle de projet et format sur disque

> **Décisions du fondateur du 2026-10-14 intégrées** (§ 9). Ce document reprend l'étude du 2026-10-13 sur un modèle de projet versionnable, avec les ajustements de la synthèse : arborescence regroupée par piste, un fichier par device, arrangement par piste, binaire adressé par contenu pour les données denses, instantané de cache et banc de charge. Le format dossier, la propriété du projet par la page, le vocabulaire « Version » / « Variante », le solo en session, la copie indépendante des clips et les blobs SHA-256 sont décidés ; ce qui reste ouvert est listé au § 9. Les pistes audio (import et lecture, pas d'enregistrement) et l'automation font partie du MVP : les types ci-dessous les couvrent. Le statut reste `draft` tant que le document n'a pas été vérifié contre le code.

## En bref

Un projet Faulk est **un dossier regroupé par piste**. Ce qui est à l'échelle humaine (une piste, un device, un clip, une liste de notes) est du **texte canonique** : JSON à clés triées pour les structures, TSV pour les longues listes homogènes, une donnée par ligne. Ce qui est à l'échelle de la machine (audio, automation très dense, état opaque d'un instrument) est du **binaire adressé par son contenu** (SHA-256) sous `blobs/`, jamais modifié et compatible Git LFS.

Une seule bibliothèque Rust lit, écrit, valide et **fusionne en trois points** ce dossier. Elle sert de pilote de fusion pour git, de moteur des « variantes » dans l'application et de garde-fou après chaque fusion. Le double objectif :

1. un utilisateur qui connaît git met son projet dans un dépôt, travaille en branches, fusionne et lit des diffs compréhensibles ;
2. un utilisateur qui ne connaîtra jamais git obtient les mêmes gestes dans l'interface : **sauver une version, ouvrir une variante, comparer A/B, fusionner**, avec git en coulisse.

Le stockage du dossier (dossier réel, OPFS, helper) et la mémoire audio sont traités dans [Stockage local du projet et mémoire audio](03-stockage-local.md). Les tâches qui construisent ce format sont dans le [découpage du MVP](../specs/02-decoupage-mvp.md).

## 1. Le modèle de données

### 1.1 Principes

| Principe | Choix | Pourquoi |
| --- | --- | --- |
| Identité | Chaque entité durable (piste, device, clip, couloir d'automation, scène, placement) a un **ULID** créé à sa naissance et jamais réutilisé. | Deux branches qui créent chacune une piste ne se marchent pas dessus ; un renommage ne change pas l'identité ; un fichier se nomme d'après l'ID de ce qu'il contient. |
| Temps musical | Positions et durées en **ticks entiers**, `PPQ = 960` ticks par noire. | Un changement de tempo d'un côté et des notes modifiées de l'autre se fusionnent sans décalage. 960 se divise par 2, 3, 4, 5, 6, 8, 10, 12, 15, 16 : triolets et quintolets exacts. Entiers : aucune dérive, diff stable. |
| Temps audio | Positions *dans un fichier audio* en **frames** entières. | La source audio ne dépend pas du tempo. |
| Ordre | L'ordre des pistes, scènes et devices est une **clé d'indexation fractionnaire** (chaîne triable, ex. `"a0"`, `"a0V"`) portée par chaque élément, pas une liste centrale. | Deux branches qui ajoutent chacune une piste ne touchent pas la même liste : pas de conflit. Une commande peut recevoir un index ; le modèle le traduit en clé. |
| Paramètres | Identifiés par un **ID stable sous forme de chaîne** (`"cutoff"`), jamais par un index. Valeur en **unité naturelle** (s, Hz, dB) déclarée par le device. | Le diff se lit (`cutoff: 800 → 1200`) ; un device qui ajoute un paramètre ne décale pas les autres. |
| Projet et session | Ce qui fait la musique va dans le projet ; ce qui relève de l'écran ou de la machine (zoom, sélection, défilement, entrée MIDI choisie, pistes repliées, caches) va dans `.faulk/`, **ignoré par git**. | Ouvrir un projet et faire défiler l'écran ne produit aucun diff. |

Le solo est un cas limite (un état d'écoute plutôt qu'un choix de mix). **Décidé le 2026-10-14** : le solo est un état de session, écrit dans `.faulk/session.json` (ignoré par git) et restauré à la réouverture, jamais versionné ; le mute est dans le projet. La commande `mixer.set_solo` est donc `live`, hors historique. Il en va de même de l'armement et du monitoring (RE-1) : `track.arm` et `track.set_monitor` écrivent `armed` et `monitor` dans la session, hors historique, et supprimer une piste les efface.

### 1.2 Les types (`crates/project`)

Nouvelle crate `crates/project` : le modèle, la lecture et l'écriture canoniques, la validation, la fusion. Pure (elle reçoit et rend des octets, aucune entrée-sortie directe), compilable en natif et en WASM (côté page, hors worklet, donc `wasm-bindgen` y est possible). `crates/commands` en dépend ; le moteur n'en dépend pas : il reçoit un graphe de rendu dérivé du modèle. Sa création modifie la table « Stack and layout » de `CLAUDE.md` : c'est une décision du fondateur.

Implémenté par PR-1 (#46) ; le code fait foi (`crates/project/src/model.rs`), ce bloc le résume :

```rust
pub struct Tick(pub i64);     // 960 per quarter note (engine::time, CT-2)
pub struct Frame(pub u64);    // sample index in an audio file (engine::time, CT-2)
pub struct Id(String);        // ULID: 26 chars, Crockford base32, time-sortable; typed: ProjectId, TrackId, SceneId, DeviceId, ClipId, PlacementId, LaneId
pub struct OrderKey(String);  // fractional index ("a0", "a0V"), compared bytewise; ties broken by id
pub struct Hash(pub [u8; 32]); // SHA-256 of a blob, 64 lower-case hex chars on disk

pub struct Project {
    pub format: u32,                     // on-disk format version (1), for migrations
    pub id: ProjectId,                   // stable project identity, created once (see 1.3)
    pub name: String,
    pub tempo: Vec<TempoPoint>,          // { tick, bpm }; exactly one point at tick 0 in the MVP
    pub meter: Vec<MeterPoint>,          // { tick, signature }; exactly one point at tick 0 in the MVP
    pub transport: TransportSettings,    // saved transport settings (see 1.3)
    pub scenes: BTreeMap<SceneId, Scene>,
    pub master: Mixer,                   // master bus (no sends)
    pub tracks: BTreeMap<TrackId, Track>,
}

pub struct TransportSettings {
    pub launch_quantize: Quantize,       // None | Sixteenth | Beat | Bar | Bars2 | Bars4 | Bars8; default Bar (engine::time)
    pub loop_region: Option<LoopRegion>, // [start, end) in ticks (engine::time)
    pub loop_enabled: bool,              // requires a loop region
    pub metronome: Metronome,            // { enabled: bool, gain_db: f64, count_in_bars: u8 }
}

pub struct Scene { pub name: String, pub order: OrderKey } // a tempo per scene: after the MVP (constant tempo)

pub struct Track {
    pub name: String,
    pub color: Option<Rgb>,              // "#rrggbb"
    pub order: OrderKey,
    pub kind: TrackKind,                 // Midi | Audio (Return: after the MVP, no sends or returns)
    pub mixer: Mixer,
    pub output: Output,                  // Master in the MVP; Track(Id) (group or return) after it
    pub devices: BTreeMap<DeviceId, Device>, // at most one instrument, first; then effects, ordered by Device::order
    pub clips: BTreeMap<ClipId, Clip>,   // MIDI clips in a MIDI track, audio clips in an audio track
    pub placements: BTreeMap<PlacementId, Placement>, // this track's arrangement
    pub automation: BTreeMap<LaneId, AutomationLane>,
}

pub struct Mixer { pub gain_db: f64, pub pan: f64, pub mute: bool } // sends: after the MVP; solo lives in the session state

pub struct Device {
    pub order: OrderKey,
    pub role: DeviceRole,                // Instrument | Effect
    pub enabled: bool,
    pub source: DeviceSource,            // Builtin { id, version } | Module { name, hash }; plugins after the MVP
    pub params: BTreeMap<String, f64>,   // param id -> value in natural unit
    pub state: Option<Hash>,             // opaque instrument state, stored as a blob
}

pub struct Clip {
    pub name: String,
    pub color: Option<Rgb>,
    pub slot: Option<SceneId>,           // scene id when the clip sits in the launch grid; one clip per slot
    pub launch_quantize: Option<Quantize>, // reserved, after the MVP: absent = project setting, never written by the MVP
    pub content: ClipContent,
}

pub enum ClipContent {
    // Musical time: length and loop in ticks.
    Midi { length: Tick, looping: Option<TickLoop>, notes: BTreeSet<Note> },
    // Source time: a range [start, start + frames) of the audio file; no length in ticks is stored.
    // Loop in source frames, relative to `start`, within [0, frames]: never in ticks.
    Audio { media: AudioMedia, start: Frame, frames: Frame, gain_db: f64, looping: Option<FrameLoop> },
}

// The file's metadata in clear, next to its reference (§ 2.2).
pub struct AudioMedia { pub hash: Hash, pub sample_rate: SampleRate, pub channels: u16, pub frames: Frame, pub name: String }

// Ord compares fields in declaration order. The last three are optional in the file:
// absent = default (cents 0, chance 100, repeat 1), so a note has one representation.
pub struct Note { pub start: Tick, pub pitch: u8, pub length: Tick, pub velocity: u8,
                  pub cents: i8 /* -50..=50 */, pub chance: u8 /* 0..=100 */, pub repeat: u8 /* 1..=16 */ }

pub struct Placement { pub clip: ClipId, pub start: Tick, pub span: Span }

pub enum Span {
    Ticks { length: Tick, offset: Tick },   // MIDI clip: trim in musical time
    Frames { length: Frame, offset: Frame }, // audio clip: trim in source time
}

pub struct AutomationLane {
    pub target: AutomationTarget,        // Device { device: DeviceId, param: String } | Mixer { param: Gain | Pan }
    pub points: BTreeMap<Tick, AutomationPoint>, // at most one point per tick; point = { value, shape: Linear | Step }
}

// Session state, never a field of Project, written to .faulk/session.json (decision 10).
pub struct SessionState { pub selected_track: Option<TrackId>, pub solo: BTreeSet<TrackId>, pub midi_input: Option<String>, pub armed: BTreeSet<TrackId>, pub monitor: BTreeSet<TrackId> }
```

`Project::validate()` rend toutes les violations, chacune avec son chemin (`tracks/<id>/clips/<id>`) : format, noms, identifiants uniques dans tout le projet, références (case de scène, clip d'un placement, device d'un couloir), accord des types (clip et piste, étendue et clip), notes dans la longueur du clip, boucles et plages, positions dans `[0, 2^40]`, nombres finis (un gain peut valoir `-inf`), gain d'une piste, du master ou du métronome, et tout point d'automation de gain, entre `MIN_GAIN_DB` et `MAX_GAIN_DB` (-96 à +6 dB, #226, #269), règles de la chaîne (un instrument au plus, en tête ; le lecteur de clips exigé sur une piste audio et refusé ailleurs ; une piste MIDI peut n'avoir aucun instrument), fréquence d'échantillonnage de 8 à 768 kHz, fin en ticks d'un placement audio calculée au tempo du projet et limites de sûreté (`project::limits`). Deux invariants tiennent par construction et sont refusés à la lecture : une clé en double dans une table (identifiant ou tick) fait échouer la désérialisation, au lieu de garder silencieusement la dernière valeur.


Choix et limites pour le MVP :

- **Clips et placements.** Le clip est le contenu ; la grille de lancement le met dans une case (`slot`), l'arrangement le pose dans le temps avec un `Placement`. Plusieurs placements d'un même clip sont des clips liés. **Décidé le 2026-10-14** : copier un clip dans l'arrangement crée par défaut un nouveau clip indépendant ; le clip lié (second placement du même clip) est une option des commandes `arrangement.duplicate` et `arrangement.place_from_slot` (tâches AR-1 et AR-5 du découpage).
- **Quantification de lancement.** Globale au projet (`TransportSettings::launch_quantize`, 1 mesure par défaut, décision du 2026-10-14). Le champ `launch_quantize` du clip est réservé pour un réglage par clip après le MVP : optionnel dans le fichier du clip, absent = réglage global ; le MVP ne l'écrit jamais.
- **Deux longueurs distinctes.** Un clip MIDI a une longueur en ticks ; un clip audio n'a qu'une plage source en frames, et aucune longueur en ticks n'est enregistrée (elle deviendrait fausse au premier changement de tempo). Ses ticks de fin se **calculent** : la durée `frames / sample_rate` est convertie en ticks au tempo du projet, arrondie au tick le plus proche (demi vers l'extérieur), puis ajoutée au tick de début (`Tempo::audio_end_tick`, figé par CT-2 à tempo constant ; avec une carte de tempo, après le MVP, la conversion partira des secondes du début). Un placement suit la même règle (`Span::Ticks` ou `Span::Frames`).
- **Clips audio non élastiques** : début en ticks, durée en frames ; un changement de tempo déplace le début, pas la vitesse. Le recalage temporel viendra plus tard comme une liste de points `(tick, frame)`.
- **Automation d'arrangement uniquement** (par piste), sur les paramètres des devices et du mixeur, dans le MVP (décision du 2026-10-14, tâches AM-1 à AM-6 du découpage) ; les enveloppes propres aux clips sont reportées.
- **Pistes audio dans le MVP** : import et lecture (`ClipContent::Audio`, `Span::Frames`), sans enregistrement audio ni recalage temporel ; une piste audio est une piste dont l'instrument est le lecteur de clips (tâches AU-1 à AU-8 du découpage).
- **Notes sans identifiant sur disque, ni en mémoire.** Une note est une *valeur* : la modifier, c'est retirer l'ancienne et ajouter la nouvelle, ce qui rend la fusion des notes simple (§ 4.3). **Décidé par CT-4 (#40)** : pas de `NoteId`, ni durable ni de session ; les commandes adressent une note par sa valeur entière, et un inverse est construit depuis ce qui a réellement changé (ajouter une note déjà présente dans l'ensemble ne change rien). L'expression par note (MPE) exigera plus tard un identifiant durable, ajouté comme colonne optionnelle ; le `note_id` de l'ABI de l'Atelier est un identifiant de voix du moteur, pas du projet.
- **Plugins tiers** : leur état est un blob opaque ; seuls les paramètres que l'hôte peut lire sont recopiés en clair dans `params`. En conflit, un état opaque ne se fusionne pas : on choisit un côté.

### 1.3 Identité du projet et réglages du transport

- **Identifiant de projet.** `project.json` porte un `id` (ULID) obligatoire, créé à la création du projet et jamais modifié ensuite ; la validation refuse un projet sans `id`. C'est lui que la page et le helper utilisent pour se rapprocher en mode Boost ([stockage](03-stockage-local.md) § 4). « Enregistrer sous » crée un nouvel `id`. Une copie du dossier faite hors de Faulk garde le même `id` : le helper vérifie donc aussi le chemin, et signale deux dossiers de même `id`. Le format commence à la version 1 avec ce champ : aucune migration n'est nécessaire.
- **Réglages du transport.** La quantification de lancement, la région de boucle et son activation, et le métronome (activation, niveau, décompte) sont dans `project.json`, ce qui correspond aux commandes `edit` de TR-3 et GR-2 dans le [découpage du MVP](../specs/02-decoupage-mvp.md) : ils survivent à la réouverture. La lecture, la position de la tête et les clips en cours de lecture restent éphémères (`live`). Le solo et l'entrée MIDI choisie sont dans l'état de session (§ 9, décision du 2026-10-14) ; le métronome et la boucle restent dans `project.json`.

## 2. Le format sur disque

### 2.1 La règle de format

- **Données à l'échelle humaine : du texte.** JSON canonique (clés triées, une valeur par ligne) pour les structures ; TSV pour les longues listes homogènes (notes, points d'automation, placements), une ligne par élément.
- **Données à l'échelle de la machine : du binaire adressé par contenu** sous `blobs/<sha256>` : audio, automation très dense (**seuils de QA-7, confirmés et appliqués par SV-1** : au-delà de 100 000 points par couloir, retour au texte sous 50 000, hystérésis entre les deux ; voir [QA-7](../spikes/QA-7-banc-de-charge-format-projet.md)), état opaque d'un instrument. Un blob n'est jamais modifié ; il est compatible Git LFS.
- Un fichier unique binaire `.faulkpack` peut exister **pour l'export et le partage seulement** ; ce n'est jamais le format de travail.

Pourquoi un dossier plutôt qu'un fichier : le chemin dit déjà *quoi* a changé, et la plupart des éditions parallèles touchent des fichiers différents, donc git fusionne sans conflit. Aucun fichier unique, si bien formaté soit-il, n'empêche qu'un ajout de piste et un ajout de scène tombent dans des lignes voisines. Le prix : l'atomicité de l'enregistrement est à traiter fichier par fichier (voir [stockage](03-stockage-local.md)), et le partage hors git passe par `.faulkpack`.

### 2.2 Arborescence

```
Mon morceau/
├── project.json                      # format version, project id, name, tempo, time signature, transport settings, scenes, master mixer
├── tracks/
│   └── <track>/                      # folder named by the track ULID, never by its name
│       ├── track.json                # name, colour, mixer, routing, order key
│       ├── devices/
│       │   └── <device>.json         # one instrument or effect: type, parameters, order key in the chain
│       ├── automation/
│       │   └── <lane>.tsv            # target device+parameter, then sorted points, one per line
│       ├── clips/
│       │   ├── <clip>.json           # clip header: name, slot, length, loop, audio reference
│       │   └── <clip>.notes.tsv      # MIDI notes, one per line
│       └── arrangement.tsv           # placements of this track's clips on the timeline
├── blobs/
│   └── <sha256>                      # audio, dense automation, opaque instrument state; never modified
├── .faulk/                           # cache snapshot, view state, temp files; ignored by git
├── .gitattributes                    # written by Faulk (LFS, merge and diff drivers, eol)
└── .gitignore                        # written by Faulk (.faulk/)
```

- Un dossier de piste est nommé par l'ID : renommer une piste change une ligne, pas un chemin. Supprimer une piste supprime un dossier. Déplacer un clip d'une piste à l'autre est un déplacement de fichiers, que git détecte comme renommage.
- **Un fichier par device** : régler le filtre de la basse et insérer un delay sur la batterie touchent deux fichiers distincts.
- **L'arrangement est par piste** (pas un fichier global) : deux personnes qui arrangent deux pistes différentes ne se croisent jamais.
- Les blobs sont partagés au niveau du projet : le même échantillon utilisé sur trois pistes n'existe qu'une fois.
- Les métadonnées d'un fichier audio (fréquence, canaux, nombre de frames, nom d'origine) sont écrites en clair à côté de sa référence, dans le JSON du clip : le projet se lit, se valide et se fusionne sans décoder l'audio, et sans l'avoir téléchargé.

### 2.3 Exemples

`clips/<clip>.notes.tsv` :

```
# faulk-notes 1	start	length	pitch	velocity
0	240	36	110
0	240	48	96
480	240	36	100
960	120	43	100
```

`automation/<lane>.tsv` (la cible est `device:<device>/<paramètre>`, `mixer:gain` ou `mixer:pan` ; au-delà du seuil de densité, l'en-tête référence un blob au lieu de lister les points, par `blob=<sha256>` et `points=<nombre>` après la cible) :

```
# faulk-automation 1	target=device:<device>/cutoff
0	800	linear
3840	2400	linear
7680	2400	step
```

`arrangement.tsv` :

```
# faulk-arrangement 1	id	start	length	clip	offset
<placement>	0	15360	<clip>	0
<placement>	15360	15360	<clip>	0
```

**Colonnes optionnelles des notes** (tâche #192, réservées par CL-1) : `cents` (accord fin, -50 à 50), `chance` (0 à 100 %, 100 par défaut) et `repeat` (1 à 16, 1 par défaut). Le TSV étant positionnel, la règle de l'écrivain est celle de l'en-tête : les quatre colonnes de base viennent toujours en premier, puis `cents`, `chance`, `repeat` **seulement si une note du clip a une valeur non par défaut dans cette colonne** ; une colonne présente écrit toutes ses valeurs, y compris les valeurs par défaut de ses autres lignes. Un clip sans note modifiée garde l'en-tête de base et les octets d'aujourd'hui, et un lecteur qui lit l'en-tête accepte n'importe quel sous-ensemble de ces colonnes, dans cet ordre. Exemple :

```
# faulk-notes 1	start	length	pitch	velocity	chance
0	240	36	110	100
480	240	36	100	50
```

Un diff de notes reste lisible (`-480 240 36 100` / `+480 240 36 90` : la vélocité a baissé), et la vue de diff de Faulk l'affiche graphiquement (§ 4.5).

### 2.4 Déterminisme : les règles de l'écrivain canonique

Le même projet produit toujours **exactement les mêmes octets**. Règles, toutes testées (sérialiser → relire → resérialiser = identique, sur des projets générés aléatoirement) :

- UTF-8 sans BOM, fin de ligne `\n`, `\n` final ; `.gitattributes` force `eol=lf`.
- Clés JSON triées par ordre d'octets, une valeur par ligne ; tables indexées par ID plutôt que tableaux.
- Lignes TSV triées par la clé naturelle (note : start, pitch, length, velocity, cents, chance, repeat ; point : tick ; placement : start puis id).
- **Écriture paresseuse** : seuls les fichiers dont le contenu canonique a changé sont réécrits.
- Aucune donnée volatile dans les fichiers versionnés : pas de date de sauvegarde, pas de version de l'application hors numéro de format, pas d'état d'écran.

### 2.5 Nombres à virgule

- Positions en entiers (ticks, frames) : aucun flottant là où l'ordre et l'égalité comptent.
- Valeurs continues écrites avec la **représentation décimale la plus courte qui relit la même valeur** (comportement par défaut de `serde_json` et de `JSON.stringify` pour un `f64`). Jamais `-0` (normalisé en `0`), jamais `NaN` (refusé à la validation). `-inf dB` s'écrit `null`.
- **Quantification à la source** : chaque paramètre déclare un pas (0,01 dB, 1 Hz…) et la commande arrondit avant d'écrire. Sans cela, un bouton tourné à la souris produit des diffs bruités et des conflits factices. L'automation enregistrée en direct est quantifiée et simplifiée (points colinéaires supprimés).

### 2.6 Versions du format et migrations

`project.json` porte `"format": 1`. Faulk lit toutes les versions passées et migre en mémoire ; il n'écrit que la version courante. Une migration s'enregistre comme une **version à part**, pour que son diff ne se mélange pas avec des changements musicaux. Les migrations sont des fonctions pures testées sur des projets de référence.

### 2.7 Lecture, écriture et limites (SV-1)

Le code fait foi (`crates/project/src/file/`) ; ce qui suit en résume le contrat.

- **Système de fichiers abstrait.** Tout passe par le trait `FileSystem` (lire, écrire un fichier de façon atomique, supprimer, tester, lister avec taille et « estampille »). `MemFs` sert aux tests et à l'amorçage du navigateur, `StdFs` au natif ; l'OPFS et le dossier choisi par l'utilisateur sont les implémentations de SV-2 et SV-4. Le trait est synchrone (les poignées synchrones de l'OPFS le permettent dans un Worker).
- **Ouvrir** (`open`) : le format est lu d'abord (`FormatProbe`) : un projet plus récent est refusé (`NewerFormat`) sans rien lire d'autre ni être réécrit ; un format plus ancien passe par les migrations, appliquées en mémoire sur les fichiers texte (`MIGRATIONS`, vide pour le format 1 ; le mécanisme est testé avec une table d'essai) ; puis chaque fichier est lu avec une taille bornée (64 Mio pour le texte), des nombres de lignes bornés par `project::limits`, des doublons refusés, et le projet est validé (`Project::validate`) : toute erreur est une `FileError` typée (`Corrupt` avec le fichier et la ligne, `Missing`, `TooLarge`, `Invalid` avec toutes les violations…), jamais une panique. Un fichier inconnu dans un dossier structurel (`.DS_Store`, notes sans clip) est ignoré, signalé dans `OpenReport.ignored` et jamais supprimé ; un dossier de piste qui ne contient aucun fichier de piste n'est pas une piste. Les blobs d'audio ou d'état qui manquent ne bloquent pas l'ouverture (`missing_blobs`, § 3.3) ; le blob d'un couloir est lu, son empreinte vérifiée.
- **Enregistrer** (`save`) : `SaveState` retient le condensé de chaque fichier tel que lu ou écrit ; un fichier dont les octets canoniques n'ont pas changé n'est pas réécrit. Avec un `Dirty` (les unités que les commandes ont marquées : projet, piste, device, clip, notes, arrangement, couloir), seules ces unités sont regardées, et les fichiers d'une entité disparue sont supprimés ; sans, tout le projet est regardé. Un état appartient à son dossier : « enregistrer sous » repart de `SaveState::new()`. Limite connue : une modification faite hors de Faulk depuis l'ouverture n'est pas vue par l'écriture paresseuse (rouvrir le dossier).
- **Interruption.** Chaque fichier est remplacé de façon atomique par le système de fichiers ; les fichiers sont écrits dans l'ordre des dépendances (devices et clips avant l'arrangement et les couloirs qui les citent), `project.json` en dernier et les suppressions après tout. Rien de plus n'est promis : un enregistrement interrompu entre deux fichiers peut laisser un mélange d'anciens et de nouveaux fichiers, que l'ouverture lit quand il respecte les invariants du projet (la moitié d'une édition qui a atteint le disque) et refuse avec une erreur typée (`Invalid`) sinon. Un journal d'enregistrement (`.faulk/`) qui permettrait de reprendre ou d'annuler est une piste d'amélioration, pas du MVP.
- **Seuils d'automation** : `Layout::QA7` (100 000 / 50 000). Le mode d'un couloir entre les deux seuils est celui qu'il avait, retenu par `SaveState` (rempli par `open`, depuis le texte comme depuis l'instantané) : réécrire un projet ouvert n'écrit rien. Un blob commence par `FKAUTO01` puis 17 octets par point ; les anciens blobs restent dans `blobs/` (§ 3.4).
- **Instantané de cache** (`.faulk/cache/project.snap`, navigateur seulement : `OpenOptions::use_cache`, `SaveOptions::write_cache`) : une entrée par fichier (chemin, taille, estampille, condensé) avec le texte ou une forme binaire compacte (notes, points) ; à l'ouverture, un fichier dont la taille et l'estampille n'ont pas changé est décodé depuis l'instantané, les autres sont lus, les ajouts et suppressions sont vus par la liste du dossier. L'instantané est validé par une somme de contrôle ; absent, corrompu ou d'un autre format, il est ignoré (`CacheUse`), et une ouverture avec instantané qui échoue est reprise depuis le texte : jamais une erreur de son fait. À l'écriture, un fichier que Faulk n'a pas écrit et dont l'estampille a changé depuis qu'il l'a vu (un `git pull`) est vérifié par son condensé et exclu de l'instantané s'il diffère ; l'écriture ne liste que les dossiers du format, jamais les dossiers de l'utilisateur, et un dossier illisible donne un enregistrement sans instantané plutôt qu'un échec. Son écriture suit les fichiers (seul le texte modifié est réencodé, le reste vient de l'instantané précédent) mais coûte une copie de l'ensemble : l'hôte choisit à quelle cadence l'écrire (voir le coût mesuré dans la pull request de SV-1).
- **Session** (`load_session`, `save_session`) : `.faulk/session.json`, tolérant (absent ou illisible : session par défaut, jamais un refus d'ouvrir), les pistes disparues sont oubliées.
- **Ouvrir dans le moteur** (`project.open`, SV-4) : la commande `project.open { folder }` (édition, non annulable, destructive : elle remplace le projet et vide l'historique) lit le dossier que l'hôte a mis dans le contexte (`Context.folder`, un `Folder` qui enveloppe un `FileSystem` en lecture seule), avec `open` (instantané de cache compris) puis `load_session`. Tout est lu, validé et construit avant que l'état ne change : un dossier refusé laisse le projet en cours, son historique et sa session intacts, et le dossier est consommé (il sert une fois). Le refus est une `CommandError::Folder { error }` dont `FolderError` est typée (`not_a_project`, `newer_format`, `unsupported_format`, `missing`, `corrupt`, `too_large`, `invalid` avec les premières violations, `io`, `migration`, `not_staged`) ; la page en tire un texte par classe. Le moteur repart de zéro : l'hôte (`Synced::run`, donc `project-wasm`, le helper plus tard et le banc de rendu) répond à une ouverture réussie par un `Reset` dans l'époque suivante et l'instantané du nouveau projet, solo et sélection de la session compris ; les pistes de l'ancien projet s'éteignent en 5 ms (`FADE_SECONDS`), rien de l'ancien projet ne sonne ensuite. Le solo n'est dans aucun fichier versionné : il revient de `.faulk/session.json`. Dans la page, le magasin (SV-2) lit son stockage et passe le dossier à `ProjectCore.stage(nom, entrées)` : un cadre binaire (`parse_folder`, borné à 512 Mio, chemins relatifs sans `..`) dont les fichiers texte et les blobs de couloirs d'automation (`FKAUTO01`) portent leurs octets, les autres blobs seulement leur taille (`needsBytes`), de sorte que la mémoire de l'ouverture est celle du projet, pas celle de son audio (le cadre et la copie qu'en fait `MemFs` coexistent un instant : la mémoire linéaire du wasm monte à environ le double du cadre et ne redescend pas avant le rechargement de la page). Le temps, mesuré sur le profil de QA-7 (Rust natif sur `MemFs`, en release, sur un poste chargé) : 0,33 s pour mettre le cadre en place et 1,77 s pour `project.open` (lecture avec instantané, validation, instantané moteur de 78 Mio encodé), soit 2,1 s, pour 1,56 à 2,45 s mesurés par QA-7 pour la seule ouverture dans la page. Non mesurés : la lecture du stockage et la construction du cadre côté JavaScript, la version wasm, et le blocage du fil principal de la page pendant l'ouverture.
- **Git** : `.gitattributes` et `.gitignore` sont écrits quand ils sont absents, jamais réécrits.
- **`.faulkpack`** (`export_pack`, `import_pack`) : zip déterministe (entrées triées, dates fixes) des fichiers du projet et des seuls blobs qu'il utilise (pas `.faulk/`, pas `.git/`, pas les blobs orphelins) ; l'export ouvre donc le projet (un projet invalide n'est pas exporté). L'import exige un dossier vide, n'accepte que les chemins du format, compte les octets produits (pas ceux déclarés), vérifie le nom de chaque blob contre son SHA-256, ouvre et valide le résultat, et ne laisse rien en cas d'échec.

## 3. Les blobs

### 3.1 Adressage par contenu

- Un blob est nommé par le **SHA-256 de ses octets** et n'est **jamais modifié** : une édition destructive (normaliser, inverser) produit un nouveau blob. Un clip référence une empreinte, pas un chemin.
- **Déduplication gratuite** : importer deux fois le même échantillon, ou le même enregistrement dans deux branches, ne coûte qu'un fichier.
- **Import** : on garde les octets d'origine, pour que l'empreinte corresponde au fichier de l'utilisateur (le stockage discute le cas des formats compressés). **Enregistrement** : écrit d'abord dans `.faulk/`, puis à l'arrêt on calcule l'empreinte, on range le blob et on crée le clip. Format d'enregistrement (FLAC ou WAV) : à décider avec l'enregistrement audio, qui suit le MVP (§ 9).
- Le SHA-256 se calcule en flux (dans le navigateur, crate `sha2` en WASM dans un worker).

SHA-256 est exactement l'identifiant (`oid`) qu'utilise Git LFS : l'empreinte de Faulk et celle de LFS sont la même, sans recalcul.

### 3.2 Avec git : LFS et pilotes

Faulk écrit dans `.gitattributes` :

```
blobs/** filter=lfs diff=lfs merge=lfs -text
*.json   text eol=lf merge=faulk diff=faulk
*.tsv    text eol=lf merge=faulk diff=faulk
```

Un blob ne peut jamais être en conflit : deux branches qui ajoutent le même fichier ajoutent le même nom avec le même contenu. Un pointeur LFS est un petit fichier texte (`version`, `oid sha256:…`, `size`) que Faulk sait écrire sans le client LFS, ce qui compte pour le navigateur (§ 6).

### 3.3 Sans git : le magasin de contenu

Pour l'utilisateur sans git et pour le navigateur, les blobs vivent dans un **magasin local par empreinte** (le dossier `blobs/` du projet, ou un magasin partagé entre projets dans OPFS). Plus tard, un magasin distant (stockage objet, ou serveur LFS) permet de télécharger un projet léger puis l'audio à la demande ; une piste dont le blob manque reste ouvrable et affiche « média absent ».

### 3.4 Nettoyage

Marquer puis balayer : un blob est vivant s'il est référencé par **l'état courant, une version sauvegardée de n'importe quelle variante, ou la pile d'annulation**. Le reste se supprime **sur demande seulement** (« Libérer l'espace : 1,2 Go de prises inutilisées »), jamais automatiquement. Sous git, l'historique LFS se nettoie avec les outils LFS.

## 4. Branches et fusions qui ont un sens musical

### 4.1 Trois niveaux de fusion

1. **Par fichier (git seul).** Deux branches qui touchent des pistes, des devices, des clips ou des couloirs différents modifient des fichiers différents : git fusionne sans rien savoir de Faulk. C'est le cas le plus fréquent.
2. **Par fichier, sémantique (pilote `faulk merge`).** Quand les deux branches touchent le *même* fichier, le pilote fusionne à trois points au niveau des champs et des éléments, pas des lignes.
3. **Au niveau du projet (validation après fusion).** Certaines incohérences n'existent qu'entre fichiers : une branche supprime une piste, l'autre y ajoute un clip ; une branche supprime un device, l'autre automatise un de ses paramètres. Après toute fusion, Faulk valide le projet et transforme ces incohérences en **conflits sémantiques** à trancher.

### 4.2 Fusion des structures (JSON)

| Base → nous / eux | Résultat |
| --- | --- |
| inchangé / changé | on prend le changement |
| changé / changé, même valeur | on prend la valeur |
| changé / changé, valeurs différentes | **conflit** sur ce champ |
| entité ajoutée d'un côté | ajoutée (IDs uniques : jamais de collision) |
| entité supprimée d'un côté, inchangée de l'autre | supprimée |
| entité supprimée d'un côté, modifiée de l'autre | **conflit** « supprimé / modifié » |

Deux insertions qui reçoivent par hasard la même clé d'ordre sont départagées par ID (ordre déterministe, pas de conflit).

### 4.3 Fusion des collections

- **Notes : fusion d'ensembles, avec appariement.** La formule naïve (nous ∩ eux) ∪ (nous − base) ∪ (eux − base) confond une note modifiée avec une note supprimée puis ajoutée : si une branche supprime la note N et que l'autre change sa vélocité (N'), N' serait conservée sans conflit et la suppression perdue. La fusion apparie donc d'abord chaque note de base avec ses versions de chaque côté : CT-4 (#40) ne donnant pas d'identifiant aux notes, l'appariement se fait par la clé stable (hauteur et début), une modification de hauteur ou de début comptant comme « supprimée puis ajoutée ». Ensuite, les règles de la table du § 4.2 s'appliquent note par note, y compris le conflit « supprimé / modifié ». Une branche qui ajoute un contre-chant et une autre qui corrige une fausse note dans le même clip se fusionnent toujours sans conflit. Deux notes de même hauteur qui se chevauchent après fusion sont signalées comme « conflit de note » à écouter.
- **Points d'automation** : table indexée par tick ; même tick changé différemment des deux côtés → conflit sur ce point. Un couloir stocké en blob se fusionne en le relisant comme table.
- **Placements** : table indexée par ID ; même placement déplacé différemment → conflit ; chevauchements après fusion → signalés.

### 4.4 Ce qu'est un conflit, en musique

| Situation | Fusion automatique ? |
| --- | --- |
| Pistes, devices, clips ou couloirs différents | Oui |
| Notes ajoutées ou retirées dans le même clip, à des endroits différents | Oui |
| Tempo changé d'un côté, notes éditées de l'autre | Oui (tout est en ticks) ; les clips audio sont repositionnés et signalés « à réécouter » |
| Même paramètre réglé à deux valeurs différentes | Conflit sur ce paramètre |
| Même clip renommé ou recoloré deux fois | Conflit mineur, avec un choix par défaut |
| Piste supprimée d'un côté, éditée de l'autre | Conflit « supprimé / modifié » |
| Deux clips posés dans la même case de la grille | Conflit |
| Automation d'un device supprimé de l'autre côté | Conflit sémantique (validation) |
| État opaque d'un instrument changé des deux côtés | Conflit, choix d'un côté |
| Source d'un module Faust modifiée des deux côtés | Fusion texte classique ; si conflit, choix ou édition dans l'Atelier |

Un conflit est toujours **localisé et nommé** (« Basse › Filter › cutoff : 800 Hz chez vous, 1 200 Hz dans la variante ») et se résout par un choix (« garder le mien », « garder le leur », « garder les deux » quand cela a un sens). Jamais de marqueurs `<<<<<<<` dans un fichier de projet.

### 4.5 Le pilote de fusion git et le diff lisible

- `faulk merge %O %A %B %P` : git fournit la base, notre version, la leur et le chemin ; le pilote écrit le résultat dans `%A` et sort en erreur s'il reste un conflit. Le fichier laissé est **toujours valide** (notre valeur à l'endroit du conflit) ; les conflits sont listés dans `.faulk/conflicts.json`, et Faulk relit la base, notre version et la leur dans l'index git pour son écran de résolution.
- `faulk diff` en `textconv` : `git diff` affiche « Basse : volume −6 dB → −4,5 dB ; clip Intro : +3 notes, −1 note » au lieu de lignes brutes.
- Une config git ne se versionne pas : `faulk git setup` (commande du helper) inscrit les pilotes. Sans cette étape, git retombe sur la fusion ligne à ligne, qui, grâce au format, produit des conflits localisés, et la validation de Faulk rattrape le reste à l'ouverture.

**Une seule implémentation** de la fusion, dans `crates/project` : appelée par le helper comme pilote git, par la page (WASM) comme `mergeDriver` d'isomorphic-git ou pour les variantes, et par les tests.

### 4.6 Dans l'interface : versions, variantes, comparer, fusionner

Vocabulaire décidé le 2026-10-14, sans le mot « branche » :

- **Version** : un instantané nommé du projet (un commit). « Sauver une version » est un geste explicite ; l'enregistrement automatique écrit le dossier de travail sans créer de version.
- **Variante** : une exploration parallèle (une branche). « Essayer une variante » la crée depuis l'état courant et bascule dessus.
- **Comparer A/B** : jouer les deux états à la même position et basculer **sans arrêter la lecture**, à la mesure suivante ; le moteur reçoit la différence entre les deux graphes de rendu. La vue montre aussi les pistes et clips ajoutés, modifiés, supprimés.
- **Fusionner** : ramener une variante (en entier, par piste ou par clip) dans la version principale ; les conflits s'affichent comme une liste à cocher, chacun avec une écoute de chaque côté.
- **Reprendre seulement ceci** : copier un clip ou une piste depuis une autre variante ou une ancienne version.

C'est aussi le terrain de l'IA : **un agent travaille dans une variante** (« Fais-moi trois variantes du refrain »), l'utilisateur compare et fusionne ce qui lui plaît.

## 5. Commandes, état et annulation

### 5.1 Options

| Option | Pour | Contre |
| --- | --- | --- |
| Journal d'opérations comme vérité (event sourcing) | Historique fin, attribution, temps réel envisageable | Rejouer coûte cher ; chaque évolution d'une commande impose de migrer l'historique ; fusionner deux journaux n'est pas plus simple que fusionner deux états ; diff illisible |
| **État + git** | Simple, robuste, lisible, compatible avec les outils git | Historique au grain des versions, pas des gestes |
| CRDT (ex. Automerge) | Fusion sans conflit, temps réel | Fusionne *sans* demander, ce qui en musique est souvent faux ; format binaire peu compatible avec git ; dépendance lourde |

### 5.2 Proposition

**L'état est la vérité ; git est l'histoire ; les commandes sont le seul moyen de changer l'état.**

1. **Modèle mutable à inverses explicites** (option (a) ci-dessous, **décidé par CT-4, #40**). Chaque commande modifie le `Project` en place et rend son inverse ; le modèle immuable à partage structurel (option (b)) n'est pas retenu.
2. **Annuler = appliquer l'inverse.** La pile d'annulation garde l'inverse de chaque commande (`x-undo: inverse`) ; l'inverse est une commande ordinaire, appliquée par le même chemin, donc le moteur et l'interface suivent une annulation comme une édition. Une commande structurelle (« supprimer une piste ») a pour inverse une commande de restauration qui porte l'entité avec ses identifiants. `none` pour les commandes `live` (notes jouées en direct, lecture, solo, sélection) et les éditions irréversibles, alors `x-destructive`. Un tour d'agent = une seule entrée (groupe).
3. **Fichiers sales** : chaque commande marque les entités touchées ; l'enregistrement réécrit seulement leurs fichiers (§ 7).
4. **Journal léger, non vérité** : les commandes d'une session sont gardées (en mémoire et dans `.faulk/`) pour l'attribution et pour **rédiger le message de la version** (« Agent : 1 piste ajoutée, 42 notes posées », avec des lignes de fin `Faulk-Author: agent:<nom>`).
5. **Les gestes de version sont des commandes** : `version.save { message }`, `version.list`, `variant.create { name }`, `variant.switch { name }`, `variant.compare { a, b }`, `variant.merge { from } → { conflicts }`, `conflict.resolve { id, choice }`, `version.restore { id }`. `version.restore` et l'abandon d'une variante sont destructeurs.

**Choix tranché par CT-4 (#40)** entre les deux contrats que ce document et le [découpage du MVP](../specs/02-decoupage-mvp.md) proposaient :

| | (a) Modèle mutable, inverses explicites (**retenu**) | (b) Modèle persistant, annulation par instantané |
| --- | --- | --- |
| Signature | `Command::apply(&self, &mut State, &mut Context) -> Result<Outcome, CommandError>` | `apply(&Project, Command) -> Result<(Project, Outcome), CommandError>` |
| Annulation | `Outcome` porte l'inverse ; chaque commande sait le calculer, et le harnais de test le vérifie | La pile garde les racines précédentes ; `x-undo: snapshot` |
| Coût | Écrire et tester un inverse par commande (règle de test mécanique) | Partage structurel obligatoire (`Arc`, `im`) dans tout le modèle |
| Risque | Un inverse faux corrompt l'historique : couvert par le harnais (`apply(inverse)` restaure l'état à l'identique) | Annuler doit piloter le moteur : un diff projet → `ControlMsg`, ou un rejeu complet pendant la lecture |

Le second risque a tranché : avec les inverses, annuler passe par le même chemin que toute commande et produit ses `ControlMsg` et ses `ProjectEvent` ; le `Project` reste une donnée possédée ordinaire. La variante hybride (cloner la racine avant un tour d'agent) reste un filet de sécurité possible, hors contrat. Le découpage du MVP § 2.4 décrit le contrat retenu.

### 5.3 Ce qu'on reporte

Le journal d'opérations comme vérité, la collaboration en temps réel et les CRDT. Le modèle proposé (IDs stables, notes comme ensemble, clés d'ordre fractionnaires) reste compatible avec une couche CRDT future.

### 5.4 Où vit le modèle

**Décidé le 2026-10-14 : la page détient le modèle dans les deux modes** (`crates/project` et `crates/commands` en WASM, crate `crates/project-wasm`) ; le moteur (worklet ou helper) ne reçoit que le graphe de rendu et ses changements. Les agents MCP passant par le helper voient leurs commandes relayées vers la page, ce qui garde le helper dans son rôle de pont. Le stockage est derrière une interface `ProjectStore` (dossier choisi par défaut, OPFS en repli, système de fichiers du helper) et git derrière `VersionStore`. En mode Boost, le helper est la seule autorité qui *écrit* le dossier (voir [stockage](03-stockage-local.md)), mais il écrit ce que la page lui envoie.

## 6. git dans le navigateur

### 6.1 État des lieux (octobre 2026)

- **isomorphic-git** : git en JavaScript pur, navigateur et workers compris, avec son propre système de fichiers (« bring your own fs ») ; des démonstrations le font tourner sur OPFS. `merge` accepte un `mergeDriver`, ce qui permet d'y brancher la fusion de Faulk ; mais pas de stratégie récursive (échec s'il y a plusieurs bases), pas d'abandon de fusion incomplète, pas de Git LFS documenté. Maintenu par deux bénévoles : **dépendance à risque, à isoler derrière une interface**.
- **CORS** : les grands hébergeurs git n'envoient pas les en-têtes nécessaires ; pousser ou tirer depuis la page exige un proxy. Le helper n'a pas ce problème.
- **Dossier réel** (`showDirectoryPicker`) : Chromium seulement. OPFS est disponible partout mais invisible pour l'utilisateur et pour son git.
- **LightningFS** peut corrompre un dépôt en cas d'arrêt brutal : préférer un adaptateur OPFS soigné, avec `flush`.

### 6.2 Deux usages

1. **Versions et variantes dans l'application** : personne ne voit le dépôt. Il peut vivre dans OPFS, les blobs **hors** des objets git (dans le magasin par empreinte), les commits ne contenant que du texte.
2. **Le projet dans le git de l'utilisateur** : vrai dossier, vrai git, LFS, dépôt distant. C'est le terrain du helper.

### 6.3 Chemin proposé

**Phase 1 (MVP, ce que planifie le [découpage](../specs/02-decoupage-mvp.md)) : un format prêt pour git, sans git ni fusion dans l'application.**

- `crates/project` : modèle, écrivain canonique, validation, migrations ; tests de déterminisme (PR-1, SV-1).
- Projet enregistré dans un dossier choisi (Chrome, Edge), dans l'OPFS en repli, ou par le helper en Boost ; export et import `.faulkpack` (SV-2, SV-3, AU-7).
- `.gitattributes` et `.gitignore` écrits par Faulk (SV-1) : un utilisateur qui connaît git versionne son dossier avec les outils git ordinaires, la fusion ligne à ligne produisant des conflits localisés grâce au format.
- Validation à l'ouverture : un projet fusionné à la main par git est vérifié et ses incohérences signalées (SV-1, SV-4).

**Phase 2 (après le MVP) : fusion sémantique, puis versions et variantes dans l'application.** D'abord la fusion à trois points dans `crates/project` (propriétés `merge(b, x, b) = x`, `merge(b, x, x) = x`, symétrie des conflits) et le binaire `faulk` livré avec le helper, pilote `merge` et `diff`, avec `faulk git setup` ; puis `VersionStore` dans un worker (isomorphic-git sur OPFS en mode Web ; git du système ou une bibliothèque Rust comme gitoxide, à évaluer, dans le helper) ; commandes `version.*` et `variant.*`, comparaison A/B, fusion avec écoute ; les cas que isomorphic-git ne gère pas se rabattent sur une fusion complète faite par Faulk à partir des trois arbres.

**Phase 3 : dépôts distants et partage.** Pousser et tirer via le helper (vrai git et LFS) ; depuis la page seule, via un proxy CORS ou un service de synchronisation. Les commits créés dans la page référencent les blobs par **pointeurs LFS écrits par Faulk** ; l'envoi passe par l'API batch de LFS (faisabilité à valider par un spike). Téléchargement paresseux de l'audio.

**Plus tard** : collaboration en temps réel, revue de variante entre musiciens, enveloppes de clip, audio élastique, expression par note.

### 6.4 Précédents hors musique

- Moteurs de jeu dont les scènes sont en texte pensé pour git, avec un outil de fusion fourni comme pilote git : précédent direct de `faulk merge`.
- Carnets de calcul : un outil de diff et de fusion sémantique branché comme pilote git, pour un JSON illisible en diff ligne à ligne.
- Outils de design avec branches internes pour des non-développeurs, et ordre des éléments par indexation fractionnaire : modèle de l'expérience visée pour les variantes.
- Bases de données versionnées : fusion cellule par cellule, conflit seulement sur la même cellule ; c'est le grain proposé ici (le champ, la note, le point).
- Recherche « local-first » (Automerge) : elle insiste sur la visualisation des différences et la fusion volontaire plutôt qu'automatique, ce qui rejoint « fusionner en demandant ».

## 7. Les grands projets

Le format doit tenir un projet très lourd sans que l'ouverture ou l'enregistrement ne deviennent lents.

- **Le texte est lu à l'ouverture et écrit à l'enregistrement, jamais entre les deux.** En mémoire, le modèle est fait de structures Rust compactes (une note = 12 à 16 octets) ; le texte n'est pas la représentation de travail.
- **Risque 1 : le nombre de fichiers dans le navigateur.** Ouvrir des milliers de petits fichiers par les API asynchrones du navigateur coûte cher. Parade : un **instantané binaire de tout le projet dans `.faulk/cache/`**, écrit à l'enregistrement ; à l'ouverture, Faulk charge l'instantané puis ne relit que les fichiers modifiés depuis (date et taille, ou empreinte). Le cache est régénérable : s'il manque ou ne correspond plus, on relit tout.
- **Risque 2 : l'enregistrement.** Suivi des fichiers sales : éditer une note réécrit un seul petit fichier (le `.notes.tsv` du clip), jamais tout le projet.
- **Annulation** : avec le modèle mutable retenu par CT-4 (§ 5.2), chaque entrée garde seulement son inverse ; l'inverse d'une suppression porte l'entité supprimée, ce qui est le seul coût proportionnel à la taille.
- **Automation dense** : au-delà de 100 000 points (proposition de QA-7, avec hystérésis : retour au texte sous 50 000), un couloir passe en blob pour ne pas produire des fichiers texte énormes et des diffs illisibles. En dessous, le blob coûte un fichier de plus, des diffs binaires et une copie complète par version sous LFS, pour un gain d'analyse de l'ordre de 2 fois.

**Banc de charge** (tâche QA-7, #130, phase 1 ; méthode, machine et chiffres dans [QA-7](../spikes/QA-7-banc-de-charge-format-projet.md)) : un projet synthétique de 100 pistes, 200 000 notes, 6 millions de points d'automation et de l'audio, mesuré sur l'ouverture, l'enregistrement, la mémoire, les échanges de structures du moteur et git, en natif et dans le navigateur (OPFS ; le dossier réel du navigateur n'est pas automatisable). Seuils de QA-7, **confirmés et appliqués par SV-1** (`project::file`) :

- **Blob d'automation** : au-delà de 100 000 points par couloir (texte jusque-là : 1,9 Mio, relu en 9 ms, un point réécrit en 15 ms, `git diff` en 49 ms ; à 1 000 000 de points : 124 ms et 430 ms), hystérésis à 50 000.
- **Instantané de cache** : utile dans le navigateur (ouverture de 2,45 s à 1,56 s pour 3 001 fichiers, de 3,93 s à 2,09 s en vérifiant la taille et la date de chaque fichier), sans gain en natif (lecture sur 8 threads : 144 à 223 ms contre 185 à 256 ms). Il se vérifie par la liste (chemin, taille, date) de tous les fichiers, ajouts et suppressions compris. Le coût restant est la construction du modèle (1,3 à 1,45 s en wasm pour 6 millions de points).
- **`blobs/`** : pas de sous-division en phase 1 (aucun écart net à plat ou en 256 dossiers jusqu'à 100 000 fichiers en natif et 10 000 en OPFS) ; à reconsidérer au-delà de 10 000 blobs.
- **Nombre de fichiers** : il domine l'écriture (3 001 fichiers : 4,3 à 12 s en natif, 2,5 s en OPFS, 10,8 s de `git add`, pour 0,6 s de sérialisation) : le suivi des fichiers sales est indispensable et le format gagne à ne pas multiplier les fichiers.

## 8. Proposition de MVP

Ce que le [découpage du MVP](../specs/02-decoupage-mvp.md) (2026-10-14) planifie, avec les tâches qui le portent :

1. `crates/project` : types du § 1.2 (sans enveloppes de clip, audio élastique ni plugins tiers), écrivain et lecteur canoniques, validation, migrations (PR-1, SV-1).
2. Format dossier du § 2, blobs du § 3, `.gitattributes` et `.gitignore`, instantané de cache et suivi des fichiers sales du § 7 (SV-1, SV-2, SV-4, AU-1).
3. Commandes de projet selon le contrat retenu par CT-4 (§ 5.2 : modèle mutable avec inverses) (PR-2).
4. Tests : aller-retour octet pour octet, propriétés de l'écrivain, fuzz du lecteur ; banc de charge (SV-1, QA-7).

Le format v1 est ainsi *prêt pour git* (texte canonique, blobs compatibles LFS, attributs écrits), sans que git ni la fusion ne soient dans l'application.

**Après le MVP** (l'étude les plaçait dans son MVP ; le découpage ne les planifie pas, point de périmètre signalé au fondateur) :

- la fusion à trois points du § 4 dans `crates/project`, avec une fusion de référence par ligne du tableau § 4.4 et les propriétés de la fusion ;
- le binaire `faulk` : pilotes git `merge` et `diff`, `git setup`, et un dépôt git réel où deux branches sont fusionnées par le pilote ;
- puis les versions et variantes dans l'application, la comparaison A/B, la fusion avec écoute.

## 9. Décisions du fondateur (2026-10-14) et questions restantes

Décidé :

1. **Solo** : dans la session (`.faulk/session.json`), jamais versionné ; le mute dans le projet. L'entrée MIDI choisie va aussi dans la session ; le métronome et la zone de boucle restent dans `project.json` (§ 1.3).
2. **Copier un clip dans l'arrangement** : copie indépendante par défaut, clip lié en option.
4. **Où vivent les projets** par défaut en mode Web : un dossier réel choisi par l'utilisateur (Chrome, Edge) ; OPFS et export en archive ailleurs ([stockage](03-stockage-local.md) § 5.1).
6. **Vocabulaire** : « Version » et « Variante », acceptés (§ 4.6).
7. **Qui détient le modèle en mode Boost** : la page ; le helper relaie et écrit le dossier pour elle (§ 5.4).
8. **Blobs dans git** : fichiers nommés par SHA-256, compatibles Git LFS ; le choix entre LFS et un magasin séparé sera pris quand le besoin viendra.

Restent ouverts :

3. **Notes** : tranché par CT-4 (#40) : sans identifiant, ni sur disque ni en session ; les commandes adressent une note par sa valeur, la fusion apparie par (hauteur, début) (§ 1.2, § 4.3).
5. **Format d'enregistrement** audio : FLAC ou WAV ? À décider avec l'enregistrement audio, qui suit le MVP.
9. **Sources Faust des modules du projet** : dans le dossier du projet (un sous-dossier texte, versionné avec lui) ou dans une bibliothèque personnelle ? L'arborescence de la synthèse ne les place pas encore (voir la [spec de l'Atelier](../specs/01-atelier-modules.md)).
10. **Collaboration** : le partage entre musiciens est-il un objectif de la première année (ce qui avance la phase 3), ou d'abord un outil personnel ?

## Sources

- isomorphic-git : https://github.com/isomorphic-git/isomorphic-git ; https://isomorphic-git.org/docs/en/merge ; https://isomorphic-git.org/docs/en/fs
- git sur OPFS (démo memfs) : https://unpkg.com/memfs@4.80.0/demo/git-opfs/README.md ; preuve de concept OPFS + ZenFS + isomorphic-git : https://gitlab.com/gitlab-org/gitlab-web-ide/-/merge_requests/536/commits
- Pilotes de fusion git (`%O %A %B`, résultat dans `%A`) : https://www.root.cz/man/5/gitattributes ; https://graphite.dev/guides/git-merge-driver
- API batch de Git LFS : https://demo.gitea.com/lunny/git-lfs/raw/commit/4457d7c7c5906025f753579f67f975792235b717/docs/api/http-v1-batch.md
- Accès aux dossiers dans le navigateur : https://web.dev/articles/files/open-a-directory
- Automerge 3 : https://automerge.org/blog/automerge-3/
