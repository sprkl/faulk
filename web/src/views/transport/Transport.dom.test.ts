// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Command, ProjectEvent } from '../../lib/generated/protocol'
import { storesContext, type Stores } from '../../lib/stores'
import { EngineStore } from '../../lib/stores/engine.svelte'
import { ProjectStore } from '../../lib/stores/project.svelte'
import { Recorder, SETTLE_MS } from '../../lib/stores/recorder.svelte'
import { UiStore } from '../../lib/stores/ui.svelte'
import Transport from './Transport.svelte'

// The real dispatcher pulls in the wasm modules of the engine and the project, which the transport bar never touches.
vi.mock('../../lib/stores/dispatcher', () => ({ Dispatcher: class {} }))

/**
 * What the bar says about the take that ended (spec of MIDI recording, § 9: a lost note is shown, never hidden): the
 * notes the take lost, a take dropped because the engine changed, a take the project refused. The recorder is the real one,
 * driven the way the dispatcher drives it; the bar reads it and nothing else.
 */
let host: HTMLDivElement
let app: ReturnType<typeof mount> | null
let ui: UiStore
let recorder: Recorder
let ran: Command[]
let applies: boolean
let clock = 0

const TARGET = { at: 'arrangement' } as const
const changed = (state: 'recording' | 'stopped' | 'committed' | 'discarded'): ProjectEvent => ({ type: 'recording_changed', take: 1, state })
const notice = () => host.querySelector<HTMLElement>('[data-testid="transport-record-notice"]')
const target = () => host.querySelector<HTMLElement>('[data-testid="transport-record-target"]')

function mountBar() {
  const stores = { project: new ProjectStore(), engine: new EngineStore(), ui, dispatcher: { run: vi.fn() }, recorder } as unknown as Stores
  app = mount(Transport, { target: host, context: storesContext(stores) })
  flushSync()
}

const startTake = () => {
  recorder.ran({ type: 'record.start', target: TARGET }, { take: 1, target: TARGET, tracks: ['a'], shift_ms: 0, count_in: false }, [changed('recording')])
  flushSync()
}

beforeEach(() => {
  ran = []
  applies = true
  clock = 0
  ui = new UiStore('arrangement', 'devices')
  ui.lang = 'en'
  recorder = new Recorder({
    run: (command) => {
      ran.push(command)
      return applies || command.type !== 'record.commit_take'
    },
    mode: () => 'web',
    slots: () => ({ epoch: 1, tracks: { a: 0 }, scenes: {}, devices: {} }),
    now: () => clock,
  })
  host = document.createElement('div')
  document.body.append(host)
})

afterEach(async () => {
  if (app) await unmount(app)
  app = null
  host.remove()
})

describe('the transport bar after a take', () => {
  it('says nothing when no take was made', () => {
    mountBar()
    expect(notice()).toBeNull()
  })

  it('shows the notes a take lost, in both languages, and clears them when the next take starts', () => {
    mountBar()
    startTake()
    // The project could not place 3 notes of the take (before tick 0, before their clip): `record.commit_take` reports them.
    recorder.ran({ type: 'record.commit_take', take: 1, target: TARGET, tracks: [] }, { clips: [], dropped: 3 }, [changed('committed')])
    flushSync()
    expect(recorder.phase).toBe('idle')
    expect(notice()?.textContent?.trim()).toBe('Notes lost from the take: 3')
    expect(notice()?.getAttribute('role')).toBe('status')
    ui.lang = 'fr'
    flushSync()
    expect(notice()?.textContent?.trim()).toBe('Notes perdues dans la prise : 3')
    // The count of a take that lost nothing is not shown, and a take in progress hides the last one's alert.
    startTake()
    expect(notice()).toBeNull()
    expect(target()).not.toBeNull()
  })

  it('shows a take dropped because the engine changed, then clears it with the next take', () => {
    mountBar()
    startTake()
    recorder.engineReplaced()
    flushSync()
    expect(notice()?.textContent?.trim()).toBe('Take dropped: the engine changed')
    ui.lang = 'fr'
    flushSync()
    expect(notice()?.textContent?.trim()).toBe('Prise abandonnée : le moteur a changé')
    startTake()
    expect(notice()).toBeNull()
  })

  it('shows a take dropped because the project refused it', () => {
    applies = false
    mountBar()
    startTake()
    recorder.feedback({
      seq: 2,
      epoch: 1,
      recordTake: 1,
      recordCapturing: true,
      recordTotal: 1,
      recordDropped: 0,
      liveOverflows: 0,
      recorded: [{ index: 0, track: 0, start: 0, length: 240, pitch: 60, velocity: 100 }],
      clips: [],
    } as never)
    recorder.ran({ type: 'record.stop' }, { take: 1 }, [changed('stopped')])
    clock = SETTLE_MS
    recorder.feedback({
      seq: 3,
      epoch: 1,
      recordTake: 1,
      recordCapturing: false,
      recordTotal: 1,
      recordDropped: 0,
      liveOverflows: 0,
      recorded: [],
      clips: [],
    } as never)
    flushSync()
    expect(ran.map((c) => c.type)).toEqual(['record.commit_take', 'record.discard_take'])
    expect(notice()?.textContent?.trim()).toBe('Take dropped: the project refused it')
    ui.lang = 'fr'
    flushSync()
    expect(notice()?.textContent?.trim()).toBe('Prise abandonnée : le projet l’a refusée')
  })

  it('shows the alert in place of the target and gives it a title, so it is read and not only seen', () => {
    mountBar()
    startTake()
    recorder.engineReplaced()
    flushSync()
    const alert = notice()!
    expect(alert.title).toBe(alert.textContent?.trim())
    expect(target()).toBeNull()
  })
})
