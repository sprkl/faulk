/**
 * The DAW's state, split as § 2.6 of the MVP breakdown says: the project store, the engine store,
 * the page's own state (language, view) and the dispatcher, the only writer of the first two. The
 * shell creates them once and hands them to the views through Svelte's context.
 */

import { getContext, setContext } from 'svelte'
import type { Command } from '../generated/protocol'
import type { MainViewId, PanelViewId } from '../../views/registry'
import { AudioExport, type ExportDeps } from '../export/audio-export.svelte'
import { browserExportDeps } from '../export/browser'
import { browserDeps } from '../storage/browser'
import { ProjectStorage, type StorageDeps } from '../storage/storage.svelte'
import { AutomatedValues } from './automated.svelte'
import { Dispatcher } from './dispatcher'
import { EngineStore } from './engine.svelte'
import { KeyboardPlayer } from './keyboard-player.svelte'
import { MeterStore } from './meters.svelte'
import { ProjectStore } from './project.svelte'
import { Recorder } from './recorder.svelte'
import { UiStore } from './ui.svelte'

export { Dispatcher } from './dispatcher'
export { EngineStore, type HelperState } from './engine.svelte'
export { AutomatedValues } from './automated.svelte'
export { KeyboardPlayer } from './keyboard-player.svelte'
export { MASTER_METER, MeterStore } from './meters.svelte'
export { ProjectStore } from './project.svelte'
export { Recorder } from './recorder.svelte'
export { ProjectStorage } from '../storage/storage.svelte'
export { UiStore } from './ui.svelte'

export interface Stores {
  project: ProjectStore
  engine: EngineStore
  ui: UiStore
  dispatcher: Dispatcher
  /** The project's folder: saved, saving or not, and the dialogs of opening and saving. */
  storage: ProjectStorage
  /** Exporting the song as a WAV file (#297): the dialog and the render. */
  audioExport: AudioExport
  /** The computer keyboard as a piano, in every view (#361). */
  keys: KeyboardPlayer
  /** What the lanes drive at the play position (AM-4): the controls show it while the transport runs. */
  automated: AutomatedValues
  /** The levels every meter of the page shows, and the latched CLIP of each. */
  meters: MeterStore
  /** MIDI recording (#470): the take in progress and its notes. */
  recorder: Recorder
}

/** `deps` is for tests: the browser seams of the project storage (`lib/storage`). */
export function createStores(view: MainViewId, panel: PanelViewId, deps: StorageDeps = browserDeps, exportDeps: ExportDeps = browserExportDeps): Stores {
  const project = new ProjectStore()
  const engine = new EngineStore()
  const dispatcher = new Dispatcher(project, engine)
  const storage = new ProjectStorage(dispatcher.host, deps)
  dispatcher.storage = storage
  const audioExport = new AudioExport(dispatcher.exportHost, exportDeps)
  dispatcher.audioExport = audioExport
  const recorder = new Recorder({
    run: (command) => dispatcher.run(command),
    slots: () => project.view?.slots ?? null,
    now: () => performance.now(),
  })
  dispatcher.recorder = recorder
  // Dev server only: lets the browser checks send a command through the one entry point
  // (`Dispatcher.run`) while a view for it (the transport, TR-4) does not exist yet, and read
  // the project back (the ids of what they made, for the next command).
  if (import.meta.env.DEV && typeof window !== 'undefined') {
    Object.assign(window, {
      // `author` runs the command as that author (`agent:<name>`), as the agents' relay will (AG-1).
      __faulkRun: (command: Command, author?: string) => dispatcher.run(command, { author }),
      __faulkView: () => project.view,
      // The history's entries (done, undone): a check counts that one gesture or key is one entry.
      __faulkHistory: () => dispatcher.history(),
      // The answer of `project.list_stored` (`storage-commands.spec.ts`).
      __faulkStored: () => dispatcher.listStored(),
      // What the last write of the project cost (`project-save.spec.ts`).
      __faulkWrite: () => storage.lastWrite,
      // The folder store of SV-2 does not exist yet: a check hands the folder's files over (bytes as numbers).
      __faulkOpen: (name: string, files: { path: string; bytes?: number[]; size?: number }[]) =>
        dispatcher.openProject(
          name,
          files.map((f) => (f.bytes ? { path: f.path, bytes: Uint8Array.from(f.bytes) } : { path: f.path, size: f.size ?? 0 })),
        ),
    })
  }
  const ui = new UiStore(view, panel)
  return {
    project,
    engine,
    ui,
    dispatcher,
    storage,
    audioExport,
    keys: new KeyboardPlayer(project, engine, ui, dispatcher),
    automated: new AutomatedValues(project, engine, ui),
    meters: new MeterStore(project, engine),
    recorder,
  }
}

const KEY = Symbol('faulk.stores')

/** Call from the shell's script, before its children render. */
export function provideStores(stores: Stores): Stores {
  return setContext(KEY, stores)
}

/** The context a component test passes to `render` (`svelte/server`) to render a view outside the shell. */
export const storesContext = (stores: Stores): Map<symbol, Stores> => new Map([[KEY, stores]])

/** Call from a view's script: the stores the shell provided. */
export function useStores(): Stores {
  const stores = getContext<Stores | undefined>(KEY)
  if (!stores) throw new Error('useStores: no stores in context; render the view inside the shell')
  return stores
}
