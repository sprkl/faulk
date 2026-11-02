import { untrack } from 'svelte'
import type { AutomationLane, Project, ProjectEvent, Track } from '../generated/protocol'
import { deviceText, labelKey } from '../devices'
import { lanesOf, pointsOf, targetKey, valueAt, lowerBound, type Labels } from '../../views/arrangement/automation/model'
import { PPQ } from '../../views/transport/time'
import { MIN_GAIN_DB } from '../project/tracks'
import type { EngineStore } from './engine.svelte'
import type { ProjectStore } from './project.svelte'
import type { UiStore } from './ui.svelte'

/** The key of the parameter a lane drives, as `values` and `drivers` have it: `d:<device>:<param>` or `m:<track>:<gain|pan>`. */
function whereOf(trackId: string, lane: AutomationLane): string {
  return 'device' in lane.target ? targetKey(lane.target) : `m:${trackId}:${lane.target.mixer.param}`
}

/** The owner id of the master's lanes in `values`, `drivers` and the marks (`m:master:gain`): a track id is never this word. */
export const MASTER = 'master'

/** What owns lanes: every track, then the master with its chain and its lanes (#378). */
function ownersOf(project: Project): [string, Pick<Track, 'devices' | 'automation'>][] {
  const out: [string, Pick<Track, 'devices' | 'automation'>][] = []
  for (const [id, track] of Object.entries(project.tracks)) if (track) out.push([id, track])
  out.push([MASTER, { devices: project.master_devices ?? {}, automation: project.master_automation ?? {} }])
  return out
}

/** The span of the transport positions where a manual change holds the lane: `[from, until)`. */
interface Hold {
  from: number
  until: number
}

/**
 * The value each automated parameter has at the play position (AM-4, #388): the controls that a lane drives (the knobs of
 * the device panel, the fader and the pan of the mixer and of the track list) show it instead of the value the project holds,
 * so that they move with the music, and they carry a mark that says a lane drives them. The engine does not report parameter
 * values; the page reads the lanes at the play position the engine reports, with the rule of the engine (`valueAt`, AM-2).
 *
 * Stopped, the engine applies each lane's value at the playhead when the transport stops and at every seek (#388), so the
 * controls show that value too: what is seen is what sounds at Play.
 *
 * A change of the parameter from anywhere (a knob, an undo, an agent) holds the lane in the engine: while the transport runs
 * until the play position reaches the lane's next point, while it is stopped until the next play or seek (AM-2). The control then
 * shows the project's value over that span, as the sound has it.
 */
export class AutomatedValues {
  /** Whether the transport runs. */
  readonly running = $derived.by(() => {
    const report = this.engine.started ? this.engine.feedback : null
    return report !== null && (report.transport === 'playing' || report.transport === 'recording')
  })

  /** Where the lanes are read: the engine's play position, or the start before the engine reports one. */
  readonly tick = $derived.by<number>(() => (this.engine.started ? (this.engine.feedback?.playheadTick ?? 0) : 0))

  // The maps are replaced, never changed in place: `$state.raw` and `$derived` are the reactivity, not the map (hence no `SvelteMap`).
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  private holds = $state.raw(new Map<string, Hold>())

  /** The value of every automated parameter at the play position, by `deviceKey` / `mixerKey`. */
  readonly values = $derived.by<ReadonlyMap<string, number>>(() => {
    const tick = this.tick
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const out = new Map<string, number>()
    const view = this.project.view
    if (!view) return out
    for (const [trackId, track] of ownersOf(view.project)) {
      for (const lane of Object.values(track.automation)) {
        if (!lane) continue
        const where = whereOf(trackId, lane)
        const hold = this.holds.get(where)
        if (hold && tick >= hold.from && tick < hold.until) continue
        const value = valueAt(pointsOf(lane), tick, lane.target)
        if (value !== null) out.set(where, value)
      }
    }
    return out
  })

  /** The title of the lane that drives each parameter ("Mixer › Volume"), by the same keys as `values`. */
  readonly drivers = $derived.by<ReadonlyMap<string, string>>(() => {
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const out = new Map<string, string>()
    const view = this.project.view
    if (!view) return out
    const lang = this.ui.lang
    const labels: Labels = {
      mixer: this.ui.t('automation.mixer'),
      mixerParam: (param) => this.ui.t(param === 'gain' ? 'automation.volume' : 'automation.pan'),
      param: (d, p) => deviceText(lang, labelKey(d, p.key)) ?? p.key,
      choice: (d, key) => deviceText(lang, labelKey(d, key)) ?? key,
    }
    for (const [trackId, track] of ownersOf(view.project)) {
      for (const info of lanesOf(track, labels)) {
        // A lane with no point does nothing (AM-1): its parameter is not driven.
        if (pointsOf(info.lane).length === 0) continue
        const where = whereOf(trackId, info.lane)
        if (!out.has(where)) out.set(where, info.title)
      }
    }
    return out
  })

