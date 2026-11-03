<script lang="ts">
  /**
   * The piano roll (PN-1): the notes of one clip of the selected MIDI track on the shared time grid,
   * with a keyboard on the left and a velocity lane under it. A gesture is previewed locally and
   * sent as ONE command when it ends (one batch is one history entry, so one undo takes the whole
   * drag back, and no command is sent while the pointer moves). Selecting notes, the clip being
   * edited, the tool and the zoom are the page's view state, not project data.
   */
  import { onDestroy, tick, untrack } from 'svelte'
  import type { Command, Note } from '../../lib/generated/protocol'
  import {
    addNotes,
    auditionOff,
    auditionOn,
    createClipAtStart,
    duplicateNotes,
    quantizeNotes,
    removeNotes,
    renameClip,
    setClipLength,
    setClipLoop,
    transposeNotes,
    updateNotes,
  } from '../../lib/clip-commands'
  import { createDrag, DRAG_SLOP_PX, fineKey, type Drag } from '../../lib/continuous'
  import TimeGrid from '../../lib/components/timegrid/TimeGrid.svelte'
  import GestureLabel from '../../lib/components/GestureLabel.svelte'
  import { lengthText as gestureLength, positionText, snapText } from '../../lib/gesture-label'
  import type { MenuItem } from '../../lib/context-menu'
  import { chordText, positionKey, withShortcut } from '../../lib/shortcuts'
  import {
    DEFAULT_PX_PER_TICK,
    SNAP_SETTINGS,
    clampView,
    snapStep,
    snapTick,
    xToTick,
    zoomAt,
    type SnapSetting,
    type View,
  } from '../../lib/components/timegrid/grid'
  import { RULER_HEIGHT, type BodyFrame } from '../../lib/components/timegrid/render'
  import { isQuantizeKey, isTextTarget, QUANTIZE_CODE } from '../../lib/keyboard'
  import { instrumentNamedKeys, isMidi, trackColor as colorOfTrack } from '../../lib/project/tracks'
  import { beginGroup, endGroup } from '../../lib/track-commands'
  import { useStores } from '../../lib/stores'
  import { barTicks, PPQ } from '../transport/time'
  import { itemAt, moveCursor, rovingKey, step as stepItem, zoomKey, type RovingKey } from '../../lib/roving'
  import { sortedScenes } from '../grid/cells'
  import { clipEntries, clipLabel, clipOrigin, editedClip, editedPlacement, ghostNotes, midiClips, type ClipEntry, type MidiClip } from './clips'
  import { drawKeys, drawLane, drawRoll } from './draw'
  import {
    DEFAULT_ROW_HEIGHT,
    DEFAULT_VELOCITY,
    acceptable,
    defaultLength,
    clampScrollY,
    drawnNote,
    hitNote,
    velocityTarget,
    initialScrollY,
    moveNotes,
    noteKey,
    SIXTEENTH,
    noteRect,
    pitchName,
    MAX_PITCH,
    notesInRect,
    pitchAt,
    rectBetween,
    resizeNotes,
    shiftVelocity,
    stepVelocity,
    MAX_VELOCITY,
    MIN_VELOCITY,
    parseDrawVelocity,
    stillApplies,
    fits,
    toggled,
    resetVelocities,
    velocityOf,
    velocityPosition,
    VELOCITY_HOME,
    afterChanges,
    DEFAULT_QUANTIZE_GRID,
    DEFAULT_QUANTIZE_STRENGTH,
    MAX_BARS,
    QUANTIZE_GRIDS,
    drawStep,
    duplicateOffset,
    endOf,
    floorTick,
    fromBoard,
    lengthFor,
    quantizeChanges,
    quantizeSaid,
    toBoard,
    transposeChanges,
    type QuantizeGrid,
    type Change,
    type Layout,
    type Rect,
  } from './roll'
  import { liveTick, playheadIn, settle } from './playhead'
  import Surface from './Surface.svelte'
  import { boardNotes, setBoard } from './clipboard'

  const { project, engine, ui, dispatcher, recorder } = useStores()
  const t = ui.t
  const run = (command: Command) => dispatcher.run(command)

  /** How long an audition sounds at least, so that a click is heard. */
  const MIN_AUDITION_MS = 150
  const KEYS_WIDTH = 56
  /** The keyboard column of a drum kit, wide enough for the names of its sounds. */
  const NAMED_KEYS_WIDTH = 96
  const LANE_HEIGHT = 56
  const ROW = DEFAULT_ROW_HEIGHT

  type Tool = 'draw' | 'select'
  type Gesture =
    | { kind: 'draw'; clip: string; pressTick: number; pitch: number; note: Note | null }
    | { kind: 'move' | 'resize'; clip: string; grabbed: Note; key: string; startX: number; startY: number; changes: Change[] }
    | { kind: 'velocity'; clip: string; grabbed: Note; key: string; startX: number; startY: number; moved: boolean; drag: Drag; changes: Change[] }
    | { kind: 'band'; clip: string; x0: number; y0: number; base: ReadonlySet<string>; rect: Rect }
    | { kind: 'loop'; clip: string; from: number; to: number }

  // What is edited: the selected MIDI track's clip.
  const track = $derived(project.selected && isMidi(project.selected.track) ? project.selected : null)
  const clips = $derived(track ? midiClips(track.track) : [])
  const edited = $derived(editedClip(clips, ui.editedClip))
  /** The placement the roll is on (#494): the one the timeline opened, else the clip's first; null for a clip not placed. */
  const placement = $derived(editedPlacement(edited, ui.editedPlacement))
  /** The placement chosen in the timeline or the list, when it still places this clip: the play position shows only there. */
  const chosenPlacement = $derived(placement && placement.id === ui.editedPlacement ? placement.id : null)
  const notes = $derived<readonly Note[]>(edited?.notes ?? [])
  const keyNames = $derived.by<ReadonlyMap<number, string> | null>(() => {
    const named = track ? instrumentNamedKeys(track.track) : []
    return named.length === 0 ? null : new Map(named.map((k) => [k.key, k.name]))
  })
  // The notes of the clip are drawn in the track's colour; the other tracks' notes at the same time range show hollow (#470).
  const trackColor = $derived(track ? colorOfTrack(track.track.color, track.index) : '#8f8a85')
  let showOthers = $state(true)
  const others = $derived(showOthers && track ? ghostNotes(project.tracks, track.id, edited, track.track, placement?.id ?? null) : [])
  const signature = $derived(project.view?.project.meter[0]?.signature ?? { numerator: 4, denominator: 4 })
  const bar = $derived(barTicks(signature))
  const clipLength = $derived(edited?.length ?? bar)
  const totalBars = $derived(Math.max(1, Math.ceil(clipLength / bar)))
  const clipName = $derived(edited ? clipLabel(edited, t('pianoRoll.clipPlain')) : '')
  const scenes = $derived(project.view ? sortedScenes(project.view.project) : [])
  /** "Scene n" or the scene's own name: the grid's label for it. */
  const sceneLabel = (id: string) => {
    const entry = scenes.find((s) => s.id === id)
    return entry ? entry.scene.name.trim() || t('pianoRoll.sceneDefault', { n: entry.index + 1 }) : t('pianoRoll.sceneGone')
  }
  /** Where a clip comes from, in words: "Grid · Scene 1", "Arrangement · bar 2" (at `start`, a placement's, else its first). */
  const originOf = (entry: MidiClip, start: number | null = null) =>
    clipOrigin(
      entry,
      {
        grid: (id) => t('pianoRoll.originGrid', { scene: sceneLabel(id) }),
        arrangement: (at) => t('pianoRoll.originArrangement', { n: Math.floor(at / bar) + 1 }),
        pool: () => t('pianoRoll.originPool'),
      },
      start,
    )
  /** An entry of the list (a clip at one of its placements): its name, then where it comes from. */
  const listLabel = (entry: ClipEntry) => {
    const origin = originOf(entry.clip, entry.placement?.start ?? null)
    const name = clipLabel(entry.clip, t('pianoRoll.clipPlain'))
    return origin ? `${name} (${origin})` : name
  }
  const entries = $derived(clipEntries(clips))
  /** The entry shown: the clip at the placement the roll is on. */
  const entryKey = $derived(placement ? `placement:${placement.id}` : edited ? `clip:${edited.id}` : '')
  const origin = $derived(edited ? originOf(edited, placement?.start ?? null) : '')

  // Renaming the clip being edited: the way to name a clip that lives in the arrangement only.
  let renaming = $state(false)
  let draft = $state('')
  let nameField: HTMLInputElement | undefined = $state()
  async function startRename() {
    if (!edited) return
    draft = clipName
    renaming = true
    await tick()
    nameField?.focus()
    nameField?.select()
  }
  function finishRename(commit: boolean) {
    if (!renaming) return
    renaming = false
    const name = draft.trim()
    if (commit && edited && name !== edited.clip.name.trim()) run(renameClip(edited.id, name))
  }
  function onRenameKey(event: KeyboardEvent) {
    if (event.key !== 'Enter' && event.key !== 'Escape') return
    event.preventDefault()
    event.stopPropagation()
    finishRename(event.key === 'Enter')
  }

  // View state.
  let tool = $state<Tool>('draw')
  let snap = $state<SnapSetting>('auto')
  // Zoom and scroll of the clip come back when the panel returns to the roll (it is torn down when another tab is shown).
  const left = ui.recall<{ clip: string | null; view: View; scrollY: number }>('piano-roll')
  let view = $state<View>(left?.view ?? { pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 })
  let scrollY = $state(left?.scrollY ?? 0)
  let stageW = $state(0)
  let stageH = $state(0)
  let laneH = $state(LANE_HEIGHT)
  /** The length of the last note drawn or resized; null follows the snap step (reset by a change of the snap and by a new clip). */
  let lastLength = $state<number | null>(null)
  /** The velocity of a note drawn (#527): its own setting in the toolbar, never what a touched note had. */
  let drawVelocity = $state(untrack(() => parseDrawVelocity(String(ui.recall<number>('piano-roll-draw-velocity') ?? DEFAULT_VELOCITY), DEFAULT_VELOCITY)))
  let rollEl = $state<HTMLElement>()
  let picked = $state.raw<ReadonlySet<string>>(new Set())
  let gesture = $state.raw<Gesture | null>(null)
  let cursor = $state<'draw' | 'move' | 'resize'>('draw')
  /** The note under the pointer (no gesture) and whether it is on its resizable end, for the handle (#612). */
  let noteHover = $state.raw<{ key: string; end: boolean } | null>(null)
  /** The pointer during a move or a resize, with whether the key that frees the snap was held at the last move (#612). */
  let pointer = $state.raw<{ x: number; y: number; free: boolean } | null>(null)
  let activePitches = $state.raw<ReadonlySet<number>>(new Set())

  const shown = $derived(clampView(view, stageW, totalBars * bar))
  const step = $derived(snapStep(snap, signature, shown.pxPerTick))
  /** The step notes are drawn on: Auto never goes below a sixteenth. */
  const drawing = $derived(drawStep(snap === 'auto', step))
  const layout = $derived<Layout>({ rowHeight: ROW, scrollY: clampScrollY(scrollY, stageH, { rowHeight: ROW, top: RULER_HEIGHT }), top: RULER_HEIGHT })
  const chosenNotes = $derived(picked.size === 0 ? [] : notes.filter((n) => picked.has(noteKey(n))))
  const chosenKeys = $derived<ReadonlySet<string>>(new Set(chosenNotes.map(noteKey)))

  const ghosts = $derived.by<Note[]>(() => {
    const g = gesture
    if (!g) return []
    if (g.kind === 'draw') return g.note ? [g.note] : []
    if (g.kind === 'move' || g.kind === 'resize' || g.kind === 'velocity') return g.changes.map((c) => c.to)
    return []
  })
  const hidden = $derived.by<ReadonlySet<string>>(() => {
    const g = gesture
    return g && (g.kind === 'move' || g.kind === 'resize' || g.kind === 'velocity') ? new Set(g.changes.map((c) => noteKey(c.from))) : new Set()
  })
  const band = $derived(gesture?.kind === 'band' ? gesture.rect : null)
  const loopShown = $derived(
    gesture?.kind === 'loop'
      ? gesture.from === gesture.to
        ? (edited?.looping ?? null)
        : { start: Math.min(gesture.from, gesture.to), end: Math.max(gesture.from, gesture.to) }
      : (edited?.looping ?? null),
  )

  // The play position inside the edited clip (#293). The engine reports about 30 times a second; while the transport
  // runs, the tick is carried on from the last report at each animation frame, so that the bar moves at the screen's pace.
  const report = $derived(engine.started ? engine.feedback : null)
  const running = $derived(report?.transport === 'playing' || report?.transport === 'recording')
  const bpm = $derived(project.view?.project.tempo[0]?.bpm ?? null)
  const transport = $derived(project.view?.project.transport ?? null)
  const transportLoop = $derived(transport?.loop_enabled ? (transport.loop_region ?? null) : null)
  let reportAt = 0
  /** The transport tick shown: the report's, carried on at each frame while the transport runs. */
  let shownTick = $state<number | null>(null)
  function advance() {
    if (!report) {
      shownTick = null
      return
    }
    const target = liveTick(report.playheadTick, running, performance.now() - reportAt, bpm, transportLoop)
    shownTick = running ? settle(shownTick, target, bpm, transportLoop) : target
  }
  $effect(() => {
    void engine.feedback
    reportAt = performance.now()
    untrack(advance)
  })
  $effect(() => {
    if (!running) return
    let id = 0
    const frame = () => {
      advance()
      id = requestAnimationFrame(frame)
    }
    id = requestAnimationFrame(frame)
    return () => cancelAnimationFrame(id)
  })
  const live = $derived.by<number | null>(() => {
    if (shownTick === null || !report || !track || !edited || !project.view) return null
    const tick = shownTick
    return playheadIn({
      report,
      slots: project.view.slots,
      trackId: track.id,
      track: track.track,
      clipId: edited.id,
      clip: edited.clip,
      length: edited.length,
      looping: edited.looping,
      tick,
      placement: chosenPlacement,
    })
  })
  // Stopped, a clip of the timeline (no grid cell) shows the transport position when it is inside the clip, none otherwise:
  // Stop brings the head back to the cue (#360), which may lie outside the clip. A clip of the grid keeps the bar where it was
  // last seen (it moves with a seek that lands in the clip); while the transport runs and this clip does not sound, there is
  // none. A new clip starts without one.
  // The position is kept with the clip it was seen in, so that another clip never inherits it, whatever the order of the updates.
  let lastSeen = $state<{ clip: string; at: number } | null>(null)
  $effect(() => {
    if (live !== null && edited) lastSeen = { clip: edited.id, at: live }
  })
  const onTimeline = $derived(edited?.clip.slot == null)
  const playhead = $derived(running || onTimeline ? live : lastSeen && lastSeen.clip === edited?.id ? lastSeen.at : null)

  /** The notes of a take being recorded on this track (#470), where this clip plays them: shown live, written at the end of the take. */
  const taking = $derived.by<Note[]>(() => {
    const played = track ? recorder.notes[track.id] : undefined
    if (!played?.length || !report || !track || !edited || !project.view) return []
    const out: Note[] = []
    for (const n of played) {
      const start = playheadIn({
        report,
        slots: project.view.slots,
        trackId: track.id,
        track: track.track,
        clipId: edited.id,
        clip: edited.clip,
        length: edited.length,
        looping: edited.looping,
        tick: n.start,
      })
      if (start !== null) out.push({ start, length: Math.max(1, Math.min(n.length, edited.length - start)), pitch: n.pitch, velocity: n.velocity })
    }
    return out
  })

  let grid: ReturnType<typeof TimeGrid> | undefined = $state()
  let stage: HTMLDivElement | undefined = $state()

  /** The note whose resize handle is drawn: the hovered one, or the one being resized (as it would be). */
  const handleNow = $derived.by(() => {
    const g = gesture
    if (g) {
      if (g.kind !== 'resize') return null
      const to = g.changes.find((c) => noteKey(c.from) === g.key)?.to
      return { note: to ?? g.grabbed, strong: true }
    }
    const over = noteHover ? notes.find((n) => noteKey(n) === noteHover!.key) : undefined
    return over && noteHover ? { note: over, strong: noteHover.end } : null
  })
  /** The label near the pointer during a move or a resize of notes (#612). */
  const gestureLines = $derived.by<string[] | null>(() => {
    const g = gesture
    if (!g || !pointer || (g.kind !== 'move' && g.kind !== 'resize') || g.changes.length === 0) return null
    const note = g.changes.find((c) => noteKey(c.from) === g.key)?.to
    if (!note) return null
    const names = {
      bar: t('pianoRoll.snap.bar'),
      beat: t('pianoRoll.snap.beat'),
      half: t('pianoRoll.snap.half-beat'),
      quarter: t('pianoRoll.snap.quarter-beat'),
    }
    const lines = [
      g.kind === 'resize' ? gestureLength(t, signature, note.length) : positionText(t, signature, note.start),
      snapText(t, signature, pointer.free ? null : step, names),
    ]
    if (!pointer.free) lines.push(t('gesture.ctrlFree', { key: chordText(['Mod']) }))
    return lines
  })

  const body = (ctx: CanvasRenderingContext2D, frame: BodyFrame) =>
    drawRoll(ctx, frame, {
      layout,
      notes,
      hidden,
      selected: chosenKeys,
      ghosts,
      band,
      clipLength,
      loop: loopShown,
      names: keyNames,
      trackColor,
      others,
      taking,
      handle: handleNow,
    })

  // The grid draws in its own frame and cannot see this state: tell it when something it draws changed.
  $effect(() => {
    void [notes, chosenKeys, ghosts, hidden, band, layout, clipLength, loopShown, keyNames, trackColor, others, taking, handleNow]
    grid?.invalidate()
  })

  // A change of the snap setting gives the next drawn note the new step.
  $effect(() => {
    void snap
    untrack(() => (lastLength = null))
  })

  // A new clip opens with a clean selection, around its notes, and its drawn length back to the snap step.
  let openedFor: string | null = left?.clip ?? null
  $effect(() => {
    const id = edited?.id ?? null
    if (id === openedFor) return
    if (id !== null && stageH === 0) return
    openedFor = id
    untrack(() => {
      picked = new Set()
      gesture = null
      scrollY = initialScrollY(notes, stageH, { rowHeight: ROW, top: RULER_HEIGHT }, keyNames ? [...keyNames.keys()] : [])
      lastLength = null
    })
  })

  // --- audition ---------------------------------------------------------------------------------

  let sounding: { track: string; pitch: number } | null = null
  let soundSeq = 0
  let soundSince = 0

  /** Sounds `pitch` on the edited track (or silences with `null`), one note at a time. */
  function sound(pitch: number | null, velocity = drawVelocity) {
    soundSeq++
    if (sounding && pitch !== null && sounding.pitch === pitch) return
    if (sounding) run(auditionOff(sounding.track, sounding.pitch))
    sounding = null
    if (pitch !== null && track) {
      run(auditionOn(track.id, pitch, velocity))
      sounding = { track: track.id, pitch }
      soundSince = performance.now()
    }
    activePitches = new Set(sounding ? [sounding.pitch] : [])
  }

  /** Lets the note go, after it has sounded long enough to be heard. */
  function release() {
    const wait = MIN_AUDITION_MS - (performance.now() - soundSince)
    if (wait <= 0) return sound(null)
    const seq = ++soundSeq
    setTimeout(() => {
      if (soundSeq === seq) sound(null)
    }, wait)
  }

  onDestroy(() => {
    ui.remember('piano-roll', { clip: openedFor, view, scrollY })
    sound(null)
  })

  // --- gestures ---------------------------------------------------------------------------------

  const local = (event: { clientX: number; clientY: number }, element: HTMLElement | undefined) => {
    const box = element!.getBoundingClientRect()
    return { x: event.clientX - box.left, y: event.clientY - box.top }
  }

  /** Drops the gesture and the note it sounds, sending nothing. */
  function cancel() {
    gesture = null
    pointer = null
    sound(null)
  }

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0 || !edited || !track || gesture) return
    const { x, y } = local(event, stage)
    stage!.setPointerCapture(event.pointerId)
    if (y < RULER_HEIGHT) {
      const tick = snapTick(xToTick(x, shown), step)
      gesture = { kind: 'loop', clip: edited.id, from: Math.min(tick, clipLength), to: Math.min(tick, clipLength) }
      return
    }
    const hit = hitNote(notes, x, y, shown, layout)
    if (hit) {
      const grabbed = notes[hit.index]!
      const key = noteKey(grabbed)
      if (event.shiftKey) {
        const next = toggled(picked, key)
        picked = next.picked
        // A shift-click on a selected note takes it out of the selection and starts nothing.
        if (!next.on) return
      } else if (!picked.has(key)) picked = new Set([key])
      gesture = { kind: hit.zone === 'end' ? 'resize' : 'move', clip: edited.id, grabbed, key, startX: x, startY: y, changes: [] }
      sound(grabbed.pitch, grabbed.velocity)
      return
    }
    if (tool === 'draw') {
      const pitch = pitchAt(y, layout)
      const note = drawnNote(xToTick(x, shown), null, pitch, drawVelocity, drawing, defaultLength(lastLength, drawing), clipLength)
      if (!note) return
      gesture = { kind: 'draw', clip: edited.id, pressTick: xToTick(x, shown), pitch, note }
      sound(pitch)
    } else {
      const base = event.shiftKey ? picked : new Set<string>()
      picked = base
      gesture = { kind: 'band', clip: edited.id, x0: x, y0: y, base, rect: rectBetween(x, y, x, y) }
    }
  }

  function onPointerMove(event: PointerEvent) {
    const { x, y } = local(event, stage)
    const g = gesture
    if (!g) {
      const hit = y >= RULER_HEIGHT ? hitNote(notes, x, y, shown, layout) : null
      cursor = hit ? (hit.zone === 'end' ? 'resize' : 'move') : 'draw'
      const over = hit ? notes[hit.index] : undefined
      const next = over ? { key: noteKey(over), end: hit!.zone === 'end' } : null
      if (next?.key !== noteHover?.key || next?.end !== noteHover?.end) noteHover = next
      return
    }
    if (!edited || edited.id !== g.clip) return cancel()
    if (g.kind === 'loop') {
      gesture = { ...g, to: Math.min(clipLength, snapTick(xToTick(x, shown), step)) }
    } else if (g.kind === 'draw') {
      const end = snapTick(xToTick(x, shown), drawing)
      const pitch = g.note?.pitch ?? g.pitch
      gesture = { ...g, note: drawnNote(g.pressTick, end, pitch, drawVelocity, drawing, defaultLength(lastLength, drawing), clipLength) }
    } else if (g.kind === 'band') {
      const rect = rectBetween(g.x0, g.y0, x, y)
      picked = new Set([...g.base, ...notesInRect(notes, rect, shown, layout)])
      gesture = { ...g, rect }
    } else if (g.kind === 'move' || g.kind === 'resize') {
      // The notes are read again at each move: a change of the clip under the drag (undo, an agent) ends it.
      if (!chosenKeys.has(g.key)) return cancel()
      let changes: Change[]
      // Ctrl (Cmd on a Mac) puts the snap aside while it is held (#612), as in the arrangement.
      const free = fineKey(event)
      const stepNow = free ? null : step
      pointer = { x, y, free }
      if (g.kind === 'move') {
        const dTick = snapTick(g.grabbed.start + (x - g.startX) / shown.pxPerTick, stepNow) - g.grabbed.start
        changes = moveNotes(chosenNotes, dTick, pitchAt(y, layout) - pitchAt(g.startY, layout), clipLength)
      } else {
        const end = snapTick(xToTick(x, shown), stepNow)
        changes = resizeNotes(chosenNotes, end - g.grabbed.start - g.grabbed.length, clipLength)
      }
      gesture = { ...g, changes }
      const moved = changes.find((c) => noteKey(c.from) === g.key)
      if (moved) sound(moved.to.pitch, moved.to.velocity)
    }
  }

  function onPointerUp() {
    finish(true)
  }

  /** Ends the gesture: its one command is sent when `commit` and the clip is still what the gesture started on. */
  function finish(commit: boolean) {
    const g = gesture
    gesture = null
    pointer = null
    noteHover = null
    release()
    if (!g || !commit || !edited || edited.id !== g.clip) return
    if (g.kind === 'draw') {
      if (!g.note || !fits(g.note, clipLength)) return
      run(addNotes(g.clip, [g.note]))
      picked = new Set([noteKey(g.note)])
      lastLength = g.note.length
    } else if (g.kind === 'move' || g.kind === 'resize' || g.kind === 'velocity') {
      // The notes the gesture holds must still be in the clip: after an undo they are gone and nothing is sent.
      if (!chosenKeys.has(g.key)) return
      const changes = acceptable(notes, g.changes)
      if (!changes || changes.length === 0 || !stillApplies(notes, changes, clipLength)) return
      run(updateNotes(g.clip, changes))
      picked = afterChanges(picked, changes)
      const grabbed = changes.find((c) => noteKey(c.from) === g.key)
      if (grabbed && g.kind === 'resize') lastLength = grabbed.to.length
    } else if (g.kind === 'loop') {
      if (g.from === g.to || Math.max(g.from, g.to) > clipLength) return
      run(setClipLoop(g.clip, { start: Math.min(g.from, g.to), end: Math.max(g.from, g.to) }))
    }
  }

  function removeSelected() {
    if (!edited || chosenNotes.length === 0) return
    const n = chosenNotes.length
    run(removeNotes(edited.id, chosenNotes))
    picked = new Set()
    announce(t('pianoRoll.saidDeleted', { n }))
  }

  /**
   * Select all, copy, cut, paste and duplicate (Ctrl or Cmd + A, C, X, V, D). Returns whether the key was one of them.
   * Shared by the stage and the window, so that they work with the focus on the toolbar too.
   */
  function editShortcut(event: KeyboardEvent): boolean {
    // Q quantizes (#523): the one bare letter that is no note.
    if (isQuantizeKey(event)) {
      quantizeNow()
      return true
    }
    if (!(event.ctrlKey || event.metaKey) || event.altKey) return false
    const key = event.key.toLowerCase()
    if (key === 'a') picked = new Set(notes.map(noteKey))
    else if (key === 'c') copyChosen()
    else if (key === 'x') {
      if (copyChosen()) removeSelected()
    } else if (key === 'v') pasteBoard()
    else if (key === 'd') duplicateChosen()
    else return false
    return true
  }

  /**
   * The editing shortcuts anywhere on the page while a clip is open (the stage may not hold the focus, for
   * example after a click on the toolbar): Ctrl+A selects the notes, not the page's text. Only when the focus is in
   * the roll or on the page itself (nothing focused): another screen keeps its own keys, and a text field its own.
   */
  function onWindowKeyDown(event: KeyboardEvent) {
    if (event.defaultPrevented || !edited || isTextTarget(event.target)) return
    const target = event.target
    if (target !== document.body && !(target instanceof Node && rollEl?.contains(target))) return
    if (editShortcut(event)) event.preventDefault()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (isTextTarget(event.target)) return
    if (event.key === 'Delete' || event.key === 'Backspace') {
      removeSelected()
    } else if (event.key === 'Escape') {
      if (gesture) cancel()
      else picked = new Set()
    } else if (editShortcut(event)) {
      // Handled by editShortcut; the key is consumed below.
    } else if (!event.ctrlKey && !event.metaKey && event.altKey && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
      // Alt+Up and Alt+Down (#529): the selection by a semitone, Alt+Shift by an octave; as from a note's cell or the free cursor.
      transposeChosen((event.key === 'ArrowUp' ? 1 : -1) * (event.shiftKey ? 12 : 1))
    } else if (!event.ctrlKey && !event.metaKey && event.altKey && !event.shiftKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
      // Alt+Left and Alt+Right (#550): the selection by the drawing step, from the grid as from a note's cell.
      nudgeChosen(event.key === 'ArrowLeft' ? -1 : 1)
    } else if (!event.ctrlKey && !event.metaKey && !event.altKey && (event.key === 'PageUp' || event.key === 'PageDown') && chosenNotes.length > 0) {
      // The selection's velocity (#494), as on a note's cell.
      const key = rovingKey(event)
      if (key?.kind === 'velocity') changeVelocity(chosenNotes, key.delta, null)
    } else return
    event.preventDefault()
  }

  // --- the notes from the keyboard and for assistive technology (#471) ---------------------------------------------
  // As the arrangement's clips: named cells over the canvas, one in the tab order. Left and Right go to the previous or
  // next note in time, Up and Down to the nearest higher or lower note; the selection follows, Shift extends it.
  // Alt+Left and Alt+Right move the selected notes by the snap step, Alt+Up and Alt+Down transpose them by a semitone
  // (Shift: an octave). Enter sounds the note. Delete removes the selection. Every edit is a command; a polite live
  // region says what it did.
  let focusKey = $state<string | null>(null)
  let itemsEl: HTMLDivElement | undefined = $state()
  let said = $state({ text: '', n: 0 })
  function announce(text: string) {
    // The same words twice are still a new message: an invisible space every other time changes the text.
    said = { text: said.n % 2 === 0 ? `${text}\u200b` : text, n: said.n + 1 }
  }
  // Undo and redo say what they did here too (#494), from wherever they were asked; what was said before the roll opened is not said again.
  let heardHistory = untrack(() => ui.historySaid.n)
  $effect(() => {
    const message = ui.historySaid
    if (message.n === heardHistory) return
    heardHistory = message.n
    untrack(() => {
      announce(message.text)
      if (itemsHadFocus) void keepItemsFocus()
    })
  })
  /** Whether the keyboard was last in the notes' grid (a focus that leaves it for another element clears it; a removed cell does not). */
  let itemsHadFocus = false
  /** An undo or a redo can take away the focused note (its key holds its values): the focus goes back to the grid's tab stop. */
  async function keepItemsFocus() {
    await tick()
    const active = document.activeElement
    if (active && active !== document.body) return
    itemsEl?.querySelector<HTMLElement>('[tabindex="0"]')?.focus()
  }
  const noteItems = $derived(notes.map((n) => ({ id: noteKey(n), row: MAX_PITCH - n.pitch, start: n.start, end: n.start + n.length, note: n })))
  const tabKey = $derived(
    (focusKey && noteItems.some((i) => i.id === focusKey) ? focusKey : null) ?? (chosenNotes[0] ? noteKey(chosenNotes[0]) : null) ?? noteItems[0]?.id ?? null,
  )
  function lengthText(ticks: number): string {
    const beats = ticks / PPQ
    return beats === 1 ? t('pianoRoll.noteBeat') : t('pianoRoll.noteBeats', { n: Number(beats.toFixed(2)) })
  }
  function noteText(n: Note): string {
    const beatsPerBar = bar / PPQ
    const beat = n.start / PPQ
    const base = t('pianoRoll.noteItem', {
      pitch: keyNames?.get(n.pitch) ?? pitchName(n.pitch),
      bar: Math.floor(beat / beatsPerBar) + 1,
      beat: Number(((beat % beatsPerBar) + 1).toFixed(2)),
      length: lengthText(n.length),
      velocity: n.velocity,
    })
    return chosenKeys.has(noteKey(n)) ? `${base}, ${t('pianoRoll.noteSelected')}` : base
  }
  /** The cells drawn: the notes in sight, and the one in the tab order wherever it is. */
  const shownNotes = $derived.by(() => {
    if (stageW === 0) return []
    return noteItems
      .map((i) => ({ item: i, rect: noteRect(i.note, shown, layout) }))
      .filter(({ item, rect }) => item.id === tabKey || (rect.x + rect.w >= 0 && rect.x <= stageW && rect.y + rect.h >= RULER_HEIGHT && rect.y <= stageH))
  })

  async function focusNote(key: string) {
    const item = noteItems.find((i) => i.id === key)
    if (!item) return
    focusKey = key
    onCursor = false
    const rect = noteRect(item.note, shown, layout)
    if (rect.x < 0 || rect.x > stageW - 8)
      view = clampView({ pxPerTick: shown.pxPerTick, scrollTick: Math.max(0, item.start - (stageW / shown.pxPerTick) * 0.1) }, stageW, totalBars * bar)
    if (rect.y < RULER_HEIGHT || rect.y + rect.h > stageH)
      scrollY = clampScrollY(layout.scrollY + rect.y - (RULER_HEIGHT + stageH) / 2, stageH, { rowHeight: ROW, top: RULER_HEIGHT })
    await tick()
    itemsEl?.querySelector<HTMLElement>(`[data-note="${CSS.escape(key)}"]`)?.focus()
  }

  // --- the free cursor (#494) -----------------------------------------------------------------------------------------
  // A pitch and a tick where no note needs to be: the only cell in the tab order of an empty clip, and where Escape on a
  // note leaves the focus. The arrows move it by the drawing step and a semitone (Shift: a bar and an octave); Enter
  // writes a note of the drawing length there (`clip.notes.add`), or goes to the note already under it.
  /** Where the cursor was left; null until it is first used (it then stands where the view looks). */
  let cursorAt = $state<{ pitch: number; tick: number } | null>(null)
  /** Whether the cursor holds the tab stop rather than a note. */
  let onCursor = $state(false)
  const cursorTab = $derived(onCursor || noteItems.length === 0)
  const cursorStep = $derived(drawing ?? SIXTEENTH)
  const cursorPos = $derived.by(() => {
    const last = floorTick(Math.max(0, clipLength - 1), cursorStep)
    if (cursorAt) return { pitch: cursorAt.pitch, tick: Math.min(cursorAt.tick, last) }
    return { pitch: pitchAt((RULER_HEIGHT + stageH) / 2, layout), tick: Math.min(floorTick(shown.scrollTick, cursorStep), last) }
  })
  /** The cursor's cell: one drawing length wide (what Enter would write), cut at the clip's end. */
  const cursorRect = $derived(
    noteRect(
      {
        start: cursorPos.tick,
        pitch: cursorPos.pitch,
        length: Math.max(1, Math.min(defaultLength(lastLength, drawing), clipLength - cursorPos.tick)),
        velocity: drawVelocity,
      },
      shown,
      layout,
    ),
  )
  const cursorText = $derived.by(() => {
    const beatsPerBar = bar / PPQ
    const beat = cursorPos.tick / PPQ
    return t('pianoRoll.cursorItem', {
      pitch: keyNames?.get(cursorPos.pitch) ?? pitchName(cursorPos.pitch),
      bar: Math.floor(beat / beatsPerBar) + 1,
      beat: Number(((beat % beatsPerBar) + 1).toFixed(2)),
    })
  })
  // Another clip: the cursor starts again where the view looks, and the notes have the tab stop again.
  $effect(() => {
    void edited?.id
    untrack(() => {
      cursorAt = null
      onCursor = false
    })
  })

  /** Brings the cursor into sight and gives it the focus. */
  async function focusCursor() {
    onCursor = true
    const rect = cursorRect
    if (rect.x < 0 || rect.x > stageW - 8)
      view = clampView({ pxPerTick: shown.pxPerTick, scrollTick: Math.max(0, cursorPos.tick - (stageW / shown.pxPerTick) * 0.1) }, stageW, totalBars * bar)
    if (rect.y < RULER_HEIGHT || rect.y + rect.h > stageH)
      scrollY = clampScrollY(layout.scrollY + rect.y - (RULER_HEIGHT + stageH) / 2, stageH, { rowHeight: ROW, top: RULER_HEIGHT })
    await tick()
    itemsEl?.querySelector<HTMLElement>('[data-cursor]')?.focus()
  }

  /** Page Up and Page Down (#494): the velocity of `chosen` by `delta`, one `clip.notes.update`, the value said. */
  function changeVelocity(chosen: readonly Note[], delta: number, focused: Note | null) {
    if (!edited) return
    if (chosen.length === 0) return announce(t('pianoRoll.saidNoSelection'))
    const changes = stepVelocity(notes, chosen, delta)
    const reference = focused ?? chosen[0]!
    if (changes.length === 0) {
      // At the limit, or refused: another note of the clip would become the same note.
      const atLimit = chosen.every((n) => (delta > 0 ? n.velocity >= MAX_VELOCITY : n.velocity <= MIN_VELOCITY))
      return announce(atLimit ? t('pianoRoll.saidVelocityLimit', { n: reference.velocity }) : t('pianoRoll.saidVelocityBlocked'))
    }
    run(updateNotes(edited.id, changes))
    picked = afterChanges(new Set(chosen.map(noteKey)), changes)
    const now = changes.find((c) => noteKey(c.from) === noteKey(reference))?.to ?? reference
    announce(t('pianoRoll.saidVelocity', { n: now.velocity }))
    // The note's key holds its velocity: the focus follows it to its new key.
    if (focused) void focusNote(noteKey(now))
  }

  function onCursorKey(event: KeyboardEvent, action: NonNullable<RovingKey>) {
    if (!edited) return
    // Delete and Escape go on to the roll: they remove or clear the selection.
    if (action.kind === 'delete' || action.kind === 'free') return
    event.preventDefault()
    event.stopPropagation()
    const at = cursorPos
    if (action.kind === 'velocity') return changeVelocity(chosenNotes, action.delta, null)
    if (action.kind === 'move') {
      // Alt+Up and Alt+Down transpose the selection from the free cursor too (#529); Alt+Left and Alt+Right move it by the drawing step (#550).
      if (action.direction === 'up' || action.direction === 'down') {
        if (chosenNotes.length === 0) return announce(t('pianoRoll.saidNoSelection'))
        transposeChosen((action.direction === 'up' ? 1 : -1) * (event.shiftKey ? 12 : 1))
      } else if (!event.shiftKey) nudgeChosen(action.direction === 'left' ? -1 : 1)
      return
    }
    if (action.kind === 'focus' || action.kind === 'extend') {
      const big = action.kind === 'extend'
      const next = moveCursor(
        { row: MAX_PITCH - at.pitch, tick: at.tick },
        action.direction,
        { tick: big ? bar : cursorStep, row: big ? 12 : 1 },
        MAX_PITCH + 1,
        clipLength,
      )
      cursorAt = { pitch: MAX_PITCH - next.row, tick: next.tick }
      void focusCursor()
      return
    }
    if (action.kind !== 'open') return
    const under = itemAt(noteItems, { row: MAX_PITCH - at.pitch, tick: at.tick })
    if (under) {
      picked = new Set([under.id])
      onCursor = false
      void focusNote(under.id)
      return
    }
    const note = drawnNote(at.tick, null, at.pitch, drawVelocity, drawing, defaultLength(lastLength, drawing), clipLength)
    if (!note || !fits(note, clipLength) || notes.some((n) => noteKey(n) === noteKey(note))) return announce(t('pianoRoll.saidNoRoom'))
    run(addNotes(edited.id, [note]))
    // Nothing to say or sound when the command did not apply (no engine, a refusal the dispatcher already shows).
    if (!notes.some((n) => noteKey(n) === noteKey(note))) return
    picked = new Set([noteKey(note)])
    onCursor = false
    announce(t('pianoRoll.saidAdded', { pitch: keyNames?.get(note.pitch) ?? pitchName(note.pitch) }))
    sound(note.pitch, note.velocity)
    release()
    void focusNote(noteKey(note))
  }

  function onNoteKey(event: KeyboardEvent) {
    const key = (event.target as HTMLElement).dataset.note
    const current = key ? noteItems.find((i) => i.id === key) : undefined
    // Ctrl or Cmd with plus or minus zooms the roll from a cell too (the page's zoom would take it otherwise).
    const zoom = zoomKey(event)
    if (zoom !== null) {
      event.preventDefault()
      event.stopPropagation()
      view = zoomAt(shown, zoom > 0 ? 1.25 : 1 / 1.25, stageW / 2, stageW, totalBars * bar)
      if (current) void focusNote(current.id)
      else if ((event.target as HTMLElement).dataset.cursor !== undefined) void focusCursor()
      return
    }
    const action = rovingKey(event)
    if (!action || !edited) return
    if ((event.target as HTMLElement).dataset.cursor !== undefined) return onCursorKey(event, action)
    if (!current) return
    event.preventDefault()
    event.stopPropagation()
    if (action.kind === 'open') {
      sound(current.note.pitch, current.note.velocity)
      release()
      return
    }
    if (action.kind === 'free') {
      // Escape frees the cursor where the note is (#494); the selection stays (Escape again on the cursor clears it).
      cursorAt = { pitch: current.note.pitch, tick: current.note.start }
      void focusCursor()
      return
    }
    if (action.kind === 'velocity') {
      const chosen = chosenKeys.has(current.id) ? chosenNotes : [current.note]
      changeVelocity(chosen, action.delta, current.note)
      return
    }
    if (action.kind === 'delete') {
      const chosen = chosenKeys.has(current.id) ? chosenNotes : [current.note]
      // The neighbour that stays: the notes deleted with this one are skipped.
      const gone = new Set(chosen.map(noteKey))
      const left = noteItems.filter((i) => i.id === current.id || !gone.has(i.id))
      const next = stepItem(left, current, 'right', true) ?? stepItem(left, current, 'left', true)
      run(removeNotes(edited.id, [...chosen]))
      picked = new Set()
      announce(t('pianoRoll.saidDeleted', { n: chosen.length }))
      if (next) void focusNote(next.id)
      else {
        // The last note gone: the cursor takes the focus where it was, so the keyboard stays in the grid.
        cursorAt = { pitch: current.note.pitch, tick: current.note.start }
        void focusCursor()
      }
      return
    }
    if (action.kind === 'move') {
      if (!chosenKeys.has(current.id)) picked = new Set([current.id])
      const chosen = chosenKeys.has(current.id) ? chosenNotes : [current.note]
      let changes: Change[] | null
      if (action.direction === 'up' || action.direction === 'down') {
        changes = transposeChanges(chosen, (action.direction === 'up' ? 1 : -1) * (event.shiftKey ? 12 : 1))
      } else {
        const nudge = drawing ?? SIXTEENTH
        changes = moveNotes(chosen, action.direction === 'left' ? -nudge : nudge, 0, clipLength)
      }
      const ok = changes ? acceptable(notes, changes) : null
      if (!ok || ok.length === 0 || ok.every((c) => noteKey(c.from) === noteKey(c.to))) return announce(t('pianoRoll.saidCannotMove'))
      run(updateNotes(edited.id, ok))
      picked = afterChanges(new Set(chosen.map(noteKey)), ok)
      const mine = ok.find((c) => noteKey(c.from) === current.id)
      if (mine) {
        announce(t('pianoRoll.saidMoved', { pitch: keyNames?.get(mine.to.pitch) ?? pitchName(mine.to.pitch) }))
        sound(mine.to.pitch, mine.to.velocity)
        release()
        void focusNote(noteKey(mine.to))
      }
      return
    }
    const target = stepItem(noteItems, current, action.direction, true)
    if (!target) return
    picked = action.kind === 'extend' ? new Set([...picked, current.id, target.id]) : new Set([target.id])
    void focusNote(target.id)
  }

  // The window loses the pointer or the tab hides: nothing may keep sounding.
  function silence() {
    keyHeld = false
    if (gesture) cancel()
    else sound(null)
  }

  // --- the velocity lane ------------------------------------------------------------------------

  let lane: HTMLDivElement | undefined = $state()

  function laneDown(event: PointerEvent) {
    if (event.button !== 0 || !edited || gesture) return
    const { x, y } = local(event, lane)
    const index = velocityTarget(notes, x, shown, picked)
    if (index === null) return
    const grabbed = notes[index]!
    const key = noteKey(grabbed)
    if (event.shiftKey) {
      const next = toggled(picked, key)
      picked = next.picked
      if (!next.on) return
    } else if (!picked.has(key)) picked = new Set([key])
    lane!.setPointerCapture(event.pointerId)
    // Absolute under the pointer, then relative: the fine mode (Ctrl, Cmd) carries on from where the value is.
    const drag = createDrag({ position: velocityPosition(y, 2, laneH - 4), home: VELOCITY_HOME, travel: laneH - 4 })
    gesture = { kind: 'velocity', clip: edited.id, grabbed, key, startX: x, startY: y, moved: false, drag, changes: [] }
    sound(grabbed.pitch, grabbed.velocity)
  }

  /** The pointer's x in the lane while it hovers it with no gesture: the bar the next drag would change is lit. */
  let hoverX = $state<number | null>(null)
  const hoverKey = $derived.by<string | null>(() => {
    if (hoverX === null || gesture || !edited) return null
    const index = velocityTarget(notes, hoverX, shown, chosenKeys)
    return index === null ? null : noteKey(notes[index]!)
  })

  function laneMove(event: PointerEvent) {
    const g = gesture
    hoverX = g ? null : local(event, lane).x
    if (!g || g.kind !== 'velocity') return
    if (!edited || edited.id !== g.clip || !chosenKeys.has(g.key)) return cancel()
    const { x, y } = local(event, lane)
    // A click or a double click moves nothing: the drag starts after a few pixels.
    if (!g.moved && Math.hypot(x - g.startX, y - g.startY) < DRAG_SLOP_PX) return
    const velocity = velocityOf(g.drag.move(g.startY - y, fineKey(event)))
    gesture = { ...g, moved: true, changes: shiftVelocity(chosenNotes, velocity - g.grabbed.velocity) }
  }

  /** The default-velocity reset: the chosen notes take it as one command (nothing when they already have it). */
  function resetVelocity() {
    if (!edited) return
    const changes = resetVelocities(notes, chosenNotes)
    if (!changes || changes.length === 0 || !stillApplies(notes, changes, clipLength)) return
    run(updateNotes(edited.id, changes))
    picked = afterChanges(picked, changes)
  }

  /** The bar under a pointer of the lane becomes the chosen note when it is not one already; false when there is none. */
  function chooseBar(event: MouseEvent): boolean {
    if (!edited || gesture) return false
    const index = velocityTarget(notes, local(event, lane).x, shown, picked)
    if (index === null) return false
    const key = noteKey(notes[index]!)
    if (!picked.has(key)) picked = new Set([key])
    return true
  }

  /**
   * The right click on the stage (#524): on a note, the actions of the selection (the note becomes the selection when it
   * is not part of it); on the empty grid, those that do not need a selection. Nothing on the ruler and when there is
   * nothing to do. The notes' cells answer the Menu key and Shift+F10 the same way.
   */
  function stageContext(event: MouseEvent) {
    if (event.defaultPrevented) return
    if (!edited || !track || gesture) return
    const { x, y } = local(event, stage)
    const cell = (event.target as Element | null)?.closest?.<HTMLElement>('[data-note]')
    const onRuler = !cell && y < RULER_HEIGHT
    if (onRuler) return
    const key =
      cell?.dataset.note ??
      (() => {
        const hit = hitNote(notes, x, y, shown, layout)
        return hit ? noteKey(notes[hit.index]!) : null
      })()
    if (key !== null && !picked.has(key)) picked = new Set([key])
    const items: MenuItem[] = []
    const some = key !== null || chosenNotes.length > 0
    const mod = (letter: string) => chordText(['Mod', letter])
    if (notes.length > 0) items.push({ id: 'quantize', label: t('pianoRoll.quantize'), shortcut: quantizeLabel, run: quantizeNow })
    if (some) {
      items.push(
        { id: 'duplicate', label: t('pianoRoll.duplicate'), shortcut: mod('D'), run: duplicateChosen },
        { id: 'copy', label: t('pianoRoll.copy'), shortcut: mod('C'), separator: true, run: copyChosen },
        {
          id: 'cut',
          label: t('pianoRoll.cut'),
          shortcut: mod('X'),
          run: () => {
            if (copyChosen()) removeSelected()
          },
        },
      )
    }
    if (boardNotes().length > 0) items.push({ id: 'paste', label: t('pianoRoll.paste'), shortcut: mod('V'), run: pasteBoard })
    if (some) items.push({ id: 'delete', label: t('pianoRoll.delete'), shortcut: chordText(['Delete']), separator: true, run: removeSelected })
    ui.openMenu(event, items, t('pianoRoll.menu'))
  }

  /** The right click on the velocity lane: the bar under it joins the selection, and the shared menu offers the default velocity. */
  function laneContext(event: MouseEvent) {
    if (!chooseBar(event)) return ui.openMenu(event, [])
    ui.openMenu(
      event,
      [{ id: 'default', label: t('controls.default'), detail: String(DEFAULT_VELOCITY), testid: 'value-menu-default', run: resetVelocity }],
      t('pianoRoll.velocity'),
    )
  }

  // --- the keyboard column ----------------------------------------------------------------------

  let keys: HTMLDivElement | undefined = $state()
  let keyHeld = false

  function keysDown(event: PointerEvent) {
    if (event.button !== 0 || !track) return
    const { y } = local(event, keys)
    if (y < RULER_HEIGHT) return
    keys!.setPointerCapture(event.pointerId)
    keyHeld = true
    sound(pitchAt(y, layout))
  }
  function keysMove(event: PointerEvent) {
    if (keyHeld) sound(pitchAt(local(event, keys).y, layout))
  }
  function keysUp() {
    if (!keyHeld) return
    keyHeld = false
    release()
  }

  // --- wheel, toolbar ---------------------------------------------------------------------------

  // A plain vertical wheel scrolls the pitches; the grid keeps Shift and Ctrl for time and zoom.
  $effect(() => {
    const element = stage
    if (!element) return
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey || event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return
      event.preventDefault()
      const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? stageH : 1
      scrollY = clampScrollY(layout.scrollY + event.deltaY * scale, stageH, { rowHeight: ROW, top: RULER_HEIGHT })
    }
    element.addEventListener('wheel', onWheel, { passive: false })
    return () => element.removeEventListener('wheel', onWheel)
  })

  function setBars(bars: number) {
    if (!edited) return
    const clamped = Math.min(MAX_BARS, Math.max(1, Math.round(bars)))
    const length = clamped * bar
    if (length === clipLength) return
    const commands = lengthCommands(edited, length)
    if (commands.length === 1) return run(commands[0]!)
    run(beginGroup())
    for (const command of commands) run(command)
    run(endGroup())
  }

  /** The commands that give a clip another length: a loop over the whole clip stays over the whole clip when the clip grows (a shorter clip trims it itself). */
  function lengthCommands(target: MidiClip, length: number): Command[] {
    const whole = target.looping?.start === 0 && target.looping.end === target.length
    if (!whole || length < target.length) return [setClipLength(target.id, length)]
    return [setClipLength(target.id, length), setClipLoop(target.id, { start: 0, end: length })]
  }

  function onLengthChange(event: Event & { currentTarget: HTMLInputElement }) {
    const input = event.currentTarget
    // An empty field is not a length: Number('') is 0, which would shorten the clip to one bar.
    const value = input.value.trim() === '' ? Number.NaN : Number(input.value)
    if (Number.isFinite(value)) setBars(value)
    input.value = String(totalBars)
  }

  const toggleLoop = () => edited && run(setClipLoop(edited.id, edited.looping ? null : { start: 0, end: clipLength }))

  // --- quantize, transpose, duplicate, copy and paste (PN-2) -----------------------------------------------------
  let quantizeGrid = $state<QuantizeGrid>(DEFAULT_QUANTIZE_GRID)
  let quantizeStrength = $state(DEFAULT_QUANTIZE_STRENGTH)
  const gridTicks = $derived(QUANTIZE_GRIDS.find((g) => g.id === quantizeGrid)?.ticks ?? 240)

  /** Quantizes the chosen notes, or every note when none is chosen. Nothing is sent when no start would move. */
  function quantizeNow() {
    // Not in the middle of a drag: the drag commits positions computed before the quantize.
    if (!edited || gesture) return
    const chosen = chosenNotes.length > 0
    const changes = quantizeChanges(chosen ? chosenNotes : notes, gridTicks, quantizeStrength, clipLength).filter((c) => noteKey(c.from) !== noteKey(c.to))
    const said = quantizeSaid(changes.length, chosen ? chosenNotes.length : notes.length)
    if (changes.length > 0) {
      run(quantizeNotes(edited.id, chosen ? [...chosenNotes] : null, gridTicks, quantizeStrength))
      if (chosen) picked = afterChanges(picked, changes)
    }
    // The result, in words, next to the button and in the live region: a quantize that moves nothing says so.
    const text = t(said.key, { n: said.n })
    quantizeNote = text
    announce(text)
    clearTimeout(quantizeTimer)
    quantizeTimer = setTimeout(() => (quantizeNote = null), 3000)
  }
  /** The key as written on the person's keyboard: the quantize works by position (`KeyQ`), so it is "A" on an AZERTY keyboard. */
  const quantizeLabel = $derived(positionKey(QUANTIZE_CODE, ui.keyLayout))
  let quantizeNote = $state<string | null>(null)
  let quantizeTimer: ReturnType<typeof setTimeout> | undefined
  onDestroy(() => clearTimeout(quantizeTimer))
  function onDrawVelocityChange(event: Event & { currentTarget: HTMLInputElement }) {
    const input = event.currentTarget
    drawVelocity = parseDrawVelocity(input.value, drawVelocity)
    ui.remember('piano-roll-draw-velocity', drawVelocity)
    input.value = String(drawVelocity)
  }
  function onStrengthChange(event: Event & { currentTarget: HTMLInputElement }) {
    const input = event.currentTarget
    const value = input.value.trim() === '' ? Number.NaN : Number(input.value)
    if (Number.isFinite(value)) quantizeStrength = Math.min(100, Math.max(0, Math.round(value)))
    input.value = String(quantizeStrength)
  }

  /** Moves the chosen notes by the drawing step (`direction` -1 or 1), heard once; said when nothing can move. */
  function nudgeChosen(direction: -1 | 1) {
    if (!edited) return
    if (chosenNotes.length === 0) return announce(t('pianoRoll.saidNoSelection'))
    const changes = moveNotes(chosenNotes, direction * (drawing ?? SIXTEENTH), 0, clipLength)
    const ok = changes ? acceptable(notes, changes) : null
    if (!ok || ok.length === 0 || ok.every((c) => noteKey(c.from) === noteKey(c.to))) return announce(t('pianoRoll.saidCannotMove'))
    const chosen = chosenNotes
    run(updateNotes(edited.id, ok))
    picked = afterChanges(new Set(chosen.map(noteKey)), ok)
    sound(ok[0]!.to.pitch, ok[0]!.to.velocity)
    release()
  }

  /** Moves the chosen notes by semitones, heard once; nothing when one would leave the keyboard. */
  function transposeChosen(semitones: number) {
    if (!edited) return
    const changes = transposeChanges(chosenNotes, semitones)
    if (!changes || changes.length === 0) {
      // Past the top or the bottom of the keyboard: said, as on a note's cell (nothing to say with no selection).
      if (chosenNotes.length > 0) announce(t('pianoRoll.saidCannotMove'))
      return
    }
    run(transposeNotes(edited.id, [...chosenNotes], semitones))
    picked = afterChanges(picked, changes)
    sound(changes[0]!.to.pitch, changes[0]!.to.velocity)
    release()
  }

  /**
   * Sends notes that end at `end` through `send`, growing the clip first to the next whole bar when they do not fit (one
   * history entry for both). False when the clip cannot hold them.
   */
  function withRoomFor(end: number, send: () => void): boolean {
    if (!edited) return false
    const length = lengthFor(end, clipLength, bar)
    if (length === null) return false
    if (length === clipLength) {
      send()
      return true
    }
    run(beginGroup())
    for (const command of lengthCommands(edited, length)) run(command)
    // The project may have refused the new length (a placement next to the clip): then there is no room, and nothing is added.
    const grown = clipLength >= length
    if (grown) send()
    run(endGroup())
    return grown
  }

  /** Copies of the chosen notes right after the selection, which they become. */
  function duplicateChosen() {
    if (!edited || chosenNotes.length === 0) return
    const source = [...chosenNotes]
    const offset = duplicateOffset(source, bar)
    if (offset <= 0) return
    const copies = source.map((n) => ({ ...n, start: n.start + offset }))
    const id = edited.id
    if (withRoomFor(endOf(copies), () => run(duplicateNotes(id, source, offset)))) picked = new Set(copies.map(noteKey))
  }

  function copyChosen(): boolean {
    if (chosenNotes.length === 0) return false
    setBoard(toBoard(chosenNotes))
    return true
  }

  /** The notes put aside, at the play position of the clip (snapped down to the grid), else at its start. */
  function pasteBoard() {
    const board = boardNotes()
    if (!edited || board.length === 0) return
    const pasted = fromBoard(board, floorTick(playhead ?? 0, step))
    const id = edited.id
    if (withRoomFor(endOf(pasted), () => run(addNotes(id, pasted)))) picked = new Set(pasted.map(noteKey))
  }
