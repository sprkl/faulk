import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-16',
  slug: 'contre-jour',
  title: 'Contre-jour',
  commit: '867ff55',
  genre: { en: 'Dark, warm, hypnotic electronic instrumental', fr: 'Instrumental électronique sombre et chaud, hypnotique' },
  bpm: 116,
  key: { tonic: 'C', mode: 'dorian' },
  bars: 64,
  duration: 134.7,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'morzak',
  brief: {
    en: 'Style profile “morzak” and a loop with one interruption: a four-bar loop that changes only by its timbre, one break, then the same loop reharmonised, which grows and stops in the middle of a phrase. 116 bpm, C dorian. The F major held for 28 bars turns to F minor at bar 29, and that is the interruption itself. Three constraints: two chord cycles alternating over a steady groove, an arpeggio of three steps over 4/4 whose gaps the lead fills, and no note changing before the interruption: only the filters move.',
    fr: 'Profil de style « morzak » et une boucle avec une interruption : une boucle de quatre mesures qui ne change que par le timbre, une seule coupure, puis la même boucle réharmonisée, qui se développe et s’arrête au milieu d’une phrase. 116 bpm, Do dorien. Le Fa majeur tenu 28 mesures devient Fa mineur à la mesure 29, et c’est l’interruption elle-même. Trois contraintes : deux cycles d’accords qui alternent sur un groove constant, un arpège de trois pas sur du 4/4 dont le lead remplit les trous, et aucune note qui change avant l’interruption : seuls les filtres bougent.',
  },
  built: {
    en: 'The thirteenth day built through the interface only. About 24 minutes from opening the app to “Saved”. 5 tracks, 20 clips (305 notes), 123 placements, 14 devices on the tracks plus Compressor and Ceiling on the master, and 4 automation lanes with 18 points. Every note was typed with step entry, about 760 keystrokes. Before publishing, the 16 melodic clips of the saved project were compared with the sheet: 16 out of 16 right, and the hits of the 4 drum clips match it too. The WAV export, first time, is 2:15 with a peak at −1.0 dBFS. The project was saved and reopened with everything back; the .faulkpack was built afterwards from the saved project folder (71 files, none missing), not exported by the app. Measured on the file, not listened to: the interruption stays carried, 2.5 dB under the loop, and the peak section is the loudest, but only 1.2 dB above the loop.',
    fr: 'Treizième jour construit par l’interface seule. Environ 24 minutes de l’ouverture de l’app à « Enregistré ». 5 pistes, 20 clips (305 notes), 123 placements, 14 devices sur les pistes plus Compressor et Ceiling sur le master, et 4 couloirs d’automation de 18 points. Toutes les notes ont été tapées en saisie pas à pas, environ 760 frappes. Avant la publication, les 16 clips mélodiques du projet enregistré ont été comparés à la fiche : 16 sur 16 justes, et les coups des 4 clips de batterie y sont conformes aussi. L’export WAV, du premier coup, dure 2:15, crête à −1,0 dBFS. Le projet enregistré, rouvert avec tout revenu ; le .faulkpack a été fabriqué ensuite à partir du dossier enregistré (71 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, sans écoute : l’interruption reste portée, 2,5 dB sous la boucle, et le sommet est le passage le plus fort, mais de 1,2 dB seulement au-dessus de la boucle.',
  },
  tested: [
    {
      en: 'Keys named as the layout writes them (#909): on AZERTY, the “?” list, the Keyboard tab and the step entry tooltip agree (Q Z S E D…, W / X, “:” rest, “;” lengthen)',
      fr: 'Les touches nommées comme les écrit la disposition (#909) : en AZERTY, la liste « ? », l’onglet Clavier et l’info-bulle de la saisie pas à pas concordent (Q Z S E D…, W / X, « : » silence, « ; » allonge)',
    },
    {
      en: 'The real clip length (#910): a five-beat clip reads “1 bar 1 beat”; + gives 2, − gives 1, the up arrow gives 2',
      fr: 'La vraie longueur du clip (#910) : un clip de cinq temps affiche « 1 mesure 1 temps » ; + donne 2, − donne 1, la flèche du haut donne 2',
    },
    {
      en: 'Ctrl+G, Alt+N, Alt+Shift+→, Ctrl+Shift+R, then step entry in one go: a phrase of 64 sixteenths typed at once, and cuts on the beat from track to track',
      fr: 'Ctrl+G, Alt+N, Alt+Maj+→, Ctrl+Maj+R, puis la saisie pas à pas d’un trait : une phrase de 64 doubles croches tapée en une fois, et des coupes au temps d’une piste à l’autre',
    },
  ],
  gaps: [
    {
      en: 'Nothing lengthens or repeats a placement up to a given bar in one gesture: a one-bar pattern held 24 bars takes 23 keystrokes or 24 linked copies, and reaching a far placement means counting arrows (#253).',
      fr: 'Rien n’allonge ni ne répète un placement jusqu’à une mesure donnée en un geste : un motif d’une mesure tenu 24 mesures demande 23 frappes ou 24 copies liées, et atteindre un placement lointain oblige à compter les flèches (#253).',
    },
    {
      en: 'Step entry still stops on a track that has no clip, so Alt+E has to be pressed again (#253).',
      fr: 'La saisie pas à pas s’arrête encore sur une piste sans clip, et il faut refaire Alt+E (#253).',
    },
    {
      en: '“Ramp to…” confirmed without an end value draws a flat segment without a word (#253).',
      fr: '« Rampe jusqu’à… » validée sans valeur d’arrivée trace un segment plat sans rien dire (#253).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6911341291',
  tracks: [
    { name: 'Arp', color: '#f08098' },
    { name: 'Basse', color: '#f18a76' },
    { name: 'Nappe', color: '#ea9356' },
    { name: 'Lead', color: '#d99e3f' },
    { name: 'Drums', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Opening', fr: 'Amorce' }, bars: 4, parts: 4 },
    { name: { en: 'Loop', fr: 'Boucle' }, bars: 8, parts: 4 },
    { name: { en: 'Loop 2', fr: 'Boucle 2' }, bars: 8, parts: 5 },
    { name: { en: 'Loop 3', fr: 'Boucle 3' }, bars: 8, parts: 5 },
    { name: { en: 'Interruption', fr: 'Interruption' }, bars: 4, parts: 4 },
    { name: { en: 'Return', fr: 'Retour' }, bars: 8, parts: 5 },
    { name: { en: 'Return 2', fr: 'Retour 2' }, bars: 8, parts: 5 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 8, parts: 5 },
    { name: { en: 'Way out', fr: 'Sortie' }, bars: 4, parts: 5 },
    { name: { en: 'Drop-off', fr: 'Chute' }, bars: 4, parts: 5 },
  ],
} satisfies JourneyEntry
