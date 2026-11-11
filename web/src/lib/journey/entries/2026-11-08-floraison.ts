import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-08',
  slug: 'floraison',
  title: 'Floraison (v2)',
  commit: '98332b8',
  genre: { en: 'Mid-tempo instrumental electronic', fr: 'Instrumental électronique à mi-tempo' },
  bpm: 108,
  key: { tonic: 'E', mode: 'minor' },
  bars: 64,
  duration: 144.0,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'No style profile. 108 bpm in E minor, one four-chord cycle (Em, C, G, D) from start to end: what changes is the sound, not the harmony. Written to put Bloom to the test, the chorus that arrived the day before: a pad that widens from dry to detuned ensemble, and a vibrato lead that answers itself a fourth higher, a mechanical transposition that Lattice brings back into the key. The bass is a 3-3-2 figure and nothing else.',
    fr: 'Aucun profil de style. 108 bpm en Mi mineur, un seul cycle de quatre accords (Em, C, G, D) du début à la fin : ce qui change, c’est le son, pas l’harmonie. Écrit pour mettre à l’épreuve Bloom, le chorus arrivé la veille : une nappe qui s’élargit, de sèche à désaccordée en ensemble, et un lead vibré qui se répond une quarte plus haut, une transposition mécanique que Lattice ramène dans la gamme. La basse est une figure 3-3-2 et rien d’autre.',
  },
  built: {
    en: 'The sixth day built through the interface only: no command, no page script, no import of the sheet’s MIDI. 54 minutes. 7 tracks, 10 patterns, 136 linked placements, 8 cuts with Ctrl+E and 3 automation lanes with 14 points. The mix exported as a WAV, the project saved, reopened from the recent projects with everything back, and exported as a .faulkpack. Version 2: in the first export the arpeggio played C minor, G minor and D minor over C, G and D major (a semitone transposition of the Em bar); its three bars were corrected in the interface, nothing else changed. Bloom, Trim, Lattice, the Filter’s envelope follower and drive, and the Compressor’s knee, lookahead and mix all audible in the export.',
    fr: 'Sixième jour construit par l’interface seule : aucune commande, aucun script de page, aucun import du MIDI de la fiche. 54 minutes. 7 pistes, 10 motifs, 136 placements liés, 8 coupes avec Ctrl+E et 3 couloirs d’automation à 14 points. Le mix exporté en WAV, le projet enregistré, rouvert depuis les projets récents avec tout revenu, et exporté en .faulkpack. Version 2 : dans le premier export, l’arpège jouait Do mineur, Sol mineur et Ré mineur sur Do, Sol et Ré majeurs (une transposition en demi-tons de la mesure de Mi mineur) ; ses trois mesures ont été corrigées dans l’interface, rien d’autre n’a changé. Bloom, Trim, Lattice, le suiveur d’enveloppe et la saturation du Filter, le genou, l’anticipation et le mélange du Compressor, tous audibles dans l’export.',
  },
  tested: [
    {
      en: 'Bloom in Ensemble on the pad with its mix automated, and in tempo-synced Vibrato on both leads',
      fr: 'Bloom en Ensemble sur la nappe avec son mélange automatisé, et en Vibrato au tempo sur les deux leads',
    },
    {
      en: 'Trim shortening the bass to eighth notes, and Lattice bringing a transposed lead back into the key',
      fr: 'Trim qui raccourcit la basse en croches, et Lattice qui ramène dans la gamme un lead transposé',
    },
    {
      en: 'The Filter’s envelope follower and drive, and the Compressor’s knee, lookahead and parallel mix on the master',
      fr: 'Le suiveur d’enveloppe et la saturation du Filter, et le genou, l’anticipation et le mélange parallèle du Compressor sur le master',
    },
    {
      en: 'Saving a preset to the library, the light theme, the English interface and the .faulkpack export',
      fr: 'L’enregistrement d’un preset dans la bibliothèque, le thème clair, l’interface en anglais et l’export .faulkpack',
    },
  ],
  gaps: [
    {
      en: 'Space no longer starts playback after clicking a track row (#809).',
      fr: 'Espace ne lance plus la lecture après un clic sur une ligne de piste (#809).',
    },
    {
      en: 'When nothing consumes the audio, position and meters freeze without a word (#810).',
      fr: 'Quand rien ne consomme l’audio, la position et les vumètres se figent sans un mot (#810).',
    },
    {
      en: 'Keyboard placements stay costly: two pieces with the same label after a cut, the Devices panel stuck on the master (#805, #806).',
      fr: 'Les placements au clavier restent coûteux : deux morceaux au même libellé après une coupe, le panneau Devices collé au master (#805, #806).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6719561674',
  tracks: [
    { name: 'Bass', color: '#f08098' },
    { name: 'Drums', color: '#f18a76' },
    { name: 'Hats', color: '#ea9356' },
    { name: 'Pad', color: '#d99e3f' },
    { name: 'Arp', color: '#c1ab3c' },
    { name: 'Lead', color: '#a0b751' },
    { name: 'Réponse', color: '#79bf72' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 3 },
    { name: { en: 'A', fr: 'A' }, bars: 16, parts: 5 },
    { name: { en: 'Break', fr: 'Cassure' }, bars: 8, parts: 5 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 16, parts: 7 },
    { name: { en: 'Exit', fr: 'Sortie' }, bars: 16, parts: 7 },
  ],
} satisfies JourneyEntry
