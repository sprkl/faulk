/**
 * Labels of the Drums device's parameters, keyed `drums.<key>` (the keys of its descriptor,
 * `DEVICES` in the generated protocol). The device's name is not here: it comes from its descriptor,
 * one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const drums = defineFeature(
  {
    'drums.kick_tune': 'Kick · Tune',
    'drums.kick_decay': 'Kick · Decay',
    'drums.kick_punch': 'Kick · Punch',
    'drums.kick_level': 'Kick · Level',
    'drums.snare_tune': 'Snare · Tune',
    'drums.snare_decay': 'Snare · Decay',
    'drums.snare_snap': 'Snare · Snap',
    'drums.snare_level': 'Snare · Level',
    'drums.clap_tone': 'Clap · Tone',
    'drums.clap_decay': 'Clap · Decay',
    'drums.clap_level': 'Clap · Level',
    'drums.hat_tone': 'Hi-hat · Tone',
    'drums.hat_decay': 'Hi-hat · Closed decay',
    'drums.hat_open_decay': 'Hi-hat · Open decay',
    'drums.hat_level': 'Hi-hat · Level',
    'drums.gain': 'Gain',
  },
  {
    'drums.kick_tune': 'Grosse caisse · Hauteur',
    'drums.kick_decay': 'Grosse caisse · Déclin',
    'drums.kick_punch': 'Grosse caisse · Punch',
    'drums.kick_level': 'Grosse caisse · Niveau',
    'drums.snare_tune': 'Caisse claire · Hauteur',
    'drums.snare_decay': 'Caisse claire · Déclin',
    'drums.snare_snap': 'Caisse claire · Mordant',
    'drums.snare_level': 'Caisse claire · Niveau',
    'drums.clap_tone': 'Clap · Timbre',
    'drums.clap_decay': 'Clap · Déclin',
    'drums.clap_level': 'Clap · Niveau',
    'drums.hat_tone': 'Charleston · Timbre',
    'drums.hat_decay': 'Charleston · Déclin fermé',
    'drums.hat_open_decay': 'Charleston · Déclin ouvert',
    'drums.hat_level': 'Charleston · Niveau',
    'drums.gain': 'Gain',
  },
)
