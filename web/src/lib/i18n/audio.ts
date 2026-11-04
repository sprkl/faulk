/** Audio clips in the grid and the timeline (AU-6, #115): opening one, its gain, moving it between tracks, files dropped on a track. */

import { defineFeature } from './feature'

export const audio = defineFeature(
  {
    'audio.openInClips': 'Show in the Clips panel',
    'audio.clipAudio': 'audio clip',
    'audio.saidNotAudioTrack': '{track} is not an audio track: an audio clip only goes to an audio track',
    'audio.gain': 'Gain',
    'audio.gainHint': 'Clip gain in dB, from −96 to +6. Leave it empty for silence.',
    'audio.gainOf': 'Gain of {name}, in dB',
    'audio.gainUnit': 'dB',
    'audio.gainSilent': '−∞',
    'audio.importingHere': 'Importing into {track}: each file is placed where the last one ends.',
    'audio.noAudioFile': 'No audio file in this drop: nothing was imported.',
    'audio.importingCell': 'Importing into {track}: each file goes into the next free slot down.',
  },
  {
    'audio.openInClips': 'Afficher dans le panneau Clips',
    'audio.clipAudio': 'clip audio',
    'audio.saidNotAudioTrack': '{track} n’est pas une piste audio : un clip audio ne va que sur une piste audio',
    'audio.gain': 'Gain',
    'audio.gainHint': 'Gain du clip en dB, de −96 à +6. Laissez vide pour le silence.',
    'audio.gainOf': 'Gain de {name}, en dB',
    'audio.gainUnit': 'dB',
    'audio.gainSilent': '−∞',
    'audio.importingHere': 'Import dans {track} : chaque fichier se place là où finit le précédent.',
    'audio.noAudioFile': 'Aucun fichier audio dans ce dépôt : rien n’a été importé.',
    'audio.importingCell': 'Import dans {track} : chaque fichier va dans la case libre suivante, vers le bas.',
  },
)
