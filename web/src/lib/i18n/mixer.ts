/**
 * The mixer view (`src/views/mixer/`). Mute, solo, arm, volume and pan reuse the `tracks.*` texts:
 * they name the same actions. A device name is not here: it comes from the descriptor, one English
 * name never translated.
 */

import { defineFeature } from './feature'

export const mixer = defineFeature(
  {
    'mixer.empty': 'No tracks yet. Add one in the track list to see its strip.',
    'mixer.strips': 'Mixer strips',
    'mixer.strip': 'Strip of {name}',
    'mixer.master': 'Master',
    'mixer.masterVolume': 'Master volume',
    'mixer.devices': 'Devices',
    'mixer.devicesHint': 'Show the devices of {name}',
    'mixer.noDevice': 'No device',
    'mixer.unknownDevice': 'Module',
    'mixer.level': 'Level of {name}',
    'mixer.peak': 'Highest peak of {name}. Click to reset.',
    'mixer.clip': 'CLIP',
  },
  {
    'mixer.empty': 'Aucune piste pour l’instant. Ajoute-en une dans la liste des pistes pour voir sa tranche.',
    'mixer.strips': 'Tranches du mixeur',
    'mixer.strip': 'Tranche de {name}',
    'mixer.master': 'Master',
    'mixer.masterVolume': 'Volume du master',
    'mixer.devices': 'Devices',
    'mixer.devicesHint': 'Afficher les devices de {name}',
    'mixer.noDevice': 'Aucun device',
    'mixer.unknownDevice': 'Module',
    'mixer.level': 'Niveau de {name}',
    'mixer.peak': 'Pic le plus haut de {name}. Clique pour le remettre à zéro.',
    'mixer.clip': 'CLIP',
  },
)
