/**
 * The device panel (`src/views/devices/`) and the tooltips of the parameters. A tooltip is keyed
 * `<device kind without "faulk.">.<parameter key>.hint`; its English text is the description in the
 * device's descriptor (`i18n.test.ts` checks they are the same), its French text is a translation.
 * A device name is not here: it comes from the descriptor, one English name never translated.
 * Labels of the parameters and of their values are in `synth.ts`, `drums.ts`, `echo.ts`, `reverb.ts`, `filter.ts`, `compressor.ts`, `ceiling.ts`, `trim.ts`, `hiccup.ts` and `ripple.ts`.
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
      'What the filter lets through: the sound below the cutoff (low-pass), above it (high-pass), around it (band-pass) or all but it (notch). A change fades over 10 ms.',
    'filter.cutoff.hint':
      'Frequency, in hertz, where the filter starts to cut, from 20 Hz to 20 kHz: the centre the LFO and the envelope move the cutoff around.',
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
      'Seed of the chance draws, from 0 to 1000: a note draws from the seed, its position in the song and its pitch, so the same seed gives the same chords at the same place on every play, from any start; another seed, another draw.',
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
    'lattice.source.hint':
      "Which notes are allowed: the project's scale (it follows the project's changes; without one, every note), the scale set in the device by root and scale, or the matrix, which says where each of the twelve notes goes.",
    'lattice.rule.hint':
      'Where a note outside the scale goes: to the nearest note of the scale (a tie goes down), always down, always up, or nowhere (it is not played). The matrix ignores it.',
    'lattice.transpose.hint': 'Semitones added to every note after it is put on the scale, from -24 to +24. A note pushed outside 0 to 127 is not played.',
    'lattice.root.hint': "Root of the device's scale, C to B (used when the source is the device).",
    'lattice.scale.hint': "The device's scale, built on its root (used when the source is the device): the project's list of scales.",
    'lattice.map_c.hint':
      'In matrix mode, the note a received C becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_c_sharp.hint':
      'In matrix mode, the note a received C sharp becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_d.hint':
      'In matrix mode, the note a received D becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_e_flat.hint':
      'In matrix mode, the note a received E flat becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_e.hint':
      'In matrix mode, the note a received E becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_f.hint':
      'In matrix mode, the note a received F becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_f_sharp.hint':
      'In matrix mode, the note a received F sharp becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_g.hint':
      'In matrix mode, the note a received G becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_a_flat.hint':
      'In matrix mode, the note a received A flat becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_a.hint':
      'In matrix mode, the note a received A becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_b_flat.hint':
      'In matrix mode, the note a received B flat becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'lattice.map_b.hint':
      'In matrix mode, the note a received B becomes: a pitch class, played at its nearest key (six semitones down to five up), or blocked (not played).',
    'scatter.chance.hint':
      'Chance that a received note comes out at another pitch, from 0 (never: the notes go through unchanged) to 100 % (every note): a repeatable draw, set by the seed.',
    'scatter.choices.hint': 'How many pitches a moved note can take, from 1 to 24, each one interval further from the received note.',
    'scatter.interval.hint':
      "Distance between two choices, from 1 to 12: semitones, or degrees of the project's scale in scale mode (12 semitones is an octave).",
    'scatter.direction.hint': 'Where the choices lie: above the received note, below it, or around it (alternately above and below).',
    'scatter.mode.hint':
      "How the interval is read: in semitones, or in degrees of the project's scale, so that every moved note is in the scale. Without a project scale, in semitones.",
    'scatter.order.hint':
      'How a moved note picks its choice: at random (a repeatable draw, set by the seed), or in turn, each moved note taking the next choice.',
    'scatter.seed.hint':
      'Seed of the draws, from 0 to 1000: a note draws from the seed, its position in the song and its pitch, so the same seed gives the same notes at the same place on every play, from any start; another seed, another draw.',
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
    'compressor.release.hint': 'In manual release, time the compressor takes to let the sound back up once it falls under the threshold, from 10 ms to 2 s.',
    'compressor.makeup.hint': 'Gain added after the compression to make up for the level it took away, from 0 to 24 dB.',
    'compressor.knee.hint':
      'Width, in decibels, of the zone around the threshold where the compression comes in gradually, from 0 (hard: the full ratio right at the threshold) to 24 dB (soft).',
    'compressor.release_mode.hint':
      'How the compressor lets the sound back up: manual (the release time) or auto (fast after a short hit, slower after a long compression, so it pumps less).',
    'compressor.detection.hint':
      'What level the compressor reacts to: peak (each sample, it catches the hits) or RMS (the average over 10 ms, it follows the loudness and lets short hits through).',
    'compressor.lookahead.hint':
      'How far ahead the compressor sees the sound, from 0 to 10 ms, so a short attack catches a hit whole. It delays the sound by that time; the other tracks are delayed to match, so everything stays in time. A change fades out and back in.',
    'compressor.sidechain_hpf.hint':
      'When on, the compressor does not react to the lows under the sidechain frequency (a 12 dB per octave high-pass on what it listens to, not on the sound): a kick or a bass no longer pumps everything else.',
    'compressor.sidechain_freq.hint': 'With the sidechain high-pass on, the frequency under which the compressor stops reacting, from 20 to 500 Hz.',
    'compressor.mix.hint':
      'Balance between the dry sound (0) and the compressed sound (1): in between is parallel compression, the hits keep their attack and the quiet parts come up.',
    'ceiling.ceiling.hint': 'Highest level the output may reach, in decibels of full scale, from -12 to 0 dB: no sample goes above it.',
    'ceiling.gain.hint': 'Gain added before the limiter, from 0 to 24 dB: it pushes the sound against the ceiling, so it gets louder without going over.',
    'ceiling.compensate.hint':
      'When on, the output is turned down by the input gain: the gain changes how hard the sound is limited, not how loud it is, to compare by ear.',
    'ceiling.lookahead.hint':
      'How far ahead the limiter sees the peaks, from 0 to 5 ms: it turns down smoothly before each one. It delays the sound by that time; the other tracks are delayed to match, so everything stays in time. A change fades out and back in.',
    'ceiling.release_mode.hint':
      'How the limiter lets the sound back up after a peak: manual (the release time) or auto (fast after a short peak, slower under sustained limiting).',
    'ceiling.release.hint':
      'In manual release, time the limiter takes to let the sound back up after a peak, from 1 ms to 1 s: short is louder, long is smoother.',
    'ceiling.true_peak.hint':
      'When on, the limiter also catches the peaks between samples (4x oversampled detection), for a converter or an encoder. It adds 12 samples of delay and a look-ahead of at least 16 samples. A change fades out and back in.',
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
    'ripple.mode.hint':
      'Phaser (a chain of all-pass stages: soft, vocal notches) or flanger (a short delay with feedback: a metallic comb). A change fades over 10 ms.',
    'ripple.stages.hint': 'Phaser: number of all-pass stages, 4, 6, 8 or 12; each pair of stages adds one notch. A change fades over 10 ms.',
    'ripple.centre.hint': 'Phaser: centre frequency of the stages, from 20 Hz to 20 kHz: the middle the LFO and the envelope move the notches around.',
    'ripple.spread.hint':
      'Phaser: how far apart the stages sit around the centre, from 0 (all at the centre, evenly spaced notches) to 4 octaves (wider, uneven notches).',
    'ripple.delay.hint':
      'Flanger: delay time, from 0.1 to 20 ms: the peaks of the comb sit every 1/delay hertz (1 kHz at 1 ms). The middle the LFO and the envelope move it around.',
    'ripple.feedback.hint':
      'Share of the effect fed back into its input, from -95 % to +95 %: sharper, ringing notches; negative feedback moves the peaks. Bounded: it never runs away.',
    'ripple.polarity.hint':
      'Sign of the effect in the mix: positive, or negative (the notches move between the former ones; the flanger then cuts the lowest frequencies). A change fades over 10 ms.',
    'ripple.mix.hint': 'How much of the effect joins the dry sound, from 0 (dry only) to 100 % (dry and effect in equal parts: the deepest notches).',
    'ripple.lfo_shape.hint':
      'Shape of the LFO that sweeps the notches: sine, triangle, rising saw, falling saw, square, or random (a new value at each cycle).',
    'ripple.lfo_sync.hint': 'Whether the LFO runs at its own rate in hertz (free) or follows the tempo (synced), each cycle on the same beat of every bar.',
    'ripple.lfo_rate.hint': 'Speed of the LFO in free mode, in cycles per second, from 0.01 to 20 Hz.',
    'ripple.lfo_division.hint': 'Length of one LFO cycle in synced mode, a note value from 1/32 to 8 bars.',
    'ripple.lfo_phase.hint': 'Where in its cycle the LFO starts, from 0 to 360 degrees: in synced mode, where in the bar the notches are highest.',
    'ripple.lfo_stereo.hint': "Offset of the right channel's LFO, from 0 to 180 degrees: at 180 the notches of the two channels move in opposition.",
    'ripple.lfo_depth.hint': 'How far the LFO sweeps the centre (phaser) or the delay (flanger) either side of its value, in octaves, from 0 (no sweep) to 5.',
    'ripple.lfo_restart.hint': 'In free mode, whether the LFO starts again from its phase each time playback starts (on) or runs on freely (off).',
    'ripple.env_amount.hint':
      'How far the level of the incoming sound moves the notches, in octaves at full level, from -5 (louder lowers them) to +5 (louder raises them); 0 for none.',
    'ripple.env_attack.hint': 'Time the envelope follower takes to rise with the sound, from 1 ms to 500 ms.',
    'ripple.env_release.hint': 'Time the envelope follower takes to fall back when the sound gets quieter, from 10 ms to 2 s.',
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
      'Ce que le filtre laisse passer : le son sous la fréquence de coupure (passe-bas), au-dessus (passe-haut), autour (passe-bande) ou tout sauf elle (coupe-bande). Un changement se fait en fondu de 10 ms.',
    'filter.cutoff.hint':
      'Fréquence, en hertz, à partir de laquelle le filtre coupe, de 20 Hz à 20 kHz : le centre autour duquel le LFO et l’enveloppe font bouger la coupure.',
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
      'Graine des tirages de chance, de 0 à 1000 : une note tire selon la graine, sa position dans le morceau et sa hauteur, donc la même graine donne les mêmes accords au même endroit à chaque lecture, depuis n’importe quel point de départ ; une autre graine, un autre tirage.',
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
    'lattice.source.hint':
      'Les notes permises : la gamme du projet (elle suit ses changements ; sans gamme, toutes les notes), la gamme réglée dans le device par sa tonique et sa gamme, ou la matrice, qui dit où va chacune des douze notes.',
    'lattice.rule.hint':
      'Où va une note hors de la gamme : sur la note la plus proche de la gamme (à égalité, vers le bas), toujours vers le bas, toujours vers le haut, ou nulle part (elle n’est pas jouée). La matrice l’ignore.',
    'lattice.transpose.hint': 'Demi-tons ajoutés à chaque note une fois ramenée sur la gamme, de -24 à +24. Une note poussée hors de 0 à 127 n’est pas jouée.',
    'lattice.root.hint': 'Tonique de la gamme du device, de do à si (quand la source est le device).',
    'lattice.scale.hint': 'La gamme du device, bâtie sur sa tonique (quand la source est le device) : la liste des gammes du projet.',
    'lattice.map_c.hint':
      'En mode matrice, la note que devient un do reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_c_sharp.hint':
      'En mode matrice, la note que devient un do♯ reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_d.hint':
      'En mode matrice, la note que devient un ré reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_e_flat.hint':
      'En mode matrice, la note que devient un mi♭ reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_e.hint':
      'En mode matrice, la note que devient un mi reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_f.hint':
      'En mode matrice, la note que devient un fa reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_f_sharp.hint':
      'En mode matrice, la note que devient un fa♯ reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_g.hint':
      'En mode matrice, la note que devient un sol reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_a_flat.hint':
      'En mode matrice, la note que devient un la♭ reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_a.hint':
      'En mode matrice, la note que devient un la reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_b_flat.hint':
      'En mode matrice, la note que devient un si♭ reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'lattice.map_b.hint':
      'En mode matrice, la note que devient un si reçu : une note, jouée à sa touche la plus proche (de six demi-tons plus bas à cinq plus haut), ou bloquée (pas jouée).',
    'scatter.chance.hint':
      'Chance qu’une note reçue sorte à une autre hauteur, de 0 (jamais : les notes passent telles quelles) à 100 % (chaque note) : un tirage reproductible, fixé par la graine.',
    'scatter.choices.hint': 'Nombre de hauteurs possibles pour une note changée, de 1 à 24, chacune un intervalle plus loin de la note reçue.',
    'scatter.interval.hint':
      'Écart entre deux choix, de 1 à 12 : en demi-tons, ou en degrés de la gamme du projet dans le mode gamme (12 demi-tons font une octave).',
    'scatter.direction.hint': 'Où sont les choix : au-dessus de la note reçue, au-dessous, ou autour (tour à tour au-dessus et au-dessous).',
    'scatter.mode.hint':
      'Comment lire l’intervalle : en demi-tons, ou en degrés de la gamme du projet, pour que chaque note changée soit dans la gamme. Sans gamme réglée, en demi-tons.',
    'scatter.order.hint':
      'Comment une note changée prend son choix : au hasard (un tirage reproductible, fixé par la graine), ou tour à tour, chaque note changée prenant le choix suivant.',
    'scatter.seed.hint':
      'Graine des tirages, de 0 à 1000 : une note tire selon la graine, sa position dans le morceau et sa hauteur ; la même graine donne donc les mêmes notes à la même place à chaque lecture, quel que soit le point de départ ; une autre graine, un autre tirage.',
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
    'compressor.release.hint': 'En relâchement manuel, temps que le compresseur met à relever le son une fois repassé sous le seuil, de 10 ms à 2 s.',
    'compressor.makeup.hint': 'Gain ajouté après la compression pour rattraper le niveau qu’elle a retiré, de 0 à 24 dB.',
    'compressor.knee.hint':
      'Largeur, en décibels, de la zone autour du seuil où la compression arrive progressivement, de 0 (dur : tout le ratio dès le seuil) à 24 dB (doux).',
    'compressor.release_mode.hint':
      'Comment le compresseur relève le son : manuel (le temps de relâchement) ou auto (vite après un coup bref, plus lentement après une longue compression, pour moins pomper).',
    'compressor.detection.hint':
      'À quel niveau le compresseur réagit : crête (chaque échantillon, il attrape les coups) ou RMS (la moyenne sur 10 ms, il suit le volume et laisse passer les coups brefs).',
    'compressor.lookahead.hint':
      'Avance avec laquelle le compresseur voit venir le son, de 0 à 10 ms, pour qu’une attaque courte attrape un coup en entier. Le son est retardé d’autant ; les autres pistes le sont aussi, pour que tout reste en place. Un changement se fait par un fondu de sortie puis d’entrée.',
    'compressor.sidechain_hpf.hint':
      'Activé, le compresseur ne réagit plus aux graves sous la fréquence du sidechain (un passe-haut à 12 dB par octave sur ce qu’il écoute, pas sur le son) : un kick ou une basse ne fait plus pomper tout le reste.',
    'compressor.sidechain_freq.hint': 'Avec le passe-haut du sidechain activé, fréquence sous laquelle le compresseur cesse de réagir, de 20 à 500 Hz.',
    'compressor.mix.hint':
      'Équilibre entre le son direct (0) et le son compressé (1) : entre les deux, c’est la compression parallèle, les coups gardent leur attaque et les passages calmes remontent.',
    'ceiling.ceiling.hint': 'Niveau le plus haut que la sortie peut atteindre, en décibels de pleine échelle, de -12 à 0 dB : aucun échantillon ne le dépasse.',
    'ceiling.gain.hint': 'Gain ajouté avant le limiteur, de 0 à 24 dB : il pousse le son contre le plafond, qui devient plus fort sans le dépasser.',
    'ceiling.compensate.hint':
      'Activée, la sortie est baissée du gain d’entrée : le gain change la force de la limitation, pas le volume, pour comparer à l’oreille.',
    'ceiling.lookahead.hint':
      'Avance avec laquelle le limiteur voit venir les crêtes, de 0 à 5 ms : il baisse le son en douceur avant chacune. Le son est retardé d’autant ; les autres pistes le sont aussi, pour que tout reste en place. Un changement se fait par un fondu de sortie puis d’entrée.',
    'ceiling.release_mode.hint':
      'Comment le limiteur relève le son après une crête : manuel (le temps de relâchement) ou auto (vite après une crête brève, plus lentement quand la limitation dure).',
    'ceiling.release.hint':
      'En relâchement manuel, temps que le limiteur met à relever le son après une crête, de 1 ms à 1 s : court sonne plus fort, long plus doux.',
    'ceiling.true_peak.hint':
      'Activée, le limiteur rattrape aussi les crêtes entre les échantillons (détection suréchantillonnée 4 fois), pour un convertisseur ou un encodeur. Ajoute 12 échantillons de retard et au moins 16 échantillons d’anticipation. Un changement se fait par un fondu de sortie puis d’entrée.',
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
    'ripple.mode.hint':
      'Phaser (une chaîne d’étages passe-tout : des creux doux, vocaux) ou flanger (un retard court avec retour : un peigne métallique). Un changement se fond sur 10 ms.',
    'ripple.stages.hint': 'Phaser : nombre d’étages passe-tout, 4, 6, 8 ou 12 ; chaque paire d’étages ajoute un creux. Un changement se fond sur 10 ms.',
    'ripple.centre.hint': 'Phaser : fréquence centrale des étages, de 20 Hz à 20 kHz : le milieu autour duquel le LFO et l’enveloppe déplacent les creux.',
    'ripple.spread.hint':
      'Phaser : écart des étages autour du centre, de 0 (tous au centre, des creux réguliers) à 4 octaves (des creux plus larges et irréguliers).',
    'ripple.delay.hint':
      'Flanger : temps de retard, de 0,1 à 20 ms : les pics du peigne tombent tous les 1/retard hertz (1 kHz à 1 ms). Le milieu autour duquel le LFO et l’enveloppe le déplacent.',
    'ripple.feedback.hint':
      'Part de l’effet renvoyée à son entrée, de −95 % à +95 % : des creux plus marqués, qui résonnent ; un retour négatif déplace les pics. Borné : il ne s’emballe jamais.',
    'ripple.polarity.hint':
      'Signe de l’effet dans le mélange : positif, ou négatif (les creux passent entre les anciens ; le flanger coupe alors les fréquences les plus graves). Un changement se fond sur 10 ms.',
    'ripple.mix.hint':
      'Quantité d’effet ajoutée au son direct, de 0 (son direct seul) à 100 % (son direct et effet à parts égales : les creux les plus profonds).',
    'ripple.lfo_shape.hint':
      'Forme du LFO qui balaie les creux : sinus, triangle, scie montante, scie descendante, carré, ou aléatoire (une nouvelle valeur à chaque cycle).',
    'ripple.lfo_sync.hint': 'Le LFO va à sa propre vitesse en hertz (libre) ou suit le tempo (au tempo), chaque cycle sur le même temps de chaque mesure.',
    'ripple.lfo_rate.hint': 'Vitesse du LFO en mode libre, en cycles par seconde, de 0,01 à 20 Hz.',
    'ripple.lfo_division.hint': 'Durée d’un cycle du LFO au tempo, une valeur de note de 1/32 à 8 mesures.',
    'ripple.lfo_phase.hint': 'Où le LFO commence dans son cycle, de 0 à 360 degrés : au tempo, l’endroit de la mesure où les creux sont le plus haut.',
    'ripple.lfo_stereo.hint': 'Décalage du LFO du canal droit, de 0 à 180 degrés : à 180, les creux des deux canaux bougent en opposition.',
    'ripple.lfo_depth.hint':
      'De combien le LFO balaie la fréquence centrale (phaser) ou le retard (flanger) de part et d’autre de sa valeur, en octaves, de 0 (aucun balayage) à 5.',
    'ripple.lfo_restart.hint': 'En mode libre, le LFO repart de sa phase à chaque démarrage de la lecture (oui) ou continue librement (non).',
    'ripple.env_amount.hint':
      'De combien le niveau du son entrant déplace les creux, en octaves à plein niveau, de −5 (plus fort les abaisse) à +5 (plus fort les monte) ; 0 pour aucun effet.',
    'ripple.env_attack.hint': 'Temps que le suiveur d’enveloppe met à monter avec le son, de 1 ms à 500 ms.',
    'ripple.env_release.hint': 'Temps que le suiveur d’enveloppe met à redescendre quand le son baisse, de 10 ms à 2 s.',
  },
)
