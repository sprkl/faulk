/** The oscillator device. */

import { defineFeature } from './feature'

export const synth = defineFeature(
  {
    'synth.title': 'Oscillator',
    'synth.waveform': 'Waveform',
    'synth.sine': 'Sine',
    'synth.saw': 'Saw',
    'synth.square': 'Square',
    'synth.triangle': 'Triangle',
    'synth.attack': 'Attack',
    'synth.decay': 'Decay',
    'synth.sustain': 'Sustain',
    'synth.release': 'Release',
    'synth.gain': 'Gain',
  },
  {
    'synth.title': 'Oscillator',
    'synth.waveform': 'Forme d’onde',
    'synth.sine': 'Sinus',
    'synth.saw': 'Dent de scie',
    'synth.square': 'Carré',
    'synth.triangle': 'Triangle',
    'synth.attack': 'Attaque',
    'synth.decay': 'Déclin',
    'synth.sustain': 'Maintien',
    'synth.release': 'Relâchement',
    'synth.gain': 'Gain',
  },
)

/**
 * Names of devices (the title of a device's panel). A device name is one English name, never
 * translated (founder's decision of 2026-10-14): the same text in every language. Parameter labels
 * and descriptions are translated.
 */
export const DEVICE_NAME_KEYS: readonly (keyof typeof synth.en)[] = ['synth.title']
