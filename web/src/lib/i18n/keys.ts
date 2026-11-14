/** Computer keyboard. */

import { defineFeature } from './feature'

export const keys = defineFeature(
  {
    'keys.title': 'Computer keyboard',
    'keys.help': 'A W S E D F T G Y H U J K play notes (by key position). Z / X change octave.',
    'keys.octave': 'Octave {n}',
    'keys.octaveShort': 'Oct {n}',
    'keys.target': 'Playing {name}.',
    'keys.noTarget': 'Select a MIDI track to play.',
    'keys.targetArmed': 'Playing {name} and the armed tracks.',
    'keys.armedOnly': 'Playing the armed tracks.',
    'keys.stepStopped': 'Step entry off.',
    'keys.typingNotes': 'Typing: notes',
    'keys.typingShortcuts': 'Typing: shortcuts',
    'keys.typingHintNotes': 'The letters play notes. Press the key left of 1 (` or ²) to use them as shortcuts.',
    'keys.typingHintShortcuts': 'The letters play nothing. Press the key left of 1 (` or ²) to play notes with them.',
  },
  {
    'keys.octaveShort': 'Oct {n}',
    'keys.title': 'Clavier de l’ordinateur',
    'keys.help': 'Les touches de la rangée du milieu jouent les notes (par position : Q Z S E D F T G Y H U J K en AZERTY). W / X changent d’octave.',
    'keys.octave': 'Octave {n}',
    'keys.target': 'Tu joues sur {name}.',
    'keys.noTarget': 'Sélectionne une piste MIDI pour jouer.',
    'keys.targetArmed': 'Tu joues sur {name} et sur les pistes armées.',
    'keys.armedOnly': 'Tu joues sur les pistes armées.',
    'keys.stepStopped': 'Saisie pas à pas désactivée.',
    'keys.typingNotes': 'Saisie : notes',
    'keys.typingShortcuts': 'Saisie : raccourcis',
    'keys.typingHintNotes': 'Les lettres jouent des notes. Appuie sur la touche à gauche du 1 (` ou ²) pour les utiliser comme raccourcis.',
    'keys.typingHintShortcuts': 'Les lettres ne jouent rien. Appuie sur la touche à gauche du 1 (` ou ²) pour jouer des notes avec.',
  },
)
