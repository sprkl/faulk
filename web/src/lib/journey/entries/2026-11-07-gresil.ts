import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-07',
  slug: 'gresil',
  title: 'Grésil',
  commit: 'fff08a7',
  genre: { en: 'Mid-tempo instrumental electronic', fr: 'Instrumental électronique à mi-tempo' },
  bpm: 100,
  key: { tonic: 'D', mode: 'dorian' },
  bars: 72,
  duration: 174.9,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'No style profile. 100 bpm in D dorian, a four-chord cycle (Dm7, G7, Dm7, Am7) held for 64 bars out of 72, written for the day’s new tools: chords stacked by a MIDI effect from a single note, an audio stutter as the only event at every seam, a filter moving on its own in time on the arpeggio, and automation lanes typed at the keyboard. A three-note arpeggio on a four-step grid, and a rain of small random high notes, the “grésil”, that comes and goes.',
    fr: 'Aucun profil de style. 100 bpm en Ré dorien, un cycle de quatre accords (Dm7, G7, Dm7, Am7) tenu 64 mesures sur 72, écrit pour les nouveaux outils du jour : des accords empilés par un effet MIDI à partir d’une seule note, un bégaiement audio comme seul événement à chaque couture, un filtre qui bouge tout seul au tempo sur l’arpège, et des couloirs d’automation tapés au clavier. Un arpège de trois notes sur une grille de quatre, et une pluie de petites notes aiguës et aléatoires, le « grésil », qui va et vient.',
  },
  built: {
    en: 'The fifth day built through the interface only: no command, no page script, no import of the sheet’s MIDI. 49 minutes. 7 tracks, 12 patterns and 246 notes typed (Stack adds the chord notes), 159 linked placements, one cut with Ctrl+E, and, new, automation lanes filled: 29 points on 4 lanes, entered at the keyboard. The mix exported as a WAV with no clipped sample, the project saved under a new name, then reopened with everything back. About 95% of the sheet: the lanes’ eased curves (only linear and steps exist) and the section markers are missing.',
    fr: 'Cinquième jour construit par l’interface seule : aucune commande, aucun script de page, aucun import du MIDI de la fiche. 49 minutes. 7 pistes, 12 motifs et 246 notes saisies (Stack ajoute les notes des accords), 159 placements liés, une coupe avec Ctrl+E, et, nouveau, des couloirs d’automation remplis : 29 points sur 4 couloirs, saisis au clavier. Le mix exporté en WAV sans aucun échantillon coupé, le projet enregistré sous un nouveau nom, puis rouvert avec tout revenu. Environ 95 % de la fiche : manquent les courbes adoucies des couloirs (seuls linéaire et palier existent) et les repères de section.',
  },
  tested: [
    {
      en: 'Stack, which builds each seventh chord of the pad from one note, and Scatter on the high texture',
      fr: 'Stack, qui construit chaque accord de septième de la nappe à partir d’une seule note, et Scatter sur la texture aiguë',
    },
    {
      en: 'Hiccup as the event of every seam, its mix automated in steps',
      fr: 'Hiccup comme événement de chaque couture, son mix automatisé en paliers',
    },
    {
      en: 'The Filter’s tempo-synced LFO on the arpeggio, with Echo, Reverb, the Compressor and Ceiling on the master',
      fr: 'Le LFO au tempo du Filter sur l’arpège, avec Echo, Reverb, le Compressor et Ceiling sur le master',
    },
    {
      en: 'Automation points and ramps at the keyboard, and “Repeat” holding a three-over-four figure as written',
      fr: 'Les points et rampes d’automation au clavier, et « Repeat » qui tient une figure de trois sur quatre comme écrite',
    },
  ],
  gaps: [
    {
      en: 'Keyboard placements: Ctrl+D put a copy on another track, and Alt+↑ ↓ silently replaced the placement it crossed (#805).',
      fr: 'Placements au clavier : Ctrl+D a posé une copie sur une autre piste, et Alt+↑ ↓ a remplacé sans le dire le placement traversé (#805).',
    },
    {
      en: 'The Devices panel stays on the master after a track is chosen in the arrangement (#806).',
      fr: 'Le panneau Devices reste sur le master après le choix d’une piste dans l’arrangement (#806).',
    },
    {
      en: 'Automation ramps are slow to type: six actions per segment, and no eased curves (#253).',
      fr: 'Les rampes d’automation sont longues à saisir : six actions par segment, et pas de courbes adoucies (#253).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6707032986',
  tracks: [
    { name: 'Bass', color: '#f08098' },
    { name: 'Drums', color: '#f18a76' },
    { name: 'Hats', color: '#ea9356' },
    { name: 'Pad', color: '#d99e3f' },
    { name: 'Arp', color: '#c1ab3c' },
    { name: 'Lead', color: '#a0b751' },
    { name: 'Grésil', color: '#79bf72' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 3 },
    { name: { en: 'A', fr: 'A' }, bars: 16, parts: 5 },
    { name: { en: 'Build', fr: 'Montée' }, bars: 8, parts: 6 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 16, parts: 7 },
    { name: { en: 'Break', fr: 'Cassure' }, bars: 8, parts: 4 },
    { name: { en: 'Exit', fr: 'Sortie' }, bars: 16, parts: 8 },
  ],
} satisfies JourneyEntry
