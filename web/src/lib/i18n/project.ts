/** Opening a project folder (`Dispatcher.openProject`): why a folder did not open, and the mode it opened in. */

import { defineFeature } from './feature'

export const project = defineFeature(
  {
    'project.open.newer': 'This project was saved by a newer version of Faulk. Reload the page to update, then open it again.',
    'project.open.notAProject': 'This folder is not a Faulk project.',
    'project.open.unreadable': 'This project could not be read from storage. Your current project is unchanged.',
    'project.open.invalid': 'This project cannot be opened: its files are damaged or incomplete. Your current project is unchanged.',
    'project.open.webOnly': 'Project opened in Web mode: the helper will play projects in a later version.',
  },
  {
    'project.open.newer': 'Ce projet a été enregistré par une version plus récente de Faulk. Recharge la page pour la mettre à jour, puis ouvre-le de nouveau.',
    'project.open.notAProject': 'Ce dossier n’est pas un projet Faulk.',
    'project.open.unreadable': 'Ce projet n’a pas pu être lu depuis le stockage. Ton projet en cours n’est pas modifié.',
    'project.open.invalid': 'Ce projet ne peut pas être ouvert : ses fichiers sont abîmés ou incomplets. Ton projet en cours n’est pas modifié.',
    'project.open.webOnly': 'Projet ouvert en mode Web : le helper jouera les projets dans une prochaine version.',
  },
)
