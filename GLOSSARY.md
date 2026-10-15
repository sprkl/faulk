# Glossaire

Le vocabulaire du domaine de Faulk. Un terme par entrée, en français, avec son identifiant de code en anglais (`CLAUDE.md` > Language). Les termes audio qui sont anglais dans le métier (clip, track, device, take) restent anglais dans la prose.

Statut des identifiants : la crate `crates/project` contient le modèle de projet (tâche PR-1 : types, identifiants, clés d'ordre, validation, état de session) et réexporte le temps musical (`engine::time`, tâche CT-2). Les identifiants marqués **(prévu)** viennent du modèle de projet et du découpage du MVP, dans le dossier [`docs/`](docs/) ; ceux qui ne le sont pas existent dans le code. Le code a toujours raison : en cas de désaccord, suivre le code et signaler la définition périmée.

Le nom d'un device n'est jamais traduit : un seul nom anglais, le même dans les deux langues de l'interface. Les libellés et descriptions de ses paramètres, eux, sont traduits.

## Le projet et ce qu'il contient

**Projet** : le morceau en cours de travail, un dossier regroupé par piste. Ce qui fait la musique y est écrit ; ce qui relève de l'écran ou de la machine va dans la session. Il porte un identifiant stable créé une fois. Code : `Project`, `ProjectId` ; `project.json` **(prévu)**.

**Piste MIDI** : une piste dont les clips contiennent des notes et dont la chaîne de devices commence par un instrument (une piste MIDI peut n'en avoir aucun tant qu'il n'est pas choisi). Code : `Track` avec `TrackKind::Midi`.

**Piste audio** : une piste dont les clips sont des fichiers audio, lus par le lecteur de clips ; import et lecture seulement dans le MVP, sans enregistrement audio. Code : `TrackKind::Audio`.

**Commandes de pistes** : créer (MIDI ou audio, avec un instrument du registre), supprimer, renommer, colorer, déplacer et dupliquer une piste, plus la sélection de la piste (session). Supprimer emporte devices, clips et placements, et l'annulation les rend à l'identique. Sans plafond sous la limite de sûreté de 4 096 pistes. Code : `track.create`, `track.delete`, `track.restore`, `track.rename`, `track.set_color`, `track.move`, `track.restore_order`, `track.duplicate`, `view.select_track`.

**Device** : un maillon de la chaîne d'une piste : un effet MIDI, un instrument ou un effet audio, dans cet ordre. Un module de l'Atelier ou un plugin tiers est un device ; une entrée MIDI physique n'en est pas un (c'est un réglage de la session). Un device a un rôle, un état activé ou non, des paramètres et une place dans la chaîne. Son nom est unique et jamais traduit. Code : `Device`, `DeviceRole`, `DeviceSource` ; côté moteur, le trait `Device`, `engine::device::DeviceRole` et le descripteur par type `DeviceDescriptor` (`kind`, par exemple `"faulk.osc_synth"`), dans `crates/engine/src/device.rs` (contrat CT-5) ; les devices intégrés et leur registre (une ligne par device, `REGISTRY`, résolu par `engine::builtin_factory`) sont dans `crates/engine/src/devices/`, un fichier par device (DV-1). Le projet ne connaît pas encore le rôle d'effet MIDI.

**Instrument** : le device qui produit le son d'une piste à partir de notes. Code : `DeviceRole::Instrument` ; le synthétiseur actuel est le device `faulk.osc_synth`, affiché « Oscillator » : `Synth` (`crates/engine/src/devices/osc_synth.rs`, DV-1).

**Effet** : un device qui transforme un signal (effet audio), après l'instrument. Code : `DeviceRole::Effect`.

**Effet MIDI** : un device qui reçoit des notes et en produit d'autres (transposition, accord, arpège), placé avant l'instrument d'une piste MIDI. Code : `engine::device::DeviceRole::MidiEffect` (contrat CT-5) ; le modèle de projet **(prévu)**.

**Événement** (de device) : ce qu'un device reçoit dans un bloc, daté par un offset en frames : note jouée ou relâchée, valeur de paramètre. Une note porte son identifiant de voix et un accord fin en cents. Code : `Event`, `VoiceId` (`engine::device`).

**Événement daté** : un événement placé à un tick dans une table que le moteur lit pendant la lecture (notes, valeurs de paramètres) ; le moteur le livre au device à l'échantillon qui contient sa position, dans un sous-bloc (un morceau de bloc qui ne traverse jamais un bouclage). Code : `Schedule`, `Timed`, `Sink` (`engine::events`, MO-4).

**Lecteur de clips** : l'instrument des pistes audio : il lit les clips audio. Une piste audio est donc une piste dont l'instrument est le lecteur de clips. Code : `faulk.clip_player` (`CLIP_PLAYER`, exigé par la validation comme instrument d'une piste audio et refusé ailleurs) ; le device lui-même **(prévu)**.

**Scène** : une ligne de la grille de lancement ; lancer une scène lance les clips de ses emplacements. Code : `Scene` ; commandes `scene.*` et `launch.scene` **(prévu)**.

**Emplacement de clip** : la case de la grille de lancement, à l'intersection d'une piste et d'une scène ; elle contient au plus un clip. Code : `Clip::slot` (identifiant de la scène).

**Tick** : l'unité du temps musical, un entier, 960 par noire (`PPQ`) ; positions et durées musicales sont en ticks, jamais en secondes. Code : `Tick`, `PPQ` (`engine::time`).

**Frame** : l'unité du temps d'un fichier audio, un index d'échantillon entier qui ne dépend pas du tempo. Code : `Frame` (`engine::time`).

**Tempo** : la vitesse du morceau en noires par minute, constante dans le MVP, de 20 à 999 ; il porte les conversions ticks ↔ secondes ↔ échantillons. Code : `Tempo` (`engine::time`).

**Signature rythmique** : le nombre de temps par mesure et la valeur d'un temps (4/4 par défaut) ; le temps est la valeur du dénominateur (en 6/8, une croche). Code : `TimeSignature` (`engine::time`).

**Région de boucle** : la plage `[début, fin)` en ticks que le transport répète. Code : `LoopRegion` (`engine::time`).

**Clip** : un contenu musical, MIDI (notes, longueur et boucle en ticks) ou audio (plage d'un fichier en frames). Le clip est le contenu ; l'emplacement le met dans la grille, le placement le pose dans l'arrangement. Copier un clip crée un clip indépendant, sauf option « lié ». Code : `Clip`, `ClipContent::{Midi, Audio}`, `AudioMedia`.

**Note** : un événement musical d'un clip MIDI, une valeur sans identifiant sur disque : début, hauteur, durée, vélocité, et trois champs optionnels : accord fin en cents (-50 à 50), chance (0 à 100 %) et répétitions (1 à 16), absents du fichier quand ils valent leur défaut. Une note hors de la longueur de son clip est refusée par les commandes, jamais tronquée en silence (seul `clip.set_length` avec `trim` coupe). Code : `Note { start, pitch, length, velocity, cents, chance, repeat }` ; le moteur joue des notes par les événements `NoteOn` et `NoteOff` du contrat de device (`engine::device::Event`).

**Arrangement** : la ligne de temps d'un morceau, propre à chaque piste : l'ensemble de ses placements. Code : `Track::placements` ; `arrangement.tsv` **(prévu)**.

**Placement** : un clip posé dans l'arrangement : début en ticks et étendue (durée et décalage de coupe, en ticks pour un clip MIDI, en frames pour un clip audio). Plusieurs placements d'un même clip sont des clips liés. Code : `Placement`, `Span`.

**Lancement** : démarrer ou arrêter un clip ou une scène depuis la grille ; il s'exécute à la prochaine limite de quantification. L'état de lecture est éphémère, jamais enregistré. Code : commandes `launch.clip`, `launch.scene`, `launch.stop_track`, `launch.stop_all` **(prévu)**.

**Quantification** (de lancement) : la durée sur laquelle un lancement s'aligne : aucune, 1/16, 1 temps, 1, 2, 4 ou 8 mesures. Elle est globale au projet, 1 mesure par défaut ; un réglage par clip viendra après le MVP. Code : `Quantize` (`engine::time`, sept valeurs, `next_boundary_at_or_after` donne la prochaine limite) ; `TransportSettings::launch_quantize` ; la commande `transport.set_launch_quantization` **(prévu)**.

**Armement** : l'état d'une piste prête à enregistrer ; le MIDI joué en direct va à la piste sélectionnée et aux pistes armées. Code : commande `track.arm` **(prévu)**.

**Tête de lecture** : la position courante de la lecture dans le temps du projet. Elle est éphémère : le moteur la renvoie à la page environ 30 fois par seconde et elle n'est jamais enregistrée. Code : `Feedback::playhead_tick` et `playhead_frame` (`crates/engine/src/msg.rs`, contrat CT-3 ; le moteur ne les remplit pas encore).

**Prise** : ce qui a été joué pendant un enregistrement ; elle devient un clip par une seule commande annulable. Code : `record.commit_take` **(prévu)**.

**Couloir d'automation** : la courbe d'un paramètre dans le temps, propre à une piste, sur un paramètre d'un device ou du mixeur. L'automation d'arrangement existe au MVP ; celle des clips vient plus tard. Code : `AutomationLane`, `AutomationTarget`.

**Point** (d'automation) : une valeur à un tick d'un couloir, avec sa forme de transition ; au plus un point par tick. Code : `AutomationPoint`.

**Clé d'ordre** : la place d'une piste, d'une scène ou d'un device parmi ses voisins, portée par l'élément lui-même sous forme de chaîne triable (`a0`, `a0V`) plutôt que par une liste centrale ; deux éléments de même clé (venus de deux variantes) sont départagés par leur identifiant. Code : `OrderKey`, `OrderKey::between`, `OrderKey::for_index`.

**Limite de sûreté** : un nombre maximal très haut (pistes, notes, points, positions) que la validation refuse, pour qu'un fichier corrompu ne fasse pas exploser la mémoire ; ce n'est pas un plafond visible (décision 7). Code : `project::limits`, `Project::validate`.

## Versions, stockage et état d'écran

**Historique** : la pile d'annulation et de rétablissement des gestes de la session de travail ; chaque entrée garde l'inverse d'une édition, ou de toutes les éditions d'un groupe (un glisser de souris, un tour d'agent), et son auteur. Il vit en mémoire, n'est ni enregistré ni versionné. Code : `History`, `Entry`, `commands::run` ; commandes `history.undo`, `history.redo`, `history.begin_group`, `history.end_group`.

**Note jouée** : une note jouée en direct (clavier, MIDI) sur une piste ; elle va au moteur et ne change ni le projet ni la session. Code : commandes `live.note_on`, `live.note_off`, `live.all_notes_off`.

**Version** : un instantané nommé du projet, qu'un geste explicite sauve ; l'enregistrement automatique n'en crée pas. Vocabulaire de l'interface (décision 9) : on ne dit jamais « commit » à l'utilisateur. Code : commandes `version.save`, `version.list`, `version.restore` **(prévu)** ; un commit git sous le capot.

**Variante** : une exploration parallèle du projet, créée depuis l'état courant, que l'on peut comparer puis fusionner. Vocabulaire de l'interface (décision 9) : on ne dit jamais « branche » à l'utilisateur. Un agent travaille dans une variante. Code : commandes `variant.create`, `variant.switch`, `variant.compare`, `variant.merge` **(prévu)** ; une branche git sous le capot.

**Blob** : un fichier binaire du projet adressé par son contenu (SHA-256), jamais modifié : audio, automation très dense, état opaque d'un instrument. Code : `Hash` ; dossier `blobs/<sha256>` **(prévu)**.

**Session** : l'état qui relève de l'écran ou de la machine et non de la musique : solo, entrée MIDI choisie, sélection, zoom, défilement, pistes repliées. Il est écrit dans `.faulk/`, ignoré par git, restauré à la réouverture et jamais versionné. Le solo est donc un état de session, alors que le mute est dans le projet. À ne pas confondre avec la connexion de pairage entre la page et le helper (`/session`, `crates/protocol`) ni avec les stores web (`web/src/lib/stores/`), qui portent l'état de la page et son chemin vers le moteur, pas cet état de session. Code : `SessionState` ; `.faulk/session.json` **(prévu)**.

## Modes et extensions

**Message moteur** : ce que le projet envoie au moteur (`ControlMsg`, encodage binaire versionné), ce que `prepare` prépare hors chemin audio pour `apply` (`RtMsg`, jamais sérialisé, déplacé dans un anneau) et ce que le moteur renvoie à la page environ 30 fois par seconde (`Feedback`). Le projet nomme les structures du moteur par des emplacements (`TrackSlot`, `ClipSlot`…), jamais par ses identifiants. Code : `crates/engine/src/msg.rs` (contrat CT-3, codecs des messages) ; `Preparer::prepare` et `Engine::apply` dans `crates/engine/src/engine.rs` (MO-1).

**Compilateur projet → moteur** : ce qui traduit chaque commande appliquée en messages moteur, d'après la commande (notes jouées, lecture, arrêt, déplacement) et ses événements lus sur l'état d'après ; chaque `Outcome` porte ainsi ses `ControlMsg`, une annulation comprise. Code : `commands::engine_msgs::compile`, appelé par `Command::apply` (PR-3).

**Instantané** (du projet, pour le moteur) : les messages qui amènent un moteur vide au projet entier (réglages du transport, pistes, chaînes, clips, emplacements de la grille, arrangement, couloirs), précédés d'un `Reset` ; c'est ce qu'envoient l'ouverture d'un projet, la bascule Web ↔ Boost et la reprise d'un flux refusé. À ne pas confondre avec une version. Code : `commands::engine_msgs::snapshot`, `resync`, `Synced::setup` (PR-3).

**Table des emplacements** : la correspondance, tenue côté projet pour un moteur, entre les identifiants du projet et les emplacements moteur ; chaque entité reçoit le plus petit emplacement libre de son type (les emplacements libérés sont réutilisés), la table grandit sans plafond sous les limites du moteur et est reconstruite par chaque instantané. Elle vit dans le contexte des commandes, pas dans l'état : l'annulation rend l'état exact, pas la table. Code : `project::EngineSlots`, `SlotTable` (`crates/project/src/slots.rs`), `Context::engine` (PR-3).

**Mode Web** : Faulk dans le navigateur seul, le moteur tourne dans un `AudioWorklet`. Code : `crates/engine-wasm`.

**Mode Boost** : Faulk avec l'assistant natif (le helper), pour la latence professionnelle et les plugins tiers ; le moteur y est le même. Code : `crates/helper`.

**Atelier** : l'éditeur intégré où l'on crée ses instruments et effets. Code : `workshop` **(prévu)**.

**Module** : un instrument ou un effet créé dans l'Atelier ; il implémente le même contrat que les devices intégrés. Code : `DeviceSource::Module` ; `Module` **(prévu)**.
