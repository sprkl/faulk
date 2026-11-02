import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-02',
  slug: 'neon-sur-la-rocade',
  title: 'Néon sur la rocade',
  commit: '85e3d9f',
  genre: { en: 'Night synth-pop instrumental', fr: 'Instrumental synth-pop nocturne' },
  bpm: 104,
  key: { tonic: 'A', mode: 'major' },
  bars: 64,
  duration: 151.85,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'Night-drive synth-pop on F#m, D, A, E. The harmony exists only as a sixteenth-note arpeggio with written velocities, over a pad that holds a single A.',
    fr: 'Synth-pop de route de nuit sur Fa#m, Ré, La, Mi. L’harmonie n’existe qu’en arpège de doubles-croches à vélocités écrites, sur une nappe qui ne tient qu’un La.',
  },
  built: {
    en: 'Built in full and exported (2:32, peak −2.9 dBFS), saved and reopened. A drum clip was drawn note by note, then edited from the keyboard: move, delete, undo. Tracks, 13 clips, 27 placements and 2 lanes went through commands, with no refusal on the sheet’s script. A clap pattern had to move to a sixth track.',
    fr: 'Construit en entier et exporté (2:32, crête −2,9 dBFS), enregistré et rouvert. Un clip de batterie a été dessiné note par note, puis édité au clavier : déplacer, supprimer, annuler. Pistes, 13 clips, 27 placements et 2 couloirs sont passés par les commandes, sans un refus sur le script de la fiche. Un motif de clap a dû partir sur une sixième piste.',
  },
  tested: [
    { en: 'One row per track, and the track inspector', fr: 'Une rangée par piste, et l’inspecteur de piste' },
    { en: 'The keyboard model of the timeline and the piano roll', fr: 'Le modèle clavier de la timeline et du piano roll' },
    { en: 'Note brightness by velocity, ghost notes of other clips', fr: 'La luminosité des notes par vélocité, les notes fantômes des autres clips' },
    { en: 'The 16-colour track palette', fr: 'La palette de 16 couleurs de pistes' },
    { en: 'Meters everywhere, and the Compressor’s gain reduction', fr: 'Les vumètres partout, et la réduction de gain du Compressor' },
  ],
  gaps: [
    {
      en: 'From the keyboard you can move and delete notes, but not create a note or a clip, nor set a velocity (#494).',
      fr: 'Au clavier, on déplace et on supprime des notes, mais on ne crée ni note ni clip, et on ne règle pas la vélocité (#494).',
    },
    {
      en: 'An arrangement clip disappears with its last placement (#473).',
      fr: 'Un clip d’arrangement disparaît avec son dernier placement (#473).',
    },
    {
      en: 'Two patterns cannot overlap on one track: the clap needed a second drum track.',
      fr: 'Deux motifs ne peuvent pas se superposer sur une piste : le clap a demandé une seconde piste de batterie.',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6572525930',
  tracks: [
    { name: 'Drums', color: '#f2a33a' },
    { name: 'Perc', color: '#e07a5f' },
    { name: 'Bass', color: '#d9534f' },
    { name: 'Arp', color: '#4fb0c6' },
    { name: 'Lead', color: '#f0c419' },
    { name: 'Pad', color: '#8e6bd6' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 3 },
    { name: { en: 'Verse', fr: 'Couplet' }, bars: 16, parts: 5 },
    { name: { en: 'Chorus', fr: 'Refrain' }, bars: 16, parts: 6 },
    { name: { en: 'Breakdown', fr: 'Cassure' }, bars: 8, parts: 5 },
    { name: { en: 'Chorus 2', fr: 'Refrain 2' }, bars: 16, parts: 8 },
  ],
} satisfies JourneyEntry
