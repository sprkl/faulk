/** Computer keyboard. */

import { defineFeature } from './feature'

export const keys = defineFeature(
  {
    'keys.title': 'Computer keyboard',
    'keys.help': 'A W S E D F T G Y H U J K play notes (by key position). Z / X change octave.',
    'keys.octave': 'Octave {n}',
    'keys.target': 'Playing {name}.',
    'keys.noTarget': 'Select a MIDI track to play.',
  },
  {
    'keys.title': 'Clavier de l’ordinateur',
    'keys.help': 'Les touches de la rangée du milieu jouent les notes (par position : Q Z S E D F T G Y H U J K en AZERTY). W / X changent d’octave.',
    'keys.octave': 'Octave {n}',
    'keys.target': 'Tu joues sur {name}.',
    'keys.noTarget': 'Sélectionne une piste MIDI pour jouer.',
  },
)
