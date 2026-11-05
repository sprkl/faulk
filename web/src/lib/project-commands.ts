import type { Command } from './generated/protocol'

/** The commands of the project menu (SV-2, SV-3): the page's storage runs them (`lib/storage`); the export (#297) renders in the page. */
export const newProject = (name?: string): Command => (name === undefined ? { type: 'project.new' } : { type: 'project.new', name })
/** Renames the project (undoable): the project menu's field and the name in the top bar (#627). */
export const renameProject = (name: string): Command => ({ type: 'project.rename', name })
export const save = (): Command => ({ type: 'project.save' })
export const saveAs = (): Command => ({ type: 'project.save_as' })
export const exportPack = (): Command => ({ type: 'project.export_pack' })
export const cancelExportPack = (): Command => ({ type: 'project.cancel_export_pack' })
export const importPack = (): Command => ({ type: 'project.import_pack' })
/** A `.faulkpack` hosted at a link (#497): the page downloads it and opens it as a new project. */
export const openUrl = (url: string): Command => ({ type: 'project.open_url', url })
/**
 * The commands for the projects the page keeps (#323): the banner of last time, the menu's lists, the
 * ways out of a conflict, the permission. The storage asks the user before it loses anything (a deleted
 * project, a version of the folder replaced), whoever sends them.
 */
export const listStored = (): Command => ({ type: 'project.list_stored' })
/** Whether a command asks for the list of the stored projects: its answer comes from the storage, which reads first. */
export const isListStored = (command: Command): boolean => command.type === 'project.list_stored'
/** Whether a command reads the live levels (`mixer.levels`, #665): the page stages the meters it held first. */
export const isMixerLevels = (command: Command): boolean => command.type === 'mixer.levels'
/** The range of a `project.analyze_render` (#665: the page renders it offline, then the command answers); `null` for any other command. */
export const analyzeRange = (command: Command): { from_bar?: number; to_bar?: number } | null =>
  command.type === 'project.analyze_render' ? { from_bar: command.from_bar ?? undefined, to_bar: command.to_bar ?? undefined } : null
/** Whether a command starts a new project: the storage asks first when changes would be lost, then runs it in the core. */
export const isNewProject = (command: Command): boolean => command.type === 'project.new'
/** The name a `project.new` gives the new project (an agent's), `undefined` when the page is to name it (#473). */
export const newProjectName = (command: Command): string | undefined => (command.type === 'project.new' ? (command.name ?? undefined) : undefined)
export const reopenLast = (): Command => ({ type: 'project.reopen_last' })
export const openFolder = (): Command => ({ type: 'project.open_folder' })
export const openRecent = (id: string): Command => ({ type: 'project.open_recent', id })
export const openBrowserProject = (dir: string): Command => ({ type: 'project.open_browser_project', dir })
export const deleteBrowserProject = (dir: string): Command => ({ type: 'project.delete_browser_project', dir })
export const keepMine = (): Command => ({ type: 'project.keep_mine' })
export const loadFolderVersion = (): Command => ({ type: 'project.load_folder_version' })
export const allowAccess = (): Command => ({ type: 'project.allow_access' })
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
  command.type === 'project.cancel_export_pack' ||
  command.type === 'project.import_pack' ||
  command.type === 'project.export_audio' ||
  isStoredProjectCommand(command) ||
  command.type === 'project.keep_mine' ||
  command.type === 'project.load_folder_version' ||
  command.type === 'project.allow_access'

/**
 * Whether a command is about the projects the page keeps and can be sent before the page has a project
 * core: the banner that offers last time's project shows on a fresh page, before audio starts.
 */
export const isStoredProjectCommand = (command: Command): boolean =>
  command.type === 'project.list_stored' ||
  command.type === 'project.reopen_last' ||
  command.type === 'project.open_folder' ||
  command.type === 'project.open_recent' ||
  command.type === 'project.open_browser_project' ||
  command.type === 'project.delete_browser_project' ||
  command.type === 'project.open_url'

/**
 * Whether a command concerns the page's storage and never the engine: the helper does not take it (it
 * keeps no project folder of its own yet), so in Boost it stays in the page.
 */
export const isStorageCommand = (command: Command): boolean => isSaveCommand(command)
