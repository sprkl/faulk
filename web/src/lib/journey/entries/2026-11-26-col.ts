import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-26',
  slug: 'col',
  title: 'Col',
  commit: '89ed221',
  genre: {
    en: 'Dark, warm electronic instrumental carried by the low end, with two peaks and a long valley between them',
    fr: 'Instrumental électronique sombre et chaud, porté par le grave, avec deux sommets et une longue vallée entre eux',
  },
  bpm: 120,
  key: { tonic: 'G', mode: 'minor' },
  bars: 96,
  duration: 195.7,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'morzak',
  brief: {
    en: 'The morzak style profile, and a form with two peaks: a col is the low pass between two summits. A fast climb to the first peak in G dorian; a long valley with no kick drum, where only the pad beats, through the tempo-synced LFO of its filter; then the second peak, higher than the first because its chords are richer and faster, the bass stepping down one degree a bar. On the last turn of that peak the D minor chord switches its third to D major, once. The way down takes its time. 120 bpm. Three constraints: two peaks, a long valley, and the rich chords for the second peak only; the arpeggio changes section by section without one of its notes being rewritten; and in the valley the pulse comes from a filter, not from the drums.',
    fr: 'Le profil de style morzak, et une forme à deux sommets : un col est le passage bas entre deux cimes. Une montée rapide au premier sommet en Sol dorien ; une longue vallée sans grosse caisse, où seule la nappe bat, par le LFO de son filtre calé sur le tempo ; puis le second sommet, plus haut que le premier parce que ses accords sont plus riches et plus rapides, la basse descendant d’un degré par mesure. Au dernier tour de ce sommet, l’accord de Ré mineur bascule sa tierce en Ré majeur, une seule fois. La descente prend son temps. 120 bpm. Trois contraintes : deux sommets, une longue vallée, et les accords riches pour le second sommet seulement ; l’arpège change de section en section sans qu’une seule de ses notes soit réécrite ; et dans la vallée, le pouls vient d’un filtre, pas de la batterie.',
  },
  built: {
    en: 'The twenty-second day built through the interface only, the whole sheet with no exception. 20 minutes from the first action to the first export, 11 of them for the notes and the arrangement. 6 tracks, 24 clips (273 notes), 31 placements, 21 devices, 8 automation lanes with 37 points. The arpeggio clips hold only sustained chords: its density goes from eighth notes to sixteenths, then to sixteenth triplets, through three lanes on Cascade’s Rate, Feel and Octaves; a solo export shows the eighths and the sixteenths at the planned bars, and the triplets were read on screen during playback, not measured. In the valley, a lane deepens the tempo-synced LFO of the pad’s filter. Before publishing, the 19 melodic clips of the saved project were compared with the sheet: 19 out of 19 right, and the hits of the 5 drum clips match it too. The lead check measured the single lead, which opens no filter, with an export where it is muted: it adds 2.1 dB over its main phrase, from 1.0 to 2.7 dB elsewhere, 3 dB under the bass fader. The final export: 3:15, with a peak at −0.3 dBFS and −17.2 dBFS RMS, after a second round that raised the valley and the last chord with two volume lanes. The project was saved and reopened, identical file by file; the .faulkpack was built afterwards from the saved project folder (92 files, none missing), not exported by the app. Measured on the file, not listened to: the first peak reaches −15.2 dBFS RMS, the start of the valley drops to −20.6, and the second peak goes higher, to −14.4.',
    fr: 'Vingt-deuxième jour construit par l’interface seule, la fiche entière sans exception. 20 minutes de la première action au premier export, dont 11 pour les notes et l’arrangement. 6 pistes, 24 clips (273 notes), 31 placements, 21 devices, 8 couloirs d’automation de 37 points. Les clips de l’arpège ne contiennent que des accords tenus : sa densité passe des croches aux doubles croches, puis aux triolets de doubles, par trois couloirs sur la Division, le Type et les Octaves de Cascade ; un export en solo montre les croches et les doubles croches aux mesures prévues, et les triolets ont été lus à l’écran pendant la lecture, pas mesurés. Dans la vallée, un couloir creuse le LFO calé sur le tempo du filtre de la nappe. Avant la publication, les 19 clips mélodiques du projet enregistré ont été comparés à la fiche : 19 sur 19 justes, et les coups des 5 clips de batterie y correspondent aussi. Le lead check a mesuré le seul lead, qui n’ouvre aucun filtre, avec un export où il est coupé : il ajoute 2,1 dB sur sa phrase principale, de 1,0 à 2,7 dB ailleurs, 3 dB sous le fader de la basse. L’export final : 3:15, avec une crête à −0,3 dBFS et −17,2 dBFS RMS, après un second tour qui a remonté la vallée et le dernier accord par deux couloirs de volume. Le projet a été enregistré puis rouvert, identique fichier par fichier ; le .faulkpack a été construit ensuite à partir du dossier enregistré (92 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, pas écouté : le premier sommet atteint −15,2 dBFS RMS, le début de la vallée tombe à −20,6, et le second sommet monte plus haut, à −14,4.',
  },
  tested: [
    {
      en: 'Automating a choice or whole-number parameter, tried for the first time: Cascade’s Rate, Feel and Octaves, followed by playback and the export (#984)',
      fr: 'Automatiser un paramètre à choix ou entier, essayé pour la première fois : la Division, le Type et les Octaves de Cascade, suivis par la lecture et l’export (#984)',
    },
    {
      en: 'Duplicating a whole track: the copy takes the volume, the four devices, the five clips and their placements; deleting it needs no confirmation, and Undo brings it back',
      fr: 'Dupliquer une piste entière : la copie reprend le volume, les quatre devices, les cinq clips et leurs placements ; la supprimer ne demande aucune confirmation, et Annuler la remet',
    },
  ],
  gaps: [
    {
      en: 'In the lane of a choice parameter, the typed label is refused without a message (“1/16” must be typed as its rank, “4”), and a new point starts on a linear curve, so the lane reads an option no point names, or “1.52” octaves (#984).',
      fr: 'Dans le couloir d’un paramètre à choix, le libellé tapé est refusé sans message (« 1/16 » doit se taper par son rang, « 4 »), et un nouveau point naît en courbe linéaire : le couloir lit une option qu’aucun point ne nomme, ou « 1,52 » octave (#984).',
    },
    {
      en: 'Nothing shows what Cascade plays: the triplets could only be read on its panel, and an arpeggio is checked through a solo export.',
      fr: 'Rien ne montre ce que joue Cascade : les triolets n’ont pu être lus que sur son panneau, et un arpège se vérifie par un export en solo.',
    },
    {
      en: 'The lead check and the levelling of the sections still took seven exports (#201).',
      fr: 'Le lead check et la mise à niveau des sections ont encore demandé sept exports (#201).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7186615394',
  tracks: [
    { name: 'Basse', color: '#f08098' },
    { name: 'Batterie', color: '#f18a76' },
    { name: 'Charley', color: '#ea9356' },
    { name: 'Arp', color: '#d99e3f' },
    { name: 'Nappe', color: '#c1ab3c' },
    { name: 'Chant', color: '#a0b751' },
  ],
  sections: [
    { name: { en: 'Climb', fr: 'Montée' }, bars: 16, parts: 3 },
    { name: { en: 'First peak', fr: 'Premier sommet' }, bars: 4, parts: 4 },
    { name: { en: 'First peak, with the song', fr: 'Premier sommet, avec le chant' }, bars: 8, parts: 6 },
    { name: { en: 'Valley', fr: 'Vallée' }, bars: 28, parts: 6 },
    { name: { en: 'Second peak', fr: 'Second sommet' }, bars: 4, parts: 5 },
    { name: { en: 'Second peak, with the song', fr: 'Second sommet, avec le chant' }, bars: 16, parts: 6 },
    { name: { en: 'Last turn, the D chord turns major', fr: 'Dernier tour, le Ré devient majeur' }, bars: 4, parts: 7 },
    { name: { en: 'Way down', fr: 'Descente' }, bars: 16, parts: 8 },
  ],
} satisfies JourneyEntry
