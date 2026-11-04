/**
 * The device panel (`src/views/devices/`) and the tooltips of the parameters. A tooltip is keyed
 * `<device kind without "faulk.">.<parameter key>.hint`; its English text is the description in the
 * device's descriptor (`i18n.test.ts` checks they are the same), its French text is a translation.
 * A device name is not here: it comes from the descriptor, one English name never translated.
 * Labels of the parameters and of their values are in `synth.ts`, `drums.ts`, `echo.ts`, `reverb.ts`, `filter.ts` and `compressor.ts`.
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
      'What the filter lets through: the sound below the cutoff (low-pass), above it (high-pass), around it (band-pass) or all but it (notch). A change fades over 10 ms.',
    'filter.cutoff.hint':
      'Frequency, in hertz, where the filter starts to cut, from 20 Hz to 20 kHz: the centre the LFO and the envelope move the cutoff around.',
    'filter.resonance.hint': 'Boost of the sound around the cutoff, from 0 (none) to 1 (a sharp peak that stays stable).',
    'filter.mix.hint': 'Balance between the dry sound (0) and the filtered sound (1).',
    'filter.drive.hint': 'Soft saturation before the filter, from 0 (none) to 1: louder and rounder, so a strong resonance bites instead of whistling.',
    'filter.lfo_shape.hint': 'Shape of the LFO that moves the cutoff: sine, triangle, rising saw, falling saw, square, or random (a new value at each cycle).',
    'filter.lfo_sync.hint': 'Whether the LFO runs at its own rate in hertz (free) or follows the tempo (synced), each cycle on the same beat of every bar.',
    'filter.lfo_rate.hint': 'Speed of the LFO in free mode, in cycles per second, from 0.01 to 20 Hz.',
    'filter.lfo_division.hint': 'Length of one LFO cycle in synced mode, a note value from 1/32 to 8 bars.',
    'filter.lfo_phase.hint': 'Where in its cycle the LFO starts, from 0 to 360 degrees: in synced mode, where in the bar the cutoff peaks.',
    'filter.lfo_stereo.hint': "Offset of the right channel's LFO, from 0 to 180 degrees: at 180 the two channels open in opposition.",
    'filter.lfo_depth.hint': 'How far the LFO moves the cutoff either side of its centre, in octaves, from 0 (no movement) to 5.',
    'filter.lfo_restart.hint': 'In free mode, whether the LFO starts again from its phase each time playback starts (on) or runs on freely (off).',
    'filter.env_amount.hint':
      'How far the level of the incoming sound moves the cutoff, in octaves at full level, from -5 (louder closes) to +5 (louder opens); 0 for none.',
    'filter.env_attack.hint': 'Time the envelope follower takes to rise with the sound, from 1 ms to 500 ms.',
    'filter.env_release.hint': 'Time the envelope follower takes to fall back when the sound gets quieter, from 10 ms to 2 s.',
    'compressor.threshold.hint': 'Level, in decibels, above which the compressor turns the sound down, from -60 to 0 dB.',
    'compressor.ratio.hint':
      'How strongly the sound above the threshold is turned down, from 1 (not at all) to 20: at 4, a signal 8 dB over the threshold comes out 2 dB over it.',
    'compressor.attack.hint':
      'Time the compressor takes to turn the sound down once it passes the threshold, from 0.1 ms to 200 ms: short catches the first hit, long lets it through.',
    'compressor.release.hint': 'Time the compressor takes to let the sound back up once it falls under the threshold, from 10 ms to 2 s.',
    'compressor.makeup.hint': 'Gain added after the compression to make up for the level it took away, from 0 to 24 dB.',
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
      'Ce que le filtre laisse passer : le son sous la fréquence de coupure (passe-bas), au-dessus (passe-haut), autour (passe-bande) ou tout sauf elle (coupe-bande). Un changement se fait en fondu de 10 ms.',
    'filter.cutoff.hint':
      'Fréquence, en hertz, à partir de laquelle le filtre coupe, de 20 Hz à 20 kHz : le centre autour duquel le LFO et l’enveloppe font bouger la coupure.',
    'filter.resonance.hint': 'Renforcement du son autour de la fréquence de coupure, de 0 (aucun) à 1 (un pic net qui reste stable).',
    'filter.mix.hint': 'Équilibre entre le son direct (0) et le son filtré (1).',
    'filter.drive.hint':
      'Saturation douce avant le filtre, de 0 (aucune) à 1 : plus fort et plus rond, pour qu’une résonance poussée morde au lieu de siffler.',
    'filter.lfo_shape.hint':
      'Forme du LFO qui fait bouger la coupure : sinus, triangle, scie montante, scie descendante, carré, ou aléatoire (une nouvelle valeur à chaque cycle).',
    'filter.lfo_sync.hint': 'Le LFO va à sa propre vitesse en hertz (libre) ou suit le tempo (au tempo), chaque cycle sur le même temps de chaque mesure.',
    'filter.lfo_rate.hint': 'Vitesse du LFO en mode libre, en cycles par seconde, de 0,01 à 20 Hz.',
    'filter.lfo_division.hint': 'Durée d’un cycle du LFO au tempo, une valeur de note de 1/32 à 8 mesures.',
    'filter.lfo_phase.hint': 'Où le LFO commence dans son cycle, de 0 à 360 degrés : au tempo, l’endroit de la mesure où la coupure culmine.',
    'filter.lfo_stereo.hint': 'Décalage du LFO du canal droit, de 0 à 180 degrés : à 180, les deux canaux s’ouvrent en opposition.',
    'filter.lfo_depth.hint': 'De combien le LFO déplace la coupure de part et d’autre de son centre, en octaves, de 0 (aucun mouvement) à 5.',
    'filter.lfo_restart.hint': 'En mode libre, le LFO repart de sa phase à chaque démarrage de la lecture (oui) ou continue librement (non).',
    'filter.env_amount.hint':
      'De combien le niveau du son entrant déplace la coupure, en octaves à pleine échelle, de -5 (plus fort ferme) à +5 (plus fort ouvre) ; 0 pour aucun effet.',
    'filter.env_attack.hint': 'Temps que le suiveur d’enveloppe met à monter avec le son, de 1 ms à 500 ms.',
    'filter.env_release.hint': 'Temps que le suiveur d’enveloppe met à redescendre quand le son baisse, de 10 ms à 2 s.',
    'compressor.threshold.hint': 'Niveau, en décibels, au-dessus duquel le compresseur baisse le son, de -60 à 0 dB.',
    'compressor.ratio.hint':
      'Force avec laquelle le son au-dessus du seuil est baissé, de 1 (pas du tout) à 20 : à 4, un signal 8 dB au-dessus du seuil ressort 2 dB au-dessus.',
    'compressor.attack.hint':
      'Temps que le compresseur met à baisser le son une fois le seuil dépassé, de 0,1 ms à 200 ms : court attrape le début du coup, long le laisse passer.',
    'compressor.release.hint': 'Temps que le compresseur met à relever le son une fois repassé sous le seuil, de 10 ms à 2 s.',
    'compressor.makeup.hint': 'Gain ajouté après la compression pour rattraper le niveau qu’elle a retiré, de 0 à 24 dB.',
  },
)
