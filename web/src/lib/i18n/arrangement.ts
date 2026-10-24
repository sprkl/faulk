/** The arrangement view (`src/views/arrangement/`, AR-4): the clips placed on the tracks along the timeline. */

import { defineFeature } from './feature'

export const arrangement = defineFeature(
  {
    'arrangement.empty': 'Add a track to start arranging.',
    'arrangement.grid': 'Arrangement timeline',
    'arrangement.tools': 'Edit the selection',
    'arrangement.split': 'Split',
    'arrangement.splitHint': 'Split the selected clips at the play position (Ctrl+E).',
    'arrangement.duplicate': 'Duplicate',
    'arrangement.duplicateHint': 'Copy the selected clips right after them, as independent clips (Ctrl+D). Alt+drag copies where you drop.',
    'arrangement.duplicateLinked': 'Duplicate linked',
    'arrangement.duplicateLinkedHint': 'Copy the selected clips right after them, as linked copies: editing one changes the others (Ctrl+Shift+D). Alt+Shift+drag copies where you drop.',
    'arrangement.delete': 'Delete',
    'arrangement.help': 'Drag a clip to move it, its edges to resize it. Hold Ctrl or Cmd to skip snapping. Drag on the ruler to loop a part, click on it to move the play position. Double-click an empty lane to create a clip, a clip to edit its notes.',
    'arrangement.count': '{n} clips',
    'arrangement.selected': '{n} selected',
    'arrangement.lane': 'Track {name}',
    'arrangement.clipDefault': 'Clip',
  },
  {
    'arrangement.empty': 'Ajoute une piste pour commencer à arranger.',
    'arrangement.grid': 'Timeline d’arrangement',
    'arrangement.tools': 'Modifier la sélection',
    'arrangement.split': 'Couper',
    'arrangement.splitHint': 'Coupe les clips sélectionnés à la position de lecture (Ctrl+E).',
    'arrangement.duplicate': 'Dupliquer',
    'arrangement.duplicateHint': 'Copie les clips sélectionnés juste après eux, en clips indépendants (Ctrl+D). Alt+glisser copie là où tu déposes.',
    'arrangement.duplicateLinked': 'Dupliquer en lié',
    'arrangement.duplicateLinkedHint': 'Copie les clips sélectionnés juste après eux, en copies liées : modifier l’une change les autres (Ctrl+Maj+D). Alt+Maj+glisser copie là où tu déposes.',
    'arrangement.delete': 'Supprimer',
    'arrangement.help': 'Glisse un clip pour le déplacer, ses bords pour le redimensionner. Maintiens Ctrl ou Cmd pour ignorer le magnétisme. Glisse sur la règle pour boucler une partie, clique dessus pour déplacer la position de lecture. Double-clique une piste vide pour créer un clip, un clip pour modifier ses notes.',
    'arrangement.count': '{n} clips',
    'arrangement.selected': '{n} sélectionnés',
    'arrangement.lane': 'Piste {name}',
    'arrangement.clipDefault': 'Clip',
  },
)
