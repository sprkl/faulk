import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-03',
  slug: 'brume-sur-le-canal',
  title: 'Brume sur le canal',
  commit: '16a774e',
  genre: { en: 'Slow, heavy dark electronic instrumental', fr: 'Instrumental électronique sombre, lent et lourd' },
  bpm: 84,
  key: { tonic: 'D', mode: 'minor' },
  bars: 64,
  duration: 184.44,
  model: 'Claude Fable',
  style: 'morzak',
  skills: ['music-producer'],
  brief: {
    en: 'Style profile “morzak”: slow half-time at 84 bpm in D minor, a four-chord cycle of one bar each, a single B section on B♭, and only the hi-hat swung. The lead has four pitches and never plays on beat 1.',
    fr: 'Profil de style « morzak » : demi-temps lent à 84 bpm en Ré mineur, un cycle de quatre accords d’une mesure, une seule section B sur Si♭, et seul le charley swingue. Le lead n’a que quatre hauteurs et ne joue jamais sur le temps 1.',
  },
  built: {
    en: 'The first day built through the interface only: no command, no page script, no import of the sheet’s MIDI. About 36 minutes, around 720 keystrokes and 145 clicks. 5 tracks, the sheet’s 9 patterns (plus 3 edited variants), 85 notes, 64 bars and the whole mix, exported (3:04, peak −8.5 dBFS), saved and reopened. About 95% of the sheet: no section markers (the playhead cannot be placed by keyboard), no project name, three notes shorter than written.',
    fr: 'Premier jour construit par l’interface seule : aucune commande, aucun script de page, aucun import du MIDI de la fiche. Environ 36 minutes, quelque 720 frappes et 145 clics. 5 pistes, les 9 motifs de la fiche (plus 3 variantes éditées), 85 notes, 64 mesures et tout le mix, exporté (3:04, crête −8,5 dBFS), enregistré et rouvert. Environ 95 % de la fiche : pas de repères de section (la tête de lecture ne se place pas au clavier), pas de nom de projet, trois notes plus courtes que prévu.',
  },
  tested: [
    { en: 'Creating notes and clips from the keyboard, with velocity', fr: 'La création de notes et de clips au clavier, avec la vélocité' },
    { en: 'The track Actions menu and the right-click menu', fr: 'Le menu Actions de la piste et le menu du clic droit' },
    { en: 'Quantize shortcuts in the piano roll', fr: 'Les raccourcis de quantification du piano roll' },
    { en: 'Clip names taken from the track, and the clip pool', fr: 'Les noms de clips d’après la piste, et la réserve de clips' },
    { en: 'The “All” zoom, then saving and reopening a project folder', fr: 'Le zoom « All », puis l’enregistrement et la réouverture d’un dossier de projet' },
  ],
  gaps: [
    {
      en: 'A clip alone on its track cannot be selected from the keyboard (#553).',
      fr: 'Un clip seul sur sa piste ne se sélectionne pas au clavier (#553).',
    },
    {
      en: 'The playhead cannot be placed by keyboard: no marker at the right bar, no cut at bar 63 (#554).',
      fr: 'La tête de lecture ne se place pas au clavier : pas de repère à la bonne mesure, pas de coupe à la mesure 63 (#554).',
    },
    {
      en: 'Duplicating makes independent clips: 66 independent clips for a dozen patterns, and no shortcut for a linked copy (#555).',
      fr: 'Dupliquer fait des clips indépendants : 66 clips indépendants pour une douzaine de motifs, et pas de raccourci pour une copie liée (#555).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6599463461',
  tracks: [
    { name: 'Drums', color: '#f18a76' },
    { name: 'Hats', color: '#ea9356' },
    { name: 'Bass', color: '#f08098' },
    { name: 'Chords', color: '#d99e3f' },
    { name: 'Lead', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 2 },
    { name: { en: 'Build', fr: 'Montée' }, bars: 8, parts: 4 },
    { name: { en: 'Groove', fr: 'Groove' }, bars: 16, parts: 5 },
    { name: { en: 'Break', fr: 'Cassure' }, bars: 8, parts: 4 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 16, parts: 6 },
    { name: { en: 'Exit', fr: 'Sortie' }, bars: 8, parts: 4 },
  ],
} satisfies JourneyEntry
