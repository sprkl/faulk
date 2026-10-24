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
  },
  {
    'reverb.size': 'Taille',
    'reverb.damping': 'Amortissement',
    'reverb.predelay': 'Pré-délai',
    'reverb.mix': 'Mélange',
  },
)
