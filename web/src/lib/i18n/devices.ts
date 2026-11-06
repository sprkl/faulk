/**
 * The device panel (`src/views/devices/`) and the tooltips of the parameters. A tooltip is keyed
 * `<device kind without "faulk.">.<parameter key>.hint`; its English text is the description in the
 * device's descriptor (`i18n.test.ts` checks they are the same), its French text is a translation.
 * A device name is not here: it comes from the descriptor, one English name never translated.
 * Labels of the parameters and of their values are in `synth.ts`, `drums.ts`, `echo.ts`, `reverb.ts`, `filter.ts`, `compressor.ts`, `trim.ts` and `hiccup.ts`.
 */

import { defineFeature } from './feature'

export const devices = defineFeature(
  {
    'devices.noTrack': 'Select a track to see its instrument and effects.',
    'devices.chain': 'Devices of {name}',
    'devices.unknown': 'Unknown device',
    'devices.noParams': 'No settings shown for this device.',
    'devices.bypass': 'Bypass {name}',
    'devices.gainReduction': 'Gain reduction',
    'devices.moveLeft': 'Move {name} earlier in the chain',
    'devices.moveRight': 'Move {name} later in the chain',
    'devices.remove': 'Remove {name}',
    'devices.browser': 'Instruments and effects',
    'devices.instruments': 'Instruments',
    'devices.effects': 'Effects',
    'devices.midiEffects': 'MIDI effects',
    'devices.use': 'Use {name} as the instrument',
    'devices.add': 'Add {name} to the track',
    'devices.addMaster': 'Add {name} to the master',
    'devices.browserMaster': 'Effects of the master',
    'devices.masterHint': 'The master’s effects process the sum of all the tracks, before the master volume.',
    'devices.audioTrack': 'An audio track plays clips: it has no instrument to choose.',
    'devices.noEffects': 'No effect yet.',
    'osc_synth.waveform.hint': 'Shape of the oscillator: sine, saw, square or triangle.',
    'osc_synth.attack.hint': 'Time the note takes to reach full level after it starts, in seconds.',
    'osc_synth.decay.hint': 'Time the level takes to fall from full to the sustain level, in seconds.',
    'osc_synth.sustain.hint': 'Level held while the note is down, from 0 to 1 of full level.',
    'osc_synth.release.hint': 'Time the level takes to fade out after the note ends, in seconds.',
    'osc_synth.gain.hint': 'Output level of the synth, from 0 to 1 (linear gain).',
    'drums.kick_tune.hint': 'Pitch the kick settles on after its fast fall, in hertz.',
    'drums.kick_decay.hint': 'How long the kick rings, in seconds.',
    'drums.kick_punch.hint': 'Depth of the initial pitch fall of the kick: 0 is soft, 1 is the sharpest attack.',
    'drums.kick_level.hint': 'Loudness of the kick, from 0 to 1 (linear gain).',
    'drums.snare_tune.hint': 'Pitch of the snare tone, in hertz.',
    'drums.snare_decay.hint': 'How long the snare rings, in seconds.',
    'drums.snare_snap.hint': 'Balance between the tone (0) and the noise (1) of the snare.',
    'drums.snare_level.hint': 'Loudness of the snare, from 0 to 1 (linear gain).',
    'drums.clap_tone.hint': 'Centre frequency of the clap noise, in hertz.',
    'drums.clap_decay.hint': 'How long the clap rings, in seconds.',
    'drums.clap_level.hint': 'Loudness of the clap, from 0 to 1 (linear gain).',
    'drums.hat_tone.hint': 'Brightness of the hi-hat, as the pitch of its metallic tone in hertz.',
    'drums.hat_decay.hint': 'How long the closed hi-hat rings, in seconds.',
    'drums.hat_open_decay.hint': 'How long the open hi-hat rings, in seconds.',
    'drums.hat_level.hint': 'Loudness of the hi-hat, from 0 to 1 (linear gain).',
    'drums.gain.hint': 'Output level of the whole kit, from 0 to 1 (linear gain).',
    'echo.mode.hint':
      'Whether the delay time is free (the time setting, in seconds) or locked to the project tempo (the note value setting). A switch takes effect as time_change says.',
    'echo.time.hint': 'Delay time in free mode, from 1 millisecond to 4 seconds. A new value takes effect as time_change says.',
    'echo.division.hint':
      'Delay time in synced mode, as a note value of the project tempo: 1/32 to a whole note, with triplets (T) and dotted values (.). Times beyond 4 seconds are shortened to 4 seconds. A new value takes effect as time_change says.',
    'echo.feedback.hint': 'Share of the echo fed back into the delay, from 0 to 0.95: more repeats, each one softer.',
    'echo.mix.hint': 'Balance between the dry sound (0) and the echoes (1).',
    'echo.time_change.hint':
      'What a new time, note value, tempo or mode does while the echo rings. Glide: slides like a tape, pitch of repeats bends up to 25%. Fade: 50 ms crossfade, pitch kept, later changes wait. Jump: at once (tempo: next block), may click.',
    'reverb.size.hint': 'Size of the room, from 0 to 1: a small room (about 0.3 seconds of reverberation) up to a hall (about 7.5 seconds).',
    'reverb.damping.hint': 'How fast the high frequencies die in the tail, from 0 to 1: bright and open at 0, dark and soft at 1.',
    'reverb.predelay.hint':
      'Time between the dry sound and the start of the reverb, from 0 to 0.2 seconds: more room to hear the sound before its tail. A new value takes effect as time_change says.',
    'reverb.time_change.hint':
      'What a new pre-delay does while the reverb rings. Glide: slides like a tape, pitch of the tail bends up to 25%. Fade: 50 ms crossfade, pitch kept, later changes wait. Jump: at once, may click.',
    'reverb.mix.hint': 'Balance between the dry sound (0) and the reverb (1).',
    'filter.mode.hint':
      'What the filter lets through: the sound below the cutoff (low-pass), above it (high-pass) or around it (band-pass). A change fades over 10 ms.',
    'filter.cutoff.hint': 'Frequency, in hertz, where the filter starts to cut, from 20 Hz to 20 kHz.',
    'filter.resonance.hint': 'Boost of the sound around the cutoff, from 0 (none) to 1 (a sharp peak that stays stable).',
    'filter.mix.hint': 'Balance between the dry sound (0) and the filtered sound (1).',
    'stack.mode.hint':
      "How the voices' intervals are read: in semitones, or in degrees of the project's scale (a third is two degrees) so the chord stays in key. Without a project scale, in semitones.",
    'stack.strum.hint': 'Time from the first to the last note of the chord, from 0 (all at once) to 0.4 s: the notes are spread like a strummed guitar.',
    'stack.tension.hint': "How the strum's gaps change, from -100 % (they widen toward the last note) to +100 % (they tighten); 0 spaces the notes evenly.",
    'stack.crescendo.hint':
      'Velocity across the strum: above 0 the notes grow louder toward the last, below 0 softer; at 100 % the first (or last) note is at the lowest velocity.',
    'stack.direction.hint': 'Order of the strum: up plays from the lowest note to the highest, down from the highest to the lowest.',
    'stack.seed.hint':
      'Seed of the chance draws, from 0 to 1000: the same seed gives the same chords on every play from the start; another seed, another draw.',
    'stack.voice1.hint': 'Whether added voice 1 plays.',
    'stack.voice1_interval.hint': "Interval of voice 1 from the received note, from -36 to +36: semitones, or degrees of the project's scale in scale mode.",
    'stack.voice1_velocity.hint': "Velocity of voice 1, from 1 to 200 % of the received note's.",
    'stack.voice1_chance.hint': 'Chance that voice 1 plays on each received note, from 0 to 100 %: a repeatable draw, set by the seed.',
    'stack.voice2.hint': 'Whether added voice 2 plays.',
    'stack.voice2_interval.hint': "Interval of voice 2 from the received note, from -36 to +36: semitones, or degrees of the project's scale in scale mode.",
    'stack.voice2_velocity.hint': "Velocity of voice 2, from 1 to 200 % of the received note's.",
    'stack.voice2_chance.hint': 'Chance that voice 2 plays on each received note, from 0 to 100 %: a repeatable draw, set by the seed.',
    'stack.voice3.hint': 'Whether added voice 3 plays.',
    'stack.voice3_interval.hint': "Interval of voice 3 from the received note, from -36 to +36: semitones, or degrees of the project's scale in scale mode.",
    'stack.voice3_velocity.hint': "Velocity of voice 3, from 1 to 200 % of the received note's.",
    'stack.voice3_chance.hint': 'Chance that voice 3 plays on each received note, from 0 to 100 %: a repeatable draw, set by the seed.',
    'stack.voice4.hint': 'Whether added voice 4 plays.',
    'stack.voice4_interval.hint': "Interval of voice 4 from the received note, from -36 to +36: semitones, or degrees of the project's scale in scale mode.",
    'stack.voice4_velocity.hint': "Velocity of voice 4, from 1 to 200 % of the received note's.",
    'stack.voice4_chance.hint': 'Chance that voice 4 plays on each received note, from 0 to 100 %: a repeatable draw, set by the seed.',
    'stack.voice5.hint': 'Whether added voice 5 plays.',
    'stack.voice5_interval.hint': "Interval of voice 5 from the received note, from -36 to +36: semitones, or degrees of the project's scale in scale mode.",
    'stack.voice5_velocity.hint': "Velocity of voice 5, from 1 to 200 % of the received note's.",
    'stack.voice5_chance.hint': 'Chance that voice 5 plays on each received note, from 0 to 100 %: a repeatable draw, set by the seed.',
    'stack.voice6.hint': 'Whether added voice 6 plays.',
    'stack.voice6_interval.hint': "Interval of voice 6 from the received note, from -36 to +36: semitones, or degrees of the project's scale in scale mode.",
    'stack.voice6_velocity.hint': "Velocity of voice 6, from 1 to 200 % of the received note's.",
    'stack.voice6_chance.hint': 'Chance that voice 6 plays on each received note, from 0 to 100 %: a repeatable draw, set by the seed.',
    'compressor.threshold.hint': 'Level, in decibels, above which the compressor turns the sound down, from -60 to 0 dB.',
    'compressor.ratio.hint':
      'How strongly the sound above the threshold is turned down, from 1 (not at all) to 20: at 4, a signal 8 dB over the threshold comes out 2 dB over it.',
    'compressor.attack.hint':
      'Time the compressor takes to turn the sound down once it passes the threshold, from 0.1 ms to 200 ms: short catches the first hit, long lets it through.',
    'compressor.release.hint': 'Time the compressor takes to let the sound back up once it falls under the threshold, from 10 ms to 2 s.',
    'compressor.makeup.hint': 'Gain added after the compression to make up for the level it took away, from 0 to 24 dB.',
    'trim.mode.hint': 'How the length is set: a time in seconds (free) or a note value that follows the project tempo (synced).',
    'trim.time.hint': 'Length of every note in free mode, in seconds, from 1 ms to 10 s.',
    'trim.division.hint': 'Length of every note in synced mode, as a note value of the project tempo, from a 64th note to 4 bars of the time signature.',
    'trim.feel.hint': 'Straight, triplet (two thirds of the note value) or dotted (one and a half times the note value), in synced mode.',
    'trim.gate.hint': 'Share of the note value a note lasts in synced mode, from 1 % to 200 % (100 %: exactly the note value).',
    'trim.trigger.hint': 'When a note comes out: when the key is pressed (start), or when it is let go (release), at the velocity it was pressed with.',
    'trim.key_track.hint': 'How the length follows the pitch, from -100 % to +100 %: at +100 % it doubles one octave below C4 and halves one octave above.',
    'hiccup.interval.hint':
      "How often a slice may be captured, on the transport's grid: every 1/16 note to every 4 bars of the time signature. Captures happen only while the transport plays.",
    'hiccup.chance.hint':
      'Chance that a capture happens at each grid point, from 0 to 100 %. The draw depends only on the seed and the grid position: playback and export give the same captures.',
    'hiccup.seed.hint': 'Seed of the chance and random length draws, from 0 to 1000: the same seed gives the same captures, another seed another sequence.',
    'hiccup.capture.hint':
      'Manual capture: turning it on captures at once and the repeats last while it stays on; off ends them. A capture starts only when the value turns on; a transport stop ends it.',
    'hiccup.size.hint':
      'Length of the repeated slice, as a note value of the project tempo: 1/64 to 1 bar, with triplets (T). A slice longer than 12 seconds is shortened to 12 seconds.',
    'hiccup.gate.hint':
      'How long the repeats last after a capture, as a share of the interval, from 1 to 100 %. The slice plays first, then repeats until the gate ends.',
    'hiccup.variation.hint':
      'Length of each repeat. Fixed: the slice. Shorter: halved every two slices, a roll speeding up (1/8, 1/16, 1/32). Random: the slice, half or a quarter, from the seed. Never under 1/8 of the slice nor 40 ms, unless the slice is shorter.',
    'hiccup.decay.hint':
      'Level lost at each repeat, from 0 to 100 %: at 50 %, each repeat is half as loud as the one before. A change applies from the next repeat.',
    'hiccup.filter.hint':
      'A filter that closes over the repeats. Low-pass: from 20 kHz down to 200 Hz at the end of the gate, darker at each repeat. High-pass: from 20 Hz up to 2 kHz, thinner at each repeat. A change applies from the next capture.',
    'hiccup.pitch.hint':
      'Pitch of the repeats, from -12 to +12 semitones, by playing the slice faster or slower. Raised, a repeat ends when the slice runs out. A change applies from the next repeat.',
    'hiccup.mode.hint':
      'How the repeats meet the sound. Layer: on top of it. Insert: the sound is muted while the repeats play. Repeats only: only the captured slices and their repeats are heard.',
    'hiccup.mix.hint': 'Level of the repeats, from 0 to 100 %. In insert mode it also sets how much the sound is muted while they play.',
  },
  {
    'devices.noTrack': 'Sélectionne une piste pour voir son instrument et ses effets.',
    'devices.chain': 'Devices de {name}',
    'devices.unknown': 'Device inconnu',
    'devices.noParams': 'Aucun réglage affiché pour ce device.',
    'devices.bypass': 'Contourner {name}',
    'devices.gainReduction': 'Réduction de gain',
    'devices.moveLeft': 'Avancer {name} dans la chaîne',
    'devices.moveRight': 'Reculer {name} dans la chaîne',
    'devices.remove': 'Retirer {name}',
    'devices.browser': 'Instruments et effets',
    'devices.instruments': 'Instruments',
    'devices.effects': 'Effets',
    'devices.midiEffects': 'Effets MIDI',
    'devices.use': 'Utiliser {name} comme instrument',
    'devices.add': 'Ajouter {name} à la piste',
    'devices.addMaster': 'Ajouter {name} au master',
    'devices.browserMaster': 'Effets du master',
    'devices.masterHint': 'Les effets du master traitent la somme de toutes les pistes, avant le volume du master.',
    'devices.audioTrack': 'Une piste audio joue des clips : elle n’a pas d’instrument à choisir.',
    'devices.noEffects': 'Aucun effet pour l’instant.',
    'osc_synth.waveform.hint': 'Forme de l’oscillateur : sinus, dent de scie, carré ou triangle.',
    'osc_synth.attack.hint': 'Temps que la note met à atteindre son plein niveau, en secondes.',
    'osc_synth.decay.hint': 'Temps que le niveau met à descendre du plein niveau au niveau de maintien, en secondes.',
    'osc_synth.sustain.hint': 'Niveau tenu tant que la note est enfoncée, de 0 à 1 du plein niveau.',
    'osc_synth.release.hint': 'Temps que le son met à s’éteindre après la fin de la note, en secondes.',
    'osc_synth.gain.hint': 'Niveau de sortie du synthé, de 0 à 1 (gain linéaire).',
    'drums.kick_tune.hint': 'Hauteur sur laquelle la grosse caisse se stabilise après sa chute rapide, en hertz.',
    'drums.kick_decay.hint': 'Durée de résonance de la grosse caisse, en secondes.',
    'drums.kick_punch.hint': 'Profondeur de la chute de hauteur initiale de la grosse caisse : 0 est doux, 1 donne l’attaque la plus vive.',
    'drums.kick_level.hint': 'Volume de la grosse caisse, de 0 à 1 (gain linéaire).',
    'drums.snare_tune.hint': 'Hauteur du timbre de la caisse claire, en hertz.',
    'drums.snare_decay.hint': 'Durée de résonance de la caisse claire, en secondes.',
    'drums.snare_snap.hint': 'Équilibre entre le timbre de la caisse claire (0) et son bruit (1).',
    'drums.snare_level.hint': 'Volume de la caisse claire, de 0 à 1 (gain linéaire).',
    'drums.clap_tone.hint': 'Fréquence centrale du bruit du clap, en hertz.',
    'drums.clap_decay.hint': 'Durée de résonance du clap, en secondes.',
    'drums.clap_level.hint': 'Volume du clap, de 0 à 1 (gain linéaire).',
    'drums.hat_tone.hint': 'Brillance du charleston, donnée par la hauteur de son timbre métallique en hertz.',
    'drums.hat_decay.hint': 'Durée de résonance du charleston fermé, en secondes.',
    'drums.hat_open_decay.hint': 'Durée de résonance du charleston ouvert, en secondes.',
    'drums.hat_level.hint': 'Volume du charleston, de 0 à 1 (gain linéaire).',
    'drums.gain.hint': 'Niveau de sortie de toute la batterie, de 0 à 1 (gain linéaire).',
    'echo.mode.hint':
      'Durée libre (réglage de durée, en secondes) ou calée sur le tempo du projet (réglage de valeur de note). Un changement de mode agit comme le dit « Au changement ».',
    'echo.time.hint': 'Durée du retard en mode libre, de 1 milliseconde à 4 secondes. Une nouvelle valeur agit comme le dit « Au changement ».',
    'echo.division.hint':
      'Durée du retard en mode au tempo, en valeur de note du tempo du projet : du 1/32 à la ronde, avec triolets (T) et valeurs pointées (.). Au-delà de 4 secondes, la durée est ramenée à 4 secondes. Une nouvelle valeur agit comme le dit « Au changement ».',
    'echo.feedback.hint': 'Part de l’écho renvoyée dans le retard, de 0 à 0,95 : plus de répétitions, chacune plus faible.',
    'echo.mix.hint': 'Équilibre entre le son direct (0) et les échos (1).',
    'echo.time_change.hint':
      'Ce que fait une nouvelle durée, valeur de note, un nouveau tempo ou mode pendant que l’écho sonne. Glissé : glisse comme une bande, la hauteur des répétitions se courbe jusqu’à 25 %. Fondu : fondu enchaîné de 50 ms, hauteur conservée, les changements suivants attendent. Saut : immédiat (tempo : au bloc suivant), peut claquer.',
    'reverb.size.hint': 'Taille de la pièce, de 0 à 1 : d’une petite pièce (environ 0,3 seconde de réverbération) à une grande salle (environ 7,5 secondes).',
    'reverb.damping.hint': 'Vitesse à laquelle les aigus s’éteignent dans la queue, de 0 à 1 : clair et ouvert à 0, sombre et doux à 1.',
    'reverb.predelay.hint':
      'Temps entre le son direct et le début de la réverbération, de 0 à 0,2 seconde : plus de place pour entendre le son avant sa queue. Une nouvelle valeur agit comme le dit « Au changement ».',
    'reverb.time_change.hint':
      'Ce que fait un nouveau pré-délai pendant que la réverbération sonne. Glissé : glisse comme une bande, la hauteur de la queue se courbe jusqu’à 25 %. Fondu : fondu enchaîné de 50 ms, hauteur conservée, les changements suivants attendent. Saut : immédiat, peut claquer.',
    'reverb.mix.hint': 'Équilibre entre le son direct (0) et la réverbération (1).',
    'filter.mode.hint':
      'Ce que le filtre laisse passer : le son sous la fréquence de coupure (passe-bas), au-dessus (passe-haut) ou autour (passe-bande). Un changement se fait en fondu de 10 ms.',
    'filter.cutoff.hint': 'Fréquence, en hertz, à partir de laquelle le filtre coupe, de 20 Hz à 20 kHz.',
    'filter.resonance.hint': 'Renforcement du son autour de la fréquence de coupure, de 0 (aucun) à 1 (un pic net qui reste stable).',
    'filter.mix.hint': 'Équilibre entre le son direct (0) et le son filtré (1).',
    'stack.mode.hint':
      'Comment lire les intervalles des voix : en demi-tons, ou en degrés de la gamme du projet (une tierce vaut deux degrés) pour que l’accord reste dans la tonalité. Sans gamme réglée, en demi-tons.',
    'stack.strum.hint': 'Temps entre la première et la dernière note de l’accord, de 0 (toutes ensemble) à 0,4 s : les notes s’égrènent comme sur une guitare.',
    'stack.tension.hint':
      'Comment évoluent les écarts de l’égrenage, de -100 % (ils s’étalent vers la dernière note) à +100 % (ils se resserrent) ; 0 espace les notes également.',
    'stack.crescendo.hint':
      'Vélocité le long de l’égrenage : au-dessus de 0 les notes montent vers la dernière, en dessous elles baissent ; à 100 % la première (ou la dernière) est à la vélocité la plus basse.',
    'stack.direction.hint': 'Ordre de l’égrenage : montant joue de la note la plus grave à la plus aiguë, descendant de la plus aiguë à la plus grave.',
    'stack.seed.hint':
      'Graine des tirages de chance, de 0 à 1000 : la même graine donne les mêmes accords à chaque lecture depuis le début ; une autre graine, un autre tirage.',
    'stack.voice1.hint': 'Si la voix ajoutée 1 joue.',
    'stack.voice1_interval.hint':
      'Intervalle de la voix 1 depuis la note reçue, de -36 à +36 : en demi-tons, ou en degrés de la gamme du projet dans le mode gamme.',
    'stack.voice1_velocity.hint': 'Vélocité de la voix 1, de 1 à 200 % de celle de la note reçue.',
    'stack.voice1_chance.hint': 'Chance que la voix 1 joue à chaque note reçue, de 0 à 100 % : un tirage reproductible, fixé par la graine.',
    'stack.voice2.hint': 'Si la voix ajoutée 2 joue.',
    'stack.voice2_interval.hint':
      'Intervalle de la voix 2 depuis la note reçue, de -36 à +36 : en demi-tons, ou en degrés de la gamme du projet dans le mode gamme.',
    'stack.voice2_velocity.hint': 'Vélocité de la voix 2, de 1 à 200 % de celle de la note reçue.',
    'stack.voice2_chance.hint': 'Chance que la voix 2 joue à chaque note reçue, de 0 à 100 % : un tirage reproductible, fixé par la graine.',
    'stack.voice3.hint': 'Si la voix ajoutée 3 joue.',
    'stack.voice3_interval.hint':
      'Intervalle de la voix 3 depuis la note reçue, de -36 à +36 : en demi-tons, ou en degrés de la gamme du projet dans le mode gamme.',
    'stack.voice3_velocity.hint': 'Vélocité de la voix 3, de 1 à 200 % de celle de la note reçue.',
    'stack.voice3_chance.hint': 'Chance que la voix 3 joue à chaque note reçue, de 0 à 100 % : un tirage reproductible, fixé par la graine.',
    'stack.voice4.hint': 'Si la voix ajoutée 4 joue.',
    'stack.voice4_interval.hint':
      'Intervalle de la voix 4 depuis la note reçue, de -36 à +36 : en demi-tons, ou en degrés de la gamme du projet dans le mode gamme.',
    'stack.voice4_velocity.hint': 'Vélocité de la voix 4, de 1 à 200 % de celle de la note reçue.',
    'stack.voice4_chance.hint': 'Chance que la voix 4 joue à chaque note reçue, de 0 à 100 % : un tirage reproductible, fixé par la graine.',
    'stack.voice5.hint': 'Si la voix ajoutée 5 joue.',
    'stack.voice5_interval.hint':
      'Intervalle de la voix 5 depuis la note reçue, de -36 à +36 : en demi-tons, ou en degrés de la gamme du projet dans le mode gamme.',
    'stack.voice5_velocity.hint': 'Vélocité de la voix 5, de 1 à 200 % de celle de la note reçue.',
    'stack.voice5_chance.hint': 'Chance que la voix 5 joue à chaque note reçue, de 0 à 100 % : un tirage reproductible, fixé par la graine.',
    'stack.voice6.hint': 'Si la voix ajoutée 6 joue.',
    'stack.voice6_interval.hint':
      'Intervalle de la voix 6 depuis la note reçue, de -36 à +36 : en demi-tons, ou en degrés de la gamme du projet dans le mode gamme.',
    'stack.voice6_velocity.hint': 'Vélocité de la voix 6, de 1 à 200 % de celle de la note reçue.',
    'stack.voice6_chance.hint': 'Chance que la voix 6 joue à chaque note reçue, de 0 à 100 % : un tirage reproductible, fixé par la graine.',
    'compressor.threshold.hint': 'Niveau, en décibels, au-dessus duquel le compresseur baisse le son, de -60 à 0 dB.',
    'compressor.ratio.hint':
      'Force avec laquelle le son au-dessus du seuil est baissé, de 1 (pas du tout) à 20 : à 4, un signal 8 dB au-dessus du seuil ressort 2 dB au-dessus.',
    'compressor.attack.hint':
      'Temps que le compresseur met à baisser le son une fois le seuil dépassé, de 0,1 ms à 200 ms : court attrape le début du coup, long le laisse passer.',
    'compressor.release.hint': 'Temps que le compresseur met à relever le son une fois repassé sous le seuil, de 10 ms à 2 s.',
    'compressor.makeup.hint': 'Gain ajouté après la compression pour rattraper le niveau qu’elle a retiré, de 0 à 24 dB.',
    'trim.mode.hint': 'Comment la longueur est réglée : une durée en secondes (libre) ou une valeur de note qui suit le tempo du projet (au tempo).',
    'trim.time.hint': 'Longueur de chaque note en mode libre, en secondes, de 1 ms à 10 s.',
    'trim.division.hint':
      'Longueur de chaque note en mode au tempo, en valeur de note du tempo du projet, de la quadruple croche à 4 mesures de la signature rythmique.',
    'trim.feel.hint': 'Binaire, triolet (les deux tiers de la valeur) ou pointé (une fois et demie la valeur), en mode au tempo.',
    'trim.gate.hint': 'Part de la valeur de note que dure une note en mode au tempo, de 1 % à 200 % (100 % : exactement la valeur).',
    'trim.trigger.hint': 'Quand la note sort : à l’appui sur la touche, ou au relâchement, à la vélocité de l’appui.',
    'trim.key_track.hint':
      'Comment la longueur suit la hauteur, de -100 % à +100 % : à +100 %, elle double une octave sous le do4 et diminue de moitié une octave au-dessus.',
    'hiccup.interval.hint':
      'Fréquence des captures possibles, sur la grille du transport : d’une double croche à 4 mesures de la signature rythmique. Les captures n’ont lieu que pendant la lecture.',
    'hiccup.chance.hint':
      'Probabilité qu’une capture ait lieu à chaque point de la grille, de 0 à 100 %. Le tirage ne dépend que de la graine et de la position : la lecture et l’export donnent les mêmes captures.',
    'hiccup.seed.hint':
      'Graine des tirages de la probabilité et des longueurs aléatoires, de 0 à 1000 : la même graine donne les mêmes captures, une autre graine une autre suite.',
    'hiccup.capture.hint':
      'Capture manuelle : l’activer capture aussitôt, et les répétitions durent tant qu’elle reste active ; la désactiver les arrête. Une capture ne part qu’au passage à Marche ; l’arrêt du transport la termine.',
    'hiccup.size.hint':
      'Longueur de la tranche répétée, en valeur de note du tempo du projet : de la quadruple croche à 1 mesure, avec des triolets (T). Une tranche de plus de 12 secondes est ramenée à 12 secondes.',
    'hiccup.gate.hint':
      'Durée des répétitions après une capture, en part de l’intervalle, de 1 à 100 %. La tranche passe d’abord, puis se répète jusqu’à la fin de la durée.',
    'hiccup.variation.hint':
      'Longueur de chaque répétition. Fixe : la tranche. Plus courte : divisée par deux toutes les deux tranches, un roulement qui accélère (1/8, 1/16, 1/32). Aléatoire : la tranche, sa moitié ou son quart, selon la graine. Jamais moins d’un huitième de la tranche ni de 40 ms, sauf si la tranche est plus courte.',
    'hiccup.decay.hint':
      'Niveau perdu à chaque répétition, de 0 à 100 % : à 50 %, chaque répétition est deux fois moins forte que la précédente. Un changement s’applique dès la répétition suivante.',
    'hiccup.filter.hint':
      'Un filtre qui se ferme au fil des répétitions. Passe-bas : de 20 kHz à 200 Hz à la fin de la durée, plus sombre à chaque répétition. Passe-haut : de 20 Hz à 2 kHz, plus fin à chaque répétition. Un changement s’applique dès la capture suivante.',
    'hiccup.pitch.hint':
      'Hauteur des répétitions, de -12 à +12 demi-tons, en lisant la tranche plus vite ou plus lentement. Plus aiguë, une répétition s’arrête à la fin de la tranche. Un changement s’applique dès la répétition suivante.',
    'hiccup.mode.hint':
      'Comment les répétitions rejoignent le son. Superposé : par-dessus. Insertion : le son se tait pendant les répétitions. Répétitions seules : on n’entend que les tranches capturées et leurs répétitions.',
    'hiccup.mix.hint': 'Niveau des répétitions, de 0 à 100 %. En mode insertion, il règle aussi combien le son se tait pendant qu’elles jouent.',
  },
)
