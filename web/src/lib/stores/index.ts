/**
 * The DAW's state, split as § 2.6 of the MVP breakdown says: the project store, the engine store,
 * the page's own state (language, view) and the dispatcher, the only writer of the first two. The
 * shell creates them once and hands them to the views through Svelte's context.
 */

import { getContext, setContext } from 'svelte'
import type { Command } from '../generated/protocol'
import type { MainViewId, PanelViewId } from '../../views/registry'
import { Dispatcher } from './dispatcher'
import { EngineStore } from './engine.svelte'
import { ProjectStore } from './project.svelte'
import { UiStore } from './ui.svelte'

export { Dispatcher } from './dispatcher'
export { EngineStore, type HelperState } from './engine.svelte'
export { ProjectStore } from './project.svelte'
export { UiStore } from './ui.svelte'

export interface Stores {
  project: ProjectStore
  engine: EngineStore
  ui: UiStore
  dispatcher: Dispatcher
}

export function createStores(view: MainViewId, panel: PanelViewId): Stores {
  const project = new ProjectStore()
  const engine = new EngineStore()
  const dispatcher = new Dispatcher(project, engine)
  // Dev server only: lets the browser checks send a command through the one entry point
  // (`Dispatcher.run`) while a view for it (the transport, TR-4) does not exist yet, and read
  // the project back (the ids of what they made, for the next command).
  if (import.meta.env.DEV && typeof window !== 'undefined') {
    Object.assign(window, {
      __faulkRun: (command: Command) => dispatcher.run(command),
      __faulkView: () => project.view,
      // The folder store of SV-2 does not exist yet: a check hands the folder's files over (bytes as numbers).
      __faulkOpen: (name: string, files: { path: string; bytes?: number[]; size?: number }[]) =>
        dispatcher.openProject(
          name,
          files.map((f) => (f.bytes ? { path: f.path, bytes: Uint8Array.from(f.bytes) } : { path: f.path, size: f.size ?? 0 })),
        ),
    })
  }
  return { project, engine, ui: new UiStore(view, panel), dispatcher }
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
