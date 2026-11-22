import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-11',
  slug: 'maree',
  title: 'Marée',
  commit: '878836f',
  genre: { en: 'Melodic instrumental electronic', fr: 'Instrumental électronique mélodique' },
  bpm: 108,
  key: { tonic: 'D', mode: 'major' },
  bars: 96,
  duration: 215.5,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'drifting-cycles',
  brief: {
    en: 'A theme and variations: an eight-bar hook stated in the first bar and never rewritten, then re-harmonised by a new chord cycle in each section. It moves to the bass, shrinks to its skeleton over a held D, then rises a minor third into F major, where it plays against itself as a canon. 108 bpm in D major, B minor visited in the middle, no chords in the intro.',
    fr: 'Un thème et variations : un crochet de huit mesures énoncé dès la première et jamais récrit, puis réharmonisé par un nouveau cycle d’accords à chaque section. Il passe à la basse, se réduit à son squelette sur un Ré tenu, puis monte d’une tierce mineure en Fa majeur, où il se joue en canon avec lui-même. 108 bpm en Ré majeur, Si mineur visité au milieu, aucun accord dans l’intro.',
  },
  built: {
    en: 'The eighth day built through the interface only. 50 minutes. 6 tracks, 22 patterns, 146 placements, 4 automation lanes with 13 points, and a whole section raised by three semitones with six key presses. Before the export, every melodic clip of the saved project was compared with the sheet bar by bar: a first pass found the hook’s held notes one step short in 12 notes, they were fixed, and the second pass gave 20 clips out of 20 right. The mix exported as a WAV, the project saved, reopened with everything back, and exported as a .faulkpack.',
    fr: 'Huitième jour construit par l’interface seule. 50 minutes. 6 pistes, 22 motifs, 146 placements, 4 couloirs d’automation de 13 points, et une section entière montée de trois demi-tons en six touches. Avant l’export, chaque clip mélodique du projet enregistré a été comparé à la fiche mesure par mesure : une première passe a trouvé les notes tenues du crochet trop courtes d’un pas sur 12 notes, elles ont été corrigées, et la seconde passe a donné 20 clips sur 20 justes. Le mix exporté en WAV, le projet enregistré, rouvert avec tout revenu, et exporté en .faulkpack.',
  },
  tested: [
    {
      en: 'Raising a whole section: copy in the arrangement, select all, three semitones up',
      fr: 'Monter une section entière : copie dans l’arrangement, tout sélectionner, trois demi-tons plus haut',
    },
    {
      en: 'Copying notes from one clip to another with Ctrl+C and Ctrl+V, and shifting every note of a clip with Alt+→',
      fr: 'Copier des notes d’un clip à l’autre avec Ctrl+C et Ctrl+V, et décaler toutes les notes d’un clip avec Alt+→',
    },
    {
      en: 'Bloom Ensemble on the pad, and deleting a track from the Actions menu',
      fr: 'Bloom Ensemble sur la nappe, et la suppression d’une piste par le menu Actions',
    },
  ],
  gaps: [
    {
      en: 'The piano roll’s tool goes back to Draw on every clip, and the click that gives focus drops an invisible note (#835).',
      fr: 'L’outil du piano roll revient sur Dessiner à chaque clip, et le clic qui donne le focus pose une note invisible (#835).',
    },
    {
      en: 'Ctrl+D on a clip overwrites the next placement on the same track without a message (#805).',
      fr: 'Ctrl+D sur un clip écrase le placement suivant de la même piste sans message (#805).',
    },
    {
      en: 'A transposed copy stays independent of its source: a fix to the hook had to be made again in each of its three copies (#253).',
      fr: 'Une copie transposée reste indépendante de sa source : une correction du crochet a dû être refaite dans chacune de ses trois copies (#253).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6790905391',
  tracks: [
    { name: 'Bass', color: '#f08098' },
    { name: 'Drums', color: '#f18a76' },
    { name: 'Arp', color: '#ea9356' },
    { name: 'Pad', color: '#d99e3f' },
    { name: 'Lead', color: '#c1ab3c' },
    { name: 'Canon', color: '#a0b751' },
  ],
  sections: [
    { name: { en: 'Theme', fr: 'Thème' }, bars: 16, parts: 4 },
    { name: { en: 'Variation 1', fr: 'Variation 1' }, bars: 16, parts: 5 },
    { name: { en: 'Variation 2', fr: 'Variation 2' }, bars: 16, parts: 4 },
    { name: { en: 'Variation 3', fr: 'Variation 3' }, bars: 16, parts: 5 },
    { name: { en: 'Variation 4', fr: 'Variation 4' }, bars: 16, parts: 7 },
    { name: { en: 'Return', fr: 'Retour' }, bars: 16, parts: 6 },
  ],
} satisfies JourneyEntry
