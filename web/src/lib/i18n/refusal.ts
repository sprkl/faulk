/**
 * What the page tells the user when the core refuses a command, keyed `refusal.<code>` after the
 * refusal code of `CommandError` (`crates/commands`), and the names of what a limit counts, keyed
 * `refusal.entity.<name>`.
 */

import { defineFeature } from './feature'

export const refusal = defineFeature(
  {
    'refusal.not_found': 'Not done: what this targets no longer exists.',
    'refusal.invalid': 'Not done: a value was refused.',
    'refusal.limit': 'Not done: the limit of {max} {what} is reached.',
    'refusal.unsupported': 'Not done: this is not available yet.',
    'refusal.folder': 'Not done: this project folder was refused.',
    'refusal.count': '{text} (×{n})',
    'refusal.entity.other': 'items',
    'refusal.entity.tracks': 'tracks',
    'refusal.entity.scenes': 'scenes',
    'refusal.entity.clips': 'clips',
    'refusal.entity.notes': 'notes',
    'refusal.entity.notesPerClip': 'notes per clip',
    'refusal.entity.devices': 'devices',
    'refusal.entity.automationPoints': 'automation points',
    'refusal.entity.placements': 'placements',
    'refusal.entity.placementsPerTrack': 'placements per track',
    'refusal.entity.clipsPerTrack': 'clips per track',
    'refusal.entity.lanes': 'automation lanes',
    'refusal.entity.exportSeconds': 'seconds of export',
  },
  {
    'refusal.not_found': 'Pas fait : la cible n’existe plus.',
    'refusal.invalid': 'Pas fait : une valeur a été refusée.',
    'refusal.limit': 'Pas fait : la limite de {max} {what} est atteinte.',
    'refusal.unsupported': 'Pas fait : ce n’est pas encore disponible.',
    'refusal.folder': 'Pas fait : ce dossier de projet a été refusé.',
    'refusal.count': '{text} (×{n})',
    'refusal.entity.other': 'éléments',
    'refusal.entity.tracks': 'pistes',
    'refusal.entity.scenes': 'scènes',
    'refusal.entity.clips': 'clips',
    'refusal.entity.notes': 'notes',
    'refusal.entity.notesPerClip': 'notes par clip',
    'refusal.entity.devices': 'effets et instruments',
    'refusal.entity.automationPoints': 'points d’automation',
    'refusal.entity.placements': 'placements',
    'refusal.entity.placementsPerTrack': 'placements par piste',
    'refusal.entity.clipsPerTrack': 'clips par piste',
    'refusal.entity.lanes': 'courbes d’automation',
    'refusal.entity.exportSeconds': 'secondes d’export',
  },
)
