---
type: Spec
title: "Enregistrement MIDI"
description: "Spec of MIDI recording into a clip: arming, count-in, metronome, new clip versus existing clip, overdub versus replace, quantise on input, undo granularity, latency compensation in Web and Boost, the commands it needs and what agents can do with them. Open product questions are marked."
status: draft
---

# Enregistrement MIDI

> Cette spec répond à la première case de l'issue #470 (chantier 2 de l'épique #468 : jouer des notes). Elle reprend le découpage des tâches RE-1 à RE-5 du [découpage du MVP](02-decoupage-mvp.md) et le précise.
>
> Les questions de produit sont marquées **[Question ouverte]** et regroupées au [§ 11](#11-questions-ouvertes) ; le lead a retenu les réponses par défaut (commentaire de #470 du 2026-11-01).
>
> **État de l'implémentation (#470, étape 2).** Livré : la capture dans le moteur (`crates/engine/src/recorder.rs`), les notes dans le rapport (version 5), les commandes `record.set_settings`, `record.start`, `record.stop`, `record.discard_take`, `record.commit_take` et `record.restore` (`crates/commands/src/record.rs`), le bouton d'enregistrement avec la cible affichée et le décompte, le bouton « Rec » des pistes, les notes de la prise dans le piano roll. Écarts avec le texte ci-dessous, à lire comme la référence :
>
> - reports acceptés par le lead (2026-11-02), voir « Différé » au § 9 :
> - le mode remplacement et la quantification à l'entrée ne sont pas livrés (§ 4.2, § 6 ; choix par défaut : ajout, quantification après coup) : #500 ;
> - l'enregistrement en Boost attend BO-1 : le bouton est désactivé en Boost, avec la raison ; le décalage manuel est déjà dans la session : #501 ;
> - `record.set_settings` ne porte que `offset_ms` ; c'est un état de session, comme l'armement, donc il n'entre pas dans l'historique (§ 9 à jour) ;
> - `record.stop` n'écrit pas la prise : la page, qui lit le rapport du moteur, lance `record.commit_take` dès que le moteur a rendu ses dernières notes (§ 9) ; `record.start` refuse une nouvelle prise tant que la précédente n'est ni écrite ni abandonnée ;
> - la cible de la grille est la scène de la dernière cellule touchée (sinon la première scène) ; avec le transport arrêté, `record.start` lance le clip de cette scène sur chaque piste armée, et la prise s'y ajoute en suivant sa boucle ;
> - dans l'arrangement, une note jouée dans un clip placé s'y ajoute ; les autres font un nouveau clip par intervalle libre, de la mesure de la première note à la mesure après la dernière, coupé au clip suivant ;
> - le décompte sonne même métronome éteint et s'arrête au repère (§ 5) ; il n'a lieu que transport arrêté.
>
> **Horodatage des entrées (#499).** En Web, une note MIDI ou une touche du clavier porte l'instant où elle a été jouée (`MIDIMessageEvent.timeStamp`, `KeyboardEvent.timeStamp`) : la page le convertit en image du contexte audio (`getOutputTimestamp()`, plus la latence de sortie estimée que la prise reçoit aussi dans `latency_ms`, si bien que les deux s'annulent) et l'envoie dans le champ facultatif `frame` de `live.input_note_on` et `live.input_note_off`. Le worklet donne au moteur l'image de chaque bloc (`currentFrame`), le moteur en tire son propre échantillon et l'enregistreur place la note sur cet échantillon, même arrivée plusieurs blocs plus tard, même de l'autre côté d'un bouclage ou d'un changement de tempo. Une note dont l'instant s'écarte de plus d'une seconde du bloc qui la prend, dans un sens ou dans l'autre, compte depuis le début de ce bloc, comme avant (l'instant peut précéder ce bloc de quelques millisecondes quand la latence estimée dépasse l'avance réelle du rendu sur la sortie : mesuré 31 à 42 ms d'avance pour 42 ms estimées dans Chromium sous Windows ; la prise est ramenée de la même estimation, donc la note reste là où elle a été entendue). Le son n'est pas retardé : la note sonne au début du bloc qui la prend, horodatée ou non. Méthode de mesure du critère de RE-2 (± 1 ms) : tests hors ligne à horodatage connu (`crates/engine/tests/live_stamps.rs`, parité native et WASM dans `crates/engine-wasm/tests/parity.rs`), et dans Chromium deux notes MIDI simulées d'horodatages distants d'un temps, envoyées au même instant, enregistrées à un temps l'une de l'autre (`web/tests/e2e/recording.spec.ts`). En Boost, l'horodatage de midir reste à faire : #501.

## 1. Ce que l'utilisateur veut

Un producteur joue une partie au clavier de l'ordinateur ou sur un contrôleur, et elle devient des notes dans un clip. Il ne place pas ses accords à la souris. Sans cela, l'app bloque un professionnel (revue de design du 2026-11-01, constat du producteur : 32 notes d'accords posées à la souris).

Deux sources d'entrée, un seul chemin : le clavier de l'ordinateur (mode « Saisie : notes », [#470](https://github.com/sprkl/faulk/issues/470)) et le MIDI (Web MIDI, MIDI natif en Boost) arrivent tous deux par `live.input_note_on` et `live.input_note_off`. L'enregistrement s'accroche à ce chemin, il n'en crée pas un second.

## 2. Vocabulaire

- **Armer** une piste : elle reçoit l'entrée en direct, qu'elle soit sélectionnée ou non (`track.arm`, état de session). Armer ne lance aucun enregistrement.
- **Enregistrer** : le transport avance et les notes jouées sur les pistes armées sont capturées. C'est l'état `recording` du transport, déjà présent dans le rapport du moteur mais jamais atteint.
- **Prise** (take) : tout ce qui est capturé entre le début et la fin d'un enregistrement. Une prise devient une seule entrée d'historique.
- **Décompte** : des mesures comptées par le métronome avant que la capture commence (`count_in_bars` existe déjà dans les réglages du métronome).

## 3. Armement

1. Seule une piste MIDI s'arme (comme aujourd'hui ; l'armement d'une piste audio viendra avec l'enregistrement audio).
2. Plusieurs pistes peuvent être armées : la prise donne **un clip par piste armée**, dans la même prise d'historique.
3. Une piste sélectionnée mais non armée est jouée et n'enregistre rien. Une piste armée l'est jusqu'à ce que l'utilisateur la désarme ; changer de piste ne désarme pas.
4. Le bouton d'armement dit « In » tant que l'enregistrement n'existe pas ; il redevient « Rec » avec la glyphe rouge quand RE-2 à RE-5 sont livrés. Ce changement de libellé fait partie de la tâche RE-5, pas d'une tâche à part.

**[Question ouverte 1]** Faut-il séparer « entrée en direct » (aujourd'hui) et « armé pour enregistrer » en deux états, ou garder un seul état qui prend le sens d'enregistrement une fois livré ? Proposition : un seul état. Une piste armée qui reçoit l'entrée est celle qui enregistrerait, c'est ce que les producteurs attendent.

## 4. Où va la prise

### 4.1 Nouveau clip ou clip existant

| Situation au démarrage | Résultat |
|---|---|
| Piste armée, cible = cellule vide de la grille | La prise crée un clip dans cette cellule (RE-3). |
| Piste armée, cible = cellule de la grille qui contient un clip | Voir § 4.2 (ajout ou remplacement). |
| Piste armée, lecture depuis la timeline, tête de lecture hors de tout clip | La prise crée un clip placé là où elle a été jouée, arrondi à la mesure (RE-4). |
| Piste armée, tête de lecture dans un clip de la timeline | Voir § 4.2. |
| Plusieurs pistes armées | Une cible par piste, choisie par les mêmes règles. |

La cible vient de la sélection du moment : la cellule sélectionnée dans la grille, sinon la position de la tête de lecture dans la timeline. **[Question ouverte 2]** Quelle vue décide de la cible quand les deux ont une sélection ? Proposition : la dernière vue active, et la cible est affichée avant de lancer (« Enregistre dans Scène 2 » ou « Enregistre à la mesure 9 »).

### 4.2 Ajout (overdub) ou remplacement

- **Par défaut : ajout (overdub).** Les notes jouées s'ajoutent à celles du clip. Rien n'est effacé, donc rien n'est perdu par erreur.
- **Remplacement** (réglage de l'enregistrement, pas de la piste) : à l'arrivée de la première note de la prise, les notes existantes de la plage enregistrée sont retirées, puis la prise s'écrit. La plage est celle où l'on a joué, de la première note à la fin de la prise. Les notes en dehors ne bougent pas.
- La boucle du transport enregistre en couches : chaque tour ajoute (ajout), ou chaque tour remplace le précédent (remplacement). Les prises superposées d'un même passage sont fusionnées en une seule prise d'historique.

**[Question ouverte 3]** Le remplacement doit-il être le comportement par défaut pour un clip de grille en boucle (tour après tour on affine) ? Proposition : non, ajout partout, remplacement à choisir.

## 5. Décompte et métronome

- Le décompte utilise `count_in_bars` des réglages du métronome (`transport.set_metronome`, qui existe). À 0, l'enregistrement démarre sur-le-champ.
- Pendant le décompte, les notes jouées sont entendues mais pas capturées. Elles ne sont pas non plus gardées « pour plus tard » : une note jouée avant la mesure 1 est perdue.
- Le métronome sonne pendant le décompte. Pendant la prise il suit le réglage du métronome (allumé ou non), l'enregistrement ne l'allume pas de lui-même.
- Un enregistrement lancé pendant la lecture saute le décompte (le transport tourne déjà) : « punch-in » à la tête de lecture.

**[Question ouverte 4]** Le décompte vaut-il aussi quand le transport tourne déjà ? Proposition : non, on enregistre à la volée. Cas à trancher avec le fondateur.

## 6. Quantification à l'entrée

- Réglage de l'enregistrement : `quantize_on_input` ∈ {aucune, 1/4, 1/8, 1/16, 1/32, 1/8 triolet, 1/16 triolet} (la liste de `clip.notes.quantize`), avec une force en pourcentage. **Par défaut : aucune** : on enregistre ce qui est joué, tel quel.
- La quantification agit sur le début des notes à la fin de la prise (pas note par note pendant le jeu), en une seule commande interne. La durée n'est pas quantifiée.
- Elle ne modifie que les notes de la prise, jamais celles qui existaient déjà dans le clip.
- Comme la quantification existe déjà après coup (`clip.notes.quantize`), l'enregistrement peut ne rien faire de plus que capturer, et l'utilisateur quantifie ensuite. L'option d'entrée sert l'agent et le flux rapide.

**[Question ouverte 5]** La quantification à l'entrée est-elle dans le MVP de l'enregistrement, ou seulement la commande après coup ? Proposition : après coup d'abord, l'option d'entrée ensuite.

## 7. Annulation

- **Une prise = une entrée d'historique**, quelle que soit sa durée et le nombre de notes (critère de RE-3 et RE-4). Annuler retire toute la prise : le clip créé disparaît, ou les notes ajoutées (et restaurées, en remplacement) reviennent à leur état d'avant.
- Pendant l'enregistrement, rien n'est écrit dans le projet : les notes vivent dans le tampon de capture du moteur. À l'arrêt, la commande de validation les écrit toutes d'un coup. Annuler pendant la prise est un abandon : la prise est jetée, sans entrée d'historique.
- Les prises en boucle fusionnent en une seule entrée (§ 4.2), sinon dix tours donneraient dix annulations.
- Annuler après un remplacement rend les notes remplacées.

## 8. Compensation de latence

La note jouée est entendue un peu après la frappe (latence de sortie), et le musicien joue sur ce qu'il entend. Les notes doivent donc être placées là où il pensait les jouer.

- **Web** (WebAssembly dans le navigateur, 20 à 60 ms de latence **estimée**, affichée comme telle) : l'horodatage de l'événement MIDI (`MIDIMessageEvent.timeStamp`, ou l'instant de la frappe pour le clavier) est converti en tick du transport, puis décalé de la latence de sortie estimée. L'estimation est imparfaite : la précision visée est de ± 1 ms **pour une entrée simulée à horodatage connu** (critère de RE-2, méthode notée), pas pour une salle réelle. Elle n'est pas annoncée comme une mesure.
- **Boost** (moteur natif, latence **mesurée**) : le helper horodate l'événement MIDI à son arrivée dans le callback audio de l'application, en échantillons, et la latence mesurée de la boucle est soustraite. Les notes arrivent en tick exacts, sans passer par le lien WebSocket (l'audio et les événements de jeu ne traversent pas ce lien, seuls les messages du moteur le font).
- Dans les deux modes le tick est calculé **dans le moteur**, à partir d'un compteur d'échantillons : la page ne place jamais de note, elle reçoit le rapport.
- Le réglage de latence est visible dans l'interface avec sa nature (estimé en Web, mesuré en Boost). Un décalage manuel en millisecondes est proposé en Web, puisque l'estimation peut être fausse.

**[Question ouverte 6]** Un réglage manuel de décalage d'enregistrement est-il exposé en Boost aussi ? Proposition : oui, pour les interfaces audio dont le pilote ment sur sa latence.

## 9. Les commandes

Toutes sont des commandes de `crates/commands/src/record.rs` (schéma, description pour les agents, annulation testée, test de correspondance avec les outils MCP). Le tableau suit le code.

| Commande | Entrée | Sortie | Annulation |
|---|---|---|---|
| `record.set_settings` | `offset_ms` (entier, -500 à 500) | l'ancien décalage | Aucune : c'est un état de session (il appartient à la machine), comme l'armement ; il est enregistré avec la session, jamais dans le projet, et n'entre pas dans l'historique (décision du lead, 2026-11-02). |
| `record.start` | `target` (`{"at": "scene", "scene": …}` ou `{"at": "arrangement"}`, par défaut l'arrangement), `latency_ms` (latence de sortie estimée par l'hôte, 0 à 1000) | le numéro de la prise, la cible, les pistes armées qui enregistrent, le décalage total `shift_ms`, s'il y a un décompte | Aucune : état de session. Refusée sans piste MIDI armée, ou tant qu'une prise n'est ni écrite ni abandonnée. |
| `record.stop` | rien | le numéro de la prise arrêtée, ou `null` | Aucune. Le transport continue ; la page écrit ensuite la prise avec `record.commit_take` dès que le moteur a rendu ses dernières notes. |
| `record.commit_take` | `take` (optionnel : la prise qu'il libère), `target`, `shift_ms`, par piste les notes jouées (début en ticks du transport, durée, hauteur, vélocité) et l'ancre du clip de la cellule | les clips touchés, les notes écartées | **Une** entrée d'historique pour toute la prise ; son inverse est `record.restore`. |
| `record.restore` | les clips à remettre (leur contenu et leurs placements, ou `null` pour les retirer) | rien | Le `record.restore` inverse. |
| `record.discard_take` | rien | le numéro de la prise abandonnée, ou `null` | Aucune : la prise n'avait rien écrit dans le projet. |
| `track.arm` | existe déjà | | existe déjà |
| `transport.set_metronome` | existe déjà (réglage du décompte) | | existe déjà |

`transport.stop` et `transport.pause` terminent aussi une prise en cours. La séparation entre `record.stop` et `record.commit_take` existe parce que le moteur, qui ne possède pas le projet, rend ses notes dans son rapport à la page, qui les écrit.

Le moteur reçoit `SetRecording` et rend les notes capturées dans son rapport (version 5 : numéro de la prise, nombre de notes, notes perdues, drapeau de capture, dernières notes), ce qui permet au piano roll de les montrer en direct (critère de RE-5). Il n'y a pas d'allocation sur le chemin audio : les tables de capture sont préparées avec le moteur, et une table pleine est comptée et affichée, jamais cachée.

**Différé** (reports acceptés par le lead le 2026-11-02) : le placement à ± 1 ms par horodatage des entrées en Boost (#501 ; en Web, livré par #499), le mode remplacement et la quantification à l'entrée (#500), l'enregistrement en Boost, désactivé jusqu'à BO-1 (#501).

## 10. Ce que les agents peuvent en faire

Un agent fait tout ce que l'humain fait, par les mêmes commandes :

- armer une piste et lancer `record.start` n'a de sens que s'il a une source de notes ; il le peut avec un contrôleur MIDI simulé (tests) ou en injectant des notes par `live.input_note_on` datées ;
- écrire directement des notes dans un clip avec `clip.notes.add` reste le chemin normal d'un agent : il ne joue pas, il compose. L'enregistrement ne sert l'agent que pour « joue ce que j'entends » avec un humain au clavier ;
- lire la prise (`clip.notes.list`) et la corriger (`clip.notes.quantize`, `clip.notes.transpose`), avec la même annulation ;
- les actions d'un agent arrivent dans le même historique que celles de l'humain : une prise lancée par un agent s'annule comme les autres.

Un agent ne peut pas armer et enregistrer à la place de l'humain sans que ce soit visible : l'état « enregistre » est dans le rapport du moteur et dans l'interface, et un agent qui le déclenche le fait dans la même session de projet.

## 11. Questions ouvertes

1. **Un ou deux états** pour « entrée en direct » et « armé pour enregistrer » (§ 3). Par défaut : un seul.
2. **Quelle vue décide de la cible** quand la grille et la timeline ont chacune une sélection (§ 4.1). Par défaut : la dernière vue active, cible affichée avant de lancer.
3. **Le remplacement est-il le défaut** pour un clip de grille en boucle (§ 4.2). Par défaut : non, ajout.
4. **Décompte quand le transport tourne déjà** (§ 5). Par défaut : non.
5. **Quantification à l'entrée dans le MVP** de l'enregistrement, ou après coup seulement (§ 6). Par défaut : après coup d'abord.
6. **Décalage manuel de latence en Boost** (§ 8). Par défaut : oui.
7. **Les notes tenues à l'arrêt** : une note encore enfoncée quand on arrête est-elle coupée à la fin de la prise, ou prolongée jusqu'au relâchement ? Par défaut : coupée à la fin de la prise.
8. **Le canal des contrôleurs** : l'enregistrement capture-t-il aussi les CC (molette, pédale de sustain) en automation de la piste ? Par défaut : non dans la première version, les notes seulement ; la pédale de sustain serait à traiter à part.

Les réponses 1, 2, 3 et 8 sont des choix de produit du fondateur ; les autres sont techniques et peuvent être prises à l'implémentation.

## 12. Hors périmètre

L'enregistrement audio, la capture de CC en automation, la prise multi-pistes avec des canaux MIDI distincts d'un même contrôleur, le « loop recording » avec choix de la meilleure prise (comping). Chacun aura sa spec.
