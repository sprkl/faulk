/** The question the page asks before an agent's call that cannot be undone runs (AG-4, `views/confirm/AgentConfirm.svelte`). */

import { defineFeature } from './feature'

export const agentConfirm = defineFeature(
  {
    'agentConfirm.eyebrow': 'Request from an agent',
    'agentConfirm.new.title': '{agent} wants to start a new project',
    'agentConfirm.open.title': '{agent} wants to open another project',
    'agentConfirm.open.titleNamed': '{agent} wants to open “{name}”',
    'agentConfirm.replace.body':
      'Faulk saves your current project first, so you can reopen it from the Project menu. Changes it cannot save are lost. This cannot be undone.',
    'agentConfirm.delete.title': '{agent} wants to delete a project from this browser',
    'agentConfirm.delete.titleNamed': '{agent} wants to delete “{name}”',
    'agentConfirm.delete.body': 'It is deleted from this browser for good. Export it first if you want to keep a copy.',
    'agentConfirm.keepMine.title': '{agent} wants to replace the folder’s version with yours',
    'agentConfirm.keepMine.body': '“{name}” was changed outside Faulk. If you allow it, your version is written into the folder and those changes are lost.',
    'agentConfirm.loadFolder.title': '{agent} wants to load the folder’s version',
    'agentConfirm.loadFolder.body': '“{name}” is reloaded from the folder. The changes made since the last save are lost.',
    'agentConfirm.command.title': '{agent} wants to run {command}',
    'agentConfirm.importAudio.title': '{agent} wants to download an audio file from {host}',
    'agentConfirm.importAudio.body':
      'File: {name}. Faulk downloads it from {host} (up to {max} MB) and adds it to the clips of “{track}”. Nothing is sent to {host} unless you allow it. You can undo the clip.',
    'agentConfirm.command.body': 'This cannot be undone.',
    'agentConfirm.allow': 'Allow',
    'agentConfirm.refuse': 'Refuse',
    'agentConfirm.countdown': 'Refused automatically in {seconds} s.',
    'agentConfirm.moreOne': '1 more request is waiting.',
    'agentConfirm.moreMany': '{count} more requests are waiting.',
  },
  {
    'agentConfirm.eyebrow': 'Demande d’un agent',
    'agentConfirm.new.title': '{agent} veut commencer un nouveau projet',
    'agentConfirm.open.title': '{agent} veut ouvrir un autre projet',
    'agentConfirm.open.titleNamed': '{agent} veut ouvrir « {name} »',
    'agentConfirm.replace.body':
      'Faulk enregistre d’abord ton projet en cours : tu pourras le rouvrir depuis le menu Projet. Les modifications qu’il ne peut pas enregistrer sont perdues. Cette action ne s’annule pas.',
    'agentConfirm.delete.title': '{agent} veut supprimer un projet de ce navigateur',
    'agentConfirm.delete.titleNamed': '{agent} veut supprimer « {name} »',
    'agentConfirm.delete.body': 'Il est supprimé de ce navigateur pour de bon. Exporte-le d’abord si tu veux en garder une copie.',
    'agentConfirm.keepMine.title': '{agent} veut remplacer la version du dossier par la tienne',
    'agentConfirm.keepMine.body':
      '« {name} » a été modifié hors de Faulk. Si tu autorises, ta version est écrite dans le dossier et ces modifications sont perdues.',
    'agentConfirm.loadFolder.title': '{agent} veut charger la version du dossier',
    'agentConfirm.loadFolder.body': '« {name} » est rechargé depuis le dossier. Les modifications faites depuis le dernier enregistrement sont perdues.',
    'agentConfirm.command.title': '{agent} veut lancer {command}',
    'agentConfirm.importAudio.title': '{agent} veut télécharger un fichier audio depuis {host}',
    'agentConfirm.importAudio.body':
      'Fichier : {name}. Faulk le télécharge depuis {host} (jusqu’à {max} Mo) et l’ajoute aux clips de « {track} ». Rien n’est envoyé à {host} sans ton accord. Tu pourras annuler le clip.',
    'agentConfirm.command.body': 'Cette action ne s’annule pas.',
    'agentConfirm.allow': 'Autoriser',
    'agentConfirm.refuse': 'Refuser',
    'agentConfirm.countdown': 'Refusé automatiquement dans {seconds} s.',
    'agentConfirm.moreOne': '1 autre demande attend.',
    'agentConfirm.moreMany': '{count} autres demandes attendent.',
  },
)
