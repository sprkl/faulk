/**
 * Labels of the Trim device's parameters and of their values, keyed `trim.<key>` (the keys of its
 * descriptor, `DEVICES` in the generated protocol). The device's name is not here: it comes from
 * its descriptor, one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const trim = defineFeature(
  {
    'trim.mode': 'Mode',
    'trim.free': 'Free',
    'trim.synced': 'Synced',
    'trim.time': 'Time',
    'trim.division': 'Note value',
    'trim.sixty_fourth': '1/64',
    'trim.thirty_second': '1/32',
    'trim.sixteenth': '1/16',
    'trim.eighth': '1/8',
    'trim.quarter': '1/4',
    'trim.half': '1/2',
    'trim.bar': '1 bar',
    'trim.two_bars': '2 bars',
    'trim.four_bars': '4 bars',
    'trim.feel': 'Feel',
    'trim.straight': 'Straight',
    'trim.triplet': 'Triplet',
    'trim.dotted': 'Dotted',
    'trim.gate': 'Gate',
    'trim.trigger': 'Trigger',
    'trim.start': 'On press',
    'trim.release': 'On release',
    'trim.key_track': 'Key tracking',
  },
  {
    'trim.mode': 'Mode',
    'trim.free': 'Libre',
    'trim.synced': 'Au tempo',
    'trim.time': 'Durée',
    'trim.division': 'Valeur de note',
    'trim.sixty_fourth': '1/64',
    'trim.thirty_second': '1/32',
    'trim.sixteenth': '1/16',
    'trim.eighth': '1/8',
    'trim.quarter': '1/4',
    'trim.half': '1/2',
    'trim.bar': '1 mesure',
    'trim.two_bars': '2 mesures',
    'trim.four_bars': '4 mesures',
    'trim.feel': 'Rythme',
    'trim.straight': 'Binaire',
    'trim.triplet': 'Triolet',
    'trim.dotted': 'Pointé',
    'trim.gate': 'Gate',
    'trim.trigger': 'Déclenchement',
    'trim.start': 'À l’appui',
    'trim.release': 'Au relâchement',
    'trim.key_track': 'Suivi de hauteur',
  },
)
