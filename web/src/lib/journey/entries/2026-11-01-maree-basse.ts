import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-01',
  slug: 'maree-basse',
  title: 'Marée basse',
  commit: '7734e31',
  genre: { en: 'Slow dub instrumental', fr: 'Dub instrumental lent' },
  bpm: 76,
  key: { tonic: 'D', mode: 'dorian' },
  bars: 48,
  duration: 158.95,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'A slow dub on a single chord, Dm7, with no hi-hat: the skank and its echoes carry the top end. The mix moves in the timeline: an echo throw on the snare, a master pan sweep into the break, the master filter closing over the last four bars.',
    fr: 'Un dub lent sur un seul accord, Dm7, sans charley : le skank et ses échos tiennent l’aigu. Le mixage bouge dans la timeline : un jet d’écho sur la caisse claire, un balayage de panoramique du master à l’entrée de la coupure, le filtre du master qui se ferme sur les quatre dernières mesures.',
  },
  built: {
    en: 'Built in full and exported (2:39, peak −2.1 dBFS), saved and reopened. The first export pushed the new output limiter, and the export dialog said to turn the master down; one more export fixed it. A point was added to the master’s pan lane with the mouse. Tracks, 12 clips, 18 placements and 3 lanes went through commands, with no refusal.',
    fr: 'Construit en entier et exporté (2:39, crête −2,1 dBFS), enregistré et rouvert. Le premier export a poussé le nouveau limiteur de sortie, et le dialogue d’export a dit de baisser le master ; un second export a suffi. Un point a été posé à la souris dans le couloir de panoramique du master. Pistes, 12 clips, 18 placements et 3 couloirs sont passés par les commandes, sans un refus.',
  },
  tested: [
    { en: 'The master’s automation lanes in the timeline', fr: 'Les couloirs du master dans la timeline' },
    { en: 'Master pan', fr: 'Le panoramique du master' },
    { en: 'The output limiter', fr: 'Le limiteur de sortie' },
    { en: 'Values typed in base units', fr: 'La saisie en unité de base' },
    { en: 'Query commands', fr: 'Les commandes de requête' },
  ],
  gaps: [
    {
      en: 'In the arrangement, the track headers could slide out of line with the clips, and nothing showed that the view scrolled vertically (#438).',
      fr: 'Dans l’arrangement, les en-têtes de pistes pouvaient se décaler des clips, et rien n’indiquait le défilement vertical (#438).',
    },
    {
      en: 'Placements cannot overlap on one track: a one-off note cut the clip under it.',
      fr: 'Deux placements ne peuvent pas se superposer sur une piste : une note unique a coupé le clip en dessous.',
    },
    {
      en: 'The results of query commands did not reach an agent driving the page.',
      fr: 'Le résultat des commandes de requête n’arrivait pas jusqu’à un agent qui pilote la page.',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6555530492',
  tracks: [
    { name: 'Drums', color: '#f2a33a' },
    { name: 'Bass', color: '#d9534f' },
    { name: 'Skank', color: '#f0c419' },
    { name: 'Horn', color: '#4fb0c6' },
    { name: 'Pad', color: '#8e6bd6' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 2 },
    { name: { en: 'A', fr: 'A' }, bars: 16, parts: 5 },
    { name: { en: 'Break', fr: 'Coupure' }, bars: 8, parts: 4 },
    { name: { en: 'A′', fr: 'A′' }, bars: 16, parts: 7 },
  ],
} satisfies JourneyEntry
