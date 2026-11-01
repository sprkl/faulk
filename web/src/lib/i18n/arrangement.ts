/** The arrangement view (`src/views/arrangement/`, AR-4): the clips placed on the tracks along the timeline. */

import { defineFeature } from './feature'

export const arrangement = defineFeature(
  {
    'arrangement.empty': 'Add a track to start arranging.',
    'arrangement.grid': 'Arrangement timeline',
    'arrangement.zoom': 'Zoom',
    'arrangement.zoomIn': 'Zoom in (more detail)',
    'arrangement.zoomOut': 'Zoom out (more bars)',
    'arrangement.follow': 'Follow',
    'arrangement.followHint': 'The view follows the play position and turns the page. Scrolling by hand stops it; Play or this button starts it again.',
    'arrangement.scroll': 'Scroll the timeline',
    'arrangement.scrollRows': 'Scroll the tracks',
    'arrangement.tools': 'Edit the selection',
    'arrangement.split': 'Split',
    'arrangement.splitHint': 'Split the selected clips at the play position (Ctrl+E).',
    'arrangement.duplicate': 'Duplicate',
    'arrangement.duplicateHint': 'Copy the selected clips right after them, as independent clips (Ctrl+D). Alt+drag copies where you drop.',
    'arrangement.duplicateLinked': 'Duplicate linked',
    'arrangement.duplicateLinkedHint':
      'Copy the selected clips right after them, as linked copies: editing one changes the others (Ctrl+Shift+D). Alt+Shift+drag copies where you drop.',
    'arrangement.delete': 'Delete',
    'arrangement.timeTools': 'Insert or delete bars',
    'arrangement.bars': 'Number of bars',
    'arrangement.insertBars': 'Insert bars',
    'arrangement.insertBarsHint':
      'Insert this many empty bars in every track, the automation and the loop included. They go at the start of the time selection, or at the bar of the play position when none is selected.',
    'arrangement.deleteBars': 'Delete selected bars',
    'arrangement.deleteBarsHint':
      'Delete the selected time from every track and close the gap, automation and loop included. Select time with Shift+drag on the ruler.',
    'arrangement.help':
      'Drag a clip to move it, its edges to resize it. Hold Ctrl or Cmd to skip snapping. Drag on the ruler to loop a part, click on it to move the play position. Double-click an empty lane to create a clip, a clip to edit its notes. To move around: the scrollbars, the wheel over the tracks (up and down), Shift + wheel for left and right (or the arrow keys and Home once the timeline is focused), and Ctrl or Cmd + wheel (or + and −, or the zoom buttons) to zoom.',
    'arrangement.count': '{n} clips',
    'arrangement.selected': '{n} selected',
    'arrangement.lane': 'Track {name}',
    'arrangement.offTimeline': 'Plays from the grid, not the timeline',
    'arrangement.backTrack': 'Back to arrangement',
    'arrangement.backTrackHint': 'Track {name} plays from the grid: give it back to its timeline.',
    'arrangement.clipDefault': 'Clip',
  },
  {
    'arrangement.empty': 'Ajoute une piste pour commencer à arranger.',
    'arrangement.grid': 'Timeline d’arrangement',
    'arrangement.zoom': 'Zoom',
    'arrangement.zoomIn': 'Zoomer (plus de détail)',
    'arrangement.zoomOut': 'Dézoomer (plus de mesures)',
    'arrangement.follow': 'Suivre',
    'arrangement.followHint': 'La vue suit la position de lecture et tourne la page. Faire défiler à la main l’arrête ; Lecture ou ce bouton la relance.',
    'arrangement.scroll': 'Faire défiler la timeline',
    'arrangement.scrollRows': 'Faire défiler les pistes',
    'arrangement.tools': 'Modifier la sélection',
    'arrangement.split': 'Couper',
    'arrangement.splitHint': 'Coupe les clips sélectionnés à la position de lecture (Ctrl+E).',
    'arrangement.duplicate': 'Dupliquer',
    'arrangement.duplicateHint': 'Copie les clips sélectionnés juste après eux, en clips indépendants (Ctrl+D). Alt+glisser copie là où tu déposes.',
    'arrangement.duplicateLinked': 'Dupliquer en lié',
    'arrangement.duplicateLinkedHint':
      'Copie les clips sélectionnés juste après eux, en copies liées : modifier l’une change les autres (Ctrl+Maj+D). Alt+Maj+glisser copie là où tu déposes.',
    'arrangement.delete': 'Supprimer',
    'arrangement.timeTools': 'Insérer ou supprimer des mesures',
    'arrangement.bars': 'Nombre de mesures',
    'arrangement.insertBars': 'Insérer des mesures',
    'arrangement.insertBarsHint':
      'Insère ce nombre de mesures vides dans toutes les pistes, automation et boucle comprises. Elles vont au début de la sélection de temps, ou à la mesure de la position de lecture s’il n’y en a pas.',
    'arrangement.deleteBars': 'Supprimer les mesures sélectionnées',
    'arrangement.deleteBarsHint':
      'Supprime le temps sélectionné de toutes les pistes et referme le trou, automation et boucle comprises. Sélectionne du temps avec Maj + glisser sur la règle.',
    'arrangement.help':
      'Glisse un clip pour le déplacer, ses bords pour le redimensionner. Maintiens Ctrl ou Cmd pour ignorer le magnétisme. Glisse sur la règle pour boucler une partie, clique dessus pour déplacer la position de lecture. Double-clique une piste vide pour créer un clip, un clip pour modifier ses notes. Pour te déplacer : les barres de défilement, la molette sur les pistes (haut et bas), Maj + molette pour la gauche et la droite (ou les flèches et Début quand la timeline a le focus), et Ctrl ou Cmd + molette (ou + et −, ou les boutons de zoom) pour zoomer.',
    'arrangement.count': '{n} clips',
    'arrangement.selected': '{n} sélectionnés',
    'arrangement.lane': 'Piste {name}',
    'arrangement.offTimeline': 'Joue depuis la grille, pas depuis la timeline',
    'arrangement.backTrack': 'Revenir à l’arrangement',
    'arrangement.backTrackHint': 'La piste {name} joue depuis la grille : rends-la à sa timeline.',
    'arrangement.clipDefault': 'Clip',
  },
)
