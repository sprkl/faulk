import type { Command } from './generated/protocol'

/** The commands of the project menu (SV-2): the page's storage runs them (`lib/storage`). */
export const newProject = (): Command => ({ type: 'project.new' })
export const save = (): Command => ({ type: 'project.save' })
export const saveAs = (): Command => ({ type: 'project.save_as' })

/**
 * Whether a command is about the project's folder rather than the engine: the page runs it even
 * when no engine plays (audio not started, or stopped after a refused stream).
 */
export const isSaveCommand = (command: Command): boolean => command.type === 'project.save' || command.type === 'project.save_as'
