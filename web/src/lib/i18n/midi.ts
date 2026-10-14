/** MIDI input. */

import { defineFeature } from './feature'

export const midi = defineFeature(
  {
    'midi.title': 'MIDI input',
    'midi.none': 'None',
    'midi.empty': 'No MIDI device detected. Plug one in, or play with the computer keyboard.',
    'midi.unsupported': 'This browser has no Web MIDI. Use Chrome, Edge or Firefox, or Boost mode.',
    'midi.denied': 'MIDI access was refused.',
    'midi.connectError': 'Could not open this MIDI input.',
  },
  {
    'midi.title': 'Entrée MIDI',
    'midi.none': 'Aucune',
    'midi.empty': 'Aucun appareil MIDI détecté. Branche-en un, ou joue au clavier de l’ordinateur.',
    'midi.unsupported': 'Ce navigateur n’a pas Web MIDI. Utilise Chrome, Edge ou Firefox, ou le mode Boost.',
    'midi.denied': 'L’accès MIDI a été refusé.',
    'midi.connectError': 'Impossible d’ouvrir cette entrée MIDI.',
  },
)
