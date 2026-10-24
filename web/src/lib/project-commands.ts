import type { Command } from './generated/protocol'

/** The commands of the project menu (SV-2, SV-3): the page's storage runs them (`lib/storage`); the export (#297) renders in the page. */
export const newProject = (): Command => ({ type: 'project.new' })
export const save = (): Command => ({ type: 'project.save' })
export const saveAs = (): Command => ({ type: 'project.save_as' })
export const exportPack = (): Command => ({ type: 'project.export_pack' })
export const importPack = (): Command => ({ type: 'project.import_pack' })
/** The export dialog's settings, as the command: the project checks them, the page renders. */
export const exportAudio = (fields: Omit<Extract<Command, { type: 'project.export_audio' }>, 'type'> = {}): Command => ({
  type: 'project.export_audio',
  ...fields,
})

/**
 * Whether a command is about the project's folder rather than the engine: the page runs it even
 * when no engine plays (audio not started, or stopped after a refused stream).
 */
export const isSaveCommand = (command: Command): boolean =>
  command.type === 'project.save' ||
  command.type === 'project.save_as' ||
  command.type === 'project.export_pack' ||
  command.type === 'project.import_pack' ||
  command.type === 'project.export_audio'

/**
 * Whether a command concerns the page's storage and never the engine: the helper does not take it (it
 * keeps no project folder of its own yet), so in Boost it stays in the page.
 */
export const isStorageCommand = (command: Command): boolean => isSaveCommand(command) || command.type === 'project.new'
