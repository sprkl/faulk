import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-25',
  slug: 'mue',
  title: 'Mue',
  commit: '832b354',
  genre: {
    en: 'Drifting electronic instrumental where each part grows out of the one before, and no part returns',
    fr: 'Instrumental électronique qui dérive, où chaque partie naît de la précédente, et où aucune partie ne revient',
  },
  bpm: 96,
  key: { tonic: 'C', mode: 'minor' },
  bars: 96,
  duration: 242.78,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'No style profile, and a form made of one stroke: five sections of 12, 16, 20, 24 and 24 bars, each born from a fragment of the one before. The rhythm of the opening chords becomes a bass cell; the notes of a sixteen-bar sung line become held chords; the grouping by three of the arpeggio they give becomes a new cell; and the opening rhythm comes back at the end, not as a part but as the cut of a pad. 96 bpm, C minor, then F minor from bar 49, after a clean break with one C held through it. Three constraints: no part comes back, everything descends from what came before; no written arpeggio, only held chords that Cascade plays out; and a single syncopated rhythm cell, 3 + 3 + 4 + 6 sixteenths, from which the chords, the bass, the kick and the closing hi-hat all come.',
    fr: 'Pas de profil de style, et une forme d’un seul trait : cinq sections de 12, 16, 20, 24 et 24 mesures, chacune née d’un fragment de la précédente. Le rythme des accords du début devient une cellule de basse ; les notes d’un chant de seize mesures deviennent des accords tenus ; le groupement par trois de l’arpège qu’ils donnent devient une nouvelle cellule ; et le rythme du début revient à la fin, non comme une partie mais comme la découpe d’une nappe. 96 bpm, Do mineur, puis Fa mineur à partir de la mesure 49, après une coupure franche traversée par un seul Do tenu. Trois contraintes : aucune partie ne revient, tout descend de ce qui précède ; aucun arpège écrit, seulement des accords tenus que Cascade égrène ; et une seule cellule rythmique syncopée, 3 + 3 + 4 + 6 doubles croches, d’où viennent les accords, la basse, le kick et le charley de la fin.',
  },
  built: {
    en: 'The twenty-first day built through the interface only, the whole sheet with no exception. 23 minutes from the first action to the first export, 14 of them for the notes and the placements. 8 tracks, 28 clips (440 notes), 31 placements, 30 devices, 5 automation lanes with 15 points and a key change at bar 49. The 440 notes were typed with step entry. The arpeggio clips hold only four sustained chords, played out by Cascade; the closing pad goes through Sluice keyed by the hi-hat, and the export shows its peaks on steps 1, 4, 7 and 11 of bar 73, the opening cell. Before publishing, the 17 melodic clips of the saved project were compared with the sheet: 17 out of 17 right, and the hits of the 11 drum clips match it too. The lead check measured both leads with an export where each is muted, without raising a lead above the bass fader: the sine lead adds 2.4 dB, the filtered square lead 2.3 dB with its filter closed and 1.4 dB at the end of its opening, from 700 Hz to 2.4 kHz with a 2 dB volume dip. Over bars 73–80 the square lead plays alone over the pad, so the rule cannot hold there by design: it read 10.4 dB at first, and the pad was raised by 5 dB to bring it to 6.3. The final export: 4:03, with a peak at −0.3 dBFS from the first render and −17.2 dBFS RMS. The project was saved and reopened, identical file by file; the .faulkpack was built afterwards from the saved project folder (110 files, none missing), not exported by the app. Measured on the file, not listened to: the sections go from −20.4 dBFS RMS to −15.5 at the densest, bar 72 drops to −21.7 with the cell alone, and the last section settles at −17.9.',
    fr: 'Vingt et unième jour construit par l’interface seule, la fiche entière sans exception. 23 minutes de la première action au premier export, dont 14 pour les notes et les placements. 8 pistes, 28 clips (440 notes), 31 placements, 30 devices, 5 couloirs d’automation de 15 points et un changement de tonalité à la mesure 49. Les 440 notes ont été saisies pas à pas. Les clips de l’arpège ne contiennent que quatre accords tenus, égrenés par Cascade ; la nappe de la fin passe dans Sluice, qui écoute le charley, et l’export montre ses crêtes aux pas 1, 4, 7 et 11 de la mesure 73, la cellule du début. Avant la publication, les 17 clips mélodiques du projet enregistré ont été comparés à la fiche : 17 sur 17 justes, et les coups des 11 clips de batterie y correspondent aussi. Le lead check a mesuré les deux leads avec un export où chacun est coupé, sans monter un lead au-dessus du fader de la basse : le lead sinus ajoute 2,4 dB, le lead carré filtré 2,3 dB filtre fermé et 1,4 dB en fin d’ouverture, de 700 Hz à 2,4 kHz avec une baisse de volume de 2 dB. Sur les mesures 73–80, le lead carré joue seul sur la nappe, la règle ne peut donc pas y tenir par construction : il lisait 10,4 dB au départ, et la nappe a été montée de 5 dB pour l’amener à 6,3. L’export final : 4:03, avec une crête à −0,3 dBFS dès le premier rendu et −17,2 dBFS RMS. Le projet a été enregistré puis rouvert, identique fichier par fichier ; le .faulkpack a été construit ensuite à partir du dossier enregistré (110 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, pas écouté : les sections vont de −20,4 dBFS RMS à −15,5 au plus dense, la mesure 72 tombe à −21,7 avec la cellule seule, et la dernière section se pose à −17,9.',
  },
  tested: [
    {
      en: 'A 6/8 time signature, tried for the first time in a new project: the transport, “Go to…” and the arrangement count the eighth note as the beat, the piano roll counts quarter notes (#981)',
      fr: 'Une signature 6/8, essayée pour la première fois dans un projet neuf : le transport, « Aller à… » et l’arrangement comptent la croche comme temps, le piano roll compte en noires (#981)',
    },
    {
      en: 'Exporting a range of bars with one track soloed, to check what Cascade plays: the solo carries into the export, though the dialog does not say so',
      fr: 'L’export d’une plage de mesures avec une piste en solo, pour vérifier ce que joue Cascade : le solo passe dans l’export, sans que le dialogue le dise',
    },
  ],
  gaps: [
    {
      en: 'Outside 4/4, the piano roll and the transport do not name the same beat: in 6/8 the sixth eighth note reads “beat 3.5” in one and “beat 6” in the other (#981).',
      fr: 'Hors 4/4, le piano roll et le transport ne nomment pas le même temps : en 6/8, la sixième croche se lit « temps 3.5 » dans l’un et « temps 6 » dans l’autre (#981).',
    },
    {
      en: 'Cascade and Sluice do not show what they do: unnamed choice groups, no level of the track Sluice listens to, no view of the notes Cascade plays; checking either took an export (#974).',
      fr: 'Cascade et Sluice ne montrent pas ce qu’ils font : des groupes de choix sans nom, pas de niveau de la piste qu’écoute Sluice, rien des notes que joue Cascade ; vérifier l’un ou l’autre a demandé un export (#974).',
    },
    {
      en: 'The lead check still takes five exports and a script outside Faulk (#201, #975).',
      fr: 'Le lead check demande encore cinq exports et un script hors de Faulk (#201, #975).',
    },
    {
      en: 'One time signature per project: a section in another metre cannot be built.',
      fr: 'Une seule signature par projet : une section dans une autre mesure ne peut pas se construire.',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7157205275',
  tracks: [
    { name: 'Basse', color: '#f08098' },
    { name: 'Batterie', color: '#f18a76' },
    { name: 'Charley', color: '#ea9356' },
    { name: 'Clavier', color: '#d99e3f' },
    { name: 'Arp', color: '#c1ab3c' },
    { name: 'Chant', color: '#a0b751' },
    { name: 'Cellule', color: '#79bf72' },
    { name: 'Trame', color: '#4ac495' },
  ],
  sections: [
    { name: { en: 'Chord loop', fr: 'Boucle d’accords' }, bars: 12, parts: 4 },
    { name: { en: 'Bass cell and song', fr: 'Cellule de basse et chant' }, bars: 16, parts: 4 },
    { name: { en: 'The song becomes chords', fr: 'Le chant devient accords' }, bars: 20, parts: 7 },
    { name: { en: 'The cell, in F minor', fr: 'La cellule, en Fa mineur' }, bars: 24, parts: 9 },
    { name: { en: 'The opening rhythm, as texture', fr: 'Le rythme du début, en texture' }, bars: 24, parts: 7 },
  ],
} satisfies JourneyEntry
