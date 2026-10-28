/**
 * Labels of the Reverb device's parameters, keyed `reverb.<key>` (the keys of its descriptor,
 * `DEVICES` in the generated protocol). The device's name is not here: it comes from its
 * descriptor, one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const reverb = defineFeature(
  {
    'reverb.size': 'Size',
    'reverb.damping': 'Damping',
    'reverb.predelay': 'Pre-delay',
    'reverb.mix': 'Mix',
    'reverb.time_change': 'Time change',
    'reverb.glide': 'Glide',
    'reverb.fade': 'Fade',
    'reverb.jump': 'Jump',
  },
  {
    'reverb.size': 'Taille',
    'reverb.damping': 'Amortissement',
    'reverb.predelay': 'Pré-délai',
    'reverb.mix': 'Mélange',
    'reverb.time_change': 'Au changement',
    'reverb.glide': 'Glissé',
    'reverb.fade': 'Fondu',
    'reverb.jump': 'Saut',
  },
)
