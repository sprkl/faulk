---
type: Spike
title: "Forge — modèles de filtre, saturation et trois esquisses de caractère"
description: "Spike #349 of the Forge epic (#348): band-limited oscillators, a nonlinear zero-delay-feedback ladder and a state-variable filter, saturation, drift and per-voice tolerances, analog-style envelopes, chorus; three character sketches rendered offline for the founder to choose by ear; cost per voice natively and in WASM, aliasing, bounded self-oscillation, native/WASM parity."
status: draft
---

# Forge — modèles de filtre, saturation et trois esquisses de caractère

Première étape de l'épopée [#348](https://github.com/sprkl/faulk/issues/348) (Forge, le synthé phare), issue [#349](https://github.com/sprkl/faulk/issues/349). Le spike doit permettre au fondateur de choisir une direction de son **à l'oreille**, et fixer les briques DSP et leur coût avant d'écrire le device. Tous les chiffres ci-dessous viennent de mesures faites pour ce spike ; ce qui est extrapolé est dit tel. Le prototype n'est pas un device : pas de descripteur, pas de paramètres pilotés par commande, pas de panneau.

## Le prototype

Crate de spike `crates/engine/spikes/forge/`, hors du workspace (son propre `[workspace]`, comme le banc de QA-7), en Rust pur sans dépendance. Il compile en natif et en `wasm32-unknown-unknown` (exports C, aucun import).

