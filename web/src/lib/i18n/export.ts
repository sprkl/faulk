/** Exporting the song as a WAV file (issue #297): the menu entry, the dialog, the progress and what it says at the end. */

import { defineFeature } from './feature'

export const audioExport = defineFeature(
  {
    'project.menu.export': 'Export audio…',

    'export.title': 'Export audio',
    'export.fileName': 'File name',
    'export.range': 'What to export',
    'export.range.song': 'The song, from the start to the end of the last clip',
    'export.range.loop': 'The loop region',
    'export.range.noLoop': 'The loop region (set one in the transport first)',
    'export.range.bars': 'Some bars',
    'export.range.from': 'From bar',
    'export.range.to': 'to bar',
    'export.range.badBars': 'The last bar cannot come before the first one.',
    'export.format': 'Format',
    'export.format.pcm24': 'WAV, 24 bits',
    'export.format.pcm16': 'WAV, 16 bits',
    'export.format.float32': 'WAV, 32-bit float',
    'export.rate': 'Sample rate',
    'export.note':
      'The file is rendered by the same engine that plays your song, faster than real time, so it is exactly what you hear. Effects such as the echo ring out after the last note.',
    'export.noChoice': 'This browser cannot ask where to save: the file goes to your downloads.',
    'export.empty': 'There is nothing to export yet. Put a clip in the arrangement first.',
    'export.start': 'Export',
    'export.cancel': 'Cancel',
    'export.close': 'Close',

    'export.picking': 'Choose where to save the file…',
    'export.rendering': 'Rendering… {percent} %',
    'export.tail': 'The song is rendered. Letting the effects ring out…',
    'export.done': 'Saved “{name}” ({size} MB, {duration}).',
    'export.downloaded': 'Downloaded “{name}” ({size} MB, {duration}).',
    'export.peak': 'Highest level: {db} dBFS.',
    'export.silent': 'The file is silent.',
    'export.limited':
      'Your mix was pushed into the output limiter (up to {db} dB of gain reduction). The file does not clip, but it was audibly limited. Lower the master volume and export again for a cleaner sound.',
    'export.tailCut': 'An effect was still ringing after 30 seconds, so the file stops there.',

    'export.failed': 'The export failed. Nothing was saved.',
    'export.refused':
      'Faulk cannot export the project as asked. Check that the arrangement has a clip, that the loop region exists and that the bars are inside the song.',
    'export.tooLong': 'This range lasts more than 30 minutes, which is the longest export. Choose fewer bars.',
    'export.tooBig':
      'This browser keeps the whole file in memory until the end, and this one is too big for that (more than 400 MB). Use Chrome or Edge, or choose fewer bars, 16 bits or 44.1 kHz.',
    'export.noAccess': 'The browser did not allow that file. Choose another place.',
    'export.writeFailed': 'The file could not be written. Nothing was saved.',
  },
  {
    'project.menu.export': 'Exporter l’audio…',

    'export.title': 'Exporter l’audio',
    'export.fileName': 'Nom du fichier',
    'export.range': 'Ce qu’il faut exporter',
    'export.range.song': 'Le morceau, du début à la fin du dernier clip',
    'export.range.loop': 'La zone de boucle',
    'export.range.noLoop': 'La zone de boucle (définis-en une dans le transport d’abord)',
    'export.range.bars': 'Quelques mesures',
    'export.range.from': 'De la mesure',
    'export.range.to': 'à la mesure',
    'export.range.badBars': 'La dernière mesure ne peut pas venir avant la première.',
    'export.format': 'Format',
    'export.format.pcm24': 'WAV, 24 bits',
    'export.format.pcm16': 'WAV, 16 bits',
    'export.format.float32': 'WAV, 32 bits flottants',
    'export.rate': 'Fréquence d’échantillonnage',
    'export.note':
      'Le fichier est calculé par le même moteur que celui qui joue ton morceau, plus vite que le temps réel : c’est exactement ce que tu entends. Les effets comme l’écho se prolongent après la dernière note.',
    'export.noChoice': 'Ce navigateur ne peut pas demander où enregistrer : le fichier va dans tes téléchargements.',
    'export.empty': 'Il n’y a encore rien à exporter. Place d’abord un clip dans l’arrangement.',
    'export.start': 'Exporter',
    'export.cancel': 'Annuler',
    'export.close': 'Fermer',

    'export.picking': 'Choisis où enregistrer le fichier…',
    'export.rendering': 'Calcul en cours… {percent} %',
    'export.tail': 'Le morceau est calculé. Les effets se terminent…',
    'export.done': '« {name} » est enregistré ({size} Mo, {duration}).',
    'export.downloaded': '« {name} » est téléchargé ({size} Mo, {duration}).',
    'export.peak': 'Niveau le plus haut : {db} dBFS.',
    'export.silent': 'Le fichier est muet.',
    'export.limited':
      'Ton mix a été poussé dans le limiteur de sortie (jusqu’à {db} dB de réduction de gain). Le fichier n’écrête pas, mais il a été limité de façon audible. Baisse le volume général et exporte à nouveau pour un son plus propre.',
    'export.tailCut': 'Un effet résonnait encore après 30 secondes, le fichier s’arrête donc là.',

    'export.failed': 'L’export a échoué. Rien n’a été enregistré.',
    'export.refused':
      'Faulk ne peut pas exporter le projet comme demandé. Vérifie que l’arrangement contient un clip, que la zone de boucle existe et que les mesures sont dans le morceau.',
    'export.tooLong': 'Cette plage dure plus de 30 minutes, ce qui est le plus long export possible. Choisis moins de mesures.',
    'export.tooBig':
      'Ce navigateur garde tout le fichier en mémoire jusqu’à la fin, et celui-ci est trop gros pour cela (plus de 400 Mo). Utilise Chrome ou Edge, ou choisis moins de mesures, le 16 bits ou 44,1 kHz.',
    'export.noAccess': 'Le navigateur n’a pas autorisé ce fichier. Choisis un autre endroit.',
    'export.writeFailed': 'Le fichier n’a pas pu être écrit. Rien n’a été enregistré.',
  },
)
