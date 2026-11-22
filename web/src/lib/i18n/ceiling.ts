/**
 * Labels of the Ceiling device's parameters and of their values, keyed `ceiling.<key>` (the keys of
 * its descriptor, `DEVICES` in the generated protocol). The device's name is not here: it comes
 * from its descriptor, one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const ceiling = defineFeature(
  {
    'ceiling.ceiling': 'Output ceiling',
    'ceiling.gain': 'Input gain',
    'ceiling.compensate': 'Compensate',
    'ceiling.off': 'Off',
    'ceiling.on': 'On',
    'ceiling.lookahead': 'Lookahead',
    'ceiling.release_mode': 'Release mode',
    'ceiling.manual': 'Manual',
    'ceiling.auto': 'Auto',
    'ceiling.release': 'Release',
    'ceiling.true_peak': 'True peak',
  },
  {
    'ceiling.ceiling': 'Plafond',
    'ceiling.gain': 'Gain d’entrée',
    'ceiling.compensate': 'Compensation',
    'ceiling.off': 'Non',
    'ceiling.on': 'Oui',
    'ceiling.lookahead': 'Anticipation',
    'ceiling.release_mode': 'Relâchement',
    'ceiling.manual': 'Manuel',
    'ceiling.auto': 'Auto',
    'ceiling.release': 'Temps de relâchement',
    'ceiling.true_peak': 'Crête vraie',
  },
)
