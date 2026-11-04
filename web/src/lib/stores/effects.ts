import type { Command } from '../generated/protocol'
import type { ExportRequest } from '../project/core'

/**
 * What the page itself does for a command the project applied, beside the project and the
 * engine: the Web MIDI input it opens and the notes the piano strip shows as sounding. Neither
 * is in the project or in a `ProjectEvent` (a played note changes nothing in the project; the
 * Web MIDI input is the page's own device, the helper's input is the helper's), so this
 * table names the commands for them. The dispatcher knows none.
 */
export interface EffectPorts {
  /** Which engine plays: the page's input device is only the page's in Web mode. */
  mode: 'web' | 'native'
  /** The page's Web MIDI input, `null` before audio starts or where Web MIDI does not exist. */
  midi: { select(id: string | null): void; refresh(): void } | null
  /** The input chosen in Web mode, reopened when coming back from Boost. */
  rememberWebInput(id: string | null): void
  markNote(note: number, velocity: number): void
  clearNotes(): void
  /** `project.save`: write the project to its folder now (or ask for one). */
  save(): void
  /** `project.save_as`: the user names the project and picks a folder, and the project moves into a folder of that name inside it. */
  saveAs(): void
  /** `project.export_pack`: hand the project to the user as a `.faulkpack` download. */
  exportPack(): void
  /** `project.import_pack`: the user picks a `.faulkpack` and its project replaces the current one. */
  importPack(): void
  /** `project.open_url`: the page downloads the `.faulkpack` at `url` and opens it as a new project. */
  openUrl(url: string): void
  /** `project.reopen_last`: the project the banner offers (a folder, or a draft). */
  reopenLast(): void
  /** `project.open_folder`: the user picks a folder and it opens. */
  openFolder(): void
  /** `project.open_recent`: a folder of the recent list. */
  openRecent(id: string): void
  /** `project.open_browser_project`: a project kept in the browser. */
  openBrowserProject(dir: string): void
  /** `project.delete_browser_project`: delete a project kept in the browser, after the user's answer. */
  deleteBrowserProject(dir: string): void
  /** `project.keep_mine`: a conflict, the project's version replaces the folder's, after the user's answer. */
  keepMine(): void
  /** `project.load_folder_version`: a conflict, the folder's version replaces the project, after the user's answer. */
  loadFolderVersion(): void
  /** `project.allow_access`: ask the browser for the permission again, then write what waited. */
  allowAccess(): void
  /** `project.export_audio`: the page renders the song to a WAV file (the dialog opens with these settings). */
  exportAudio(request: ExportRequest): void
  /** `clip.choose_audio`: the user picks audio files, and each is imported into the track (AU-1). */
  chooseAudio(track: string): void
  /** `clip.cancel_audio_import`: the import under way stops. */
  cancelAudioImport(): void
  /** `transport.stop`, `transport.seek`, `transport.pause`: the playhead is cued, even to where it already stands (the engine frees every held gesture of a lane, #394). */
  cued(): void
}

export function applyEffects(command: Command, ports: EffectPorts) {
  switch (command.type) {
    case 'midi.select_input':
      if (ports.mode !== 'web') return
      ports.rememberWebInput(command.id ?? null)
      ports.midi?.select(command.id ?? null)
      return
    case 'midi.list_inputs':
      if (ports.mode === 'web') ports.midi?.refresh()
      return
    case 'live.note_on':
    case 'live.input_note_on':
      ports.markNote(command.note, command.velocity)
      return
    case 'live.note_off':
    case 'live.input_note_off':
      ports.markNote(command.note, 0)
      return
    case 'live.all_notes_off':
    case 'live.all_sound_off':
      ports.clearNotes()
      return
    case 'transport.stop':
    case 'transport.seek':
    case 'transport.pause':
      ports.cued()
      return
    case 'project.save':
      ports.save()
      return
    case 'project.save_as':
      ports.saveAs()
      return
    case 'project.export_pack':
      ports.exportPack()
      return
    case 'project.import_pack':
      ports.importPack()
      return
    case 'project.open_url':
      ports.openUrl(command.url)
      return
    case 'project.reopen_last':
      ports.reopenLast()
      return
    case 'project.open_folder':
      ports.openFolder()
      return
    case 'project.open_recent':
      ports.openRecent(command.id)
      return
    case 'project.open_browser_project':
      ports.openBrowserProject(command.dir)
      return
    case 'project.delete_browser_project':
      ports.deleteBrowserProject(command.dir)
      return
    case 'project.keep_mine':
      ports.keepMine()
      return
    case 'project.load_folder_version':
      ports.loadFolderVersion()
      return
    case 'project.allow_access':
      ports.allowAccess()
      return
    case 'clip.choose_audio':
      ports.chooseAudio(command.track)
      return
    case 'clip.cancel_audio_import':
      ports.cancelAudioImport()
      return
    case 'project.export_audio':
      ports.exportAudio({
        region: command.region ?? undefined,
        bit_depth: command.bit_depth ?? undefined,
        sample_rate: command.sample_rate ?? undefined,
        from_bar: command.from_bar ?? undefined,
        to_bar: command.to_bar ?? undefined,
      })
      return
  }
}
