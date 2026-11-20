/**
 * Labels of the Swing device's parameters and of their values, keyed `swing.<key>` (the keys of its
 * descriptor, `DEVICES` in the generated protocol; #168). The device's name is not here: it comes
 * from its descriptor, one English name never translated.
 */

import { defineFeature } from './feature'

export const swing = defineFeature(
  {
    'swing.amount': 'Amount',
    'swing.grid': 'Grid',
    'swing.eighth': '1/8',
    'swing.sixteenth': '1/16',
    'swing.thirty_second': '1/32',
    'swing.window': 'Window',
    'swing.humanize_time': 'Humanize time',
    'swing.humanize_velocity': 'Humanize velocity',
    'swing.seed': 'Seed',
  },
  {
    'swing.amount': 'Quantité',
    'swing.grid': 'Grille',
    'swing.eighth': '1/8',
    'swing.sixteenth': '1/16',
    'swing.thirty_second': '1/32',
    'swing.window': 'Fenêtre',
    'swing.humanize_time': 'Humanisation du temps',
    'swing.humanize_velocity': 'Humanisation de la vélocité',
    'swing.seed': 'Graine',
  },
)