  constructor(
    private readonly project: ProjectStore,
    private readonly engine: EngineStore,
    private readonly ui: UiStore,
  ) {
    project.watch = (events) => this.observe(events)
    // A stop or a seek frees every hold in the engine, even when the playhead stays where it is (#394).
    project.cued = () => {
      // eslint-disable-next-line svelte/prefer-svelte-reactivity
      if (this.holds.size > 0) this.holds = new Map()
    }
    // The engine gives every lane its hand back at a jump (AM-2): the transport starting or stopping (Play is a jump, a stop applies
    // the lanes), a seek (the play position changes while stopped), the play position going back (a seek, the loop) or leaping ahead
    // while it runs. A forward seek of less than half a second of music looks like playing and goes unseen.
    $effect.root(() => {
      // A relay of the engine (a switch of mode, a restart from a snapshot) starts a new epoch of slots with every hold lost, and
      // applies each lane at the playhead: the tick and the transport may not move, so the epoch is what tells (#425).
      let epoch: number | undefined
      $effect(() => {
        const next = this.project.view?.slots.epoch
        untrack(() => {
          if (epoch !== undefined && next !== epoch) this.releaseAll()
          epoch = next
        })
      })
      let last: number | null = null
      let wasRunning = false
      $effect(() => {
        const tick = this.tick
        const running = this.running
        untrack(() => {
          if (this.holds.size > 0 && last !== null) {
            const moved = running ? tick < last || tick - last > this.jumpTicks() : tick !== last
            // eslint-disable-next-line svelte/prefer-svelte-reactivity
            if (running !== wasRunning || moved) this.holds = new Map()
          }
          last = tick
          wasRunning = running
        })
      })
    })
  }

  /** Lets every hold go: the engine has dropped them. */
  private releaseAll() {
    // eslint-disable-next-line svelte/prefer-svelte-reactivity -- the map is replaced, never changed in place
    if (this.holds.size > 0) this.holds = new Map()
  }

  /** Lets go of the holds whose key `drop` names: the lanes they were about are gone, a restoration brings new ones the engine plays (#425). */
  private release(drop: (where: string) => boolean) {
    const kept = [...this.holds].filter(([where]) => !drop(where))
    // eslint-disable-next-line svelte/prefer-svelte-reactivity -- the map is replaced, never changed in place
    if (kept.length !== this.holds.size) this.holds = new Map(kept)
  }

  /** Half a second of music at the project's tempo, in ticks: a play position that moves further between two reports has jumped. */
  private jumpTicks(): number {
    const bpm = this.project.view?.project.tempo[0]?.bpm ?? 120
    return (bpm / 60) * PPQ * 0.5
  }

  /** The value a device parameter is automated to at the play position, or `undefined` when no lane drives it (or a change holds it). */
  device(device: string, param: string): number | undefined {
    return this.values.get(`d:${device}:${param}`)
  }

  /** The value a track's volume (dB) or pan is automated to at the play position. */
  mixer(track: string, param: 'gain' | 'pan'): number | undefined {
    return this.values.get(`m:${track}:${param}`)
  }

  /** The volume a track's controls show: the lane's value when a lane drives it (-96 dB, the lane's silence, reads as silent), else `project`. */
  shownGain(track: string, project: number | null | undefined): number | null | undefined {
    const driven = this.mixer(track, 'gain')
    return driven === undefined ? project : driven <= MIN_GAIN_DB ? null : driven
  }

  /** The pan a track's controls show: the lane's value when a lane drives it, else `project`. */
  shownPan(track: string, project: number): number {
    return this.mixer(track, 'pan') ?? project
  }

  /** The tooltip of a device parameter's control when a lane drives it, else `undefined`. */
  deviceMark(device: string, param: string): string | undefined {
    return this.markOf(`d:${device}:${param}`)
  }

  /** The tooltip of a track's volume or pan control when a lane drives it, else `undefined`. */
  mixerMark(track: string, param: 'gain' | 'pan'): string | undefined {
    return this.markOf(`m:${track}:${param}`)
  }

  private markOf(where: string): string | undefined {
    const lane = this.drivers.get(where)
    return lane === undefined ? undefined : this.ui.t('automation.mark', { lane })
  }

