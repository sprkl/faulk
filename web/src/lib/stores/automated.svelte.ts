import type { AutomationLane, ProjectEvent } from '../generated/protocol'
import { pointsOf, targetKey, valueAt, lowerBound } from '../../views/arrangement/automation/model'
import type { EngineStore } from './engine.svelte'
import type { ProjectStore } from './project.svelte'

/** The span of the transport positions where a manual change holds the lane: `[from, until)`. */
interface Hold {
  from: number
  until: number
}

/**
 * The value each automated parameter has at the play position (AM-4): while the transport runs, the controls that a lane
 * drives (the knobs of the device panel, the fader and the pan of the mixer) show it instead of the value the project
 * holds, so that they move with the music. The engine does not report parameter values; the page reads the lanes at the
 * play position the engine reports, with the rule of the engine (`valueAt`, AM-2). Stopped, the controls show the project's value.
 *
 * A change of the parameter from anywhere (a knob, an undo, an agent) holds the lane in the engine until the play position
 * reaches the lane's next point (AM-2): the control then shows the project's value over that span, as the sound has it.
 */
export class AutomatedValues {
  /** Where the lanes are read: the engine's play position while the transport runs, else `null`. */
  readonly tick = $derived.by<number | null>(() => {
    const report = this.engine.started ? this.engine.feedback : null
    return report && (report.transport === 'playing' || report.transport === 'recording') ? report.playheadTick : null
  })

  // The maps are replaced, never changed in place: `$state.raw` and `$derived` are the reactivity, not the map (hence no `SvelteMap`).
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  private holds = $state.raw(new Map<string, Hold>())

  /** The value of every automated parameter at the play position, by `deviceKey` / `mixerKey`; empty when the transport is not running. */
  readonly values = $derived.by<ReadonlyMap<string, number>>(() => {
    const tick = this.tick
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const out = new Map<string, number>()
    const view = this.project.view
    if (tick === null || !view) return out
    for (const [trackId, track] of Object.entries(view.project.tracks)) {
      if (!track) continue
      for (const lane of Object.values(track.automation)) {
        if (!lane) continue
        const key = targetKey(lane.target)
        const where = 'device' in lane.target ? key : `m:${trackId}:${lane.target.mixer.param}`
        const hold = this.holds.get(where)
        if (hold && tick >= hold.from && tick < hold.until) continue
        const value = valueAt(pointsOf(lane), tick, lane.target)
        if (value !== null) out.set(where, value)
      }
    }
    return out
  })

  constructor(
    private readonly project: ProjectStore,
    private readonly engine: EngineStore,
  ) {
    project.watch = (events) => this.observe(events)
  }

  /** The value a device parameter is automated to now, or `undefined` when no running lane drives it. */
  device(device: string, param: string): number | undefined {
    return this.values.get(`d:${device}:${param}`)
  }

  /** The value a track's volume (dB) or pan is automated to now. */
  mixer(track: string, param: 'gain' | 'pan'): number | undefined {
    return this.values.get(`m:${track}:${param}`)
  }

  /** Holds the lanes of a parameter from the play position to their next point. */
  private hold(where: string, lanes: Iterable<AutomationLane>) {
    const tick = this.tick
    if (tick === null) return
    let until = Number.NEGATIVE_INFINITY
    for (const lane of lanes) {
      const points = pointsOf(lane)
      const i = lowerBound(points, Math.floor(tick) + 1)
      // After the last point the engine holds the value until the next jump.
      until = Math.max(until, points[i] ? points[i]!.tick : Number.POSITIVE_INFINITY)
    }
    if (until === Number.NEGATIVE_INFINITY) return
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const holds = new Map(this.holds)
    holds.set(where, { from: tick, until })
    this.holds = holds
  }

  /** Called with the events of every applied command, before the project mirror takes them. */
  private observe(events: readonly ProjectEvent[]) {
    if (this.tick === null) return
    const view = this.project.view
    if (!view) return
    for (const event of events) {
      if (event.type === 'device_param_changed') {
        const lanes: AutomationLane[] = []
        for (const track of Object.values(view.project.tracks)) {
          for (const lane of Object.values(track?.automation ?? {})) {
            if (lane && 'device' in lane.target && lane.target.device.device === event.device && lane.target.device.param === event.param) lanes.push(lane)
          }
        }
        if (lanes.length > 0) this.hold(`d:${event.device}:${event.param}`, lanes)
      } else if (event.type === 'mixer_changed' && event.track) {
        const track = view.project.tracks[event.track]
        if (!track) continue
        for (const param of ['gain', 'pan'] as const) {
          const changed = param === 'gain' ? (track.mixer.gain_db ?? null) !== (event.gain_db ?? null) : track.mixer.pan !== event.pan
          if (!changed) continue
          const lanes: AutomationLane[] = []
          for (const lane of Object.values(track.automation)) {
            if (lane && 'mixer' in lane.target && lane.target.mixer.param === param) lanes.push(lane)
          }
          if (lanes.length > 0) this.hold(`m:${event.track}:${param}`, lanes)
        }
      }
    }
  }
}
