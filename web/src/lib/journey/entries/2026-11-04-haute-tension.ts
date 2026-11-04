import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-04',
  slug: 'haute-tension',
  title: 'Haute tension',
  commit: 'd5b5493',
  genre: { en: 'Fast, tense dark electronic instrumental', fr: 'Instrumental électronique sombre, rapide et tendu' },
  bpm: 152,
  key: { tonic: 'A', mode: 'minor' },
  bars: 96,
  duration: 153.24,
  model: 'Claude Fable',
  style: 'morzak',
  skills: ['music-producer'],
  brief: {
    en: 'Style profile “morzak”, in its fast family: 152 bpm in A harmonic minor, 96 bars, and a cycle of only two chords, Am and E, one bar each. A closed hi-hat plays off the beat from the first bar to the last, the peak sits at 69% and a 16-bar exit takes the layers away in the order they came. Once, in bars 65 to 68, E major turns into E minor.',
    fr: 'Profil de style « morzak », dans sa famille rapide : 152 bpm en La mineur harmonique, 96 mesures, et un cycle de deux accords seulement, Am et E, une mesure chacun. Un charley fermé joue en contretemps de la première à la dernière mesure, le pic est à 69 % et une sortie de 16 mesures retire les couches dans l’ordre inverse de leur arrivée. Une seule fois, aux mesures 65 à 68, Mi majeur devient Mi mineur.',
  },
  built: {
    en: 'The second day built through the interface only: no command, no page script, no import of the sheet’s MIDI. About 36 minutes, around 640 keystrokes and 185 clicks. 5 tracks, the sheet’s 10 patterns (as 13 clips), 89 notes, 96 bars placed as 140 mostly linked placements, and the whole mix, exported (2:33, peak −8.7 dBFS), saved and reopened. About 97% of the sheet: no section markers (not tried), no project name, two notes shorter than written.',
    fr: 'Deuxième jour construit par l’interface seule : aucune commande, aucun script de page, aucun import du MIDI de la fiche. Environ 36 minutes, quelque 640 frappes et 185 clics. 5 pistes, les 10 motifs de la fiche (en 13 clips), 89 notes, 96 mesures posées en 140 placements presque tous liés, et tout le mix, exporté (2:33, crête −8,7 dBFS), enregistré et rouvert. Environ 97 % de la fiche : pas de repères de section (non essayés), pas de nom de projet, deux notes plus courtes que prévu.',
  },
  tested: [
    {
      en: 'Placing at a bar from the keyboard (Ctrl+G) and “Repeat until…”, which cut the arrangement time in half',
      fr: 'Le placement à une mesure au clavier (Ctrl+G) et « Repeat until… », qui ont divisé par deux le temps d’arrangement',
    },
    { en: 'Draw velocity in the piano roll', fr: 'La vélocité d’écriture du piano roll' },
    { en: 'Right-click menus and the in-row track rename', fr: 'Les menus du clic droit et le renommage de piste dans la ligne' },
    { en: 'Tap tempo and the CPU load in the engine pill', fr: 'Le tap tempo et la charge CPU dans la pastille moteur' },
    {
      en: 'Exporting 2:33 of audio in about 10 seconds, then saving and reopening the project folder',
      fr: 'L’export de 2:33 d’audio en une dizaine de secondes, puis l’enregistrement et la réouverture du dossier de projet',
    },
  ],
  gaps: [
    {
      en: 'No note length from the keyboard: quarter notes on the “and” need a detour, a half note or a held note is impossible (#623).',
      fr: 'Pas de longueur de note au clavier : une noire sur le « et » demande un détour, une blanche ou une tenue est impossible (#623).',
    },
    {
      en: '“Place at the playhead” drops a copy without saying so and does not select it (#624).',
      fr: '« Place at the playhead » pose une copie sans le dire et ne la sélectionne pas (#624).',
    },
    {
      en: 'F2 on a track row does nothing, though the menu lists it (#625).',
      fr: 'F2 sur une ligne de piste ne fait rien, alors que le menu l’indique (#625).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6619058308',
  tracks: [
    { name: 'Drums', color: '#f18a76' },
    { name: 'Hats', color: '#ea9356' },
    { name: 'Bass', color: '#f08098' },
    { name: 'Chords', color: '#d99e3f' },
    { name: 'Lead', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 2 },
    { name: { en: 'Build', fr: 'Montée' }, bars: 16, parts: 4 },
    { name: { en: 'Groove', fr: 'Groove' }, bars: 16, parts: 5 },
    { name: { en: 'Break', fr: 'Cassure' }, bars: 8, parts: 4 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 32, parts: 8 },
    { name: { en: 'Exit', fr: 'Sortie' }, bars: 16, parts: 4 },
  ],
} satisfies JourneyEntry
