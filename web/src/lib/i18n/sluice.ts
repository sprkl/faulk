/**
 * Labels of the Sluice device's parameters and of their values, keyed `sluice.<key>` (the keys of
 * its descriptor, `DEVICES` in the generated protocol). The device's name is not here: it comes
 * from its descriptor, one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const sluice = defineFeature(
  {
    'sluice.threshold': 'Open threshold',
    'sluice.close': 'Close threshold',
    'sluice.depth': 'Depth',
    'sluice.attack': 'Attack',
    'sluice.hold': 'Hold',
    'sluice.release': 'Release',
    'sluice.invert': 'Invert',
    'sluice.off': 'Off',
    'sluice.on': 'On',
    'sluice.hpf': 'Listen above',
    'sluice.lpf': 'Listen below',
    'sluice.lookahead': 'Lookahead',
  },
  {
    'sluice.threshold': 'Seuil d’ouverture',
    'sluice.close': 'Seuil de fermeture',
    'sluice.depth': 'Profondeur',
    'sluice.attack': 'Attaque',
    'sluice.hold': 'Maintien',
    'sluice.release': 'Relâchement',
    'sluice.invert': 'Inversé',
    'sluice.off': 'Non',
    'sluice.on': 'Oui',
    'sluice.hpf': 'Écoute au-dessus de',
    'sluice.lpf': 'Écoute en dessous de',
    'sluice.lookahead': 'Anticipation',
  },
)
