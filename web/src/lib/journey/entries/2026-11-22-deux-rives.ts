import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-22',
  slug: 'deux-rives',
  title: 'Deux rives',
  commit: '3964cf2',
  genre: {
    en: 'Electronic instrumental in two halves, dark and swung, then bright and straight',
    fr: 'Instrumental électronique en deux moitiés, sombre et balancée, puis claire et droite',
  },
  bpm: 100,
  key: { tonic: 'E', mode: 'minor' },
  bars: 80,
  duration: 193.56,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'No style profile, and a form of two pieces that are one: a dark bank in E minor, half-time, with a swung hi-hat, then a bright bank in E♭ major, four on the floor, with a straight hi-hat. Between them a door: one held note, G, the third of both keys. 100 bpm. Three constraints: only one note goes through the door (from bar 33 to 36 only G remains, the keyboard of the first bank and the pad of the second); a three-note cell, a rising minor third then a falling second, is the only melody (E – G – F♯, then G – B♭ – A♭); and the swing lives only in the sixteenth notes, so the second bank, written in eighth notes, stays straight under a single Swing.',
    fr: 'Aucun profil de style, et une forme en deux morceaux qui n’en font qu’un : une rive sombre en Mi mineur, en demi-temps, au charley balancé, puis une rive claire en Mi♭ majeur, quatre temps au sol, au charley droit. Entre les deux, une porte : une seule note tenue, Sol, tierce des deux tonalités. 100 bpm. Trois contraintes : une seule note passe la porte (de la mesure 33 à 36 il ne reste que Sol, au clavier de la première rive et à la nappe de la seconde) ; une cellule de trois notes, une tierce mineure montante puis une seconde descendante, est la seule mélodie (Mi – Sol – Fa♯, puis Sol – Si♭ – La♭) ; et le balancement ne tient qu’aux doubles croches, si bien que la seconde rive, écrite en croches, reste droite sous un seul Swing.',
  },
  built: {
    en: 'The eighteenth day built through the interface only, the whole sheet with nothing left outside Faulk: the three things the previous day could not have (a swing at the wanted value, pumping from the kick, a second key) are all built. 23 minutes from the first action to the first export. 8 tracks, 24 clips (257 notes), 30 placements, 26 devices (8 instruments, a Swing, 15 effects, and Compressor and Ceiling on the master), and 3 automation lanes with 14 points. The 257 notes were typed with step entry; the key change was placed at bar 37 from the Scale panel. Before publishing, the 18 melodic clips of the saved project were compared with the sheet: 18 out of 18 right, and the hits of the 6 drum clips match it too. Two WAV exports, one with the lead muted for the lead check: 3:14, with a peak at −0.4 dBFS on the final one. The project was saved and reopened from Recent projects, identical; the .faulkpack was built afterwards from the saved project folder (94 files, none missing), not exported by the app. Measured on the file, not listened to: the level sits near −16 dBFS RMS from the first bank to the second, rises to −15.1 at the peak, and drops to −25.1 on the door, nine decibels under its neighbours, which is its job and is still to be heard. The hi-hat swing, measured on a solo export, is at 60 %.',
    fr: 'Dix-huitième jour construit par l’interface seule, la fiche entière sans rien laisser hors de Faulk : les trois choses que le jour d’avant n’avait pas pu avoir (un balancement à la valeur voulue, un pompage déclenché par la grosse caisse, une seconde tonalité) sont toutes construites. 23 minutes de la première action au premier export. 8 pistes, 24 clips (257 notes), 30 placements, 26 devices (8 instruments, un Swing, 15 effets, et Compressor et Ceiling sur le master) et 3 couloirs d’automation de 14 points. Les 257 notes ont été saisies pas à pas ; le changement de tonalité a été posé à la mesure 37 par le panneau Gamme. Avant la publication, les 18 clips mélodiques du projet enregistré ont été comparés à la fiche : 18 sur 18 justes, et les coups des 6 clips de batterie y correspondent aussi. Deux exports WAV, dont un sans le lead pour le lead check : 3:14, avec une crête à −0,4 dBFS sur le dernier. Le projet a été enregistré puis rouvert depuis les projets récents, identique ; le .faulkpack a été construit ensuite à partir du dossier enregistré (94 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, pas écouté : le niveau reste autour de −16 dBFS RMS de la première rive à la seconde, monte à −15,1 au sommet et tombe à −25,1 sur la porte, neuf décibels sous ses voisines, ce qui est son rôle et reste à écouter. Le balancement du charley, mesuré sur un export en solo, est à 60 %.',
  },
  tested: [
    {
      en: 'Swing (#949): one on the hi-hat, sixteenth grid, odd sixteenths 30 ms late (60 %); the eighth notes of the second bank stay straight',
      fr: 'Swing (#949) : un sur le charley, grille de double croche, doubles croches impaires en retard de 30 ms (60 %) ; les croches de la seconde rive restent droites',
    },
    {
      en: 'The Compressor’s detection source (#960): the bass and the pad listen to the drums, 5 dB of reduction read during playback',
      fr: 'La source de détection du Compressor (#960) : la basse et la nappe écoutent la batterie, 5 dB de réduction lus en lecture',
    },
    {
      en: 'A key change placed at a bar (#961): bar 37, and the piano roll writes E♭, B♭ and A♭ after it while keeping F♯ before',
      fr: 'Un changement de tonalité posé à une mesure (#961) : la mesure 37, et le piano roll écrit Mi♭, Si♭ et La♭ après elle en gardant Fa♯ avant',
    },
    {
      en: 'Alt+P from the piano roll to the open clip’s placement (#950), used three times',
      fr: 'Alt+P depuis le piano roll jusqu’au placement du clip ouvert (#950), utilisé trois fois',
    },
  ],
  gaps: [
    {
      en: 'The lead rule and the lead check contradict each other on a sine lead: 3 dB under the bass gives 0.9 dB of difference, “buried”; the final lead, 1 dB under the bass, adds 1.3 dB (#965).',
      fr: 'La règle des faders et le lead check se contredisent sur un lead sinus : 3 dB sous la basse donne 0,9 dB d’écart, « enterré » ; le lead final, 1 dB sous la basse, ajoute 1,3 dB (#965).',
    },
    {
      en: 'No RMS level over a range in Faulk: the lead check took four exports and a script outside the app (#201).',
      fr: 'Pas de niveau RMS sur une plage dans Faulk : le lead check a demandé quatre exports et un script hors de l’app (#201).',
    },
    {
      en: 'The key change marker loses focus after Alt+→: the second press does nothing (#964).',
      fr: 'Le repère de changement de tonalité perd le focus après Alt+→ : le second appui ne fait rien (#964).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7084894060',
  tracks: [
    { name: 'Basse', color: '#f08098' },
    { name: 'Batterie', color: '#f18a76' },
    { name: 'Charley', color: '#ea9356' },
    { name: 'Clavier', color: '#d99e3f' },
    { name: 'Cellule', color: '#c1ab3c' },
    { name: 'Arp', color: '#a0b751' },
    { name: 'Nappe', color: '#79bf72' },
    { name: 'Chant', color: '#4ac495' },
  ],
  sections: [
    { name: { en: 'First bank: intro', fr: 'Première rive : entrée' }, bars: 8, parts: 3 },
    { name: { en: 'First bank: body', fr: 'Première rive : corps' }, bars: 16, parts: 5 },
    { name: { en: 'First bank: end', fr: 'Première rive : fin' }, bars: 8, parts: 5 },
    { name: { en: 'The door', fr: 'La porte' }, bars: 4, parts: 3 },
    { name: { en: 'Second bank: body', fr: 'Seconde rive : corps' }, bars: 16, parts: 5 },
    { name: { en: 'Second bank: peak', fr: 'Seconde rive : sommet' }, bars: 16, parts: 6 },
    { name: { en: 'Second bank: dissolve', fr: 'Seconde rive : dissolution' }, bars: 8, parts: 5 },
    { name: { en: 'End', fr: 'Fin' }, bars: 4, parts: 2 },
  ],
} satisfies JourneyEntry
