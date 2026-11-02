import type { JourneyEntry } from '../types'

export default {
  date: '2026-10-31',
  slug: 'ligne-haute',
  title: 'Ligne haute',
  commit: '8c9191c',
  genre: { en: 'Club instrumental', fr: 'Instrumental club' },
  bpm: 122,
  key: { tonic: 'A', mode: 'minor' },
  bars: 64,
  duration: 128.83,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'A club track on two chords, Am7 and Fmaj7, where every transition is a filter move and never a cymbal: a high-pass sweep on the master leads into the break, and the bass filter opens across the peak.',
    fr: 'Un morceau club sur deux accords, Am7 et Fmaj7, où chaque transition est un geste de filtre et jamais une cymbale : un balayage passe-haut du master mène au break, et le filtre de la basse s’ouvre pendant le sommet.',
  },
  built: {
    en: 'Built in full and exported from Faulk (2:09, peak −3.4 dBFS), saved to a folder and reopened intact. The sounds were set by typing values into the parameters. Tracks, 11 clips, 22 placements and 4 automation lanes went through commands, with no refusal. The master’s lane could only be made that way: the timeline did not show it yet.',
    fr: 'Construit en entier et exporté depuis Faulk (2:09, crête −3,4 dBFS), enregistré dans un dossier et rouvert intact. Les sons ont été réglés en tapant les valeurs des paramètres. Pistes, 11 clips, 22 placements et 4 couloirs d’automation sont passés par les commandes, sans un refus. Le couloir du master ne pouvait se faire que comme ça : la timeline ne l’affichait pas encore.',
  },
  tested: [
    { en: 'The Filter device, on a track and on the master', fr: 'Le device Filter, sur une piste et sur le master' },
    { en: 'Automation lanes on the master', fr: 'Les couloirs d’automation du master' },
    { en: 'Typed parameter values', fr: 'La saisie des valeurs de paramètres' },
    { en: 'Drawing notes with the mouse in the piano roll', fr: 'Le dessin des notes à la souris dans le piano roll' },
    { en: 'Follow after Stop', fr: 'Suivre après Stop' },
  ],
  gaps: [
    {
      en: 'A bare number was read in the displayed unit: “450” on a cutoff shown in kHz gave 20 kHz, with no warning (#410).',
      fr: 'Un nombre nu était lu dans l’unité affichée : « 450 » sur un cutoff affiché en kHz donnait 20 kHz, sans avertissement (#410).',
    },
    {
      en: 'The master’s automation lane could not be edited in the timeline (#407).',
      fr: 'Le couloir d’automation du master ne s’éditait pas dans la timeline (#407).',
    },
    {
      en: 'Drums has five sounds and no shaker: the intro’s shaker fell back on the closed hat.',
      fr: 'Drums n’a que cinq sons et pas de shaker : celui de l’intro s’est replié sur le charley fermé.',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6541152444',
  tracks: [
    { name: 'Drums', color: '#c75a4a' },
    { name: 'Bass', color: '#8a5ac7' },
    { name: 'Stabs', color: '#4a8ac7' },
    { name: 'Hook', color: '#c7a84a' },
    { name: 'Pad', color: '#6ac78a' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 2 },
    { name: { en: 'Build', fr: 'Montée' }, bars: 8, parts: 3 },
    { name: { en: 'Groove', fr: 'Groove' }, bars: 16, parts: 4 },
    { name: { en: 'Break', fr: 'Break' }, bars: 8, parts: 4 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 16, parts: 6 },
    { name: { en: 'Outro', fr: 'Sortie' }, bars: 8, parts: 3 },
  ],
} satisfies JourneyEntry
