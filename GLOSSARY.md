# Glossaire

Le vocabulaire du domaine de Faulk. Un terme par entrée, en français, avec son identifiant de code en anglais (`CLAUDE.md` > Language). Les termes audio qui sont anglais dans le métier (clip, track, device, take) restent anglais dans la prose.

Statut des identifiants : la crate `crates/project` existe (tâche CT-1) et ne contient encore que le temps musical (`engine::time`, tâche CT-2). Les identifiants marqués **(prévu)** viennent du modèle de projet et du découpage du MVP, dans le dossier [`docs/`](docs/) ; ceux qui ne le sont pas existent dans le code. Le code a toujours raison : en cas de désaccord, suivre le code et signaler la définition périmée.

Le nom d'un device n'est jamais traduit : un seul nom anglais, le même dans les deux langues de l'interface. Les libellés et descriptions de ses paramètres, eux, sont traduits.

## Le projet et ce qu'il contient

**Projet** : le morceau en cours de travail, un dossier regroupé par piste. Ce qui fait la musique y est écrit ; ce qui relève de l'écran ou de la machine va dans la session. Il porte un identifiant stable créé une fois. Code : `Project`, `project.json` **(prévu)**.

**Piste MIDI** : une piste dont les clips contiennent des notes et dont la chaîne de devices commence par un instrument. Code : `Track` avec `TrackKind::Midi` **(prévu)**.

**Piste audio** : une piste dont les clips sont des fichiers audio, lus par le lecteur de clips ; import et lecture seulement dans le MVP, sans enregistrement audio. Code : `TrackKind::Audio` **(prévu)**.

**Device** : un maillon de la chaîne d'une piste : un instrument, un effet audio, et plus tard un effet MIDI. Un module de l'Atelier ou un plugin tiers est un device ; une entrée MIDI physique n'en est pas un (c'est un réglage de la session). Un device a un rôle, un état activé ou non, des paramètres et une place dans la chaîne. Son nom est unique et jamais traduit. Code : `Device`, `DeviceRole` ; côté moteur, le trait `Device` et le descripteur par type (`kind`, par exemple `"faulk.osc_synth"`) **(prévu)**.

**Instrument** : le device qui produit le son d'une piste à partir de notes. Code : `DeviceRole::Instrument` **(prévu)** ; le synthétiseur actuel est `Synth` (`crates/engine/src/synth.rs`).

**Effet** : un device qui transforme un signal (effet audio) ; un effet MIDI transformera des notes, après le MVP. Code : `DeviceRole::Effect` **(prévu)**.

**Lecteur de clips** : l'instrument des pistes audio : il lit les clips audio. Une piste audio est donc une piste dont l'instrument est le lecteur de clips. Code : `faulk.clip_player` **(prévu)**.

**Scène** : une ligne de la grille de lancement ; lancer une scène lance les clips de ses emplacements. Code : `Scene`, commandes `scene.*` et `launch.scene` **(prévu)**.

**Emplacement de clip** : la case de la grille de lancement, à l'intersection d'une piste et d'une scène ; elle contient au plus un clip. Code : `Clip::slot` (identifiant de la scène) **(prévu)**.

**Tick** : l'unité du temps musical, un entier, 960 par noire (`PPQ`) ; positions et durées musicales sont en ticks, jamais en secondes. Code : `Tick`, `PPQ` (`engine::time`).

**Frame** : l'unité du temps d'un fichier audio, un index d'échantillon entier qui ne dépend pas du tempo. Code : `Frame` (`engine::time`).

**Tempo** : la vitesse du morceau en noires par minute, constante dans le MVP, de 20 à 999 ; il porte les conversions ticks ↔ secondes ↔ échantillons. Code : `Tempo` (`engine::time`).

**Signature rythmique** : le nombre de temps par mesure et la valeur d'un temps (4/4 par défaut) ; le temps est la valeur du dénominateur (en 6/8, une croche). Code : `TimeSignature` (`engine::time`).

**Région de boucle** : la plage `[début, fin)` en ticks que le transport répète. Code : `LoopRegion` (`engine::time`).

