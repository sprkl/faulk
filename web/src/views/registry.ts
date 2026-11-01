/**
 * The DAW's views, one line each. A view is a folder under `src/views/` with its root component;
 * adding one touches this line and nothing else. The shell (`src/views/shell/`) lists the views of
 * each place in this order and shows the current one: `main` views in the main area, switched from
 * the top bar; `panel` views in the bottom panel, switched from its own tabs. Keep the list sorted
 * by id.
 */

import type { Component } from 'svelte'
import { storage } from '../lib/stores/storage'
import { VIEW_KEY } from '../lib/stores/ui.svelte'
import type { MessageKey } from '../lib/i18n/index'
import ArrangementView from './arrangement/ArrangementView.svelte'
import DevicesView from './devices/DevicesView.svelte'
import GridView from './grid/GridView.svelte'
import HistoryView from './history/HistoryView.svelte'
import MixerView from './mixer/MixerView.svelte'
import PianoRollView from './piano-roll/PianoRollView.svelte'
import PlayView from './play/PlayView.svelte'

export type ViewPlace = 'main' | 'panel'

export interface ViewDef {
  /** Stable identifier, unique across views. */
  id: string
  /** Its name in the switcher of its place. */
  label: MessageKey
  /** Where the shell shows it. */
  place: ViewPlace
  component: Component
}

export const views = [
  { id: 'arrangement', label: 'views.arrangement', place: 'main', component: ArrangementView },
  { id: 'devices', label: 'views.devices', place: 'panel', component: DevicesView },
  { id: 'grid', label: 'views.grid', place: 'main', component: GridView },
  { id: 'history', label: 'views.history', place: 'panel', component: HistoryView },
  { id: 'mixer', label: 'views.mixer', place: 'main', component: MixerView },
  { id: 'piano-roll', label: 'views.pianoRoll', place: 'panel', component: PianoRollView },
  { id: 'play', label: 'views.play', place: 'main', component: PlayView },
] as const satisfies readonly ViewDef[]

export type ViewId = (typeof views)[number]['id']
export type MainViewId = Extract<(typeof views)[number], { place: 'main' }>['id']
export type PanelViewId = Extract<(typeof views)[number], { place: 'panel' }>['id']

/** The views of one place, in registry order. */
export function viewsIn<P extends ViewPlace>(place: P) {
  return views.filter((view): view is Extract<(typeof views)[number], { place: P }> => view.place === place)
}

/** The view shown when none was chosen before. */
export const DEFAULT_VIEW: MainViewId = 'arrangement'
export const DEFAULT_PANEL: PanelViewId = 'devices'

export function viewById(id: string): ViewDef | undefined {
  return views.find((view) => view.id === id)
}

/** The main view the user chose last time, when it still exists; else the default view. */
export function startView(): MainViewId {
  let id: string | null = null
  try {
    id = storage()?.getItem(VIEW_KEY) ?? null
  } catch {
    // Storage refused: the default.
  }
  return viewsIn('main').find((view) => view.id === id)?.id ?? DEFAULT_VIEW
}
