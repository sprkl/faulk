---
type: Spec
title: "Atelier et modules"
description: "Proposal for the Atelier: anatomy of an audio plugin mapped to a minimal Faulk module ABI and manifest, synthesis and effect families, a three-level editor (block graph, Faust, Rust) with escape hatches, an AI create-listen-measure loop, sandboxing in both runtimes, and a concrete MVP."
status: draft
---

# Atelier et modules

> **Proposition ; décisions du fondateur du 2026-10-14 intégrées** là où elles s'appliquent (licence de faustwasm, § 7 et § 8). Ce document reprend l'étude du 2026-10-13 sur l'Atelier. Pour le reste il propose, il ne décide pas : les décisions à prendre sont à la fin. Les faits datés ont été vérifiés en ligne le 2026-10-13 ; ce qui n'a pas pu l'être est marqué « à vérifier ». Le [découpage du MVP](02-decoupage-mvp.md) place l'Atelier après le MVP.

## Résumé

1. **Un module Faulk est un petit plugin** : une fonction `process` appelée par blocs, des paramètres décrits (bornes, unité, échelle, lissage), des événements de notes datés à l'échantillon, un état sauvegardable, une latence et une traîne déclarées, et aucune interface propre. Il reprend la sémantique de CLAP (le format le plus simple et le plus explicite), avec un ABI WASM minimal propre à Faulk.
2. **Un seul binaire pour les deux modes.** Le module est compilé une fois en WASM ; le navigateur l'exécute dans l'`AudioWorklet`, le helper dans `wasmtime`. Mêmes octets : la parité entre les modes devient un objectif testable bit à bit, à condition que les fonctions mathématiques importées soient identiques dans les deux runtimes (voir § 5.4) ; sans cela, on la vérifie à une tolérance près.
3. **L'éditeur a trois niveaux reliés par des sorties de secours**, jamais par des murs : (a) le graphe de blocs, qui *est* du Faust ; (b) le code Faust, avec sa vue en diagramme ; (c) Rust compilé en WASM. On peut toujours passer au niveau suivant sans perdre son travail.
4. **L'IA écrit d'abord du Faust** : un langage petit, purement fonctionnel, qui se termine toujours, compile en quelques instants dans le navigateur, et dont la bibliothèque standard couvre presque toute la synthèse et les effets classiques. La boucle : décrire → générer → compiler → rendre hors ligne une batterie de tests → mesurer → écouter → ajuster.
5. **Des garde-fous non désactivables** entourent chaque module : purge des NaN, anti-subnormaux, limiteur de protection sur l'écoute, budgets mémoire et CPU, compteur d'instructions contre les boucles infinies.
6. **MVP** : modules Faust seulement, 15 blocs, 3 modules exemples (synthé soustractif, delay, distorsion), panneaux code, graphe en lecture, boutons, analyse, versions A/B. Rust, interfaces personnalisées, sidechain, spectral, échantillons et recréation par mesure viennent après.

## 1. Anatomie d'un plugin audio, et ce qu'un module Faulk en garde

### 1.1 Les concepts

| Concept | Ce que c'est | CLAP | VST3 | Module Faulk (proposition) |
| --- | --- | --- | --- | --- |
| Traitement | Fonction appelée sur le thread audio pour un bloc de N échantillons : lit entrées et événements datés, écrit les sorties. Ni allocation, ni verrou, ni E/S. | `clap_plugin.process(clap_process_t*)` : trames, buffers par port, files d'événements, transport. | `IAudioProcessor::process(ProcessData&)` : bloc variable jusqu'à `maxSamplesPerBlock`, `IParameterChanges`, `IEventList`. | `fk_process(frames, n_events)` : le module lit ses entrées et événements dans sa mémoire, écrit ses sorties. |
| Préparation | L'hôte fixe fréquence et taille maximale de bloc ; le plugin alloue tout à ce moment. | `activate(sample_rate, min_frames, max_frames)`, puis `start_processing`. | `setupProcessing`, `setActive`, `setProcessing(true)`. | `fk_init(sample_rate, max_frames)` : seul moment où le module peut allouer. |
| Buffers | Tableaux `f32` (parfois `f64`) non entrelacés, un par canal. | `clap_audio_buffer`. | `AudioBusBuffers`. | Canaux `f32` non entrelacés dans la mémoire du module, à des adresses exposées (`fk_in_ptr`, `fk_out_ptr`). |
| Fréquence et bloc | 44,1 / 48 / 96 kHz ; blocs de 32 à 2048 trames. Dans le navigateur, quantum de 128 trames, 64 avec `renderSizeHint` (voir les [contraintes](../constraints/01-contraintes.md)). | Taille variable, bornée. | Idem. | Toute taille entre 1 et `max_frames` : l'hôte peut découper un bloc pour placer un événement à l'échantillon près. |
| Paramètres | Valeurs continues ou discrètes avec identifiant stable, bornes, défaut, unité ; lissées côté plugin contre les clics. | `clap_param_info` : `id`, bornes, drapeaux `STEPPED`, `PERIODIC`, `HIDDEN`, `READONLY`, `BYPASS`, `ENUM`, `AUTOMATABLE…`, `MODULATABLE…`. | Valeurs normalisées 0–1, files de changements datés. | Décrits dans le manifeste (§ 1.4), en unités réelles (Hz, s, dB) : plus lisible pour un agent. |
| Automation | L'hôte envoie des événements de valeur datés dans le bloc. | `CLAP_EVENT_PARAM_VALUE` avec décalage. | `IParamValueQueue`. | Événements `ParamValue { offset, id, value }`. |
| Modulation | Décalage temporaire ajouté à la valeur, sans la modifier. | `CLAP_EVENT_PARAM_MOD`, éventuellement par note. | Pas d'équivalent aussi net. | Prévu dans l'ABI (`ParamMod`), ignoré au MVP. |
| État et préréglages | Le plugin sérialise son état. | Extensions `state`, `preset-load`, `preset-discovery`. | `getState/setState`. | Au MVP, l'état = les valeurs des paramètres, tenues par l'hôte. Plus tard : un blob opaque (`fk_state_save/load`). Un préréglage = un JSON de valeurs par identifiant. |
| Notes | Note on/off datés, vélocité, identifiant de note, expressions par note. | `note-ports`, `NOTE_ON/OFF/CHOKE/END`, `NOTE_EXPRESSION`. | `IEventList`. | `NoteOn { offset, note_id, key, velocity }`, `NoteOff`. MIDI brut traduit par l'hôte. |
| Voix | Allocation, vol de voix avec fondu court, retrigger. | Le plugin gère ses voix (`voice-info`). | Le plugin gère ses voix. | Deux régimes : **voix gérées par l'hôte** (le module décrit une voix, l'hôte instancie N copies et réutilise l'allocation du synthé du moteur) ; **voix gérées par le module** (niveau Rust : unisson, legato, glide). |
| Latence | Retard introduit (lookahead, FFT, suréchantillonnage à phase linéaire), compensé par l'hôte. | Extension `latency`. | `getLatencySamples()`. | `latency_samples` dans le manifeste ; `fk_latency()` si elle dépend des paramètres (un changement redémarre l'instance). |
| Traîne | Durée de son après une entrée silencieuse. | Extension `tail`. | `getTailSamples()`. | `tail` dans le manifeste (secondes ou `"infinite"`), plus détection de silence par l'hôte. |
| Bus | Entrées, sorties, sidechain. | `audio-ports` et variantes. | `setBusArrangements`. | MVP : instrument 0→2, effet 2→2 (un effet mono est dupliqué). Sidechain et sorties multiples plus tard. |
| Interface | Séparée du DSP ; ne touche jamais le thread audio. | Extension `gui` ; `webview` en brouillon. | Contrôleur séparé (`IEditController`). | Aucune interface propre au MVP : boutons générés depuis le manifeste. Plus tard : une vue web isolée qui ne parle que par commandes. |
| Transport | Tempo, position, lecture. | `clap_event_transport`. | `ProcessContext`. | Paramètres liés à l'hôte (`host_bindings` : `bpm`, `beat`, `playing`), écrits par l'hôte comme des paramètres cachés. |
| Threads | Quelle fonction sur quel thread. | Documenté par fonction. | Thread UI vs thread audio. | Le module n'a aucun thread ; tout est appelé par l'hôte, séquentiellement. |

