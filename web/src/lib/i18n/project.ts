/** Opening a project folder (`Dispatcher.openProject`): why a folder did not open, and the mode it opened in. */

import { defineFeature } from './feature'

export const project = defineFeature(
  {
    'project.open.newer': 'This project was saved by a newer version of Faulk. Reload the page to update, then open it again.',
    'project.open.notAProject': 'This folder is not a Faulk project.',
    'project.open.unreadable': 'This project could not be read from storage. Your current project is unchanged.',
    'project.open.invalid': 'This project cannot be opened: its files are damaged or incomplete. Your current project is unchanged.',
    'project.open.webOnly': 'Project opened in Web mode: the helper will play projects in a later version.',

    'project.menu': 'Project',
    'project.menu.new': 'New project',
    'project.menu.open': 'Open a folder…',
    'project.menu.save': 'Save',
    'project.menu.saveAs': 'Save as…',
    'project.menu.recent': 'Recent projects',
    'project.menu.noRecent': 'No recent project yet.',
    'project.menu.unsupported':
      'This browser cannot save to a folder. Your project stays in this tab, and Faulk warns you before you leave it. To keep it on your computer, use Chrome or Edge.',
    'project.untitled': 'Untitled',

    'save.unsaved': 'Not saved',
    'save.pending': 'Unsaved changes',
    'save.saving': 'Saving…',
    'save.saved': 'Saved at {time}',
    'save.savedPlain': 'Saved',
    'save.paused': 'Saving paused',
    'save.failed': 'Could not save',
    'save.conflict': 'Folder changed',
    'save.folder': 'Saved in the folder “{name}”',
    'save.noFolder': 'Not in a folder yet: Save as chooses one',

    'save.banner.permission':
      'Faulk can no longer write in “{name}”. Your changes are kept here. Allow access to carry on saving.',
    'save.banner.allow': 'Allow',
    'save.banner.error': 'Faulk could not write in “{name}”. Your changes are kept here and it will try again.',
    'save.banner.retry': 'Try again',
    'save.banner.external': '“{name}” was changed outside Faulk while you had unsaved changes. Which version do you keep?',
    'save.banner.keepMine': 'Keep mine',
    'save.banner.loadDisk': 'Load the folder’s version',
    'save.banner.reopen': 'Reopen “{name}”, the project you were working on?',
    'save.banner.reopenButton': 'Reopen',
    'save.banner.dismiss': 'Dismiss',

    'storage.unsupported':
      'This browser cannot save to a folder. Your project stays in this tab. Use Chrome or Edge to keep it on your computer.',
    'storage.refused': 'The browser did not allow that folder. Choose another one, for example a new folder inside Documents.',
    'storage.notEmpty': 'That folder is not empty. Choose an empty folder: Faulk never writes among other files.',
    'storage.locked': 'This project is open in another tab. Close it there first, or use that tab.',
    'storage.saveFailed': 'The project could not be written to that folder. It was not saved.',
    'storage.permissionDenied': 'Without access to the folder, Faulk cannot open or save the project there.',
    'storage.reloaded': 'The project was changed outside Faulk and has been reloaded.',

    'confirm.new.title': 'Start a new project?',
    'confirm.open.title': 'Open another project?',
    'confirm.body': 'The current project has changes that are not saved. If you go on, they are lost.',
    'confirm.bodyFolder': 'Faulk could not save the latest changes to “{name}”. If you go on, they are lost.',
    'confirm.discard': 'Discard the changes',
    'confirm.cancel': 'Cancel',
  },
  {
    'project.open.newer': 'Ce projet a été enregistré par une version plus récente de Faulk. Recharge la page pour la mettre à jour, puis ouvre-le de nouveau.',
    'project.open.notAProject': 'Ce dossier n’est pas un projet Faulk.',
    'project.open.unreadable': 'Ce projet n’a pas pu être lu depuis le stockage. Ton projet en cours n’est pas modifié.',
    'project.open.invalid': 'Ce projet ne peut pas être ouvert : ses fichiers sont abîmés ou incomplets. Ton projet en cours n’est pas modifié.',
    'project.open.webOnly': 'Projet ouvert en mode Web : le helper jouera les projets dans une prochaine version.',

    'project.menu': 'Projet',
    'project.menu.new': 'Nouveau projet',
    'project.menu.open': 'Ouvrir un dossier…',
    'project.menu.save': 'Enregistrer',
    'project.menu.saveAs': 'Enregistrer sous…',
    'project.menu.recent': 'Projets récents',
    'project.menu.noRecent': 'Aucun projet récent pour l’instant.',
    'project.menu.unsupported':
      'Ce navigateur ne peut pas enregistrer dans un dossier. Ton projet reste dans cet onglet, et Faulk te prévient avant que tu le quittes. Pour le garder sur ton ordinateur, utilise Chrome ou Edge.',
    'project.untitled': 'Sans titre',

    'save.unsaved': 'Non enregistré',
    'save.pending': 'Modifications non enregistrées',
    'save.saving': 'Enregistrement…',
    'save.saved': 'Enregistré à {time}',
    'save.savedPlain': 'Enregistré',
    'save.paused': 'Enregistrement en pause',
    'save.failed': 'Échec de l’enregistrement',
    'save.conflict': 'Dossier modifié',
    'save.folder': 'Enregistré dans le dossier « {name} »',
    'save.noFolder': 'Pas encore dans un dossier : « Enregistrer sous » en choisit un',

    'save.banner.permission':
      'Faulk ne peut plus écrire dans « {name} ». Tes modifications sont gardées ici. Autorise l’accès pour continuer à enregistrer.',
    'save.banner.allow': 'Autoriser',
    'save.banner.error': 'Faulk n’a pas pu écrire dans « {name} ». Tes modifications sont gardées ici et il va réessayer.',
    'save.banner.retry': 'Réessayer',
    'save.banner.external': '« {name} » a été modifié hors de Faulk alors que tu avais des modifications non enregistrées. Quelle version gardes-tu ?',
    'save.banner.keepMine': 'Garder la mienne',
    'save.banner.loadDisk': 'Charger celle du dossier',
    'save.banner.reopen': 'Rouvrir « {name} », le projet sur lequel tu travaillais ?',
    'save.banner.reopenButton': 'Rouvrir',
    'save.banner.dismiss': 'Fermer',

    'storage.unsupported':
      'Ce navigateur ne peut pas enregistrer dans un dossier. Ton projet reste dans cet onglet. Utilise Chrome ou Edge pour le garder sur ton ordinateur.',
    'storage.refused': 'Le navigateur n’a pas autorisé ce dossier. Choisis-en un autre, par exemple un nouveau dossier dans Documents.',
    'storage.notEmpty': 'Ce dossier n’est pas vide. Choisis un dossier vide : Faulk n’écrit jamais au milieu d’autres fichiers.',
    'storage.locked': 'Ce projet est ouvert dans un autre onglet. Ferme-le là-bas d’abord, ou utilise cet onglet.',
    'storage.saveFailed': 'Le projet n’a pas pu être écrit dans ce dossier. Il n’est pas enregistré.',
    'storage.permissionDenied': 'Sans accès au dossier, Faulk ne peut pas y ouvrir ni y enregistrer le projet.',
    'storage.reloaded': 'Le projet a été modifié hors de Faulk et vient d’être rechargé.',

    'confirm.new.title': 'Commencer un nouveau projet ?',
    'confirm.open.title': 'Ouvrir un autre projet ?',
    'confirm.body': 'Le projet en cours a des modifications non enregistrées. Si tu continues, elles sont perdues.',
    'confirm.bodyFolder': 'Faulk n’a pas pu enregistrer les dernières modifications dans « {name} ». Si tu continues, elles sont perdues.',
    'confirm.discard': 'Abandonner les modifications',
    'confirm.cancel': 'Annuler',
  },
)
