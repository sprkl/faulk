import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-06',
  slug: 'clairiere',
  title: 'Clairière',
  commit: 'ccfe337',
  genre: { en: 'Bright, airy instrumental electronic', fr: 'Instrumental électronique clair et aéré' },
  bpm: 120,
  key: { tonic: 'C', mode: 'lydian' },
  bars: 96,
  duration: 193.6,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'No style profile, and the opposite of the dark minor days before it: 120 bpm in C lydian, two chords (Cmaj7 and D) carrying 88 bars out of 96, and a euclidean pulse E(5,16) from the first step up to the exit. Every lead phrase ends on an eighth-note triplet. Three automation lanes move the arpeggio’s filter, the lead’s echo and the pad’s level.',
    fr: 'Aucun profil de style, et l’opposé des jours sombres et mineurs d’avant : 120 bpm en Do lydien, deux accords (Cmaj7 et D) qui portent 88 mesures sur 96, et une pulsation euclidienne E(5,16) du premier palier jusqu’à la sortie. Chaque phrase du lead finit par un triolet de croches. Trois couloirs d’automation font bouger le filtre de l’arpège, l’écho du lead et le volume de la nappe.',
  },
  built: {
    en: 'The fourth day built through the interface only: no command, no page script, no import of the sheet’s MIDI. About 50 minutes, around 1,400 keystrokes and 330 clicks. 6 tracks, 13 patterns and 323 notes, all at the lengths the sheet writes (held notes, triplets), 206 linked placements, two cuts with Ctrl+E, the project saved under a new name and reopened, and the mix exported as a WAV. About 95% of the sheet: the three automation lanes were created but stay empty.',
    fr: 'Quatrième jour construit par l’interface seule : aucune commande, aucun script de page, aucun import du MIDI de la fiche. Environ 50 minutes, quelque 1 400 frappes et 330 clics. 6 pistes, 13 motifs et 323 notes, toutes aux longueurs que la fiche écrit (notes tenues, triolets), 206 placements liés, deux coupes avec Ctrl+E, le projet enregistré sous un nouveau nom puis rouvert, et le mix exporté en WAV. Environ 95 % de la fiche : les trois couloirs d’automation sont créés mais restent vides.',
  },
  tested: [
    {
      en: 'Note length at the keyboard and the triplet snap, for the held notes and the lead’s triplets',
      fr: 'La longueur des notes au clavier et l’aimantation en triolets, pour les notes tenues et les triolets du lead',
    },
    {
      en: '“Repeat” for the arpeggio figures, and the euclidean generator for the 5 drum patterns',
      fr: '« Repeat » pour les figures de l’arpège, et le générateur euclidien pour les 5 motifs de batterie',
    },
    {
      en: 'The sheet’s new automation block, which wrote three lanes',
      fr: 'Le nouveau bloc d’automation de la fiche, qui a écrit trois couloirs',
    },
    {
      en: 'Cutting a placement with Ctrl+E, then Save as and reopening the project',
      fr: 'La coupe d’un placement avec Ctrl+E, puis « Save as » et la réouverture du projet',
    },
  ],
  gaps: [
    {
      en: 'Automation: the three lanes stay empty, because points cannot be placed without a coordinate pointer (#769).',
      fr: 'Automation : les trois couloirs restent vides, faute de pouvoir placer des points sans pointeur à coordonnées (#769).',
    },
    {
      en: '“Repeat” rounds a drum bar’s period to the quantize grid, so the bar is repeated off the beat (#770).',
      fr: '« Repeat » arrondit la période d’une mesure de batterie à la grille de quantification, la mesure se répète hors du temps (#770).',
    },
    {
      en: 'Keyboard paste goes to the playhead, not to the cursor cell (#771).',
      fr: 'Le collage au clavier va à la tête de lecture, pas à la cellule du curseur (#771).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6673844973',
  tracks: [
    { name: 'Drums', color: '#f18a76' },
    { name: 'Bass', color: '#f08098' },
    { name: 'Pad', color: '#ea9356' },
    { name: 'Arp', color: '#d99e3f' },
    { name: 'Lead', color: '#c1ab3c' },
    { name: 'Perc', color: '#a0b751' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 3 },
    { name: { en: 'A', fr: 'A' }, bars: 16, parts: 5 },
    { name: { en: 'Build', fr: 'Montée' }, bars: 8, parts: 6 },
    { name: { en: 'Drop', fr: 'Drop' }, bars: 16, parts: 6 },
    { name: { en: 'B', fr: 'B' }, bars: 8, parts: 4 },
    { name: { en: 'Break', fr: 'Cassure' }, bars: 8, parts: 4 },
    { name: { en: 'Drop 2', fr: 'Drop 2' }, bars: 16, parts: 6 },
    { name: { en: 'Exit', fr: 'Sortie' }, bars: 16, parts: 6 },
  ],
} satisfies JourneyEntry
