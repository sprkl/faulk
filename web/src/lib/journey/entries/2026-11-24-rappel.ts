import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-24',
  slug: 'rappel',
  title: 'Rappel',
  commit: '737d17b',
  genre: {
    en: 'Melodic electronic instrumental between two lights, with a false ending and a second life',
    fr: 'Instrumental électronique mélodique, entre deux lumières, avec une fausse fin et une seconde vie',
  },
  bpm: 106,
  key: { tonic: 'E', mode: 'minor' },
  bars: 88,
  duration: 202.39,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'drifting-cycles',
  brief: {
    en: 'The drifting-cycles style profile, and a false ending followed by a second life: a first short piece in E minor rises fast, peaks early and comes apart down to a single note; four bars of almost nothing make it sound over; then that last note becomes a pedal, everything starts again at half time under moving chords, and the real peak arrives a tone higher, in F♯ minor from bar 61, with the first melody passed down to the bass. 106 bpm. Three constraints: a new chord cycle for each section, with an anchor chord, while the eight-note hook never changes its notes and is reharmonised; one pedal, one suspended chord and a single key change, the C♯ sus4 heard unresolved, then held, and resolved only on the first beat of the peak; and the opening melody is never transposed on its own: when the key goes up, it goes down three octaves to the bass and another voice sings above it.',
    fr: 'Le profil de style drifting-cycles, et une fausse fin suivie d’une seconde vie : un premier morceau court en Mi mineur monte vite, culmine tôt et se défait jusqu’à une seule note ; quatre mesures de presque rien font croire que c’est fini ; puis cette dernière note devient une pédale, tout repart en demi-temps sous des accords qui bougent, et le vrai sommet arrive un ton plus haut, en Fa♯ mineur à partir de la mesure 61, la première mélodie passée à la basse. 106 bpm. Trois contraintes : un nouveau cycle d’accords par section, avec un accord d’ancre, tandis que l’accroche de huit notes ne change jamais de notes et se fait réharmoniser ; une pédale, un accord suspendu et un seul changement de tonalité, le Do♯ sus4 entendu sans résolution, puis tenu, et résolu seulement au premier temps du sommet ; et la mélodie du début n’est jamais transposée seule : quand la tonalité monte, elle descend de trois octaves à la basse et une autre voix chante au-dessus.',
  },
  built: {
    en: 'The twentieth day built through the interface only, the whole sheet with nothing left outside Faulk except the filtered noise rise, since Faulk has no noise source yet. 22 minutes from the first action to the first export. 7 tracks, 27 clips (308 notes), 29 placements, 25 devices, 4 automation lanes with 19 points, a key change at bar 61 and 2 project presets. The 308 notes were typed with step entry. Before publishing, the 19 melodic clips of the saved project were compared with the sheet: 19 out of 19 right, and the hits of the 8 drum clips match it too. The lead check measured both leads with an export where each is muted, in two rounds of three exports, without raising a lead above the bass fader: the kept render adds 1.5 dB for the filtered square lead at the end of its opening, from 800 Hz to 3.2 kHz with a 2 dB volume dip, and 1.1 dB for the sine lead. The final export: 3:22, with a peak at −0.3 dBFS and −17.4 dBFS RMS. The project was saved and reopened, identical file by file; the .faulkpack was built afterwards from the saved project folder (100 files, none missing), not exported by the app. Measured on the file, not listened to: the first piece rises to −15.6 dBFS RMS, the false ending drops to −64.8 (its four bars hold no part, only the tails of the dissolution, and are counted with it below), and the second peak comes back at −16.1.',
    fr: 'Vingtième jour construit par l’interface seule, la fiche entière sans rien laisser hors de Faulk sauf la montée de bruit filtré, faute de source de bruit dans Faulk. 22 minutes de la première action au premier export. 7 pistes, 27 clips (308 notes), 29 placements, 25 devices, 4 couloirs d’automation de 19 points, un changement de tonalité à la mesure 61 et 2 presets de projet. Les 308 notes ont été saisies pas à pas. Avant la publication, les 19 clips mélodiques du projet enregistré ont été comparés à la fiche : 19 sur 19 justes, et les coups des 8 clips de batterie y correspondent aussi. Le lead check a mesuré les deux leads avec un export où chacun est coupé, en deux tours de trois exports, sans monter un lead au-dessus du fader de la basse : le rendu gardé ajoute 1,5 dB pour le lead carré filtré en fin d’ouverture, de 800 Hz à 3,2 kHz avec une baisse de volume de 2 dB, et 1,1 dB pour le lead sinus. L’export final : 3:22, avec une crête à −0,3 dBFS et −17,4 dBFS RMS. Le projet a été enregistré puis rouvert, identique fichier par fichier ; le .faulkpack a été construit ensuite à partir du dossier enregistré (100 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, pas écouté : le premier morceau monte à −15,6 dBFS RMS, la fausse fin tombe à −64,8 (ses quatre mesures ne contiennent aucune partie, seulement les queues de la dissolution, et sont comptées avec elle ci-dessous) et le second sommet revient à −16,1.',
  },
  tested: [
    {
      en: 'Insert and delete time in the arrangement, tried for the first time: placements, automation points and section markers follow, and one undo puts everything back',
      fr: 'Insérer et supprimer du temps dans l’arrangement, essayés pour la première fois : placements, points d’automation et marqueurs de section suivent, et un seul Annuler remet tout',
    },
    {
      en: 'Project presets: the Echo and the Reverb of one lead saved in the project and recalled on the other lead, and they come back with the reopened project',
      fr: 'Les presets de projet : l’Echo et la Reverb d’un lead enregistrés dans le projet et rappelés sur l’autre lead, et ils reviennent avec le projet rouvert',
    },
  ],
  gaps: [
    {
      en: 'The key change stays at bar 61 after time is inserted or deleted at bar 41, while everything else follows (#962).',
      fr: 'Le changement de tonalité reste à la mesure 61 après une insertion ou une suppression de temps à la mesure 41, alors que tout le reste suit (#962).',
    },
    {
      en: 'Insert and delete time: raw history lines, empty fields, an ambiguous end of range, and no shortcut (#978).',
      fr: 'Insérer et supprimer du temps : lignes d’historique brutes, champs vides, fin de plage ambiguë, et aucun raccourci (#978).',
    },
    {
      en: 'Offsetting a filter opening still lowers the measured lead at the end of the opening: with a 3 dB dip the square lead reads buried (#975).',
      fr: 'Compenser une ouverture de filtre fait encore baisser le lead mesuré en fin d’ouverture : avec une baisse de 3 dB, le lead carré se lit enterré (#975).',
    },
    {
      en: 'No noise source for a filtered rise, and no second oscillator for a stacked, detuned lead.',
      fr: 'Pas de source de bruit pour une montée filtrée, ni de second oscillateur pour un lead empilé et désaccordé.',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7126238531',
  tracks: [
    { name: 'Basse', color: '#f08098' },
    { name: 'Batterie', color: '#f18a76' },
    { name: 'Charley', color: '#ea9356' },
    { name: 'Arp', color: '#d99e3f' },
    { name: 'Nappe', color: '#c1ab3c' },
    { name: 'Accroche', color: '#a0b751' },
    { name: 'Chant', color: '#79bf72' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 2 },
    { name: { en: 'Rise', fr: 'Montée' }, bars: 16, parts: 7 },
    { name: { en: 'First peak', fr: 'Premier sommet' }, bars: 8, parts: 6 },
    { name: { en: 'Dissolve and false ending', fr: 'Dissolution et fausse fin' }, bars: 12, parts: 7 },
    { name: { en: 'Second life', fr: 'Reprise' }, bars: 16, parts: 7 },
    { name: { en: 'Second peak', fr: 'Second sommet' }, bars: 16, parts: 6 },
    { name: { en: 'Return', fr: 'Retour' }, bars: 8, parts: 6 },
    { name: { en: 'Cut', fr: 'Coupure' }, bars: 4, parts: 1 },
  ],
} satisfies JourneyEntry
