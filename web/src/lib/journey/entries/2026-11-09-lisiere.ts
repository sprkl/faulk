import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-09',
  slug: 'lisiere',
  title: 'Lisière',
  commit: 'c9bb408',
  genre: { en: 'Nocturnal instrumental electronic', fr: 'Instrumental électronique nocturne' },
  bpm: 116,
  key: { tonic: 'F', mode: 'minor' },
  bars: 80,
  duration: 166.6,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'morzak',
  brief: {
    en: 'The first track made with the producer’s new rules: a form other than the usual build-up, and a chord picture per section. A loop held for 32 bars that only changes in timbre, cut dead in the middle of a bar, taken up again on a second cycle where the tonic chord flips from major to minor, then opened for the last peak onto a richer grid whose bass steps down one degree per bar. 116 bpm in F minor with a dorian major IV.',
    fr: 'Premier morceau fait avec les nouvelles règles du producteur : une autre forme que la montée habituelle, et une image d’accords par section. Une boucle tenue trente-deux mesures qui ne change que de timbre, coupée net au milieu d’une mesure, reprise sur un second cycle où l’accord de tonique bascule de majeur à mineur, puis ouverte pour le dernier sommet sur une grille plus riche dont la basse descend d’un degré par mesure. 116 bpm en Fa mineur avec le IV majeur du dorien.',
  },
  built: {
    en: 'The seventh day built through the interface only. 60 minutes. 6 tracks, 16 patterns, 142 placements and 10 cuts with Ctrl+E, 3 automation lanes, and a real audio file imported onto an audio track for the interruption. Before the export, every melodic clip of the saved project was compared with the sheet bar by bar: 14 clips out of 14 right. The mix exported as a WAV, the project saved, reopened with everything back, and exported as a .faulkpack with its audio.',
    fr: 'Septième jour construit par l’interface seule. 60 minutes. 6 pistes, 16 motifs, 142 placements et 10 coupes avec Ctrl+E, 3 couloirs d’automation, et un vrai fichier audio importé sur une piste audio pour l’interruption. Avant l’export, chaque clip mélodique du projet enregistré a été comparé à la fiche mesure par mesure : 14 clips sur 14 justes. Le mix exporté en WAV, le projet enregistré, rouvert avec tout revenu, et exporté en .faulkpack avec son audio.',
  },
  tested: [
    {
      en: 'The new form rotation: a loop with one interruption instead of the usual build-up',
      fr: 'La nouvelle rotation des formes : une boucle à une interruption au lieu de la montée habituelle',
    },
    {
      en: 'Two chord cycles alternating by section, then a richer grid for the last peak',
      fr: 'Deux cycles d’accords qui alternent par section, puis une grille plus riche pour le dernier sommet',
    },
    {
      en: 'Importing an audio file onto an audio track, saved in the project and in the .faulkpack',
      fr: 'L’import d’un fichier audio sur une piste audio, enregistré dans le projet et dans le .faulkpack',
    },
    {
      en: 'Hiccup with its mix automated around the cut, and Scatter’s settings',
      fr: 'Hiccup avec son mélange automatisé autour de la coupure, et les réglages de Scatter',
    },
  ],
  gaps: [
    {
      en: 'Note editing at the keyboard can mislead: a focus click draws an invisible note, and ← → on a focused note does not follow time order (#253).',
      fr: 'L’édition des notes au clavier peut tromper : un clic de focalisation dessine une note invisible, et ← → sur une note focalisée ne suit pas l’ordre du temps (#253).',
    },
    {
      en: 'Automation curves are linear or in steps only, typed point by point (#253).',
      fr: 'Les courbes d’automation sont seulement linéaires ou en paliers, saisies point par point (#253).',
    },
    {
      en: 'After a cut, two placements still share the same label (#805).',
      fr: 'Après une coupe, deux placements portent encore le même libellé (#805).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6729964498',
  tracks: [
    { name: 'Bass', color: '#f08098' },
    { name: 'Drums', color: '#f18a76' },
    { name: 'Pad', color: '#ea9356' },
    { name: 'Arp', color: '#d99e3f' },
    { name: 'Lead', color: '#c1ab3c' },
    { name: 'Tail', color: '#a0b751' },
  ],
  sections: [
    { name: { en: 'Loop A', fr: 'Boucle A' }, bars: 32, parts: 5 },
    { name: { en: 'Rupture', fr: 'Rupture' }, bars: 4, parts: 3 },
    { name: { en: 'Loop B', fr: 'Boucle B' }, bars: 24, parts: 5 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 20, parts: 5 },
  ],
} satisfies JourneyEntry
