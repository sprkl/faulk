/**
 * The DAW's views, one line each. A view is a folder under `src/views/` with its root component;
 * adding one touches this line and nothing else (the shell, `src/lib/app/App.svelte`, renders the
 * current one). Keep the list sorted by id.
 */

import type { Component } from 'svelte'
import PlayView from './play/PlayView.svelte'

export interface ViewDef {
  /** Stable identifier, unique across views. */
  id: string
  component: Component
}

export const views = [{ id: 'play', component: PlayView }] as const satisfies readonly ViewDef[]

export type ViewId = (typeof views)[number]['id']

/** The view shown when none is chosen. */
export const DEFAULT_VIEW: ViewId = 'play'

export function viewById(id: string): ViewDef | undefined {
  return views.find((view) => view.id === id)
}
