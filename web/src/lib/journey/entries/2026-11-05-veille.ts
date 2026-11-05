import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-05',
  slug: 'veille',
  title: 'Veille',
  commit: '19455be',
  genre: { en: 'Dark, warm nocturnal electronic instrumental', fr: 'Instrumental électronique sombre et chaud, nocturne' },
  bpm: 112,
  key: { tonic: 'F', mode: 'minor' },
  bars: 112,
  duration: 241.66,
  model: 'Claude Fable',
  style: 'morzak',
  skills: ['music-producer'],
  brief: {
    en: 'Style profile “morzak”, in its mid-groove family: 112 bpm in F minor, 112 bars, and a four-chord cycle of one bar each (Fm, Cm, D♭, E♭) held all the way through. The arpeggio changes from eighths to sixteenths to triplets, the lead keeps five pitches and grows by rhythm and register, and two automation lanes move the arpeggio’s filter and the lead’s echo. Once, in bars 77 to 80, F minor turns into F major.',
    fr: 'Profil de style « morzak », dans sa famille mi-groove : 112 bpm en Fa mineur, 112 mesures, et un cycle de quatre accords d’une mesure (Fm, Cm, D♭, E♭) tenu tout le morceau. L’arpège passe des croches aux doubles-croches puis aux triolets, la mélodie garde cinq hauteurs et se développe par le rythme et le registre, et deux couloirs d’automation font bouger le filtre de l’arpège et l’écho du lead. Une seule fois, aux mesures 77 à 80, Fa mineur devient Fa majeur.',
  },
  built: {
    en: 'The third day built through the interface only: no command, no page script, no import of the sheet’s MIDI. About 52 minutes, around 1,700 keystrokes and 260 clicks. 346 notes typed at the keyboard, 183 linked placements made with “Place at the playhead” and “Repeat until…”, the project scale set to F minor, the project renamed, saved under a new name and reopened, and the mix exported as a WAV named in the File name field. About 90% of the sheet: the automation lanes were created but stay empty, every note has the snap length, and no triplet lengths.',
    fr: 'Troisième jour construit par l’interface seule : aucune commande, aucun script de page, aucun import du MIDI de la fiche. Environ 52 minutes, quelque 1 700 frappes et 260 clics. 346 notes tapées au clavier, 183 placements liés faits avec « Place at the playhead » et « Repeat until… », la gamme du projet réglée sur Fa mineur, le projet renommé, enregistré sous un nouveau nom puis rouvert, et le mix exporté en WAV nommé dans le champ de nom de fichier. Environ 90 % de la fiche : les couloirs d’automation sont créés mais restent vides, chaque note a la longueur du pas de la grille (snap), sans longueurs de triolet.',
  },
  tested: [
    {
      en: 'The project scale (new), set to F minor before writing the notes',
      fr: 'La gamme du projet (nouveauté), réglée sur Fa mineur avant d’écrire les notes',
    },
    {
      en: '“Place at the playhead” and “Repeat until…” for 183 linked placements',
      fr: '« Place at the playhead » et « Repeat until… » pour 183 placements liés',
    },
    { en: 'Renaming the project and “Save as”, then reopening it', fr: 'Le renommage du projet et « Save as », puis la réouverture' },
    {
      en: 'Exporting a 4-minute WAV named by the File name field',
      fr: 'L’export d’un WAV de 4 minutes nommé par le champ de nom de fichier',
    },
  ],
  gaps: [
    {
      en: 'Automation: the two lanes were created, but the ramps were not built, so they stay empty.',
      fr: 'Automation : les deux couloirs sont créés, mais les rampes n’ont pas été construites, ils restent vides.',
    },
    {
      en: 'No held notes: every note is written at the snap length (#623).',
      fr: 'Pas de notes tenues : chaque note est écrite à la longueur du pas de la grille (#623).',
    },
    {
      en: 'No triplet lengths in the piano roll (#711).',
      fr: 'Pas de longueurs de triolet dans le piano roll (#711).',
    },
    {
      en: 'An arpeggio costs a lot without repeating a selection from end to end (#710).',
      fr: 'Un arpège coûte très cher sans pouvoir répéter une sélection de bout en bout (#710).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6648350498',
  tracks: [
    { name: 'Drums', color: '#f18a76' },
    { name: 'Bass', color: '#f08098' },
    { name: 'Chords', color: '#ea9356' },
    { name: 'Arp', color: '#d99e3f' },
    { name: 'Lead', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 16, parts: 4 },
    { name: { en: 'Groove', fr: 'Groove' }, bars: 24, parts: 5 },
    { name: { en: 'Build', fr: 'Montée' }, bars: 16, parts: 5 },
    { name: { en: 'B', fr: 'B' }, bars: 8, parts: 4 },
    { name: { en: 'Break', fr: 'Cassure' }, bars: 8, parts: 4 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 24, parts: 7 },
    { name: { en: 'Exit', fr: 'Sortie' }, bars: 16, parts: 6 },
  ],
} satisfies JourneyEntry
