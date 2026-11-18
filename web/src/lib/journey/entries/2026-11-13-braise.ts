import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-13',
  slug: 'braise',
  title: 'Braise',
  commit: '355027b',
  genre: { en: 'Dark, warm electronic instrumental, one arpeggio that grows', fr: 'Instrumental électronique sombre et chaud, un arpège qui grossit' },
  bpm: 112,
  key: { tonic: 'A', mode: 'dorian' },
  bars: 96,
  duration: 208.7,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'morzak',
  brief: {
    en: 'Style profile “morzak” and a single-crescendo form: one arpeggio figure that grows without breaking, first in quarter notes, then eighths, then sixteenths, with the bass walking in and a four-chord descent only for the peak. 112 bpm in A dorian, a two-chord cycle held for 64 bars. Three constraints: a richer, faster chord grid for the peak only, the D chord turning minor once (bar 88), and no kick on every beat before bar 65.',
    fr: 'Profil de style « morzak » et une forme à crescendo unique : une figure d’arpège qui grossit sans rupture, à la noire, puis à la croche, puis à la double croche, la basse qui se met à marcher et une descente de quatre accords pour le sommet seulement. 112 bpm en La dorien, un cycle de deux accords tenu 64 mesures. Trois contraintes : une grille plus riche et plus rapide pour le sommet seulement, l’accord de Ré qui devient mineur une seule fois (mesure 88), aucun kick sur chaque temps avant la mesure 65.',
  },
  built: {
    en: 'The tenth day built through the interface only. About 36 minutes, against 34 the day before. 5 tracks, 24 clips (382 notes), 197 placements, 15 devices and 4 automation lanes with 12 points. Each clip was created where it plays, from the keyboard. Before the export, the 19 melodic clips of the saved project were compared with the sheet: 19 out of 19 right. The WAV export is 3:29, with a peak at −0.3 dBFS. The project was saved, reopened with everything back; the .faulkpack was built afterwards from the saved project folder (78 files, none missing), not exported by the app. Measured on the WAV, not listened to: the peak is not louder than the second accretion, so the level climb stops at bar 49.',
    fr: 'Dixième jour construit par l’interface seule. Environ 36 minutes, contre 34 la veille. 5 pistes, 24 clips (382 notes), 197 placements, 15 devices et 4 couloirs d’automation de 12 points. Chaque clip a été créé là où il joue, au clavier. Avant l’export, les 19 clips mélodiques du projet enregistré ont été comparés à la fiche : 19 sur 19 justes. L’export WAV dure 3:29, avec une crête à −0,3 dBFS. Le projet enregistré, rouvert avec tout revenu ; le .faulkpack a été fabriqué ensuite à partir du dossier enregistré (78 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le WAV, sans écoute : le sommet n’est pas plus fort que la seconde accrétion, la montée de niveau s’arrête à la mesure 49.',
  },
  tested: [
    {
      en: 'Clips never cut in silence (#859, #862, #866): six gestures tried on an occupied spot, six refusals with a message, 197 clips before and after',
      fr: 'Les clips jamais coupés en silence (#859, #862, #866) : six gestes essayés sur une place occupée, six refus avec un message, 197 clips avant et après',
    },
    {
      en: 'Alt+N and “New clip at the playhead”, Ctrl+G and Home on the free cursor (#864): 14 clips created this way',
      fr: 'Alt+N et « Nouveau clip à la tête de lecture », Ctrl+G et Début sur le curseur libre (#864) : 14 clips créés ainsi',
    },
    {
      en: 'The Devices panel following the track (#874), checked on four tracks; the audio output notice (#876) could not be triggered',
      fr: 'Le panneau Devices qui suit la piste (#874), vérifié sur quatre pistes ; l’avis de sortie audio (#876) n’a pas pu être déclenché',
    },
  ],
  gaps: [
    {
      en: 'Entering notes takes one keystroke per note: the 50-note vocal line took about 330 actions (#882).',
      fr: 'La saisie des notes demande une frappe par note : la ligne de chant de 50 notes a pris environ 330 actions (#882).',
    },
    {
      en: 'An automation segment cannot take a curve shape: the ease-in and smooth curves came out as straight lines, and a step rises over one beat (#881).',
      fr: 'Un segment d’automation ne peut pas prendre une forme de courbe : les courbes ease-in et smooth sont des droites, et un saut monte sur un temps (#881).',
    },
    {
      en: 'A refusal to place a clip on an occupied spot does not always name the clip or the bar, and a clip made with Alt+N keeps one bar when its placement has 2, 4 or 8 (#253).',
      fr: 'Un refus de placer un clip sur une place occupée ne nomme pas toujours le clip ni la mesure, et un clip créé par Alt+N garde une mesure quand son placement en fait 2, 4 ou 8 (#253).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6848219876',
  tracks: [
    { name: 'Arp', color: '#f08098' },
    { name: 'Basse', color: '#f18a76' },
    { name: 'Nappe', color: '#ea9356' },
    { name: 'Chant', color: '#d99e3f' },
    { name: 'Drums', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Seed', fr: 'Graine' }, bars: 16, parts: 3 },
    { name: { en: 'Seed 2', fr: 'Graine 2' }, bars: 16, parts: 4 },
    { name: { en: 'Accretion', fr: 'Accrétion' }, bars: 16, parts: 5 },
    { name: { en: 'Accretion 2', fr: 'Accrétion 2' }, bars: 16, parts: 5 },
    { name: { en: 'Plateau', fr: 'Plateau' }, bars: 20, parts: 5 },
    { name: { en: 'Switch', fr: 'Bascule' }, bars: 4, parts: 5 },
    { name: { en: 'End', fr: 'Fin' }, bars: 8, parts: 3 },
  ],
} satisfies JourneyEntry