### 1.2 Ce qui existe pour des plugins en WASM

- **WCLAP** (organisation WebCLAP, dépôts actifs en septembre 2026) : un plugin CLAP compilé en WASM, exécutable dans le navigateur (hôte de test en `AudioWorkletNode`) et en natif via un pont `wasmtime` qui annonce environ 1,5 fois le CPU d'un build natif. Exige `clap_entry`, une table de fonctions extensible, un `malloc` exporté ; WASI 0.1 optionnel.
- **Web Audio Modules 2.0** : un standard de plugins web en JavaScript autour de l'`AudioWorkletProcessor`.
- **faustwasm** (`@grame/faustwasm`, 0.19.0, LGPL-3.0) : le compilateur Faust en WASM, qui compile du Faust en module WASM dans le navigateur, avec nœuds mono et polyphoniques, rendu hors ligne et diagrammes SVG.

**Recommandation.** Ne pas adopter WCLAP comme ABI interne : l'ABI C de CLAP (pointeurs de structures, table de fonctions, `malloc` exporté, WASI) est lourd à héberger proprement dans un worklet. Faulk définit un ABI minimal `fk_module` dont la sémantique se calque sur CLAP, pour qu'un adaptateur WCLAP ↔ Faulk reste possible plus tard : une sortie de secours, pas une dépendance.

### 1.3 L'ABI proposé (v0)

Des fonctions C simples. Le module n'importe qu'une bibliothèque mathématique fournie (§ 5.4).

```text
fk_abi_version() -> u32                       // 0 for this version
fk_init(sample_rate: f32, max_frames: u32) -> i32   // allocates everything; 0 = ok
fk_reset()                                     // clears DSP state (not parameters)
fk_in_ptr(channel: u32) -> u32                 // input buffer address (f32 × max_frames)
fk_out_ptr(channel: u32) -> u32                // output buffer address
fk_events_ptr() -> u32                         // event queue address (fixed capacity)
fk_process(frames: u32, n_events: u32)         // processes one block
fk_latency() -> u32                            // optional
fk_state_size() / fk_state_save(ptr) / fk_state_load(ptr, len)   // optional, after the MVP
```

Événement (16 octets, aligné) : `offset: u32, kind: u16, flags: u16, a: u32, b: f32`, où `kind` ∈ `NoteOn` (a = note_id << 8 | key, b = vélocité 0–1), `NoteOff`, `ParamValue` (a = id, b = valeur), `ParamMod` (réservé), `Choke`.

