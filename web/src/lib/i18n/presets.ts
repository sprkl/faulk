/**
 * The presets of a device (#191), in the header of its panel: the preset it shows, the previous and
 * next ones, whether its settings still match, and saving, renaming, copying and deleting. A
 * factory preset's name is not here: like a device name it is one English name, never translated.
 */

import { defineFeature } from './feature'

export const presets = defineFeature(
  {
    'presets.none': 'No preset',
    'presets.previous': 'Previous preset of {name}',
    'presets.next': 'Next preset of {name}',
    'presets.modified': 'Modified: the settings differ from the preset',
    'presets.menu': 'Presets of {name}',
    'presets.saveProject': 'Save as a preset of this project…',
    'presets.saveLibrary': 'Save in my library…',
    'presets.copyLibrary': 'Copy to my library',
    'presets.rename': 'Rename this preset…',
    'presets.delete': 'Delete this preset',
    'presets.name': 'Name of the preset',
    'presets.save': 'Save',
    'presets.renameAction': 'Rename',
    'presets.cancel': 'Cancel',
    'presets.where.factory': 'Built into Faulk',
    'presets.where.project': 'Saved in this project',
    'presets.where.library': 'In my library',
    'presets.writeFailed': 'A preset could not be written to your library. Faulk keeps it in this tab and tries again.',
  },
  {
    'presets.none': 'Aucun preset',
    'presets.previous': 'Preset précédent de {name}',
    'presets.next': 'Preset suivant de {name}',
    'presets.modified': 'Modifié : les réglages diffèrent du preset',
    'presets.menu': 'Presets de {name}',
    'presets.saveProject': 'Enregistrer comme preset de ce projet…',
    'presets.saveLibrary': 'Enregistrer dans ma bibliothèque…',
    'presets.copyLibrary': 'Copier dans ma bibliothèque',
    'presets.rename': 'Renommer ce preset…',
    'presets.delete': 'Supprimer ce preset',
    'presets.name': 'Nom du preset',
    'presets.save': 'Enregistrer',
    'presets.renameAction': 'Renommer',
    'presets.cancel': 'Annuler',
    'presets.where.factory': 'Fourni avec Faulk',
    'presets.where.project': 'Enregistré dans ce projet',
    'presets.where.library': 'Dans ma bibliothèque',
    'presets.writeFailed': "Un preset n'a pas pu être écrit dans ta bibliothèque. Faulk le garde dans cet onglet et réessaie.",
  },
)