  /** Holds the lanes of a parameter from the play position to their next point, or, stopped, until the next play or seek. */
  private hold(where: string, lanes: Iterable<AutomationLane>) {
    const tick = this.tick
    let until = Number.NEGATIVE_INFINITY
    for (const lane of lanes) {
      const points = pointsOf(lane)
      const i = lowerBound(points, Math.floor(tick) + 1)
      // After the last point the engine holds the value until the next jump.
      until = Math.max(until, this.running && points[i] ? points[i]!.tick : Number.POSITIVE_INFINITY)
    }
    if (until === Number.NEGATIVE_INFINITY) return
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const holds = new Map(this.holds)
    holds.set(where, { from: tick, until })
    this.holds = holds
  }

  /** Called with the events of every applied command, before the project mirror takes them. */
  private observe(events: readonly ProjectEvent[]) {
    const view = this.project.view
    if (!view) return
    for (const event of events) {
      if (event.type === 'track_deleted') {
        // The track goes with its lanes; its restoration (an undo) gives lanes in new slots, which the engine plays.
        const gone = view.project.tracks[event.track]
        const devices = Object.keys(gone?.devices ?? {})
        this.release((where) => where.startsWith(`m:${event.track}:`) || devices.some((device) => where.startsWith(`d:${device}:`)))
      } else if (event.type === 'automation_lane_deleted' || event.type === 'automation_points_changed') {
        // A lane deleted (alone, with its device, or by an instrument swap) and restored is a new lane in the engine; so is a lane that
        // gets points while it had none (the engine drops a lane with no point, and its hold). Read before the mirror takes the event.
        const automation = event.track ? view.project.tracks[event.track]?.automation : (view.project.master_automation ?? {})
        const lane = automation?.[event.lane]
        if (!lane || (event.type === 'automation_points_changed' && pointsOf(lane).length > 0)) continue
        const where = whereOf(event.track ?? MASTER, lane)
        // Another lane with points on the same parameter (a merged project) keeps its own hold in the engine; one with none is not in the engine.
        if (
          Object.entries(automation!).some(
            ([id, other]) => id !== event.lane && other && pointsOf(other).length > 0 && whereOf(event.track ?? MASTER, other) === where,
          )
        )
          continue
        this.release((held) => held === where)
      } else if (event.type === 'device_param_changed') {
        const lanes: AutomationLane[] = []
        for (const [, track] of ownersOf(view.project)) {
          for (const lane of Object.values(track.automation)) {
            if (lane && 'device' in lane.target && lane.target.device.device === event.device && lane.target.device.param === event.param) lanes.push(lane)
          }
        }
        if (lanes.length > 0) this.hold(`d:${event.device}:${event.param}`, lanes)
      } else if (event.type === 'gesture_unchanged') {
        // A gesture that left the value as the project holds it: the engine still gives way to it (#417), so does the page.
        const owner = event.track ?? MASTER
        const track = event.track ? view.project.tracks[event.track] : { automation: view.project.master_automation ?? {} }
        if (!track) continue
        const lanes: AutomationLane[] = []
        const target = event.target
        for (const lane of Object.values(track.automation)) {
          if (!lane) continue
          if (
            'device' in target &&
            'device' in lane.target &&
            lane.target.device.device === target.device.device &&
            lane.target.device.param === target.device.param
          )
            lanes.push(lane)
          if ('mixer' in target && 'mixer' in lane.target && lane.target.mixer.param === target.mixer.param) lanes.push(lane)
        }
        if (lanes.length > 0) this.hold('device' in target ? `d:${target.device.device}:${target.device.param}` : `m:${owner}:${target.mixer.param}`, lanes)
      } else if (event.type === 'mixer_changed') {
        // A track's strip, or the master's when `track` is null.
        const owner = event.track ?? MASTER
        const track = event.track ? view.project.tracks[event.track] : { mixer: view.project.master, automation: view.project.master_automation ?? {} }
        if (!track) continue
        for (const param of ['gain', 'pan'] as const) {
          const changed = param === 'gain' ? (track.mixer.gain_db ?? null) !== (event.gain_db ?? null) : track.mixer.pan !== event.pan
          if (!changed) continue
          const lanes: AutomationLane[] = []
          for (const lane of Object.values(track.automation)) {
            if (lane && 'mixer' in lane.target && lane.target.mixer.param === param) lanes.push(lane)
          }
          if (lanes.length > 0) this.hold(`m:${owner}:${param}`, lanes)
        }
      }
    }
  }
}
