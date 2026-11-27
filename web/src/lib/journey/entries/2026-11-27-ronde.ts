import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-27',
  slug: 'ronde',
  title: 'Ronde',
  commit: 'e49a88a',
  genre: {
    en: 'Bright melodic electronic instrumental, a rondo whose chorus comes back three times',
    fr: 'Instrumental électronique mélodique et clair, un rondo dont le refrain revient trois fois',
  },
  bpm: 122,
  key: { tonic: 'C', mode: 'major' },
  bars: 80,
  duration: 160.58,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'drifting-cycles',
  brief: {
    en: 'The drifting-cycles style profile, and a rondo: the opening groove, A, comes back three times without its notes changing. Between its returns, two visits: B, with no drums, in the relative minor, on another sound and over a bass pedal; C, with no bass, three against four, on a suspended chord that resolves only when A returns. Each return of A keeps something of the visit before it: the register after B, the rhythm of C in the hi-hat, and the sound of B as a counter-line in the last one, which rises a tone, to D major, at 70 % of the piece. 122 bpm. Three constraints: one chord cycle per section, with an anchor chord; a single late rise of a tone, after a thinned bar and over a common note, and the suspended chord of C held until A comes back; and A never changes its notes, so the piece is built as three scenes of the grid sent to the arrangement.',
    fr: 'Le profil de style drifting-cycles, et un rondo : le groove du début, A, revient trois fois sans que ses notes changent. Entre ses retours, deux visites : B, sans batterie, dans le relatif mineur, sur un autre son et sous une pédale de basse ; C, sans basse, en trois contre quatre, sur un accord suspendu qui ne se résout qu’au retour de A. Chaque retour de A garde quelque chose de la visite d’avant : le registre après B, le rythme de C dans le charley, et le son de B en contre-chant au dernier, qui se hausse d’un ton, en Ré majeur, à 70 % du morceau. 122 bpm. Trois contraintes : un cycle d’accords par section, avec un accord d’ancrage ; une seule hausse tardive d’un ton, après une mesure amincie et sur une note commune, et l’accord suspendu de C tenu jusqu’au retour de A ; et A ne change jamais ses notes, donc le morceau se construit comme trois scènes de la grille envoyées dans l’arrangement.',
  },
  built: {
    en: 'The twenty-third day built through the interface only, the whole sheet with no exception. 30 minutes from the first action to the first export, 18 of them for the notes and the arrangement. The piece started in the grid: three named scenes, 13 clips created in their slots with step entry in 6 minutes, each scene launched to hear it, then “Send scene to arrangement (linked)” five times. Each send placed every clip once, at its own length, so 20 “Lengthen to…” were needed to fill the sections (#987). The returns of the chorus are copies of the same clips, an octave or a tone up. 7 tracks, 24 clips (307 notes), 37 placements, 23 devices, 5 automation lanes with 16 points and a key change at bar 57; the five sounds start from the Oscillator’s factory presets and are then bent. Before publishing, the 19 melodic clips of the saved project were compared with the sheet: 19 out of 19 right, and the hits of the 5 drum clips match it too. The lead check measured both leads with an export where each is muted, without raising a lead above the bass fader. The square lead adds 1.1 dB at the end of its filter opening, 1.3 octaves offset by a 1 dB volume dip. With the 2 dB dip of the rule it read 0.9 dB, buried, so the dip was reduced rather than the lead raised (#975). The bell lead adds 1.5 dB where it leads, and 0.6 dB as a counter-line under the square lead, buried there and left as it was. The final export: 2:41, with a peak at −0.3 dBFS and −17.5 dBFS RMS. The project was saved and reopened, identical file by file; the .faulkpack was built afterwards from the saved project folder (94 files, none missing), not exported by the app. Measured on the file, not listened to: the second chorus reads −16.3 dBFS RMS and the last one −16.4, so the last return stands out by its rise of a tone, not by its level.',
    fr: 'Vingt-troisième jour construit par l’interface seule, la fiche entière sans exception. 30 minutes de la première action au premier export, dont 18 pour les notes et l’arrangement. Le morceau est parti de la grille : trois scènes nommées, 13 clips créés dans leurs cases à la saisie pas à pas en 6 minutes, chaque scène lancée pour l’entendre, puis « Envoyer la scène dans l’arrangement (liée) » cinq fois. Chaque envoi a posé chaque clip une fois, à sa propre longueur : il a fallu 20 « Allonger jusqu’à… » pour remplir les sections (#987). Les retours du refrain sont des copies des mêmes clips, une octave ou un ton plus haut. 7 pistes, 24 clips (307 notes), 37 placements, 23 devices, 5 couloirs d’automation de 16 points et un changement de tonalité à la mesure 57 ; les cinq sons partent des presets d’usine de l’Oscillator, puis sont tordus. Avant la publication, les 19 clips mélodiques du projet enregistré ont été comparés à la fiche : 19 sur 19 justes, et les coups des 5 clips de batterie y correspondent aussi. Le lead check a mesuré les deux leads avec un export où chacun est coupé, sans monter un lead au-dessus du fader de la basse. Le lead carré ajoute 1,1 dB en fin d’ouverture de son filtre, 1,3 octave compensée par une baisse de volume de 1 dB. Avec les 2 dB de baisse de la règle, il se lisait à 0,9 dB, enterré : la baisse a été réduite plutôt que le lead monté (#975). Le lead cloche ajoute 1,5 dB là où il mène, et 0,6 dB en contre-chant sous le lead carré, enterré à cet endroit et laissé tel quel. L’export final : 2:41, avec une crête à −0,3 dBFS et −17,5 dBFS RMS. Le projet a été enregistré puis rouvert, identique fichier par fichier ; le .faulkpack a été construit ensuite à partir du dossier enregistré (94 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, pas écouté : le deuxième refrain se lit à −16,3 dBFS RMS et le dernier à −16,4 ; le dernier retour se distingue donc par sa hausse d’un ton, pas par son niveau.',
  },
  tested: [
    {
      en: 'Building a piece from the grid, tried for the first time: three scenes, clips created in their slots, a scene launched while another plays, then each scene sent to the arrangement as linked clips (#987)',
      fr: 'Construire un morceau depuis la grille, essayé pour la première fois : trois scènes, des clips créés dans leurs cases, une scène lancée pendant qu’une autre joue, puis chaque scène envoyée dans l’arrangement en clips liés (#987)',
    },
    {
      en: 'The Oscillator’s factory presets as a starting point for five sounds, browsed with ‹ ›',
      fr: 'Les presets d’usine de l’Oscillator comme point de départ de cinq sons, parcourus par ‹ ›',
    },
    {
      en: 'Dragging a clip from one grid slot to another with the mouse: the clip moves, and Undo puts it back',
      fr: 'Glisser un clip d’une case de la grille à une autre à la souris : le clip se déplace, et Annuler le remet',
    },
  ],
  gaps: [
    {
      en: 'Sending a scene to the arrangement places each clip once at its own length, with no message and no selection: five sends needed 20 “Lengthen to…”, one placement at a time (#987).',
      fr: 'Envoyer une scène dans l’arrangement pose chaque clip une fois à sa propre longueur, sans message ni sélection : cinq envois ont demandé 20 « Allonger jusqu’à… », un placement à la fois (#987).',
    },
    {
      en: 'Offsetting a filter opening by the rule’s 2 dB buries the lead at the end of the opening, for the third time; 1 dB was used instead (#975).',
      fr: 'Compenser une ouverture de filtre des 2 dB de la règle enterre le lead en fin d’ouverture, pour la troisième fois ; 1 dB a été pris à la place (#975).',
    },
    {
      en: 'The lead check and the levelling still took six exports for two leads (#201).',
      fr: 'Le lead check et la mise à niveau ont encore demandé six exports pour deux leads (#201).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7213753183',
  tracks: [
    { name: 'Basse', color: '#f08098' },
    { name: 'Accords', color: '#f18a76' },
    { name: 'Nappe', color: '#ea9356' },
    { name: 'Accroche', color: '#d99e3f' },
    { name: 'Cloche', color: '#c1ab3c' },
    { name: 'Batterie', color: '#a0b751' },
    { name: 'Charley', color: '#79bf72' },
  ],
  sections: [
    { name: { en: 'Chorus, without the hook', fr: 'Refrain, sans l’accroche' }, bars: 8, parts: 4 },
    { name: { en: 'Chorus', fr: 'Refrain' }, bars: 8, parts: 6 },
    { name: { en: 'Visit B, in the relative minor', fr: 'Visite B, dans le relatif mineur' }, bars: 8, parts: 4 },
    { name: { en: 'Chorus, an octave up', fr: 'Refrain, une octave plus haut' }, bars: 16, parts: 6 },
    { name: { en: 'Visit C, three against four', fr: 'Visite C, trois contre quatre' }, bars: 8, parts: 6 },
    { name: { en: 'Chorus, the filter opens', fr: 'Refrain, le filtre s’ouvre' }, bars: 8, parts: 7 },
    { name: { en: 'Last chorus, in D major', fr: 'Dernier refrain, en Ré majeur' }, bars: 16, parts: 7 },
    { name: { en: 'Coda', fr: 'Coda' }, bars: 8, parts: 3 },
  ],
} satisfies JourneyEntry
