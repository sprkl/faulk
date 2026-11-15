import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-15',
  slug: 'qui-repond',
  title: 'Qui répond ?',
  commit: 'd3491e7',
  genre: { en: 'Playful, rhythmic electronic instrumental in call and response', fr: 'Instrumental électronique joueur et rythmique, en appel et réponse' },
  bpm: 110,
  key: { tonic: 'G', mode: 'mixolydian' },
  bars: 64,
  duration: 141.1,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'No style profile, and a call-and-response form: the sections are speakers, not steps of density. The lead asks a question over the chords with no drums; the bass and drums answer with no lead; then they cut in on each other faster and faster, every two bars, then every beat, and play together only at the end. 110 bpm, G mixolydian. Three constraints: lead and bass never sound together before bar 49, the lead has only five notes (one written exception, at bar 48), and no kick drum in a call.',
    fr: 'Aucun profil de style, et une forme en appel et réponse : les sections sont des voix qui parlent, pas des paliers de densité. Le lead pose une question sur les accords, sans batterie ; la basse et la batterie répondent, sans lead ; puis ils se coupent la parole de plus en plus vite, toutes les deux mesures, puis à chaque temps, et ne jouent ensemble qu’à la fin. 110 bpm, Sol mixolydien. Trois contraintes : le lead et la basse jamais ensemble avant la mesure 49, le lead n’a que cinq notes (une exception écrite, mesure 48), et pas de grosse caisse dans un appel.',
  },
  built: {
    en: 'The twelfth day built through the interface only. About 21 minutes, against 27 the day before, 13 of them for all the notes and the arrangement. 5 tracks, 22 clips (510 notes), 85 placements, 14 devices and 3 automation lanes with 16 points. Before publishing, the 18 melodic clips of the saved project were compared with the sheet: 18 out of 18 right, and the hits of the 4 drum clips match it too. The WAV export, first time, is 2:21 with a peak at −2.2 dBFS. The project was saved and reopened with everything back; the .faulkpack was built afterwards from the saved project folder (74 files, none missing), not exported by the app. Taking in the previous day’s listen, no section is left empty and the held notes go through the reverb: measured on the file, not listened to, the quietest stretch is the held final chord, at −27.2 dBFS RMS; bar 24, the lead alone in the echo, is at −26.0.',
    fr: 'Douzième jour construit par l’interface seule. Environ 21 minutes, contre 27 la veille, dont 13 pour toutes les notes et l’arrangement. 5 pistes, 22 clips (510 notes), 85 placements, 14 devices et 3 couloirs d’automation de 16 points. Avant la publication, les 18 clips mélodiques du projet enregistré ont été comparés à la fiche : 18 sur 18 justes, et les coups des 4 clips de batterie y sont conformes aussi. L’export WAV, du premier coup, dure 2:21, crête à −2,2 dBFS. Le projet enregistré, rouvert avec tout revenu ; le .faulkpack a été fabriqué ensuite à partir du dossier enregistré (74 fichiers, aucun manquant), non exporté par l’app. Suite à l’écoute de la veille, aucune section n’est laissée vide et les notes tenues passent par la réverbération : mesuré sur le fichier, sans écoute, le passage le plus faible est l’accord final tenu, à −27,2 dBFS RMS ; la mesure 24, le lead seul dans l’écho, est à −26,0.',
  },
  tested: [
    {
      en: 'An empty clip following its placement (#902): 18 clips of 2, 4 or 8 bars took their length with no separate setting',
      fr: 'Un clip vide qui suit son placement (#902) : 18 clips de 2, 4 ou 8 mesures ont pris leur longueur sans réglage à part',
    },
    {
      en: 'Step entry from clip to clip (#903): the mode held, the “letters write / play” indicator read before typing, Comma lengthened by the note length; no keystroke lost, about 1,200 keystrokes for 465 melodic notes',
      fr: 'La saisie pas à pas d’un clip à l’autre (#903) : le mode a tenu, le témoin « les lettres écrivent / jouent » se lisait avant de taper, la virgule allongeait de la longueur de note ; aucune frappe perdue, environ 1 200 frappes pour 465 notes mélodiques',
    },
    {
      en: 'Refusals that protect a neighbour: “Not repeated: Beat F, bars 34 to 34, is in the way” absorbed a mistake without damaging anything',
      fr: 'Les refus qui protègent un voisin : « Non répété : Beat F, mesures 34 à 34, est sur le chemin » a absorbé une erreur sans rien abîmer',
    },
  ],
  gaps: [
    {
      en: 'On an AZERTY keyboard, the “?” list names the note and octave keys as on QWERTY: following it plays a C♯ instead of changing octave (#906).',
      fr: 'Sur un clavier AZERTY, la liste « ? » nomme les touches de notes et d’octave comme sur un QWERTY : la suivre joue un Do♯ au lieu de changer d’octave (#906).',
    },
    {
      en: 'In a new project, an empty clip lengthened by one beat holds five beats while the piano roll says “2 bars” (#907).',
      fr: 'Dans un projet neuf, un clip vide allongé d’un temps fait cinq temps alors que le piano roll affiche « 2 mesures » (#907).',
    },
    {
      en: 'Step entry still stops on every track that has no clip: four Alt+E to press again in this song (#253).',
      fr: 'La saisie pas à pas s’arrête encore sur chaque piste sans clip : quatre Alt+E à refaire dans ce morceau (#253).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6891714227',
  tracks: [
    { name: 'Lead', color: '#f08098' },
    { name: 'Basse', color: '#f18a76' },
    { name: 'Accords', color: '#ea9356' },
    { name: 'Contre-chant', color: '#d99e3f' },
    { name: 'Drums', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Call A', fr: 'Appel A' }, bars: 8, parts: 2 },
    { name: { en: 'Answer A', fr: 'Réponse A' }, bars: 8, parts: 4 },
    { name: { en: 'Call B', fr: 'Appel B' }, bars: 8, parts: 3 },
    { name: { en: 'Answer B', fr: 'Réponse B' }, bars: 8, parts: 4 },
    { name: { en: 'Fusion', fr: 'Fusion' }, bars: 8, parts: 4 },
    { name: { en: 'Hocket', fr: 'Hoquet' }, bars: 8, parts: 5 },
    { name: { en: 'Resolution', fr: 'Résolution' }, bars: 8, parts: 4 },
    { name: { en: 'Send-off', fr: 'Envoi' }, bars: 8, parts: 5 },
  ],
} satisfies JourneyEntry
