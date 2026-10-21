import { SINGLE_SYNTH, type ProjectEvent, type ProjectView, type SynthParam } from '../generated/protocol'
import { DEFAULT_PARAMS } from '../params'
import { deviceOfKind, sortedTracks } from '../project/tracks'

/**
 * The project as the page sees it: a reactive mirror of the page's project core (§ 2.6 of the
 * MVP breakdown), written only by the dispatcher. It is fed by the `ProjectEvent`s of applied
 * commands: any event that changes the project makes it read the core's project again; the
 * history flags come from `history_changed`.
 */
export class ProjectStore {
  /** The project and the session as the core holds them; `null` until audio starts and creates the core. */
  view = $state.raw<ProjectView | null>(null)
  canUndo = $state(false)
  canRedo = $state(false)

  /** The synth's settings: the engine defaults overlaid with what the project holds for its device. */
  params = $derived.by<Record<SynthParam, number>>(() => {
    const held = this.view?.project.tracks[SINGLE_SYNTH.track]?.devices[SINGLE_SYNTH.device]?.params ?? {}
    const params = { ...DEFAULT_PARAMS }
    for (const key of Object.keys(params) as SynthParam[]) params[key] = held[key] ?? params[key]
    return params
  })

  /** The tracks in the order the user sees. */
  tracks = $derived(this.view ? sortedTracks(this.view.project) : [])
  /** The selected track: where the computer keyboard and live MIDI go (session state). */
  selected = $derived(this.tracks.find((entry) => entry.id === this.view?.session.selected_track) ?? null)
  /** The soloed tracks (session state). */
  soloed = $derived(this.view?.session.solo ?? [])
  /** The armed tracks (session state): live input reaches them besides the selected track. */
  armed = $derived(this.view?.session.armed ?? [])
  /** The tracks that monitor their live input (session state). */
  monitored = $derived(this.view?.session.monitor ?? [])

  /**
   * The oscillator of the selected track with its settings (the engine defaults overlaid with
   * what the project holds), or null when the selection has none: what the Instrument view edits.
   */
  synth = $derived.by<{ track: string; device: string; params: Record<SynthParam, number> } | null>(() => {
    const selected = this.selected
    const device = selected && deviceOfKind(selected.track, SINGLE_SYNTH.kind)
    if (!selected || !device) return null
    const params = { ...DEFAULT_PARAMS }
    for (const key of Object.keys(params) as SynthParam[]) params[key] = device.params[key] ?? params[key]
    return { track: selected.id, device: device.id, params }
  })

  /** Mirrors a freshly created core, or the project after a refresh. */
  show(view: ProjectView) {
    this.view = view
  }

  /** Sets one device parameter in the mirror; false when the device is not in it. */
  private setParam(device: string, param: string, value: number): boolean {
    const view = this.view
    if (!view) return false
    for (const [trackId, track] of Object.entries(view.project.tracks)) {
      const held = track?.devices[device]
      if (!track || !held) continue
      const project = {
        ...view.project,
        tracks: {
          ...view.project.tracks,
          [trackId]: { ...track, devices: { ...track.devices, [device]: { ...held, params: { ...held.params, [param]: value } } } },
        },
      }
      this.view = { ...view, project }
      return true
    }
    return false
  }

  /**
   * Takes the events of an applied command. `read` gives the core's project; it is called at
   * most once, and only when an event changed something other than the history flags or came from a controller.
   */
  apply(events: ProjectEvent[], read: () => ProjectView) {
    let changed = false
    for (const event of events) {
      if (event.type === 'history_changed') {
        this.canUndo = event.can_undo
        this.canRedo = event.can_redo
      } else if (event.type === 'control_change') {
        // A controller on the live input changes nothing in the project: a sweep must not read it again.
      } else if (event.type === 'device_param_changed' && this.setParam(event.device, event.param, event.value)) {
        // Patched in place: a knob drag does not read the whole project again.
      } else changed = true
    }
    if (changed) this.view = read()
  }
}