</script>

<svelte:window onblur={silence} onkeydown={onWindowKeyDown} />
<svelte:document onvisibilitychange={silence} />

{#if !track}
  <p class="empty" data-testid="roll-empty">{t(project.selected ? 'pianoRoll.audioTrack' : 'pianoRoll.noTrack')}</p>
{:else if !edited}
  <div class="empty" data-testid="roll-empty">
    <p>{t('pianoRoll.noClip')}</p>
    <button class="primary" data-testid="roll-create-clip" onclick={() => run(createClipAtStart(track.id))}>{t('pianoRoll.createClip')}</button>
  </div>
{:else}
  <div
    class="roll"
    bind:this={rollEl}
    data-testid="piano-roll"
    data-clip={edited.id}
    data-note-count={notes.length}
    data-taking={taking.length}
    data-selected-count={chosenNotes.length}
    data-clip-length={clipLength}
    data-key-names={keyNames ? JSON.stringify(Object.fromEntries(keyNames)) : ''}
    data-loop={edited.looping ? `${edited.looping.start}-${edited.looping.end}` : ''}
    data-notes={notes.length <= 32 ? JSON.stringify(notes) : undefined}
    data-first-note={notes[0] ? JSON.stringify(notes[0]) : undefined}
    data-selection={chosenNotes.length <= 32 ? JSON.stringify(chosenNotes) : undefined}
  >
    <div class="toolbar">
      {#if entries.length > 1}
        <select
          aria-label={t('pianoRoll.clip')}
          data-testid="roll-clip"
          value={entryKey}
          onchange={(e) => {
            const entry = entries.find((x) => x.key === e.currentTarget.value)
            if (entry) ui.editClip(entry.clip.id, entry.placement?.id ?? null)
          }}
        >
          {#each entries as entry (entry.key)}
            <option value={entry.key}>{listLabel(entry)}</option>
          {/each}
        </select>
      {/if}
      {#if renaming}
        <input
          class="rename"
          data-testid="roll-clip-rename"
          aria-label={t('pianoRoll.clipName')}
          maxlength="200"
          bind:this={nameField}
          bind:value={draft}
          onkeydown={onRenameKey}
          onblur={() => finishRename(true)}
        />
      {:else}
        <button
          class="name"
          type="button"
          title={t('pianoRoll.renameHint')}
          data-testid="roll-clip-name"
          ondblclick={startRename}
          onkeydown={(e) => e.key === 'F2' && startRename()}>{clipName}</button
        >
      {/if}
      <span class="origin" data-testid="roll-clip-origin">{origin}</span>
      <div class="group" role="radiogroup" aria-label={t('pianoRoll.tools')}>
        <button role="radio" aria-checked={tool === 'draw'} title={t('pianoRoll.drawHint')} data-testid="roll-tool-draw" onclick={() => (tool = 'draw')}
          >{t('pianoRoll.draw')}</button
        >
        <button role="radio" aria-checked={tool === 'select'} title={t('pianoRoll.selectHint')} data-testid="roll-tool-select" onclick={() => (tool = 'select')}
          >{t('pianoRoll.select')}</button
        >
      </div>
      <label class="field">
        <span>{t('pianoRoll.snap')}</span>
        <select aria-label={t('pianoRoll.snap')} data-testid="roll-snap" bind:value={snap}>
          {#each SNAP_SETTINGS as setting (setting)}
            <option value={setting}>{t(`pianoRoll.snap.${setting}`)}</option>
          {/each}
        </select>
      </label>
      <div class="field" role="group" aria-label={t('pianoRoll.length')}>
        <span>{t('pianoRoll.length')}</span>
        <button class="step" aria-label={t('pianoRoll.shorter')} data-testid="roll-shorter" disabled={totalBars <= 1} onclick={() => setBars(totalBars - 1)}
          >−</button
        >
        <input
          type="number"
          min="1"
          max={MAX_BARS}
          step="1"
          value={totalBars}
          aria-label={t('pianoRoll.length')}
          data-testid="roll-length"
          onchange={onLengthChange}
        />
        <button
          class="step"
          aria-label={t('pianoRoll.longer')}
          data-testid="roll-longer"
          disabled={totalBars >= MAX_BARS}
          onclick={() => setBars(totalBars + 1)}>+</button
        >
      </div>
      <button aria-pressed={edited.looping !== null} title={t('pianoRoll.loopHint')} data-testid="roll-loop" onclick={toggleLoop}>{t('pianoRoll.loop')}</button>
      <button aria-pressed={showOthers} title={t('pianoRoll.othersHint')} data-testid="roll-others" onclick={() => (showOthers = !showOthers)}
        >{t('pianoRoll.others')}</button
      >
      <button disabled={chosenNotes.length === 0} data-testid="roll-delete" onclick={removeSelected}>{t('pianoRoll.delete')}</button>
      <div class="group" role="group" aria-label={t('pianoRoll.edit')}>
        <button title={t('pianoRoll.duplicateHint')} disabled={chosenNotes.length === 0} data-testid="roll-duplicate" onclick={duplicateChosen}
          >{t('pianoRoll.duplicate')}</button
        >
        <button
          title={t('pianoRoll.transposeDownHint')}
          aria-label={t('pianoRoll.transposeDown')}
          disabled={chosenNotes.length === 0}
          data-testid="roll-transpose-down"
          onclick={() => transposeChosen(-1)}>−1</button
        >
        <button
          title={t('pianoRoll.transposeUpHint')}
          aria-label={t('pianoRoll.transposeUp')}
          disabled={chosenNotes.length === 0}
          data-testid="roll-transpose-up"
          onclick={() => transposeChosen(1)}>+1</button
        >
      </div>
      <label class="field" title={t('pianoRoll.drawVelocityHint')}>
        <span>{t('pianoRoll.drawVelocity')}</span>
        <input
          type="number"
          min={MIN_VELOCITY}
          max={MAX_VELOCITY}
          step="1"
          value={drawVelocity}
          aria-label={t('pianoRoll.drawVelocity')}
          data-testid="roll-draw-velocity"
          onchange={onDrawVelocityChange}
        />
      </label>
      <div class="field" role="group" aria-label={t('pianoRoll.quantize')}>
        <select aria-label={t('pianoRoll.quantizeGrid')} data-testid="roll-quantize-grid" bind:value={quantizeGrid}>
          {#each QUANTIZE_GRIDS as grid (grid.id)}
            <option value={grid.id}>{t(`pianoRoll.qgrid.${grid.id}`)}</option>
          {/each}
        </select>
        <input
          type="number"
          min="0"
          max="100"
          step="1"
          value={quantizeStrength}
          aria-label={t('pianoRoll.quantizeStrength')}
          title={t('pianoRoll.quantizeStrength')}
          data-testid="roll-quantize-strength"
          onchange={onStrengthChange}
        />
        <span>%</span>
        <button
          title={withShortcut(t('pianoRoll.quantizeHint'), [quantizeLabel])}
          aria-keyshortcuts={quantizeLabel}
          disabled={notes.length === 0}
          data-testid="roll-quantize"
          onclick={quantizeNow}>{t('pianoRoll.quantize')}</button
        >
        {#if quantizeNote}<span class="said" data-testid="roll-quantize-said">{quantizeNote}</span>{/if}
      </div>
      <span class="count" data-testid="roll-count" title={chosenNotes.length > 0 ? t('pianoRoll.selected', { n: chosenNotes.length }) : undefined}
        >{t('pianoRoll.count', { n: notes.length })}</span
      >
    </div>

    <div class="board" style:--keys="{keyNames ? NAMED_KEYS_WIDTH : KEYS_WIDTH}px" style:--lane="{LANE_HEIGHT}px">
      <div
        class="keys"
        bind:this={keys}
        role="presentation"
        data-testid="roll-keys"
        onpointerdown={keysDown}
        onpointermove={keysMove}
        onpointerup={keysUp}
        onpointercancel={keysUp}
        onlostpointercapture={keysUp}
      >
        <Surface label={t('pianoRoll.keyboard')} draw={(ctx, w, h, colors, font) => drawKeys(ctx, w, h, layout, colors, font, activePitches, keyNames)} />
      </div>
      <div
        class="stage"
        bind:this={stage}
        bind:clientWidth={stageW}
        bind:clientHeight={stageH}
        role="presentation"
        data-testid="roll-stage"
        data-scroll-y={layout.scrollY}
        data-cursor={gesture ? undefined : cursor}
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={() => finish(false)}
        onpointerleave={() => {
          if (!gesture) noteHover = null
        }}
        onlostpointercapture={() => gesture && finish(false)}
        oncontextmenu={stageContext}
        onkeydown={onKeyDown}
      >
        <TimeGrid bind:this={grid} {signature} {totalBars} bind:view {snap} {playhead} drawBody={body} label={t('pianoRoll.grid', { name: clipName })} />
        {#if gestureLines && pointer}
          <GestureLabel x={pointer.x} y={pointer.y} width={stageW} height={stageH} lines={gestureLines} />
        {/if}
        <!-- The notes as named cells over the canvas (#471): the keyboard's and the screen reader's way to them. The pointer goes through. -->
        <!-- The cells are the focusable parts of the grid (roving tabindex), as in the launch grid; the grid only listens for their keys. -->
        <!-- svelte-ignore a11y_interactive_supports_focus -->
        <div
          class="items"
          bind:this={itemsEl}
          role="grid"
          aria-label={t('pianoRoll.notesGrid', { name: clipName })}
          aria-multiselectable="true"
          data-testid="roll-items"
          style:clip-path="inset({RULER_HEIGHT}px 0 0 0)"
          onkeydown={onNoteKey}
          onfocusout={(event) => {
            const to = event.relatedTarget as Node | null
            if (to && !itemsEl?.contains(to)) itemsHadFocus = false
          }}
          onfocusin={(event) => {
            itemsHadFocus = true
            const target = event.target as HTMLElement
            if (target.dataset.note) {
              focusKey = target.dataset.note
              onCursor = false
            } else if (target.dataset.cursor !== undefined) {
              // Pinned where it was found: it no longer follows the view.
              cursorAt ??= cursorPos
              onCursor = true
            }
          }}
        >
          <div role="row">
            {#if stageW > 0}
              <div
                class="item cursor"
                role="gridcell"
                tabindex={cursorTab ? 0 : -1}
                aria-selected="false"
                aria-label={cursorText}
                data-cursor=""
                data-testid="roll-cursor"
                data-pitch={cursorPos.pitch}
                data-tick={cursorPos.tick}
                style:left="{cursorRect.x}px"
                style:top="{cursorRect.y}px"
                style:width="{cursorRect.w}px"
                style:height="{cursorRect.h}px"
              ></div>
            {/if}
            {#each shownNotes as cell (cell.item.id)}
              <div
                class="item"
                role="gridcell"
                tabindex={cell.item.id === tabKey && !cursorTab ? 0 : -1}
                aria-selected={chosenKeys.has(cell.item.id)}
                aria-label={noteText(cell.item.note)}
                data-note={cell.item.id}
                data-testid="roll-item"
                style:left="{cell.rect.x}px"
                style:top="{cell.rect.y}px"
                style:width="{cell.rect.w}px"
                style:height="{cell.rect.h}px"
              ></div>
            {/each}
          </div>
        </div>
        <p class="sr-only" aria-live="polite" data-testid="roll-said" data-n={said.n}>{said.text}</p>
      </div>
      <div class="corner"></div>
      <div
        class="lane"
        bind:this={lane}
        bind:clientHeight={laneH}
        role="presentation"
        data-testid="roll-lane"
        data-hover={hoverKey ?? undefined}
        onpointerdown={laneDown}
        onpointermove={laneMove}
        onpointerup={onPointerUp}
        onpointercancel={() => finish(false)}
        onpointerleave={() => (hoverX = null)}
        title={t('pianoRoll.velocityHint')}
        ondblclick={(event) => {
          if (chooseBar(event)) resetVelocity()
        }}
        oncontextmenu={laneContext}
      >
        <Surface
          label={t('pianoRoll.velocity')}
          draw={(ctx, w, h, colors) => drawLane(ctx, w, h, shown, notes, chosenKeys, ghosts, hidden, clipLength, colors, hoverKey, trackColor)}
        />
      </div>
    </div>
  </div>
{/if}

<style>
  /* The notes' cells: invisible over the canvas, except the keyboard's focus ring; the ruler stays clear. */
  .items {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
  }
  .item {
    position: absolute;
    border-radius: var(--fk-radius-work);
  }
  .item:focus-visible {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: 1px;
  }
  /* The free cursor (#494): seen only while it has the keyboard, as a tinted cell where Enter would write the note. */
  .cursor:focus-visible {
    background: var(--fk-accent-tint);
    outline-style: dashed;
  }
  .roll {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--fk-space-2) var(--fk-space-3);
    padding: var(--fk-space-1) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
    font-size: 12px;
  }
  .name {
    font-weight: 600;
    background: none;
    border: 0;
    padding: 0 var(--fk-space-1);
    color: inherit;
    font: inherit;
    font-weight: 600;
  }
  .origin {
    color: var(--fk-text-muted);
  }
  .rename {
    font: inherit;
    width: 14em;
  }
  .group {
    display: flex;
    gap: 2px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
    color: var(--fk-text-muted);
  }
  button,
  select,
  input {
    height: 24px;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: var(--fk-surface-2);
    color: var(--fk-text);
    font: inherit;
  }
  button {
    padding: 0 var(--fk-space-2);
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  /* On: a fill in the text colour (one voice: the violet is for the selection, the focus, the main action and Boost). */
  button[aria-pressed='true'],
  button[aria-checked='true'] {
    background: var(--fk-text);
    border-color: var(--fk-text);
    color: var(--fk-bg);
  }
  .step {
    width: 24px;
    padding: 0;
  }
  input[type='number'] {
    width: 52px;
    padding: 0 var(--fk-space-1);
    text-align: center;
  }
  .said {
    color: var(--fk-text-muted);
    white-space: nowrap;
  }
  .count {
    margin-left: auto;
    /* Notes only: the selection is in the title, so that a selection never widens the last line and wraps the toolbar (the grid would move). */
    white-space: nowrap;
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
  }
  .board {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: var(--keys) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) var(--lane);
  }
  .keys {
    border-right: 1px solid var(--fk-line);
    cursor: pointer;
    touch-action: none;
  }
  .stage {
    /* The notes' cells (#471) are placed in the stage's own coordinates. */
    position: relative;
    min-width: 0;
    min-height: 0;
    touch-action: none;
    user-select: none;
  }
  .stage[data-cursor='move'] :global(.timegrid) {
    cursor: grab;
  }
  .stage[data-cursor='resize'] :global(.timegrid) {
    cursor: ew-resize;
  }
  .corner {
    border-top: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  /* No border on the lane: the pointer's y and the drawn bars share one origin (its top line is drawn on the canvas). */
  .lane {
    touch-action: none;
    cursor: ns-resize;
  }
  .empty {
    margin: 0;
    padding: var(--fk-space-4);
    color: var(--fk-text-muted);
  }
  .empty p {
    margin: 0 0 var(--fk-space-2);
  }
  .primary {
    border-color: var(--fk-accent-fg);
    background: var(--fk-accent-tint);
  }
</style>
