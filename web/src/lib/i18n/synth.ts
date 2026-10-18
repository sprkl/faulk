/**
 * Labels of the oscillator device's parameters and of their values, keyed
 * `<device kind without "faulk.">.<key>` (the keys of its descriptor, `DEVICES` in the generated
 * protocol). The device's name is not here: it comes from its descriptor, one English name never
 * translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const synth = defineFeature(
  {
    'osc_synth.waveform': 'Waveform',
    'osc_synth.sine': 'Sine',
    'osc_synth.saw': 'Saw',
    'osc_synth.square': 'Square',
    'osc_synth.triangle': 'Triangle',
    'osc_synth.attack': 'Attack',
    'osc_synth.decay': 'Decay',
    'osc_synth.sustain': 'Sustain',
    'osc_synth.release': 'Release',
    'osc_synth.gain': 'Gain',
  },
  {
    'osc_synth.waveform': 'Forme d’onde',
    'osc_synth.sine': 'Sinus',
    'osc_synth.saw': 'Dent de scie',
    'osc_synth.square': 'Carré',
    'osc_synth.triangle': 'Triangle',
    'osc_synth.attack': 'Attaque',
    'osc_synth.decay': 'Déclin',
    'osc_synth.sustain': 'Maintien',
    'osc_synth.release': 'Relâchement',
    'osc_synth.gain': 'Gain',
  },
)

/**
 * Keys of the DAW's catalogue that hold a device name (none since DV-1: the panel takes the name
 * from the device's descriptor). A device name is one English name, never translated: a key listed
 * here must hold the same text in every language (`i18n.test.ts`).
 */
export const DEVICE_NAME_KEYS: readonly (keyof typeof synth.en)[] = []