| Brique | Fichier | Modèle retenu |
|---|---|---|
| Oscillateurs | `src/osc.rs` | Mélange continu scie → impulsion (largeur 5–95 %), synchronisation forcée, sous-oscillateur carré une octave sous A. Bande limitée par **PolyBLEP à deux points**, chaque discontinuité (retour de la scie, fronts de l'impulsion, remise à zéro de la synchro) placée à sa position fractionnaire. L'oscillateur sort avec un échantillon de retard, pour corriger aussi l'échantillon d'avant le saut. |
| Suréchantillonnage | `src/halfband.rs`, `src/synth.rs` | Les voix tournent à **2× (96 kHz)** ; la somme des voix redescend à 48 kHz par un FIR demi-bande de 63 coefficients (fenêtre de Kaiser, 16 multiplications par échantillon de sortie ; testé : plat à 0,1 dB près jusqu'à 18 kHz, au moins 80 dB d'atténuation à 30 et 40 kHz). La décimation est linéaire : elle se fait **une fois par canal**, pas par voix. Latence : 15,5 échantillons à 48 kHz (0,32 ms). |
| Passe-bas en échelle | `src/filter.rs` (`Ladder`) | 4 pôles (24 dB/oct), **zero-delay feedback** (intégrateurs trapézoïdaux, boucle résolue dans l'échantillon). Une saturation `tanh` dans la boucle de résonance, résolue par une prédiction linéaire, un pas de Newton puis une dernière évaluation. Résonance 0–1 → retour 0–4,3 : au-delà de 4 il auto-oscille. Compensation partielle de la perte de graves quand la résonance monte. |
| Variables d'état | `src/filter.rs` (`Svf`) | 2 pôles (12 dB/oct), zero-delay feedback, cœur **linéaire** ; son caractère vient de la saturation autour (entrée, sortie). Amortissement minimal 0,02 (Q = 50) : il sonne mais n'auto-oscille pas. Plus doux et plus propre que l'échelle, par choix. |
| Saturation | `src/voice.rs`, `src/math.rs` | `tanh` rationnelle (`x (27 + x²) / (27 + 9 x²)`, bornée à ±1, à 2,5 % près de `tanh`, sans appel de bibliothèque) à l'entrée du filtre (réglage *drive*) et dans un étage de sortie légèrement décentré (harmoniques paires), suivi d'un filtre anti-continu. |
| Enveloppes | `src/env.rs` | Charge et décharge de condensateur : l'attaque vise 1,3 et s'arrête à 1 (courbe qui monte vite et s'arrondit en haut), déclin et relâchement exponentiels, redéclenchement depuis le niveau courant. Un « claquement » (*snap*) ajoute au filtre une impulsion de 4 ms en début de note. |
| Imperfections | `src/voice.rs` | Par emplacement de voix, tirées d'une graine fixe : écart d'accord de chaque oscillateur (±5 cents à âge 1), de coupure (±0,15 octave), de temps d'enveloppe (±12 %), phase de départ. Dérive lente de chaque oscillateur : marche aléatoire lissée sur 0,4 s, ±6 cents à âge 1. Un seul réglage « âge » (0–1) dose le tout. Mêmes graines, mêmes rendus, au bit près. |
| Chorus | `src/chorus.rs` | Stéréo, deux prises modulées par canal (LFO en quadrature), interpolation linéaire, passe-bas sur le signal traité (le côté sombre d'une ligne à retard analogique). Lignes allouées à la construction. |
| Jeu | `src/synth.rs` | 16 emplacements de voix, poly ou mono, legato et glide, unisson 1–16 avec désaccord et largeur stéréo, vol de voix (libre, sinon la plus ancienne relâchée, sinon la plus ancienne). Les pas de contrôle (16 échantillons à 96 kHz) se poursuivent d'un bloc à l'autre : un bloc peut avoir n'importe quelle taille, donc les notes tombent à l'échantillon près. |

Le chemin audio (`Synth::render` et tout ce qu'il appelle) n'alloue pas, ne verrouille pas et ne panique pas sur ses entrées ; c'est vérifié par le détecteur d'allocation (ci-dessous).

## Les trois esquisses

Rendus : `.qa/renders/forge-sketch-a.wav`, `forge-sketch-b.wav`, `forge-sketch-c.wav` (48 kHz, 24 bits, stéréo, 24 s). Pour régénérer : voir « Reproduire ».

**Mêmes notes partout** : un seul morceau, construit par `song::bass()`, `song::pad()` et `song::lead()`, joué par les trois esquisses ; seuls les sons (patchs), le chorus et l'équilibre des trois parties changent. 96 BPM, la mineur, neuf mesures puis 1,5 s de queue :

- mesures 1–8 : une **basse** en croches (la, fa, do, sol) ;
- à partir de la mesure 3 : une **nappe d'accords** (la m7, fa maj7, do, sol) ;
- à partir de la mesure 5 : une **phrase de lead** qui se résout sur la mesure 9.

Les trois fichiers sont ramenés **au même volume moyen** (−18 dBFS RMS) pour que la comparaison ne favorise pas le plus fort.

### (a) Chaud et gras

Filtre en échelle partout, saturation forte (*drive* 4 pour la basse, 5 pour le lead, 2,5 pour la nappe), âge 0,5.

À écouter :

- **La basse** : trois voix à l'unisson, légèrement désaccordées, plus un oscillateur une octave en dessous et le sous-oscillateur. Chaque note démarre par un petit claquement du filtre, puis se referme. Quand le filtre s'ouvre, la saturation épaissit le son au lieu de seulement l'éclaircir.
- **La nappe** : sombre et ronde, elle s'ouvre lentement à chaque accord. Les accords tenus battent lentement : c'est la dérive des oscillateurs.
- **Le lead** : il glisse d'une note à l'autre (60 ms), la résonance chante sur l'ouverture du filtre, avec un léger vibrato. Il grogne un peu quand il monte.

### (b) Doux et polyphonique

Filtre à variables d'état, saturation légère, chorus, âge 0,3.

À écouter :

- **La basse** : une onde pulsée et le sous-oscillateur, plus ronde et plus propre que (a), moins de grain.
- **La nappe** : large et brillante. Le chorus fait bouger le son entre les deux enceintes, et la largeur d'impulsion varie lentement : la nappe « respire ».
- **Le lead en cuivres**, joué polyphonique : le filtre s'ouvre en 80 ms, ce qui donne une attaque qui gonfle au lieu de claquer. L'ensemble est plus propre et plus « années 80 » que (a).

### (c) Verre et acier (la direction choisie pour ce spike)

Synchronisation forcée balayée par l'enveloppe, résonance haute, enveloppes courtes, presque aucune dérive (âge 0,05), unisson très large au lieu du chorus. Pourquoi cette troisième direction : (a) et (b) explorent la chaleur ; (c) teste l'autre pôle, un son précis, brillant et mordant, qui sert les musiques électroniques actuelles, et montre que les mêmes briques couvrent aussi ce terrain.

À écouter :

- **La basse** : chaque note commence par un « miaulement » métallique qui retombe (l'oscillateur synchronisé balaie 14 demi-tons) ; la résonance est haute.
- **La nappe** : impulsions étroites plus une octave, résonance haute, filtre qui suit le clavier, des voix très écartées en stéréo. Le son est cristallin, presque parfaitement juste.
- **Le lead** : la synchro balaie 9 demi-tons à chaque attaque, la saturation de sortie le rend agressif ; il glisse plus vite que (a).

## Mesures

**Machine** : Intel Core Ultra 7 268V (8 cœurs), Windows 11 Pro, charge mesurée avant les mesures : 8 %. Rust 1.99 (release, `lto`, `codegen-units = 1`, sans `target-cpu`). WASM : `wasm32-unknown-unknown` sans SIMD (comme le module livré), exécuté par **Node 24.16 (V8, le moteur de l'`AudioWorklet` de Chromium)**. Un seul poste, partagé avec d'autres sessions : les chiffres sont des ordres de grandeur, à ±10 % d'une exécution à l'autre.

### Coût CPU

Charge mesurée : un synthé, *n* notes tenues pendant 10 s d'audio à 48 kHz, en blocs de 128 ; chaque voix fait tourner deux scies et le sous-oscillateur dans le filtre, enveloppe de filtre en mouvement, dérive active ; le chorus est actif (coûts partagés comptés une fois). Pourcentage d'un cœur = temps de calcul / durée audio, médiane de 7 exécutions. « 16 voix » est mesuré directement, pas extrapolé.

| Variante | Natif, 0 voix | Natif, 16 voix | Natif, par voix | WASM, 0 voix | WASM, 16 voix | WASM, par voix |
|---|---|---|---|---|---|---|
| Échelle, 2× | 0,12 % | **8,6 %** | 0,53 % | 0,12 % | **6,7 %** | 0,42 % |
| Variables d'état, 2× | 0,12 % | 3,0 % | 0,18 % | 0,12 % | 3,2 % | 0,19 % |
| Échelle, 1× | 0,09 % | 4,3 % | 0,26 % | 0,06 % | 3,4 % | 0,21 % |
| Variables d'état, 1× | 0,09 % | 1,6 % | 0,09 % | 0,06 % | 1,7 % | 0,10 % |

- **Budget de #348** (16 voix sous 15 % d'un cœur à 48 kHz) : tenu dans toutes les variantes, avec une marge d'environ 2× pour la variante la plus chère (échelle à 2×). Cette marge doit couvrir ce qui manque au prototype : matrice de modulation, LFO par voix, bruit, second filtre éventuel, et une machine plus lente. Extrapolation, non mesurée : une machine deux fois plus lente mettrait l'échelle à 2× vers 13–17 %.
- Par voix, l'échelle coûte 2,2× (WASM) à 2,9× (natif) le filtre à variables d'état : trois `tanh` et une division de Newton par échantillon.
- Les esquisses entières (trois parties ensemble, unisson compris) : (a) 1,84 s de calcul pour 24 s d'audio, soit 7,7 % d'un cœur ; (b) 3,0 % ; (c) 2,6 % (natif).
- WASM est ici plus rapide que le natif sur l'échelle ; ce n'est pas expliqué (le natif est compilé sans `target-cpu`). Ce n'est pas un risque pour le budget, qui se juge en WASM.

### Aliasing

Scie de l'oscillateur à **4,1 kHz** (et non 4 kHz pile : à 48 kHz, 48 000 / 4 000 = 12, et tout repli tomberait exactement sur un harmonique, donc invisible), 65 536 échantillons, fenêtre de Blackman-Harris. « Repli » = toute composante qui n'est pas un harmonique ; dBc = par rapport à la fondamentale.

| Oscillateur | Pire repli, toute la bande | Pire repli sous 19 kHz | Repli total / harmoniques |
|---|---|---|---|
| Naïf, 1× | −15,6 dBc | −18,0 dBc | −9,1 dB |
| PolyBLEP, 1× | −23,7 dBc | −34,1 dBc | −23,3 dB |
| Naïf, 2× | −24,9 dBc | −26,2 dBc | −15,8 dB |
| **PolyBLEP, 2× (retenu)** | −26,8 dBc (à 23,4 kHz) | **−52,6 dBc** | −28,2 dB |

Le pire repli de la variante retenue est à 23,4 kHz : il vient de la bande de transition du demi-bande (19,5–28,5 kHz, calculée à partir de la fenêtre, non mesurée), au-dessus de l'audible. Sous 19 kHz, le gain du 2× est de 18 dB.

La voix entière (scie dans l'échelle grande ouverte, puis étage de sortie), pour voir ce qu'ajoute la saturation :

| Note, *drive* | 1× : pire repli sous 19 kHz | 2× : pire repli sous 19 kHz |
|---|---|---|
| do 6 (1 047 Hz), 1,5 | −50,6 dBc | −57,0 dBc |
| do 6 (1 047 Hz), 6 | −40,2 dBc | −46,4 dBc |
| do 8 (4 186 Hz), 1,5 | −38,3 dBc | −45,2 dBc |
| do 8 (4 186 Hz), 6 | −27,9 dBc | −34,2 dBc |

La saturation poussée sur une note très aiguë replie encore à −34 dBc même à 2× : c'est la limite du modèle retenu (voir « Risques »).

### Auto-oscillation, NaN, dénormaux, allocation, déterminisme

Tests (`cargo test` dans la crate du spike, 13 tests) :

- **Auto-oscillation bornée** : l'échelle à résonance maximale, à 30 Hz, 200 Hz, 1 kHz, 5 kHz, 15 kHz et 24 kHz (= fs/4 à 96 kHz) ; une impulsion puis 3 s de silence : elle **oscille** (RMS > 0,05 sur la dernière seconde) et ne dépasse jamais 1,0 ; attaquée par un carré d'amplitude 20 : sortie toujours dans ±1. C'est aussi garanti par construction : la boucle passe par un `tanh` borné à ±1, et la coupure est plafonnée à fs/4, où chaque étage du filtre garde son état dans ±1.
- **Extrêmes** : chaque patch des trois esquisses (9), avec les deux filtres, cinq combinaisons de coupure (10 Hz / 20 kHz), résonance (0 / 1), *drive* (0,5 / 20) et âge (0 / 1), à 1× et 2×, unisson 3 sur un accord de huit notes de 24 à 108 (24 voix demandées pour 16 emplacements : le vol de voix est exercé), en blocs de tailles irrégulières : **aucun NaN, aucun infini, aucun dénormal** dans la sortie, crête au plus 7,95 fois le niveau du patch, et **silence exact** (des zéros) à la fin du relâchement. Ce test a trouvé deux défauts du prototype, corrigés : des dénormaux en sortie (états vidés seulement par pas de contrôle) et une queue du filtre anti-continu qui ne revenait jamais à zéro.
- **Allocation** : les trois esquisses entières rendues sous `assert_no_alloc` (détecteur d'allocation, comme les tests du moteur) : aucune allocation.
- **Déterminisme** : chaque esquisse rendue deux fois donne les mêmes bits.
- Réponse en fréquence : l'échelle descend de 24 dB/oct et le filtre à variables d'état de 12 dB/oct (±3 dB et ±2 dB), mesurés à 2 et 4 kHz pour une coupure à 500 Hz.

### Parité natif / WASM

Les trois esquisses rendues en natif et en WASM sont **identiques au bit près** (écart maximal 0). Ce n'était pas le cas au premier essai : avec les fonctions de la bibliothèque standard (`exp2`, `tan`, `sin`…), qui viennent de bibliothèques mathématiques différentes en natif et en WASM et diffèrent au dernier bit, (a) et (b) restaient à 5·10⁻⁵ près, mais **(c) s'écartait de 0,30** (à 12,5 s). La cause est la nature d'un synthé : il prend des décisions discrètes sur ses signaux (une enveloppe finit un échantillon plus tôt, donc une note suivante prend un autre emplacement de voix, avec d'autres tolérances), et un écart au dernier bit devient un autre son. Le prototype calcule donc ses fonctions transcendantes lui-même (`src/math.rs` : `exp2`, `log2`, `tan`, `sin` par polynômes, avec seulement `+ − × ÷`, `floor` et des opérations sur les bits, que IEEE 754 rend exactes sur toutes les cibles).

## Modèles retenus et pourquoi

1. **PolyBLEP à deux points et voix à 2×**, décimation une fois par canal. Le PolyBLEP seul à 1× laisse un repli à −34 dBc sous 19 kHz pour une scie à 4,1 kHz ; à 2× on passe à −53 dBc, et la saturation et le filtre en profitent aussi. Le coût double par voix mais reste dans le budget. Des tables à bande limitée n'ont pas été essayées : elles ne régleraient pas le repli de la saturation, que le 2× réduit aussi.
2. **Échelle zero-delay feedback avec `tanh` dans la boucle, un pas de Newton** : c'est elle qui donne la résonance qui chante et qui sature ; la sortie est bornée par construction. Les étages eux-mêmes restent linéaires (une saturation par étage coûterait quatre `tanh` de plus par échantillon) : à essayer à l'oreille si (a) est choisie et manque de grain.
3. **Variables d'état à cœur linéaire**, saturation autour : le filtre doux de #348, deux fois moins cher.
4. **`tanh` rationnelle** au lieu de la fonction de bibliothèque : sans appel de bibliothèque, identique en natif et en WASM, et bornée (son gain de coût n'a pas été mesuré à part).
5. **Mathématiques portables** pour tout ce qui touche au son : condition de la parité natif / WASM.

## Risques

- **Repli de la saturation** sur les notes aiguës à fort *drive* (−34 dBc à 4,2 kHz avec *drive* 6, même à 2×). Remèdes possibles, à mesurer dans la tâche des filtres : anti-repli par primitive (ADAA) sur l'étage de sortie, ou 4× pour les seules voix aiguës. Pas mesuré ici.
- **Budget CPU** : mesuré sur un seul poste récent. L'unisson multiplie les voix (7 voix d'unisson × 4 notes = 28 voix) : le budget doit compter les voix réelles, et Forge doit plafonner l'unisson à 16 voix au total ou dégrader proprement.
- **Parité** : la règle « pas de fonction transcendante de la bibliothèque sur le chemin du son » doit s'appliquer à tout le device Forge (et sans doute au moteur : `osc_synth` et `drums` utilisent `sin`, `exp`, `exp2` ou `tan` de la bibliothèque et tiennent la parité à 10⁻⁵ du harnais de rendu, sans doute parce qu'aucune de leurs décisions discrètes n'amplifie l'écart). Décision à prendre dans la tâche des filtres.
- **Synchronisation** : le PolyBLEP de la remise à zéro est exact, mais un front de l'oscillateur synchronisé dans le même échantillon que la remise est ignoré (rare tant que la fréquence reste loin de fs/2).
- **Vol de voix sans fondu** : le prototype vole une voix sur-le-champ (clic possible). Le device doit reprendre le fondu de 3 ms d'`osc_synth`.
- **Latence** : le demi-bande ajoute 0,32 ms à 48 kHz. Ember (#142) demande « aucune latence ajoutée » : à déclarer dans le descripteur, ou à compenser.
- **Le son** : personne d'autre que le fondateur ne l'a jugé. Les patchs ont été réglés sur des mesures (niveaux par mesure, crêtes), pas à l'oreille.

## Découpage proposé pour la suite de l'épopée

1. **Ember (#142)**, inchangé, mais construit sur les briques de ce spike : oscillateur PolyBLEP, voix à 2× et demi-bande, enveloppes analogiques, mathématiques portables.
2. **Module DSP partagé du moteur** (`engine::dsp`) : reprend `math`, `osc`, `halfband`, `env` et `filter` du spike, avec leurs tests (réponse en fréquence, auto-oscillation bornée, extrêmes sans NaN ni dénormaux, parité natif / WASM au bit près sur un rendu). Remplace la tâche « Filtres modélisés et saturation » de #348, et y ajoute la mesure d'un anti-repli de la saturation (ADAA ou 4× sélectif).
3. **Imperfections par voix et réglage « âge »** : tolérances par emplacement et dérive, déterministes à graine fixe (déjà prototypées ici).
4. **Matrice de modulation** (8 routages).
5. **Unisson et chorus**, avec le plafond de voix réelles et le fondu au vol de voix.
6. **Système de presets** (format, `device.load_preset`, `device.save_preset`, navigateur), s'il n'existe pas.
7. **Banque de presets**, par séances d'écoute avec le fondateur.
8. **Panneau dessiné** (exception à décider au moment de la tâche).

## Question au fondateur

Quelle direction pour Forge : **(a) chaud et gras**, **(b) doux et polyphonique**, **(c) verre et acier**, ou un mélange ? Par exemple « l'échelle et la saturation de (a), avec le chorus et les cuivres de (b) », ou « (a) par défaut, et la synchro de (c) comme réglage ». Deux points précis :

- l'âge par défaut : 0,5 dans (a) (dérive audible sur les accords tenus), 0,3 dans (b), 0,05 dans (c) ;
- le filtre à variables d'état doit-il pouvoir auto-osciller comme l'échelle, ou rester doux ?

## Reproduire

Depuis la racine du dépôt (Git Bash), avec un seul dossier `target` :

```bash
export CARGO_TARGET_DIR=$PWD/target
cd crates/engine/spikes/forge
cargo test                                      # 13 tests (dev optimisé, détecteur d'allocation actif)
cargo run --release --bin forge -- render ../../../../.qa/renders ../../../../target/forge-raw
cargo run --release --bin forge -- bench        # coût natif
cargo run --release --bin forge -- aliasing     # repli
cargo build --release --target wasm32-unknown-unknown --lib
cd ../../../..
node crates/engine/spikes/forge/bench/wasm.mjs target/wasm32-unknown-unknown/release/forge_spike.wasm target/forge-raw
cd crates/engine/spikes/forge && cargo run --release --bin forge -- compare ../../../../target/forge-raw
```
