/**
 * Labels of the Compressor device's parameters, keyed `compressor.<key>` (the keys of its
 * descriptor, `DEVICES` in the generated protocol). The device's name is not here: it comes from its
 * descriptor, one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const compressor = defineFeature(
  {
    'compressor.threshold': 'Threshold',
    'compressor.ratio': 'Ratio',
    'compressor.attack': 'Attack',
    'compressor.release': 'Release',
    'compressor.makeup': 'Makeup',
  },
  {
    'compressor.threshold': 'Seuil',
    'compressor.ratio': 'Ratio',
    'compressor.attack': 'Attaque',
    'compressor.release': 'Relâchement',
    'compressor.makeup': 'Compensation',
  },
)
