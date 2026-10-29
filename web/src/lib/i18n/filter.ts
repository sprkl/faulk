/**
 * Labels of the Filter device's parameters and of their values, keyed `filter.<key>` (the keys of
 * its descriptor, `DEVICES` in the generated protocol). The device's name is not here: it comes
 * from its descriptor, one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const filter = defineFeature(
  {
    'filter.mode': 'Mode',
    'filter.lowpass': 'Low-pass',
    'filter.highpass': 'High-pass',
    'filter.bandpass': 'Band-pass',
    'filter.cutoff': 'Cutoff',
    'filter.resonance': 'Resonance',
    'filter.mix': 'Mix',
  },
  {
    'filter.mode': 'Mode',
    'filter.lowpass': 'Passe-bas',
    'filter.highpass': 'Passe-haut',
    'filter.bandpass': 'Passe-bande',
    'filter.cutoff': 'Fréquence de coupure',
    'filter.resonance': 'Résonance',
    'filter.mix': 'Mélange',
  },
)