**Limite à trancher avant de figer l'ABI v0 : `note_id` n'a que 24 bits** dans cet encodage, puisque les 8 bits faibles de `a` portent `key`. Un identifiant au-delà de `0xFFFFFF` serait tronqué et pourrait désigner une autre note. Deux voies, à choisir avec la largeur de `NoteId` dans le contrat de commande (CT-4 du [découpage du MVP](02-decoupage-mvp.md)) : (a) borner `NoteId` à 24 bits, validé par la commande et par l'hôte avant l'encodage (l'hôte peut aussi renuméroter les notes actives en identifiants de voix courts, qui ne dépassent jamais le nombre de notes tenues) ; (b) donner des champs séparés à l'identifiant et à la touche (par exemple la touche dans `flags`, ou un événement de 20 ou 24 octets). Tant que ce choix n'est pas fait, l'ABI v0 n'est pas figé.

Un module Faust n'implémente pas cet ABI : le backend WASM de Faust a ses propres exports (`init`, `compute`, paramètres à des adresses données par son JSON). L'hôte a donc **deux saveurs** derrière un même trait `ModuleInstance` : `faust-wasm` (adaptateur qui lit le JSON de Faust) et `fk-abi` (Rust et autres langages). Les deux tournent dans les deux runtimes. Côté moteur, `ModuleInstance` s'adapte au contrat `Device` que fige la tâche CT-5 du découpage.

### 1.4 Le manifeste

Un fichier `module.json`, validé par un schéma JSON généré depuis une structure Rust (`serde` + `schemars`, comme `crates/commands`). Pour un module Faust, il est **généré** depuis le JSON du compilateur (paramètres, bornes, métadonnées `[unit:Hz]`, `[scale:log]`) ; l'utilisateur ou l'agent ne surcharge que ce que Faust ne dit pas.

```json
{
  "faulk_module": "0.1",
  "id": "local.sous-tractif",
  "name": { "fr": "Sous-tractif", "en": "Subtractive" },
  "kind": "instrument",
  "source": { "lang": "faust", "entry": "main.dsp", "compiler": "faust 2.x (to pin)" },
  "binary": { "flavor": "faust-wasm", "sha256": "…" },
  "audio": { "inputs": 0, "outputs": 2 },
  "notes": { "voices": 8, "voice_manager": "host", "voice_params": { "freq": "freq", "gain": "gain", "gate": "gate" } },
  "params": [
    {
      "id": "cutoff",
      "path": "/Sous-tractif/Cutoff",
      "name": { "fr": "Coupure", "en": "Cutoff" },
      "min": 40, "max": 18000, "default": 1200,
      "unit": "Hz", "scale": "log", "smoothing_ms": 10,
      "automatable": true, "modulatable": true, "group": "Filtre",
      "description": "Low-pass cutoff before resonance."
    }
  ],
  "host_bindings": { "bpm": "bpm" },
  "latency_samples": 0,
  "tail": { "seconds": 2.0 },
  "budget": { "memory_mb": 16, "cpu_us_per_128": null },
  "lineage": { "parent": "sha256:…", "author": "agent:…", "prompt_ref": "…" },
  "license": "MIT"
}
```

Règles :

- **L'identifiant d'un paramètre est stable** d'une version à l'autre (dérivé du chemin Faust, figé à la première version) : rechargement à chaud sans perdre automation ni réglages, comparaison A/B paramètre par paramètre. C'est le même identifiant que celui du fichier de device du projet ([modèle de projet](../architecture/02-modele-projet.md)).
- **La description des paramètres est écrite pour un agent**, comme celle des commandes.
- `budget.cpu_us_per_128` est **mesuré** par Faulk à la compilation (§ 5), jamais déclaré par l'auteur.

## 2. Synthèse et effets

Pour chaque technique : ce qu'elle produit, ses briques, et où les trouver dans les bibliothèques Faust (préfixes : `os` oscillateurs, `no` bruits, `en` enveloppes, `fi` filtres, `ve` effets analogiques virtuels, `de` délais, `re` réverbérations, `co` compresseurs, `pf` phaser et flanger, `pm` modélisation physique, `aa` anti-aliasing, `an` analyse, `sy` synthés, `dx` FM à opérateurs, `ef` effets divers, `so` fichiers son ; liste complète : <https://faustlibraries.grame.fr/>).

### 2.1 Synthèse

| Technique | Principe | Bonne pour | Briques | Faust | Coût |
| --- | --- | --- | --- | --- | --- |
| **Soustractive** | Forme d'onde riche filtrée par un filtre résonant, mise en forme par enveloppes et LFO. | Basses, nappes, leads ; la plus intuitive. | Oscillateurs à bande limitée (PolyBLEP, tables), bruit, filtre SVF ou ladder, ADSR, LFO, VCA, glide. | `os.sawtooth`, `os.square`, `no.noise`, `fi.resonlp`, `ve.moog_vcf`, `en.adsr`, `os.lf_*` | Faible |
| **Additive** | Somme de sinusoïdes contrôlées chacune en fréquence et amplitude. | Orgues, cloches, sons évolutifs, resynthèse. | Banque de sinus, enveloppes par partiel, lois d'amplitude, inharmonicité. | `par(i, N, os.osc(f*(i+1)) * a(i))` | Croît avec les partiels |
| **FM / PM** | Un modulateur fait varier la fréquence ou la phase d'une porteuse ; ratio et index fixent le spectre. Algorithmes à 4–6 opérateurs. | Cloches, pianos électriques, basses métalliques, percussions. Riche pour peu de CPU, peu intuitive. | Opérateurs sinus à phase modulable, enveloppes par opérateur, rétroaction, matrice d'algorithmes. | `os.osc` avec phase, `dx.*`, `sy.fm` | Faible |
| **Table d'ondes** | Lecture d'un cycle stocké, avec balayage entre tables. | Pads évolutifs, basses « numériques ». | Tables à bande limitée par octave, interpolation, balayage, import de cycles. | `rdtable`, `rwtable`, `soundfile` | Faible, mémoire moyenne |
| **Granulaire** | Grains fenêtrés (10–100 ms) rejoués avec position, hauteur, densité, dispersion aléatoires. | Textures, étirement temporel, glitch. | Tampon, planificateur de grains, fenêtres, aléatoire. | Faisable mais à l'étroit ; plus naturel en Rust. | Moyen à élevé |
| **Échantillons** | Lecture d'enregistrements par note et vélocité, boucles, enveloppes. | Réalisme, batteries, découpage. | Décodage, lecteur à interpolation, zones, boucles. | `soundfile` | Faible CPU, mémoire forte |
| **Modélisation physique** | Simuler l'objet : Karplus-Strong, guides d'ondes, modal. | Cordes, vents, percussions, objets imaginaires très expressifs. | Délais fractionnaires, filtres d'amortissement, allpass, résonateurs, excitateurs, non-linéarités. | `pm.*`, `fi.fb_fcomb`, `pm.modalModel` (à vérifier) | Faible à moyen |
| **Bruit et percussion** | Bruits filtrés et enveloppés, hauteur qui chute, métal par carrés inharmoniques. | Batteries synthétiques. | Bruit blanc/rose, enveloppes rapides, passe-bande, saturation. | `no.*`, `en.ar`, `fi.bandpass` | Faible |
| **Vectorielle** | Mélange de 2 à 4 sources piloté par un point 2D. | Pads mouvants, transitions. | Mélangeur 4 voies, trajectoire 2D. | Mélange pondéré | Celui des sources |
| **Distorsion de phase** | Sinus lu avec une phase déformée, qui enrichit le spectre comme un filtre qui s'ouvre. | Sons numériques, basses. | Accumulateur de phase, fonction de déformation, enveloppe. | À écrire (quelques lignes) | Faible |

Tout se ramène à **un petit nombre de briques transversales** : accumulateur de phase et oscillateurs à bande limitée, bruit, enveloppes, filtres, délais fractionnaires, tables interpolées, non-linéarités, mélangeurs. Une technique « manquante » est presque toujours une manière de câbler ces briques, souvent avec une **rétroaction d'un échantillon** (Karplus-Strong, FM à rétroaction, filtres) : l'éditeur doit la permettre (§ 3).

### 2.2 Effets

| Famille | Variantes | Points délicats | Faust |
| --- | --- | --- | --- |
| **Filtres** | SVF (topologie TPT, stable en modulation rapide, sorties simultanées) ; ladder 4 pôles ; peigne ; bi-quads d'égalisation. | Modulation audio-rate stable ; résonance auto-oscillante bornée. | `fi.svf.*` (à vérifier), `fi.resonlp`, `ve.moog_vcf`, `fi.fb_fcomb`, `fi.ff_fcomb`, `fi.peak_eq`, `fi.low_shelf` |
| **Delays** | Simple, ping-pong, multi-taps, synchronisé, à bande, modulé. | Interpolation pour changer le temps sans clic ; taille maximale fixée d'avance. | `de.delay`, `de.fdelay`, `de.sdelay` |
| **Réverbérations** | Schroeder ; Freeverb ; FDN (lignes mélangées par une matrice unitaire) ; convolution. | Coloration métallique des petites tailles ; la convolution demande une FFT partitionnée. | `re.mono_freeverb`, `re.stereo_freeverb`, `re.jcrev`, `re.satrev`, `re.fdnrev0`, `re.zita_rev1_stereo` (à vérifier) |
| **Dynamique** | Compresseur, limiteur (lookahead), gate, expandeur, sidechain. | Le lookahead ajoute de la latence ; détection en log ; lissage sans pompage. | `co.compressor_stereo`, `co.limiter_*`, `ef.gate_mono` (à vérifier), `an.amp_follower` |
| **Distorsion** | tanh, arctan, soft clip, hard clip, wavefolder, redresseur, bitcrusher, avec pré- et post-filtres. | **Aliasing** : remèdes par suréchantillonnage (×2 à ×8) ou anti-aliasing par antidérivée (ADAA). | `aa.*`, `ef.cubicnl`, `ma.tanh` |
| **Modulation** | Chorus, flanger, phaser, trémolo, vibrato, rotary. | Interpolation des délais modulés ; LFO stéréo déphasés. | `pf.flanger_stereo`, `pf.phaser2_stereo`, `ef.*` |
| **Hauteur et temps** | Pitch shift par délai, harmoniseur, time-stretch. | La qualité demande du spectral ou du granulaire soigné. | `ef.transpose` |
| **Spectral (FFT)** | Vocodeur de phase, gel spectral, débruitage, morphing. | STFT avec recouvrement, latence d'un bloc FFT, traitement par trame. | Faust y est mal adapté : **premier cas qui justifie le niveau Rust**. |

Références : J. O. Smith, *Physical Audio Signal Processing* ; bibliothèques Faust, réverbérations et modélisation physique (voir les sources).

## 3. Un éditeur sans plafond

### 3.1 Pourquoi un éditeur de blocs tourne en rond

1. **Le jeu de blocs est fermé** : il manque toujours le bloc nécessaire, et on ne peut pas l'écrire.
2. **Il n'y a pas de sortie** : le patch ne peut pas devenir du code.
3. **Il interdit les boucles d'un échantillon** : pas de Karplus-Strong, pas de filtre maison, pas de FM à rétroaction.
4. **Il confond le par-voix et le global** (une réverbération par voix coûte 8 fois plus).
5. **Il cache le signal** : pas de scope, pas de spectre, pas de valeur aux points du graphe.
6. **Il n'a ni tables, ni échantillons, ni trames.**

L'Atelier répond point par point : blocs ouverts (un bloc = une fonction Faust que chacun peut écrire), passage au niveau suivant, rétroaction native (opérateur `~`), séparation explicite voix / effet, sondes sur les fils, et un niveau Rust pour ce qui sort du flux d'échantillons.

### 3.2 Trois niveaux, une source de vérité par module

```
 (a) Graphe de blocs ──génère──▶ (b) Code Faust ──compile (libfaust WASM)──▶ module.wasm
        ▲   vue éditable du            │  vue diagramme (SVG Faust),          ▲
        │   sous-ensemble « patch »    │  toujours disponible                 │
        └──────── relu si le code ─────┘                                      │
                  reste dans le sous-ensemble                                 │
                                       └─éjection : faust -lang rust ─▶ (c) Rust ──cargo ─▶ module.wasm
```

- **(a) Graphe.** Chaque bloc est une fonction Faust de la bibliothèque Faulk ; chaque fil est une composition Faust (`:` séquence, `,` parallèle, `<:` répartition, `:>` fusion, `~` rétroaction). Le graphe est **stocké comme du Faust** (positions des nœuds en métadonnées) : un patch est toujours un programme valide et versionnable.
- **(b) Faust.** Le code est la source de vérité. Tant qu'il reste dans le sous-ensemble « patch » (des appels de blocs reliés par les cinq opérateurs), le graphe reste éditable. S'il en sort, le graphe passe en lecture seule, remplacé par le diagramme SVG de Faust ; on ne perd que l'édition graphique, et l'interface le dit.
- **Un bloc maison.** Une fonction Faust écrite dans le code (ou par l'agent) devient un bloc dès qu'elle a une signature et des métadonnées : le jeu de blocs est ouvert.
- **(c) Rust.** Pour ce que Faust exprime mal : traitement par trames (FFT), planification de grains, gestion de voix propre, structures de données, chargement d'échantillons. Deux entrées :
  - **éjection** : le backend Rust du compilateur Faust (`-lang rust`) donne un point de départ ;
  - **écriture directe** avec un SDK `faulk-module` (macros pour l'ABI, le manifeste, le lissage, l'anti-subnormaux) et, au choix, **fundsp** (0.23, MIT/Apache-2.0, `no_std` possible, graphes par opérateurs vérifiés à la compilation, oscillateurs, filtres, réverbérations, suréchantillonnage, analyse de réponse ; le frontal `Net` préalloue avant le temps réel).
- **Deux graphes à ne pas confondre** : le graphe *dans* un module (Faust, à l'échantillon, rétroaction permise) et le graphe *entre* modules (la chaîne d'une piste, gérée par le moteur, par blocs). Un module Rust est un nœud du second.

**Compiler Rust dans le navigateur n'est pas réaliste à court terme.** Le niveau (c) passe par une machine qui a `cargo` : le helper (il compile, surveille, recharge à chaud) ou l'agent ACP local. En mode Web seul, on charge un `.wasm` déjà compilé. Piste pour plus tard : un langage compilable dans le navigateur (AssemblyScript, qui a déjà une bibliothèque WCLAP `as-clap`).

### 3.3 La bibliothèque standard de blocs

Une **façade mince** sur les bibliothèques Faust : des noms stables, des unités réelles, des paramètres lissés par défaut, une description courte pour l'agent. Exemple : `fk.lowpass(cutoff_hz, resonance)` appelle le filtre choisi, lisse ses entrées, borne la résonance. Si Faust change, la façade absorbe le changement.

Contenu du MVP : § 6.2. Ensuite, par ordre de besoin : tables d'ondes avec mipmaps, FM à 4 opérateurs, résonateur modal, guide d'ondes de corde, chorus/flanger/phaser, gate, égaliseur paramétrique, bitcrusher, lecteur d'échantillons, suréchantillonnage générique.

## 4. La création assistée par IA

### 4.1 La boucle

```
décrire ─▶ l'agent écrit du Faust ─▶ compilation (navigateur) ─┬─ erreur ─▶ renvoyée telle quelle à l'agent
                                                              └─ ok ─▶ rendu hors ligne d'une batterie de tests
                                                                        ─▶ mesures (JSON + images)
                                                                        ─▶ garde-fous : rejet si NaN, explosion, silence, CPU
                                                                        ─▶ échange à chaud dans l'écoute (fondu)
                                                                        ─▶ l'utilisateur joue, tourne les boutons, commente
                                                                        ─▶ l'agent lit mesures + réglages + commentaires ─▶ itération
```

1. **Faust d'abord.** Pas de boucle non bornée, pas d'allocation à l'exécution : le code généré est sûr par construction et compile vite. Les modèles le connaissent moins bien que Rust : on compense par la façade `fk.*`, des exemples dans le contexte de l'agent et les erreurs de compilation renvoyées intégralement.
2. **Tout passe par des commandes** : `module.create`, `module.update_source`, `module.compile`, `module.render_tests`, `module.analyze`, `module.set_param`, `module.version.commit`, `module.ab.select`, `module.preset.save`. L'agent de l'Atelier (ACP), un agent externe (MCP) et l'interface utilisent exactement ces commandes.
3. **L'agent ne règle pas les nombres au jugé.** Il propose la structure et des ordres de grandeur ; quand une cible est mesurable (§ 4.6), un optimiseur numérique (Nelder-Mead, CMA-ES) ajuste les paramètres.
4. **L'oreille de l'utilisateur reste juge** : les mesures éliminent les versions cassées ; le choix se fait à l'écoute, en A/B à volume égalisé.

### 4.2 Batterie de tests et analyses

Rendu hors ligne (le même moteur, sans temps réel ; faustwasm a un processeur hors ligne) d'une batterie fixe, pour que deux versions soient comparables :

| Pour un instrument | Pour un effet |
| --- | --- |
| Notes isolées C1, C3, C5, C7 à vélocités 30, 100, 127 ; accord de 8 notes (vol de voix) ; note longue ; relâche ; répétitions rapides. | Impulsion ; balayage sinusoïdal logarithmique ; sinus de −40 à +6 dBFS ; bruit rose ; boucle de batterie et voix de référence ; silence (traîne, bruit propre). |

Mesures (JSON lisible par l'agent, plus spectrogramme et forme d'onde) :

- **Sûreté** : NaN/Inf, crête, DC, subnormaux, silence inattendu, explosion.
- **Niveau** : crête, RMS, sonie intégrée approchée, plage dynamique.
- **Spectre** : centroïde et son évolution, harmoniques et inharmonicité, bande occupée, indicateur d'aliasing.
- **Temps** : attaque, décroissance, relâche estimées ; traîne mesurée.
- **Effets** : réponse en fréquence, courbe de transfert statique, THD par niveau, latence mesurée.
- **CPU** : microsecondes par bloc de 128 trames à 48 kHz, rapportées au budget de 2,67 ms.

### 4.3 Paramètres, boutons, préréglages

- **Les boutons viennent du code** : chaque `hslider`, `vslider`, `nentry`, `checkbox` Faust, avec ses métadonnées (`[unit:Hz]`, `[scale:log]`, `[style:menu{…}]`), devient un bouton du manifeste. L'agent est guidé pour exposer *peu* de paramètres utiles.
- **Préréglages générés** : l'agent propose des jeux de valeurs nommés, rendus et mesurés comme le reste ; seuls ceux qui passent les garde-fous sont proposés.
- **Le retour de l'utilisateur est une donnée** : un bouton tourné puis « mieux » est transmis à l'agent, qui peut changer le défaut ou la plage.

### 4.4 Garde-fous

Ils vivent dans l'hôte, autour du module, et le code du module ne peut pas les retirer :

| Garde-fou | Où | Effet |
| --- | --- | --- |
| Purge NaN/Inf | Sortie de chaque module | Remplace par 0, coupe le module avec un fondu, remonte une erreur à l'Atelier et à l'agent. |
| Anti-subnormaux | Sortie + options de compilation | Faust compilé avec `-ftz 2` ; SDK Rust avec une fonction dédiée ; vidage en sortie. |
| Limiteur de protection | Bus d'écoute de l'Atelier | Plafond à −1 dBFS, coupure si la crête dépasse longtemps. Pas dans le rendu d'un projet (il changerait le son). |
| Détecteur de DC | Analyse | Avertit et propose `fi.dcblocker` plutôt que de filtrer d'office. |
| Budget CPU | Compilation (mesure hors ligne) + exécution native | Refuse l'insertion au-delà du budget ; coupe un module qui dépasse durablement en natif. |
| Budget mémoire | Instanciation | Mémoire WASM bornée (`maximum`), croissance interdite pendant `process`. |
| Volume égalisé en A/B | Comparaison | Même sonie des deux côtés, pour ne pas confondre « plus fort » et « mieux ». |

### 4.5 Versions et A/B

- Chaque compilation réussie crée une **version immuable**, identifiée par le hachage (source + version du compilateur + manifeste), avec son parent, son auteur (utilisateur ou agent), la demande qui l'a produite et ses mesures. L'historique est un arbre.
- **A/B** : les deux versions tournent en parallèle (le CPU double pendant la comparaison) ; bascule avec un fondu de 10 à 20 ms, à volume égalisé ; les paramètres de même identifiant sont synchronisés. Une écoute à l'aveugle (ABX) est simple à ajouter.
- **Rechargement à chaud** : la nouvelle instance est initialisée hors du chemin audio (compilation WASM asynchrone), reçoit les valeurs des paramètres par identifiant, puis remplace l'ancienne par fondu. L'état interne (voix en cours, queue de réverbération) n'est pas transféré au MVP.

### 4.6 Recréer par mesure

Seulement par mesure, jamais par décompilation, et sans nom, marque ni apparence de l'original ([contraintes](../constraints/01-contraintes.md)). L'original étant un plugin tiers, cela demande le **mode Boost** (le helper l'héberge et le mesure).

1. **Mesurer l'original** :
   - **balayage sinusoïdal exponentiel** (méthode de Farina) : réponse impulsionnelle linéaire et distorsions harmoniques par ordre, séparées dans le temps ;
   - **sinus par paliers** : courbe de transfert statique, THD selon le niveau ;
   - **deux tons** : intermodulation ;
   - **sauts de niveau** : attaque, relâche, ratio et genou d'un compresseur ;
   - **bruit et extraits musicaux** : comportement global ;
   - pour un instrument : notes sur toute la tessiture et plusieurs vélocités.
   - Chaque mesure est refaite pour plusieurs positions des boutons : on mesure une *fonction* des réglages.
2. **Choisir une structure** : l'agent propose une topologie à partir des mesures.
3. **Ajuster** : un optimiseur règle les paramètres pour minimiser la distance.
4. **Distances** : erreur de réponse en fréquence (lissée par tiers d'octave), erreur de courbe de transfert, écart de THD, écart des temps d'enveloppe, distance spectrale multi-résolution en échelle log-mel. Aucune n'est une vraie mesure perceptive : le seuil « acceptable » se fixe à l'écoute.
5. **Rapport** : courbes de l'original et de la recréation superposées, pour chaque réglage mesuré.

## 5. Isolation et sûreté temps réel

### 5.1 Ce que WASM garantit, et ce qu'il ne garantit pas

- Garanti : un module ne lit ni n'écrit hors de sa mémoire, n'appelle que ce qu'on lui importe, et une faute devient une **trappe** que l'hôte rattrape.
- Non garanti : qu'il se termine, qu'il soit rapide, qu'il n'alloue pas pendant `process`, qu'il ne produise ni NaN ni subnormaux. WASM n'offre **aucun contrôle FTZ/DAZ** ; seuls les bits des NaN ne sont pas déterministes. Les instructions *relaxed SIMD* sont non déterministes : à interdire dans les modules pour garder la parité.

### 5.2 Dans le navigateur (AudioWorklet)

- Une instance WASM **par module**, avec sa `WebAssembly.Memory` bornée par `maximum`, et une liste blanche d'imports (bibliothèque mathématique seulement).
- Compilation **asynchrone** à partir des octets ; jamais `new WebAssembly.Module` synchrone dans le worklet.
- Le moteur et les modules sont des instances séparées : le worklet copie les buffers entre leurs mémoires à chaque bloc. Coût faible a priori, à mesurer.
- Une trappe pendant `process` est rattrapée dans le `process()` du worklet : le module est contourné (bypass pour un effet, silence pour un instrument), l'erreur remonte à l'interface.
- **Pas de préemption** : tous les modules partagent le thread de rendu ; un module lent fait craquer la session, une boucle infinie la bloque. Faust ne peut pas boucler indéfiniment. Pour les modules Rust, l'hôte **instrumente le binaire au chargement** : un compteur décrémenté à chaque arrière de boucle et à chaque appel, réarmé à chaque bloc, qui déclenche une trappe à épuisement (« gas metering »). Il fonctionne pareil dans les deux runtimes.
- Mesure du CPU : la disponibilité de `performance.now()` dans `AudioWorkletGlobalScope` n'est pas documentée : **à vérifier**. Le budget repose d'abord sur la **mesure hors ligne** à la compilation, et en direct sur les signaux indirects (sous-alimentation du rendu).

### 5.3 Dans le helper (wasmtime)

- `wasmtime` embarqué dans le moteur natif, une `Store` par instance, avec un `ResourceLimiter` (mémoire bornée, croissance refusée pendant `process`).
- Interruption : *fuel* (déterministe, plus coûteux) ou *epochs* (moins coûteux, non déterministe). Pour la parité, le même compteur instrumenté que dans le navigateur, avec les epochs comme filet.
- Coût : environ 1,5 × le CPU natif annoncé par le pont WCLAP ; à mesurer sur nos modules.
- Les plugins tiers natifs ne sont pas des modules : leur isolation (processus séparé) est un autre sujet.

### 5.4 Parité bit à bit entre les modes

Si les deux runtimes exécutent les mêmes octets sans relaxed SIMD, les résultats ne peuvent différer que par les **fonctions mathématiques importées**. Le backend WASM de Faust importerait les fonctions mathématiques de l'hôte (en JS, `Math.sin`…) : **à vérifier dans la sortie de faustwasm**. Si c'est le cas, Faulk fournit ces fonctions dans un petit module WASM de `libm` commun aux deux runtimes, et la parité redevient testable par comparaison exacte.

### 5.5 Allocation pendant `process`

Un module alloue dans `fk_init`, jamais dans `process`. Comparer la taille de la mémoire avant et après `process` ne détecte que `memory.grow` : un allocateur peut allouer et libérer dans des pages déjà réservées sans que cette taille change. Ce contrôle ne prouve donc pas l'absence d'allocation ; il n'est qu'un filet bon marché, avec le refus de `memory.grow` pendant `process` (`ResourceLimiter` en natif, `maximum` dans le navigateur). Le contrôle effectif est fait dans les tests :

- **modules Rust** : le SDK `faulk-module` installe, dans ses builds de test, un allocateur global instrumenté qui compte les allocations et les libérations ; le test de l'Atelier exécute la batterie de rendu et échoue si un compteur bouge pendant `fk_process` (même principe que le détecteur d'allocation du moteur) ;
- **modules Faust** : le code généré n'a pas d'allocateur (sa mémoire est dimensionnée à la compilation, délais compris) ; le test vérifie que le module n'exporte ni n'importe de fonction d'allocation, en plus du contrôle de taille.

## 6. Proposition de MVP pour l'Atelier

### 6.1 Périmètre

- Modules **Faust seulement**, compilés dans le navigateur par faustwasm, exécutés par le moteur dans l'`AudioWorklet` ; le même `.wasm` dans le helper via wasmtime dès que le helper charge des modules.
- Instruments (voix gérées par l'hôte, convention Faust `freq`/`gain`/`gate`, effet global optionnel `effect = …`) et effets stéréo.
- L'agent de l'Atelier écrit et modifie le code ; l'utilisateur aussi.
- Boucle complète : compiler, tester, mesurer, écouter, versionner, comparer en A/B.

### 6.2 Les 15 premiers blocs (`fk.*`)

| # | Bloc | Rôle | Base Faust |
| --- | --- | --- | --- |
| 1 | `fk.note` | Entrées de voix `freq`, `gain`, `gate`, glide optionnel | convention polyphonique Faust |
| 2 | `fk.osc(shape, freq)` | Sinus, scie, carré/pulse, triangle à bande limitée | `os.osc`, `os.sawtooth`, `os.square`, `os.triangle` |
| 3 | `fk.noise(color)` | Bruit blanc, rose | `no.noise`, `no.pink_noise` |
| 4 | `fk.adsr` | Enveloppe ADSR (secondes) | `en.adsr` |
| 5 | `fk.lfo(shape, rate)` | LFO, synchronisable via `bpm` | `os.lf_*` |
| 6 | `fk.lowpass` / `fk.highpass` / `fk.bandpass` | Filtre SVF résonant, modulable | filtres SVF, `fi.resonlp` |
| 7 | `fk.ladder` | Passe-bas 4 pôles résonant | `ve.moog_vcf` |
| 8 | `fk.gain`, `fk.pan`, `fk.mix` | VCA, panoramique, mélange sec/mouillé | basiques |
| 9 | `fk.delay(time, feedback)` | Délai fractionnaire, rétroaction, filtre dans la boucle | `de.sdelay`, `de.fdelay` |
| 10 | `fk.drive(shape, amount)` | Saturation tanh / soft / hard / fold, ADAA | `aa.*` |
| 11 | `fk.tone`, `fk.dcblock` | Pente simple, coupe DC | `fi.lowpass`, `fi.dcblocker` |
| 12 | `fk.comp`, `fk.limit` | Compresseur et limiteur stéréo | `co.*` |
| 13 | `fk.reverb(size, damp)` | Réverbération FDN stéréo | `re.zita_rev1_stereo` ou `re.fdnrev0` |
| 14 | `fk.comb`, `fk.allpass` | Résonance (Karplus-Strong, phaser, réverbérations maison) | `fi.fb_fcomb`, `fi.allpass_comb` |
| 15 | `fk.smooth`, `fk.follow`, `fk.sh` | Lissage, suiveur d'enveloppe, échantillonneur-bloqueur | `si.smoo`, `an.amp_follower`, `ba.sAndH` |

Les noms exacts des fonctions Faust sont à confirmer au premier compile ; la façade `fk.*` sert justement à absorber ces écarts.

### 6.3 Les trois modules exemples

Esquisses : la structure est le message, les signatures exactes seront validées par le compilateur. Ils servent aussi de **tests de référence** : rendus hors ligne dans les deux modes, ils fondent les tests de parité et de non-régression.

**1. Sous-tractif (instrument).** Deux oscillateurs désaccordés + bruit, passe-bas résonant avec enveloppe de filtre, ADSR d'amplitude, LFO sur la coupure.

```faust
declare name "Sous-tractif";
declare options "[midi:on][nvoices:8]";
import("stdfaust.lib");

freq = hslider("freq[hide:1]", 220, 20, 20000, 0.01);
gain = hslider("gain[hide:1]", 0.5, 0, 1, 0.01);
gate = button("gate[hide:1]");

detune = hslider("h:Osc/Detune[unit:cents]", 7, 0, 50, 0.1) : si.smoo;
noiseLvl = hslider("h:Osc/Noise", 0, 0, 1, 0.01) : si.smoo;
cutoff = hslider("h:Filter/Cutoff[unit:Hz][scale:log]", 1200, 40, 18000, 1) : si.smoo;
reso = hslider("h:Filter/Resonance", 2, 0.7, 12, 0.01) : si.smoo;
envAmt = hslider("h:Filter/Env amount[unit:oct]", 2, 0, 6, 0.01);
lfoRate = hslider("h:Filter/LFO rate[unit:Hz][scale:log]", 0.5, 0.05, 20, 0.01);
lfoAmt = hslider("h:Filter/LFO amount[unit:oct]", 0, 0, 3, 0.01);

ampEnv = en.adsr(0.005, 0.3, 0.7, 0.4, gate);
fltEnv = en.adsr(0.002, 0.25, 0.2, 0.3, gate);

oscs = os.sawtooth(freq) + os.square(freq * pow(2, detune / 1200)) + no.noise * noiseLvl;
fc = min(cutoff * pow(2, envAmt * fltEnv + lfoAmt * os.osc(lfoRate)), 18000);

process = oscs * 0.33 : fi.resonlp(fc, reso, 1) : *(ampEnv * gain) <: _, _;
effect = _, _;   // global, after voices: e.g. fk.reverb later
```

**2. Echo (effet).** Temps en division de noire calé sur le tempo de l'hôte, rétroaction filtrée, mélange.

```faust
declare name "Echo";
import("stdfaust.lib");

bpm = hslider("bpm[hide:1][faulk:host:bpm]", 120, 20, 300, 0.01);
division = nentry("Division[style:menu{'1/4':1;'1/8':0.5;'1/8 pointée':0.75;'1/16':0.25}]", 0.75, 0.25, 1, 0.25);
fb = hslider("Feedback", 0.45, 0, 0.95, 0.01) : si.smoo;
tone = hslider("Tone[unit:Hz][scale:log]", 4000, 200, 18000, 1) : si.smoo;
mix = hslider("Mix", 0.3, 0, 1, 0.01) : si.smoo;

maxDelay = 262144; // samples, fixed at compile time
delaySamples = 60 / bpm * division * ma.SR : si.smoo;

echo = (+ : de.sdelay(maxDelay, 1024, delaySamples)) ~ (fi.lowpass(1, tone) : *(fb));
channel(x) = x * (1 - mix) + echo(x) * mix;

process = channel, channel;
```

Version suivante : ping-pong par rétroaction croisée (`ro.cross(2)`), modulation légère du temps.

**3. Grain (effet de distorsion).** Pré-accentuation, saturation anti-aliasée, filtre de tonalité, compensation de gain, mélange.

```faust
declare name "Grain";
import("stdfaust.lib");

drive = hslider("Drive[unit:dB]", 12, 0, 48, 0.1) : ba.db2linear : si.smoo;
tone = hslider("Tone[unit:Hz][scale:log]", 6000, 500, 18000, 1) : si.smoo;
outDb = hslider("Output[unit:dB]", 0, -24, 12, 0.1) : ba.db2linear : si.smoo;
mix = hslider("Mix", 1, 0, 1, 0.01) : si.smoo;

shaper = aa.tanh1; // first-order antiderivative anti-aliasing
wet = fi.dcblocker : *(drive) : shaper : fi.lowpass(2, tone) : /(sqrt(drive));
channel(x) = (x * (1 - mix) + wet(x) * mix) * outDb;

process = channel, channel;
```

### 6.4 Les panneaux de l'Atelier

| Panneau | Contenu |
| --- | --- |
| Agent | Discussion avec l'agent (ACP via le helper ; plus tard WebMCP), commandes exécutées visibles et annulables en une fois par tour. |
| Code | Éditeur Faust (coloration, erreurs du compilateur au bon endroit, complétion sur `fk.*` et les bibliothèques Faust). |
| Graphe | Au MVP : le diagramme SVG de Faust en lecture, avec navigation dans les sous-blocs. Édition graphique du sous-ensemble « patch » après le MVP. |
| Boutons | Générés depuis le manifeste, groupés (`h:`/`v:` de Faust). |
| Écoute | Clavier (souris, ordinateur, MIDI), boucle de test, balayage, entrée micro pour les effets. |
| Analyse | Oscilloscope, spectre, spectrogramme, crête/RMS/DC, compteur de NaN, CPU par bloc ; sondes sur un fil du diagramme après le MVP. |
| Versions | Arbre des versions, mesures par version, bascule A/B à volume égalisé, restauration. |
| Console | Sortie du compilateur, alertes des garde-fous, rapports de tests. |

### 6.5 Ce qui est reporté

| Reporté | Pourquoi | Condition pour l'ouvrir |
| --- | --- | --- |
| Niveau Rust (SDK `faulk-module`, fundsp) | Compilation hors navigateur, ABI `fk-abi` et instrumentation à écrire. | Premier besoin spectral ou granulaire ; helper capable de lancer `cargo`. |
| Édition graphique (graphe → Faust) | Le diagramme en lecture suffit pour comprendre. | Bibliothèque `fk.*` stable. |
| Échantillons et tables chargées | Chaîne d'import, stockage dans le projet, `soundfile`. | Besoin d'un lecteur ou d'un granulaire. |
| Spectral (FFT) | Traitement par trames, mieux en Rust. | Niveau Rust. |
| Modulation non destructive, MPE | Dans l'ABI, pas dans le moteur. | Modulateurs de l'hôte. |
| Sidechain, sorties multiples | Bus supplémentaires dans le moteur. | Besoin de compression déclenchée. |
| Interfaces personnalisées | Isolation d'une vue web, sécurité. | Communauté de modules. |
| État opaque (`fk_state_*`) | Inutile tant que l'état = les paramètres. | Tables ou échantillons dans un module. |
| Recréation par mesure | Mode Boost et hébergement de plugins tiers. | Helper avec CLAP/VST3. |
| Import/export WCLAP | Ouverture vers d'autres hôtes. | ABI v0 stable. |
| Partage et bibliothèque communautaire | Licences, signature, modération. | Modèle de distribution décidé. |

## 7. Risques

1. **Taille et temps de compilation de libfaust en WASM** dans le navigateur (plusieurs mégaoctets ; temps de compilation inconnu) : **à mesurer** avant de promettre un rechargement « instantané ».
2. **Pas de préemption dans l'AudioWorklet** : le compteur instrumenté limite le pire, pas le moyen.
3. **Mesure du CPU dans le worklet** incertaine (§ 5.2).
4. **Parité des fonctions mathématiques** entre JS et wasmtime (§ 5.4) : sinon, les tests de parité devront tolérer un écart.
5. **Qualité du Faust généré par l'IA** : moins d'exemples d'entraînement que pour Rust ; à évaluer sur une série de demandes types.
6. **Licences** : faustwasm est LGPL-3.0 ; le fondateur a tranché le 2026-10-14 que c'est acceptable si Faulk est open source, et à revoir sinon (le modèle de distribution reste une décision ouverte de la [vision](../product/01-vision.md)) ; les bibliothèques Faust ont des licences par fichier, et le code généré hérite de celles des fonctions utilisées.
7. **Surcoût de copie** entre la mémoire du moteur et celle de chaque module : à mesurer avec une chaîne de 10 modules.
8. **Dépendance à un seul compilateur** pour le niveau principal : atténuée par l'éjection vers Rust et par l'autonomie du `.wasm` produit.

## 8. Questions ouvertes pour le fondateur

1. Le niveau Rust est-il pour le fondateur (développeur) seulement, ou faut-il viser un langage compilable dans le navigateur pour la communauté ?
2. Polyphonie : 8 voix par défaut avec vol de voix ; faut-il de l'unisson dès le MVP (gestion des voix par le module) ?
3. Limiteur de protection de l'Atelier : seuil et comportement (couper, ou limiter et prévenir) ?
4. Les versions de modules vivent-elles dans le projet, dans une bibliothèque personnelle, ou les deux ? (Lié à la question des sources Faust dans le [modèle de projet](../architecture/02-modele-projet.md).)
5. ~~Faulk peut-il distribuer faustwasm (LGPL-3.0) avec le modèle de distribution choisi ?~~ Décidé le 2026-10-14 : acceptable si Faulk est open source, à revoir sinon.
6. Le premier module « recréé par mesure » : lequel, et sur quels critères d'écoute le jugera-t-on réussi ?

## Sources

- CLAP, en-têtes et extensions : <https://github.com/free-audio/clap>, paramètres : <https://github.com/free-audio/clap/blob/main/include/clap/ext/params.h>
- VST3, architecture : <https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/API+Documentation/Index.html>
- WCLAP : <https://github.com/WebCLAP>, hôte navigateur : <https://github.com/WebCLAP/browser-test-host>
- Web Audio Modules 2.0 : <https://zenodo.org/record/6769098>
- faustwasm : <https://github.com/grame-cncm/faustwasm>, version et licence : <https://www.jsdelivr.com/package/npm/@grame/faustwasm>
- Options du compilateur Faust (`-ftz`, `-svg`, `-json`, `-lang`) : <https://faustdoc.grame.fr/manual/options/>
- Bibliothèques Faust : <https://faustlibraries.grame.fr/>, réverbérations : <https://faustlibraries.grame.fr/libs/reverbs/>, modélisation physique : <https://faustlibraries.grame.fr/libs/physmodels>
- fundsp : <https://github.com/SamiPerttu/fundsp>
- wasmtime, interruption (fuel, epochs) : <https://docs.wasmtime.dev/examples-interrupting-wasm.html>
- Non-déterminisme de WASM : <https://github.com/WebAssembly/design/blob/master/Nondeterminism.md>
- `AudioWorkletGlobalScope` : <https://developer.mozilla.org/docs/Web/API/AudioWorkletGlobalScope>
- Subnormaux et audio web : <https://lists.w3.org/Archives/Public/public-audio/2017JulSep/0006.html>
- J. O. Smith, *Physical Audio Signal Processing* : <https://ccrma.stanford.edu/~jos/pasp/Zita_Rev1.html>
- A. Farina, balayage sinusoïdal exponentiel (AES 2000) : <http://pcfarina.eng.unipr.it/Public/Papers/134-AES00.PDF>
- auraloss (distances spectrales multi-résolution) : <https://github.com/csteinmetz1/auraloss>