**Clip** : un contenu musical, MIDI (notes, longueur et boucle en ticks) ou audio (plage d'un fichier en frames). Le clip est le contenu ; l'emplacement le met dans la grille, le placement le pose dans l'arrangement. Copier un clip crée un clip indépendant, sauf option « lié ». Code : `Clip`, `ClipContent::{Midi, Audio}` **(prévu)**.

**Note** : un événement musical d'un clip MIDI, une valeur sans identifiant sur disque : début, hauteur, durée, vélocité. Code : `Note { start, pitch, length, velocity }` **(prévu)** ; le moteur actuel joue des notes par `Synth::note_on` et `Synth::note_off`.

**Arrangement** : la ligne de temps d'un morceau, propre à chaque piste : l'ensemble de ses placements. Code : `Track::placements`, `arrangement.tsv` **(prévu)**.

**Placement** : un clip posé dans l'arrangement : début en ticks et étendue (durée et décalage de coupe, en ticks pour un clip MIDI, en frames pour un clip audio). Plusieurs placements d'un même clip sont des clips liés. Code : `Placement`, `Span` **(prévu)**.

**Lancement** : démarrer ou arrêter un clip ou une scène depuis la grille ; il s'exécute à la prochaine limite de quantification. L'état de lecture est éphémère, jamais enregistré. Code : commandes `launch.clip`, `launch.scene`, `launch.stop_track`, `launch.stop_all` **(prévu)**.

**Quantification** (de lancement) : la durée sur laquelle un lancement s'aligne : aucune, 1/16, 1 temps, 1, 2, 4 ou 8 mesures. Elle est globale au projet, 1 mesure par défaut ; un réglage par clip viendra après le MVP. Code : `Quantize` (`engine::time`, sept valeurs, `next_boundary_at_or_after` donne la prochaine limite) ; `TransportSettings::launch_quantize` et la commande `transport.set_launch_quantization` **(prévu)**.

**Armement** : l'état d'une piste prête à enregistrer ; le MIDI joué en direct va à la piste sélectionnée et aux pistes armées. Code : commande `track.arm` **(prévu)**.

**Tête de lecture** : la position courante de la lecture dans le temps du projet. Elle est éphémère : le moteur la renvoie à la page environ 30 fois par seconde et elle n'est jamais enregistrée. Code : champ du `Feedback` **(prévu)**.

**Prise** : ce qui a été joué pendant un enregistrement ; elle devient un clip par une seule commande annulable. Code : `record.commit_take` **(prévu)**.

**Couloir d'automation** : la courbe d'un paramètre dans le temps, propre à une piste, sur un paramètre d'un device ou du mixeur. L'automation d'arrangement existe au MVP ; celle des clips vient plus tard. Code : `AutomationLane`, `AutomationTarget` **(prévu)**.

**Point** (d'automation) : une valeur à un tick d'un couloir, avec sa forme de transition ; au plus un point par tick. Code : `AutomationPoint` **(prévu)**.

## Versions, stockage et état d'écran

**Version** : un instantané nommé du projet, qu'un geste explicite sauve ; l'enregistrement automatique n'en crée pas. Vocabulaire de l'interface (décision 9) : on ne dit jamais « commit » à l'utilisateur. Code : commandes `version.save`, `version.list`, `version.restore` **(prévu)** ; un commit git sous le capot.

**Variante** : une exploration parallèle du projet, créée depuis l'état courant, que l'on peut comparer puis fusionner. Vocabulaire de l'interface (décision 9) : on ne dit jamais « branche » à l'utilisateur. Un agent travaille dans une variante. Code : commandes `variant.create`, `variant.switch`, `variant.compare`, `variant.merge` **(prévu)** ; une branche git sous le capot.

**Blob** : un fichier binaire du projet adressé par son contenu (SHA-256), jamais modifié : audio, automation très dense, état opaque d'un instrument. Code : `Hash`, dossier `blobs/<sha256>` **(prévu)**.

**Session** : l'état qui relève de l'écran ou de la machine et non de la musique : solo, entrée MIDI choisie, sélection, zoom, défilement, pistes repliées. Il est écrit dans `.faulk/`, ignoré par git, restauré à la réouverture et jamais versionné. Le solo est donc un état de session, alors que le mute est dans le projet. À ne pas confondre avec la connexion de pairage entre la page et le helper (`/session`, `crates/protocol`) et de la classe web `Session` (`web/src/lib/session.svelte.ts`), qui porte l'état de la page et son chemin vers le moteur, pas cet état de session. Code : `SessionState`, `.faulk/session.json` **(prévu)**.

## Modes et extensions

**Mode Web** : Faulk dans le navigateur seul, le moteur tourne dans un `AudioWorklet`. Code : `crates/engine-wasm`.

**Mode Boost** : Faulk avec l'assistant natif (le helper), pour la latence professionnelle et les plugins tiers ; le moteur y est le même. Code : `crates/helper`.

**Atelier** : l'éditeur intégré où l'on crée ses instruments et effets. Code : `workshop` **(prévu)**.

**Module** : un instrument ou un effet créé dans l'Atelier ; il implémente le même contrat que les devices intégrés. Code : `Module`, `DeviceSource::Module` **(prévu)**.
