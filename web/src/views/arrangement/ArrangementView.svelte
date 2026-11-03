<script lang="ts">
  /**
   * The arrangement (AR-4): the clips placed on the tracks along the shared time grid. A gesture is previewed
   * locally and sent as ONE command when it ends, so a drag is one history entry and one undo, and no command
   * goes while the pointer moves. The selection, the snap, the zoom and the scroll are the page's view state,
   * not project data.
   */
  import { onDestroy, onMount, tick, untrack } from 'svelte'
  import {
    barStart,
    createClipAt,
    deletePlacements,
    deleteTime,
    duplicatePlacements,
    insertTime,
    movePlacements,
    moveToTrack,
    repeatPlacement,
    resizePlacement,
    splitPlacement,
  } from '../../lib/arrangement-commands'
  import { addPoints, createLane, createMasterLane, deleteLane, removePoints, updatePoints } from '../../lib/automation-commands'
  import type { Command, Shape } from '../../lib/generated/protocol'
  import { keepsBrowserMenu, type MenuItem } from '../../lib/context-menu'
  import { DRAG_SLOP_PX, fineKey } from '../../lib/continuous'
  import { chordText } from '../../lib/shortcuts'
  import TimeGrid from '../../lib/components/timegrid/TimeGrid.svelte'
  import ValueField from '../../lib/components/ValueField.svelte'
  import { deviceText, labelKey } from '../../lib/devices'
  import {
    DEFAULT_PX_PER_TICK,
    DEFAULT_TOTAL_BARS,
    SNAP_SETTINGS,
    clampView,
    snapStep,
    snapTick,
    tickToX,
    totalTicksOf,
    visibleRange,
    xToTick,
    zoomAt,
    type SnapSetting,
    type View,
  } from '../../lib/components/timegrid/grid'
  import { RULER_HEIGHT, type BodyFrame } from '../../lib/components/timegrid/render'
  import { arrangementShortcut, isTextTarget, repeatShortcut } from '../../lib/keyboard'
  import { MASTER_COLOR, isMidi, trackColor, trackLabel, type DragHooks } from '../../lib/project/tracks'
  import { offTimeline } from '../../lib/off-timeline'
  import { backTrackToArrangement } from '../../lib/scene-commands'
  import { useStores } from '../../lib/stores'
  import { MASTER } from '../../lib/stores/automated.svelte'
  import { MASTER_METER } from '../../lib/stores'
  import { beginGroup, endGroup, moveTrack, selectTrack } from '../../lib/track-commands'
  import LevelMeter from '../../lib/components/LevelMeter.svelte'
  import { seek, setLoopRegion, toggleLoop } from '../../lib/transport-commands'
  import { liveTick, settle } from '../piano-roll/playhead'
  import { MAX_GOTO_BAR, PPQ, barTicks, beatTicks } from '../transport/time'
  import { itemAt, moveCursor, rovingKey, step as stepItem, timelineCursorStep, zoomKey, type Cursor, type Direction, type RovingKey } from '../../lib/roving'
  import { CLIP_DRAG_TYPE, clipTrackType, placeCommand, poolOf } from '../../lib/clip-pool'
  import { addMarker, deleteMarker, moveMarker, renameMarker } from '../../lib/arrangement-commands'
  import {
    blocksOf,
    clampScrollY,
    maxScrollY,
    copyShift,
    hitBlock,
    isFree,
    lastEnd,
    moveDelta,
    moved,
    clampRows,
    trackTargets,
    passCount,
    markerJump,
    openingView,
    wholeSongView,
    type MarkerView,
    resizeTo,
    resized,
    laneHeight,
    laneTop,
    BLOCK_INSET,
    rowAt,
    splittable,
    type Block,
  } from './arrangement'
  import { drawArrangement } from './draw'
  import { drawAutomation, type LaneScene } from './automation/draw'
  import {
    asPoint,
    dragDelta,
    hitPoint,
    moveSelection,
    newPoint,
    planMove,
    pointsInBox,
    posOfY,
    previewPoints,
    yOfPos,
    type Delta,
  } from './automation/gestures'
  import { CLIP_ROW, CLIP_ROW_MIN, clampClipRow, layoutOf, rowsBetween, type Row } from './automation/layout'
  import LaneHeader from './LaneHeader.svelte'
  import BarDialog from './BarDialog.svelte'
  import { lanesOf, optionsOf, pointAtTick, pointsOf, targetKey, valueAt, type LaneInfo, type LanePoint, type Labels } from './automation/model'

  const { project, engine, ui, dispatcher, meters } = useStores()
  const t = ui.t
  const run = (command: Command) => dispatcher.run(command)

  /** Bars the grid keeps free after the last clip, so that there is always room to place one. */
  const SPARE_BARS = 16

  type Gesture =
    | {
        kind: 'move' | 'copy'
        grabbed: Block
        ids: string[]
        snapshot: Block[]
        grabTick: number
        delta: number
        /** Tracks down from the grabbed clip's own (negative is up): a move only, a copy stays on its track. */
        rows: number
        moved: boolean
        linked: boolean
        startX: number
        startY: number
      }
    | { kind: 'resize'; grabbed: Block; edge: 'start' | 'end'; to: number | null; moved: boolean; startX: number }
    | { kind: 'ruler'; from: number; to: number; moved: boolean; startX: number; select: boolean }
    | {
        // A point of a lane (and the points selected with it) being dragged.
        kind: 'point'
        lane: string
        trackId: string
        grabbed: LanePoint
        /** The selected points when the drag started, earliest first, and the whole lane then. */
        snapshot: LanePoint[]
        all: readonly LanePoint[]
        grabTick: number
        grabPos: number
        delta: Delta
        vertical: boolean
        moved: boolean
        startX: number
        startY: number
      }
    | {
        // A press on an empty part of a lane: a click adds a point, a drag draws a box.
        kind: 'box'
        lane: string
        trackId: string
        startX: number
        startY: number
        x: number
        y: number
        moved: boolean
        add: boolean
      }

  const entries = $derived(project.tracks)
  /** The master as a pseudo-entry at the bottom of the layout, with the sentinel id `MASTER`. */
  const masterEntry = $derived({
    id: MASTER,
    track: {
      devices: project.view?.project.master_devices ?? {},
      automation: project.view?.project.master_automation ?? {},
    },
  })
  /** All entries for the layout: the real tracks, then the master row pinned at the bottom. */
  const allEntries = $derived([...entries, masterEntry])
  const entryById = $derived(new Map(allEntries.map((e) => [e.id, e])))
  const trackHeightOf = (id: string) => (entryById.get(id)?.track as { height?: number | null } | undefined)?.height
  // The tracks the launch grid holds: their timeline is silent until they are given back.
  const detached = $derived(engine.started ? offTimeline(engine.feedback, project.view?.slots) : new Set<string>())
  const blocks = $derived(blocksOf(entries))
  /** The tracks that make no sound: muted, or silenced by another track's solo. */
  const silent = $derived(new Set(entries.filter((e) => e.track.mixer.mute || project.silencedBySolo(e.id)).map((e) => e.id)))
  const signature = $derived(project.view?.project.meter[0]?.signature ?? { numerator: 4, denominator: 4 })
  const bar = $derived(barTicks(signature))
  /** The tick of the last point of any lane: a lane may reach further than the clips (an agent, a fade-out), and must stay in reach. */
  const lastPoint = $derived.by(() => {
    let last = 0
    for (const e of allEntries) {
      for (const lane of Object.values(e.track.automation)) {
        if (lane) for (const tick in lane.points) last = Math.max(last, Number(tick))
      }
    }
    return last
  })
  const totalBars = $derived(Math.max(DEFAULT_TOTAL_BARS, Math.ceil(Math.max(lastEnd(blocks), lastPoint) / bar) + SPARE_BARS))
  const transport = $derived(project.view?.project.transport ?? null)
  /** The section markers (#473), in timeline order. */
  const markers = $derived<MarkerView[]>(
    Object.entries(project.view?.project.markers ?? {})
      .flatMap(([id, m]) => (m ? [{ id, tick: m.tick, name: m.name }] : []))
      .sort((a, b) => a.tick - b.tick || (a.id < b.id ? -1 : 1)),
  )
  /** Where the song ends: its last clip, automation point or marker. */
  const songEnd = $derived(Math.max(lastEnd(blocks), lastPoint, markers.at(-1)?.tick ?? 0))

  // View state.
  let snap = $state<SnapSetting>('auto')
  // Zoom, scroll and the open lanes come back as the view was left (it is torn down when another view is shown),
  // and with the project's layout when it is opened again (#473).
  const layoutRev = ui.layoutRev
  const left = ui.recall<{ view?: View; scrollY?: number; open?: string[] }>('arrangement')
  let view = $state<View>(validView(left?.view) ?? { pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 })
  /** No remembered view: the arrangement opens on 16 to 32 bars as soon as it knows its width (#473). */
  let openToFit = !validView(left?.view)
  let scrollY = $state(typeof left?.scrollY === 'number' && Number.isFinite(left.scrollY) ? left.scrollY : 0)
  const keep = () => ui.remember('arrangement', { view, scrollY, open: [...open] }, layoutRev)
  onDestroy(keep)
  $effect(keep)

  /** A remembered view that still reads as one (it may come from an older layout). */
  function validView(v: View | undefined): View | null {
    return v && Number.isFinite(v.pxPerTick) && v.pxPerTick > 0 && Number.isFinite(v.scrollTick) ? v : null
  }
  let stageW = $state(0)
  let stageH = $state(0)
  let picked = $state.raw<ReadonlySet<string>>(new Set())
  let gesture = $state.raw<Gesture | null>(null)
  let cursor = $state<'default' | 'move' | 'resize' | 'add'>('default')
  /** Whether the view follows the play position while the transport runs. A hand scroll ends it; Follow or the next Play restores it. */
  let follow = $state(true)

  const shown = $derived(clampView(view, stageW, totalBars * bar))
  const step = $derived(snapStep(snap, signature, shown.pxPerTick))
  // --- the automation lanes (AM-4) -----------------------------------------------------------------
  // Which tracks show their lanes is the page's view state; the lanes themselves are in the project.
  // The open lanes of the project's layout, kept to the tracks it still has (and the master).
  let open = $state.raw<ReadonlySet<string>>(
    new Set((Array.isArray(left?.open) ? left.open : []).filter((id) => typeof id === 'string' && untrack(() => allEntries.some((e) => e.id === id)))),
  )
  /** The selected points: one lane's, by tick (kept to the points the lane still has). */
  let autoPick = $state.raw<{ lane: string; ticks: ReadonlySet<number> } | null>(null)
  /** The point under the pointer, for its readout. */
  let hover = $state.raw<{ lane: string; tick: number } | null>(null)
  /** The context menu on points, at a client position. */
  let menu = $state.raw<{ x: number; y: number } | null>(null)
  /** The point whose value is being typed. */
  let editing = $state.raw<{ lane: string; tick: number; x: number; y: number } | null>(null)
  let editAnchor: HTMLDivElement | undefined = $state()

  const labels = $derived<Labels>({
    mixer: t('automation.mixer'),
    mixerParam: (param) => t(param === 'gain' ? 'automation.volume' : 'automation.pan'),
    param: (d, p) => deviceText(ui.lang, labelKey(d, p.key)) ?? p.key,
    choice: (d, key) => deviceText(ui.lang, labelKey(d, key)) ?? key,
  })
  /** The lanes each unfolded track (and the master) shows. */
  const shownLanes = $derived(new Map(allEntries.map((e) => [e.id, open.has(e.id) ? lanesOf(e.track, labels) : ([] as LaneInfo[])])))
  /** The master row's height: the `row` token (28 px), the lowest a track's row goes. */
  const MASTER_ROW = CLIP_ROW_MIN
  // The rows' height: the `lane` token of the stylesheet (44 px, 28 px in the compact density), else each track's own, kept in the project (#471, #490).
  let laneBase = $state(CLIP_ROW)
  const layout = $derived(
    layoutOf(
      allEntries,
      open,
      (id) => (shownLanes.get(id) ?? []).map((l) => l.id),
      // The master holds no clips: its row stays at the control line's height, so that 16 tracks and the master fit at 1080p.
      (id) => (id === MASTER ? MASTER_ROW : ui.laneHeightPreview?.track === id ? ui.laneHeightPreview.height : (trackHeightOf(id) ?? undefined)),
      laneBase,
    ),
  )
  // Read again when the density changes (`data-density` on the page's root or body), so that the compact rows follow.
  $effect(() => {
    const read = () => (laneBase = clampClipRow(parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--fk-lane'))))
    read()
    const observer = new MutationObserver(read)
    for (const node of [document.documentElement, document.body]) observer.observe(node, { attributes: true, attributeFilter: ['data-density'] })
    return () => observer.disconnect()
  })

  // --- reordering by drag: a row's colour bar picks it up, dropping it on another row puts it at that row's place
  // (one `track.move` on release; nothing is sent while the pointer moves). The inspector's Move up and Move down are the keyboard's way.
  let dragId = $state<string | null>(null)
  let overId = $state<string | null>(null)
  const drag: DragHooks = {
    start: (id) => (dragId = id),
    over: (id) => {
      if (dragId) overId = id
    },
    drop: (id) => {
      const moving = dragId
      dragId = overId = null
      const to = entries.find((entry) => entry.id === id)
      if (moving && to && moving !== id) run(moveTrack(moving, to.index))
    },
    end: () => (dragId = overId = null),
  }
  /** A lane shown, by its id, with its track (or the master). */
  const laneRefs = $derived.by(() => {
    // A new map each time the lanes change: the derived value is what is reactive.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const refs = new Map<string, { trackId: string; info: LaneInfo; color: string }>()
    for (const e of allEntries) {
      const color =
        e.id === MASTER
          ? MASTER_COLOR
          : trackColor('color' in e.track ? (e.track.color as string | null | undefined) : null, 'index' in e ? (e.index as number) : 0)
      for (const info of shownLanes.get(e.id) ?? []) refs.set(info.id, { trackId: e.id, info, color })
    }
    return refs
  })

  const y0 = $derived(clampScrollY(scrollY, layout.total, stageH))
  // The selection is kept to the placements the project still has (an undo, an agent).
  const selection = $derived(blocks.filter((b) => picked.has(b.id)))
  const selectedIds = $derived(new Set(selection.map((b) => b.id)))
  /** The selected points, kept to the points the lane still has, earliest first. */
  const autoSel = $derived.by(() => {
    const pick = autoPick
    const ref = pick ? laneRefs.get(pick.lane) : undefined
    if (!pick || !ref) return null
    const points = pointsOf(ref.info.lane)
    const chosen: LanePoint[] = []
    for (const tick of pick.ticks) {
      const p = pointAtTick(points, tick)
      if (p) chosen.push(p)
    }
    chosen.sort((a, b) => a.tick - b.tick)
    return chosen.length > 0 ? { lane: pick.lane, ref, points: chosen } : null
  })

  /** A drag of points, as it would leave the lane. */
  const pointPreview = $derived.by(() => {
    const g = gesture
    if (!g || g.kind !== 'point' || !g.moved) return null
    const ref = laneRefs.get(g.lane)
    if (!ref) return null
    const next = moveSelection(g.snapshot, g.delta, ref.info.axis)
    const at = g.snapshot.findIndex((p) => p.tick === g.grabbed.tick)
    return {
      lane: g.lane,
      plan: planMove(g.all, g.snapshot, next),
      points: previewPoints(g.all, new Set(g.snapshot.map((p) => p.tick)), next),
      selected: new Set(next.map((p) => p.tick)),
      grabbed: next[at] ?? null,
    }
  })

  const ghosts = $derived.by<Block[]>(() => {
    const g = gesture
    if (!g || !g.moved) return []
    if (g.kind === 'resize') return g.to === null ? [] : [resized(g.grabbed, g.edge, g.to)]
    if (g.kind !== 'move' && g.kind !== 'copy') return []
    return moved(
      blocks.filter((b) => g.ids.includes(b.id)),
      g.delta,
      g.rows,
    )
  })
  const hidden = $derived.by<ReadonlySet<string>>(() => {
    const g = gesture
    if (!g || !g.moved) return new Set()
    if (g.kind === 'move') return new Set(g.ids)
    if (g.kind === 'resize' && g.to !== null) return new Set([g.grabbed.id])
    return new Set()
  })
  const loopNow = $derived.by(() => {
    const g = gesture
    if (g?.kind === 'ruler' && !g.select && g.moved && g.from !== g.to) return { start: Math.min(g.from, g.to), end: Math.max(g.from, g.to) }
    return transport?.loop_region ?? null
  })
  // The time selection (#396): Shift+drag on the ruler. Insert and Delete bars act on it.
  let timeSel = $state.raw<{ start: number; end: number } | null>(null)
  let barCount = $state(4)
  // A project opened or started replaces the one the range was chosen on: drop it (#445). Undo and redo keep it shown (#435).
  $effect(() => {
    void project.openedRev
    timeSel = null
  })
  const timeNow = $derived.by(() => {
    const g = gesture
    if (g?.kind === 'ruler' && g.select && g.moved && g.from !== g.to) return { start: Math.min(g.from, g.to), end: Math.max(g.from, g.to) }
    return timeSel
  })
  const MAX_INSERT_BARS = 9999
  /** The "Insert bars" field is open (#575): the count is asked there, since the toolbar no longer holds a number field. */
  let inserting = $state(false)
  function insertBarsFromDialog(count: number) {
    inserting = false
    barCount = count
    const at = timeSel ? timeSel.start : barStart(position ?? 0, bar)
    timeSel = null
    run(insertTime(at, count * bar))
  }
  function deleteSelectedTime() {
    const sel = timeSel
    if (!sel) return
    timeSel = null
    run(deleteTime(sel.start, sel.end))
  }
  const loopOn = $derived(transport?.loop_enabled ?? false)

  // --- the play position ---------------------------------------------------------------------------
  // As in the piano roll: the engine reports about 30 times a second, and while the transport runs the tick is carried on
  // at each animation frame, so that the bar moves at the screen's pace. Here it is the transport's own position.
  const report = $derived(engine.started ? engine.feedback : null)
  const running = $derived(report?.transport === 'playing' || report?.transport === 'recording')
  const bpm = $derived(project.view?.project.tempo[0]?.bpm ?? null)
  const transportLoop = $derived(transport?.loop_enabled ? (transport.loop_region ?? null) : null)
  let reportAt = 0
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

  // --- following the play position, the scrollbar and the zoom -----------------------------------------
  const totalTicks = $derived(totalTicksOf(totalBars, signature))
  /** Share of the window kept before the play position when the view turns the page. */
  const PAGE_LEAD = 0.05
  const ZOOM_STEP = 1.25

  /** Brings the play position into the window, as a page: it lands near the left edge. Does nothing while it is visible. */
  function showPosition() {
    const tick = shownTick
    if (tick === null || stageW === 0) return
    const [from, to] = visibleRange(shown, stageW)
    if (tick >= from && tick <= to) return
    view = clampView({ pxPerTick: shown.pxPerTick, scrollTick: tick - (stageW / shown.pxPerTick) * PAGE_LEAD }, stageW, totalTicks)
  }
  $effect(() => {
    void shownTick
    // Stop, Back to start and a seek bring the view to the position too (#395); a resize does only while it runs,
    // or until the stage has its first width (a view mounted while stopped).
    if (running || untrack(() => stageW) === 0) void stageW
    if (follow) untrack(showPosition)
  })
  // A new Play follows again.
  $effect(() => {
    if (running) untrack(() => (follow = true))
  })
  function followAgain() {
    follow = true
    showPosition()
  }
  const zoomBy = (factor: number) => {
    view = zoomAt(shown, factor, stageW / 2, stageW, totalTicks)
  }
  /** The whole song in the window (#473): bar 1 to its end and a bar more. */
  function fitSong() {
    const fit = wholeSongView(songEnd, bar, stageW)
    if (!fit) return
    follow = false
    view = clampView(fit, stageW, totalTicks)
  }
  // The first opening in a project: 16 to 32 bars, once the stage has its width.
  $effect(() => {
    if (!openToFit || stageW <= 0) return
    const opening = untrack(() => openingView(songEnd, bar, stageW))
    if (!opening) return
    openToFit = false
    view = clampView(
      opening,
      stageW,
      untrack(() => totalTicks),
    )
  })

  // --- the section markers (#473) --------------------------------------------------------------------
  /**
   * The marker a jump just went to, until the engine's report shows the playhead there: two quick presses go on to the
   * next marker instead of finding the old position again.
   */
  let jumpedTo = $state<number | null>(null)
  /** The marker jumped to last: two markers on one tick are reached one after the other. */
  let lastMarker: string | null = null
  let jumpTimer: ReturnType<typeof setTimeout> | undefined
  /** Where the report stood when the last jump was made: a report that still says so is the old one, not the jump's. */
  let jumpFrom: number | null = null
  $effect(() => {
    const tick = shownTick
    if (tick !== null && jumpedTo !== null && tick !== jumpFrom && Math.abs(tick - jumpedTo) < bar / 4) untrack(() => (jumpedTo = null))
  })
  onDestroy(() => clearTimeout(jumpTimer))
  /** The tick the playhead stands on: a jump not reported yet, else the report's, else the start. */
  const headTick = () => Math.max(0, jumpedTo ?? shownTick ?? 0)
  function addMarkerHere() {
    run(addMarker(Math.floor(headTick() / bar) * bar))
  }
  /** Puts the playhead on a tick (`transport.seek`); the next marker jump starts from there even before the engine reports it. */
  function placeHead(tick: number) {
    run(seek(tick))
    jumpFrom = shownTick
    jumpedTo = tick
    // A seek the engine does not report (stopped elsewhere, refused) must not hold the next jump for ever.
    clearTimeout(jumpTimer)
    jumpTimer = setTimeout(() => (jumpedTo = null), 1000)
  }
  /** Bar and beat (from 1) of a tick, for what the live region says. */
  const barBeat = (tick: number) => ({ bar: Math.floor(tick / bar) + 1, beat: Number(((tick % bar) / beatTicks(signature) + 1).toFixed(2)) })
  function jumpMarker(direction: 1 | -1) {
    const from = headTick()
    const to = markerJump(markers, from, direction, lastMarker)
    if (!to) return
    placeHead(to.tick)
    lastMarker = to.id
    if (stageW > 0) view = clampView({ pxPerTick: shown.pxPerTick, scrollTick: to.tick - (stageW / shown.pxPerTick) * PAGE_LEAD }, stageW, totalTicks)
    announce(t('arrangement.markerSaid', { name: to.name, bar: Math.floor(to.tick / bar) + 1 }))
  }
  /** The marker being renamed, and the text typed. */
  let renaming = $state<{ id: string; text: string } | null>(null)
  function commitRename() {
    const r = renaming
    renaming = null
    if (!r) return
    const marker = markers.find((m) => m.id === r.id)
    if (marker && r.text.trim() && r.text.trim() !== marker.name) run(renameMarker(r.id, r.text))
  }
  function onMarkerKey(event: KeyboardEvent, marker: MarkerView) {
    // Alt+Left and Alt+Right move the marker by the snap step (#554), so a section can be placed without the mouse.
    if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
      event.preventDefault()
      event.stopPropagation()
      const to = Math.max(0, marker.tick + (event.key === 'ArrowLeft' ? -nudge : nudge))
      if (to === marker.tick) return announce(t('arrangement.saidCannotMove'))
      run(moveMarker(marker.id, to))
      // Out of sight, the view follows it: the button would be removed with the focus on it.
      const [from, end] = visibleRange(shown, stageW)
      if (stageW > 0 && (to < from || to > end)) {
        follow = false
        view = clampView({ pxPerTick: shown.pxPerTick, scrollTick: to - (stageW / shown.pxPerTick) * PAGE_LEAD }, stageW, totalTicks)
      }
      announce(t('arrangement.saidMarkerMoved', { name: marker.name, ...barBeat(to) }))
      return
    }
    if (event.key === 'F2') {
      event.preventDefault()
      renaming = { id: marker.id, text: marker.name }
    } else if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault()
      run(deleteMarker(marker.id))
    }
  }
  // Alt+K, Alt+Page Up/Down and Alt+Z, wherever the focus is in the page (the arrangement is shown).
  onMount(() => {
    const onKey = (event: KeyboardEvent) => {
      const modal = !!document.querySelector('[aria-modal="true"]')
      // Ctrl+Shift+R (#555): cancelled wherever the focus is (a field and an open dialog included), or the browser would reload
      // the page; it opens the field once per press, outside a text field and a dialog.
      if (repeatShortcut(event)) {
        event.preventDefault()
        if (!modal && !event.repeat && !isTextTarget(event.target)) openRepeat()
        return
      }
      if (modal) return
      const action = arrangementShortcut(event)
      if (!action) return
      event.preventDefault()
      if (action === 'addMarker') addMarkerHere()
      else if (action === 'prevMarker') jumpMarker(-1)
      else if (action === 'nextMarker') jumpMarker(1)
      else fitSong()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })
  const canZoomIn = $derived(zoomAt(shown, ZOOM_STEP, stageW / 2, stageW, totalTicks).pxPerTick > shown.pxPerTick)
  const canZoomOut = $derived(zoomAt(shown, 1 / ZOOM_STEP, stageW / 2, stageW, totalTicks).pxPerTick < shown.pxPerTick)

  // The scrollbar is a native one under the grid, so that it is visible and draggable: its width is the whole timeline.
  let scrollbar: HTMLDivElement | undefined = $state()
  const SYNC_PX = 1.5
  $effect(() => {
    const element = scrollbar
    if (!element) return
    const left = shown.scrollTick * shown.pxPerTick
    if (Math.abs(element.scrollLeft - left) > SYNC_PX) element.scrollLeft = left
  })
  function onBarScroll() {
    const element = scrollbar
    if (!element) return
    // The echo of the view's own scroll lands within a pixel of it; anything else is the hand.
    // (the browser clamps the bar when the timeline shrinks under it: that is not the hand either)
    const expected = Math.min(shown.scrollTick * shown.pxPerTick, element.scrollWidth - element.clientWidth)
    if (Math.abs(element.scrollLeft - expected) <= SYNC_PX) return
    follow = false
    view = clampView({ pxPerTick: shown.pxPerTick, scrollTick: element.scrollLeft / shown.pxPerTick }, stageW, totalTicks)
  }
  /** A scroll by the wheel or the keys, in the grid, is a hand scroll. */
  function handScroll(event: WheelEvent | KeyboardEvent) {
    if ('key' in event) {
      // The arrows on a clip's cell move between clips: the view moves only when the clip is out of sight (`focusItem`).
      if ((event.target as Element | null)?.closest?.('[data-block]')) return
      if (!event.altKey && !event.ctrlKey && !event.metaKey && ['ArrowLeft', 'ArrowRight', 'Home'].includes(event.key)) follow = false
      return
    }
    if (event.ctrlKey || event.metaKey) return
    if (event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) follow = false
  }

  let grid: ReturnType<typeof TimeGrid> | undefined = $state()
  let stage: HTMLDivElement | undefined = $state()

  /** Where a lane's row is in the stage. */
  const geoOf = (row: Row) => ({ top: RULER_HEIGHT + row.top - y0, height: row.height })
  const NO_TICKS: ReadonlySet<number> = new Set()

  const body = (ctx: CanvasRenderingContext2D, frame: BodyFrame) => {
    drawArrangement(ctx, frame, {
      layout,
      scrollY: y0,
      blocks,
      hidden,
      selected: selectedIds,
      ghosts,
      loop: loopNow,
      loopOn,
      timeSel: timeNow,
      fallbackName: t('arrangement.clipDefault'),
      silent,
      markers,
    })
    const lanes: LaneScene[] = []
    for (const row of rowsBetween(layout, y0, y0 + frame.height)) {
      const ref = row.kind === 'lane' && row.lane ? laneRefs.get(row.lane) : undefined
      if (!ref || !row.lane) continue
      const preview = pointPreview?.lane === row.lane ? pointPreview : null
      const points = preview ? preview.points : pointsOf(ref.info.lane)
      const pointed = hover?.lane === row.lane ? pointAtTick(points, hover.tick) : null
      lanes.push({
        ...geoOf(row),
        axis: ref.info.axis,
        points,
        selected: preview ? preview.selected : autoPick?.lane === row.lane ? autoPick.ticks : NO_TICKS,
        color: ref.color,
        readout: preview ? preview.grabbed : pointed,
      })
    }
    const g = gesture
    const box =
      g?.kind === 'box' && g.moved
        ? { x0: Math.min(g.startX, g.x), y0: Math.min(g.startY, g.y), x1: Math.max(g.startX, g.x), y1: Math.max(g.startY, g.y) }
        : null
    const stats = drawAutomation(ctx, frame, { lanes, box })
    // For the e2e measure: how many points the last frame looked at, and whether it drew them as columns.
    if (stage) {
      stage.dataset.lanePoints = String(stats.visible)
      stage.dataset.laneDense = String(stats.dense)
    }
  }

  // The grid draws in its own frame and cannot see this state: tell it when something it draws changed.
  $effect(() => {
    void [blocks, silent, selectedIds, ghosts, hidden, loopNow, loopOn, timeNow, y0, layout, laneRefs, autoPick, hover, pointPreview, gesture]
    grid?.invalidate()
  })

  // --- gestures ------------------------------------------------------------------------------------

  const local = (event: { clientX: number; clientY: number }) => {
    const box = stage!.getBoundingClientRect()
    return { x: event.clientX - box.left, y: event.clientY - box.top }
  }
  /** The step of this event: Ctrl (Cmd on a Mac) puts the snap aside while it is held. */
  const stepOf = (event: Pick<MouseEvent, 'ctrlKey' | 'metaKey'>) => (fineKey(event) ? null : step)

  /** Runs several commands as one history entry. */
  function inGroup(commands: Command[]) {
    if (commands.length === 1) return run(commands[0]!)
    run(beginGroup())
    for (const command of commands) run(command)
    run(endGroup())
  }

  const cancel = () => (gesture = null)

  /** The blocks of the project that the id names, as they are now. */
  const current = (id: string) => blocks.find((b) => b.id === id)
  /** Whether the project still holds the block as the gesture grabbed it. */
  const unchanged = (b: Block) => {
    const now = current(b.id)
    return !!now && now.start === b.start && now.end === b.end && now.offset === b.offset
  }

  function select(trackId: string) {
    if (project.selected?.id !== trackId) run(selectTrack(trackId))
  }

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0 || gesture || !stage) return
    const { x, y } = local(event)
    stage.setPointerCapture(event.pointerId)
    if (y < RULER_HEIGHT) {
      const tick = snapTick(xToTick(x, shown), stepOf(event))
      gesture = { kind: 'ruler', from: tick, to: tick, moved: false, startX: x, select: event.shiftKey }
      return
    }
    // A press on the clips or the lanes takes the keyboard focus off a clip cell (the ruler keeps it): the selection it makes
    // is then what Ctrl+D copies (#528).
    if ((document.activeElement as HTMLElement | null)?.dataset?.block !== undefined) focusGrid()
    const where = rowAt(layout, y, y0)
    if (where && where.kind === 'lane') return laneDown(event, where, x, y)
    if (where?.kind === 'add') {
      if (where.trackId !== MASTER) select(where.trackId)
      return
    }
    autoPick = null
    const hit = hitBlock(blocks, layout, x, y, shown, y0)
    if (!hit) {
      if (where && where.trackId !== MASTER) select(where.trackId)
      if (!event.shiftKey) picked = new Set()
      return
    }
    const { block, zone } = hit
    select(block.track)
    const copy = event.altKey && zone === 'body'
    if (event.shiftKey && !copy) {
      const on = !picked.has(block.id)
      picked = new Set(on ? [...picked, block.id] : [...picked].filter((id) => id !== block.id))
      // A shift-click on a selected clip takes it out of the selection and starts nothing.
      if (!on) return
    } else if (!picked.has(block.id)) picked = new Set([block.id])
    // The piano roll follows the clip selected here (#470); a double click still brings the panel up.
    ui.editClip(block.clip, block.id)
    if (zone === 'body') {
      const ids = blocks.filter((b) => picked.has(b.id)).map((b) => b.id)
      gesture = {
        kind: copy ? 'copy' : 'move',
        grabbed: block,
        ids,
        snapshot: blocks.filter((b) => picked.has(b.id)),
        grabTick: xToTick(x, shown),
        delta: 0,
        rows: 0,
        moved: false,
        linked: copy && event.shiftKey,
        startX: x,
        startY: y,
      }
    } else {
      gesture = { kind: 'resize', grabbed: block, edge: zone, to: null, moved: false, startX: x }
    }
  }

  function onPointerMove(event: PointerEvent) {
    const { x, y } = local(event)
    const g = gesture
    if (!g) {
      const where = y >= RULER_HEIGHT ? rowAt(layout, y, y0) : null
      if (where?.kind === 'lane' && where.lane) {
        const ref = laneRefs.get(where.lane)
        const found = ref ? hitPoint(pointsOf(ref.info.lane), ref.info.axis, x, y, shown, geoOf(where)) : null
        const next = found ? { lane: where.lane, tick: found.tick } : null
        if (next?.lane !== hover?.lane || next?.tick !== hover?.tick) hover = next
        cursor = found ? 'move' : 'add'
        return
      }
      if (hover) hover = null
      const hit = y >= RULER_HEIGHT ? hitBlock(blocks, layout, x, y, shown, y0) : null
      cursor = hit ? (hit.zone === 'body' ? 'move' : 'resize') : 'default'
      return
    }
    if (g.kind === 'ruler') {
      const to = snapTick(xToTick(x, shown), stepOf(event))
      gesture = { ...g, to, moved: g.moved || Math.abs(x - g.startX) >= DRAG_SLOP_PX }
      return
    }
    if (g.kind === 'box') {
      gesture = { ...g, x, y, moved: g.moved || Math.abs(x - g.startX) >= DRAG_SLOP_PX || Math.abs(y - g.startY) >= DRAG_SLOP_PX }
      return
    }
    if (g.kind === 'point') {
      const ref = laneRefs.get(g.lane)
      const row = layout.rows.find((r) => r.kind === 'lane' && r.lane === g.lane)
      // The lane read again at each move: a change of it under the drag (undo, an agent) ends the drag.
      if (!ref || !row || !sameLane(pointsOf(ref.info.lane), g.all)) return cancel()
      const dragged = g.moved || Math.abs(x - g.startX) >= DRAG_SLOP_PX || Math.abs(y - g.startY) >= DRAG_SLOP_PX
      if (!dragged) return
      const vertical = g.vertical || Math.abs(y - g.startY) >= DRAG_SLOP_PX
      const delta = dragDelta(g.snapshot, g.grabbed, xToTick(x, shown), g.grabTick, posOfY(y, geoOf(row)), g.grabPos, stepOf(event), vertical ? 1 : 0)
      gesture = { ...g, moved: true, vertical, delta }
      return
    }
    // The blocks are read again at each move: a change of the arrangement under the drag (undo, an agent) ends it.
    if (!unchanged(g.grabbed) || (g.kind !== 'resize' && !g.snapshot.every(unchanged))) return cancel()
    const dragged = g.moved || Math.abs(x - g.startX) >= DRAG_SLOP_PX || (g.kind !== 'resize' && Math.abs(y - g.startY) >= DRAG_SLOP_PX)
    if (!dragged) return
    if (g.kind === 'resize') {
      gesture = { ...g, moved: true, to: resizeTo(g.grabbed, g.edge, xToTick(x, shown), stepOf(event)) }
    } else {
      const chosen = blocks.filter((b) => g.ids.includes(b.id))
      // A move follows the pointer from track to track (the master and the free ground below keep the last track).
      const under = g.kind === 'move' ? rowAt(layout, y, y0) : null
      const rows = g.kind === 'move' ? clampRows(chosen, under ? under.track - g.grabbed.row : g.rows, entries.length) : 0
      gesture = { ...g, moved: true, rows, delta: moveDelta(chosen, g.grabbed, xToTick(x, shown), g.grabTick, stepOf(event)) }
    }
  }

  function onPointerUp(event: PointerEvent) {
    finish(true, event)
  }

  /** Ends the gesture: its one command is sent when `commit` and the arrangement is still what the gesture started on. */
  function finish(commit: boolean, event?: Pick<MouseEvent, 'ctrlKey' | 'metaKey'>) {
    const g = gesture
    gesture = null
    if (!g || !commit) return
    if (g.kind === 'ruler') {
      if (g.select) {
        if (g.moved && g.from !== g.to) timeSel = { start: Math.min(g.from, g.to), end: Math.max(g.from, g.to) }
        else timeSel = null
        return
      }
      if (!g.moved || g.from === g.to) return run(seek(g.to))
      // A region needs the loop on to be heard: setting it and turning the loop on is one step to undo.
      const start = Math.min(g.from, g.to)
      const end = Math.max(g.from, g.to)
      inGroup(loopOn ? [setLoopRegion(start, end)] : [setLoopRegion(start, end), toggleLoop()])
      return
    }
    if (g.kind === 'box') return finishBox(g, event ? stepOf(event) : step)
    if (g.kind === 'point') return finishPoint(g)
    if (!g.moved || !unchanged(g.grabbed)) return
    if (g.kind !== 'resize' && !g.snapshot.every(unchanged)) return
    if (g.kind === 'resize') {
      if (g.to !== null) run(resizePlacement(g.grabbed.id, g.edge, g.to))
      return
    }
    const ids = blocks.filter((b) => g.ids.includes(b.id)).map((b) => b.id)
    if (ids.length !== g.ids.length || (g.delta === 0 && g.rows === 0)) return
    if (g.kind === 'move' && g.rows !== 0) {
      sendToTracks(
        blocks.filter((b) => g.ids.includes(b.id)),
        g.rows,
        g.delta,
        g.grabbed.id,
      )
    } else if (g.kind === 'move') {
      run(movePlacements(ids, g.delta))
      const moved = blocks.find((b) => b.id === g.grabbed.id)
      if (moved) announce(t('arrangement.saidMoved', { n: ids.length, bar: barsOf(moved).from }))
    } else copyAndSelect(ids, g.linked, g.delta)
  }

  /** The tracks a clip may move to, in track order: a MIDI clip only goes to a MIDI track. */
  const trackList = $derived(entries.map((e) => ({ id: e.id, midi: isMidi(e.track) })))

  /**
   * Moves clips `rows` tracks (negative is up) and `delta` ticks, in one `arrangement.move_to_track`; refused out loud at the first
   * or last track and for a target that is not a MIDI track. The moved clips become the selection (they get new identifiers in
   * their new track). Returns where each landed, by the identifier it had, or `null` when nothing moved.
   */
  function sendToTracks(chosen: readonly Block[], rows: number, delta: number, follow?: string): Record<string, Block> | null {
    const targets = trackTargets(chosen, rows, trackList)
    if ('refused' in targets) {
      if (targets.refused === 'edge') announce(t('arrangement.saidNoTrack'))
      else {
        const entry = entries.find((e) => e.id === targets.track)
        announce(t('arrangement.saidNotMidiTrack', { track: entry ? trackLabel(entry, t) : '' }))
      }
      return null
    }
    run(moveToTrack(targets.moves, delta))
    const landed: Record<string, Block> = {}
    for (const b of chosen) {
      const now = blocks.find((n) => n.row === b.row + rows && n.start === b.start + delta && n.end === b.end + delta && n.offset === b.offset)
      if (now) landed[b.id] = now
    }
    const all = Object.values(landed)
    if (all.length === 0) return null
    picked = new Set(all.map((b) => b.id))
    const lead = (follow ? landed[follow] : undefined) ?? all[0]!
    const entry = entries[lead.row]
    select(lead.track)
    ui.editClip(lead.clip, lead.id)
    announce(t('arrangement.saidMovedTrack', { n: all.length, track: entry ? trackLabel(entry, t) : '', bar: barsOf(lead).from }))
    return landed
  }

  /** Copies, then selects the copies (the placements that were not there before). */
  function copyAndSelect(ids: string[], linked: boolean, by?: number): boolean {
    const before = new Set(blocks.map((b) => b.id))
    run(duplicatePlacements(ids, linked, by))
    const fresh = blocks.filter((b) => !before.has(b.id)).map((b) => b.id)
    if (fresh.length > 0) picked = new Set(fresh)
    if (fresh.length > 0) announce(t('arrangement.saidCopied', { n: fresh.length }))
    return fresh.length > 0
  }

  // --- the lanes' gestures ---------------------------------------------------------------------------

  /** Whether the lane holds the points a gesture started on. */
  function sameLane(now: readonly LanePoint[], then: readonly LanePoint[]): boolean {
    if (now === then) return true
    return now.length === then.length && now.every((p, i) => p.tick === then[i]!.tick && p.value === then[i]!.value && p.shape === then[i]!.shape)
  }

  /** A press in a lane's row: on a point it selects it and starts to move it, elsewhere it adds a point (a click) or draws a box (a drag). */
  function laneDown(event: PointerEvent, row: Row, x: number, y: number) {
    const ref = row.lane ? laneRefs.get(row.lane) : undefined
    if (!ref || !row.lane) return
    if (row.trackId !== MASTER) select(row.trackId)
    picked = new Set()
    const points = pointsOf(ref.info.lane)
    const geo = geoOf(row)
    const hit = hitPoint(points, ref.info.axis, x, y, shown, geo)
    const same = autoPick?.lane === row.lane
    if (!hit) {
      if (!event.shiftKey) autoPick = null
      gesture = { kind: 'box', lane: row.lane, trackId: row.trackId, startX: x, startY: y, x, y, moved: false, add: event.shiftKey }
      return
    }
    let ticks: ReadonlySet<number> = same ? autoPick!.ticks : NO_TICKS
    if (event.shiftKey) {
      const on = !ticks.has(hit.tick)
      ticks = new Set(on ? [...ticks, hit.tick] : [...ticks].filter((t) => t !== hit.tick))
      autoPick = ticks.size > 0 ? { lane: row.lane, ticks } : null
      // A shift-click on a selected point takes it out of the selection and starts nothing.
      if (!on) return
    } else if (!ticks.has(hit.tick)) {
      ticks = new Set([hit.tick])
      autoPick = { lane: row.lane, ticks }
    }
    const snapshot = [...ticks].map((t) => pointAtTick(points, t)).filter((p): p is LanePoint => p !== null)
    snapshot.sort((a, b) => a.tick - b.tick)
    gesture = {
      kind: 'point',
      lane: row.lane,
      trackId: row.trackId,
      grabbed: hit,
      snapshot,
      all: points,
      grabTick: xToTick(x, shown),
      grabPos: posOfY(y, geo),
      delta: { ticks: 0, position: 0 },
      vertical: false,
      moved: false,
      startX: x,
      startY: y,
    }
  }

  /** The end of a press on an empty part: a click adds a point, a drag selects the points in its box. */
  function finishBox(g: Extract<Gesture, { kind: 'box' }>, tickStep: number | null) {
    const ref = laneRefs.get(g.lane)
    const row = layout.rows.find((r) => r.kind === 'lane' && r.lane === g.lane)
    if (!ref || !row) return
    const points = pointsOf(ref.info.lane)
    const geo = geoOf(row)
    if (g.moved) {
      const inside = pointsInBox(points, ref.info.axis, g.startX, g.startY, g.x, g.y, shown, geo)
      const before = g.add && autoPick?.lane === g.lane ? autoPick.ticks : NO_TICKS
      const ticks = new Set([...before, ...inside])
      autoPick = ticks.size > 0 ? { lane: g.lane, ticks } : null
      return
    }
    const fresh = newPoint(ref.info.axis, shown, geo, g.startX, g.startY, tickStep)
    // A tick that already holds a point (the grab zone is smaller than a snap step): that point is the one pointed at.
    if (pointAtTick(points, fresh.tick)) {
      autoPick = { lane: g.lane, ticks: new Set([fresh.tick]) }
      return
    }
    run(addPoints(g.lane, [asPoint(fresh)]))
    autoPick = { lane: g.lane, ticks: new Set([fresh.tick]) }
  }

  /** The end of the drag of points: one update when only values moved, else the old points out and the new ones in, as one history group. */
  function finishPoint(g: Extract<Gesture, { kind: 'point' }>) {
    const ref = laneRefs.get(g.lane)
    if (!g.moved || !ref || !sameLane(pointsOf(ref.info.lane), g.all)) return
    const next = moveSelection(g.snapshot, g.delta, ref.info.axis)
    const plan = planMove(g.all, g.snapshot, next)
    if (!plan) return
    if (plan.kind === 'update') run(updatePoints(g.lane, plan.points))
    else inGroup([removePoints(g.lane, plan.remove), addPoints(g.lane, plan.add)])
    autoPick = { lane: g.lane, ticks: new Set(next.map((p) => p.tick)) }
  }

  /** Sets the curve of the selected points. */
  function setCurve(shape: Shape) {
    const sel = autoSel
    if (!sel) return
    const changed = sel.points.filter((p) => p.shape !== shape).map((p) => asPoint({ ...p, shape }))
    if (changed.length > 0) run(updatePoints(sel.lane, changed))
  }

  /** Deletes the selected points. */
  function removePicked() {
    const sel = autoSel
    if (!sel) return
    run(
      removePoints(
        sel.lane,
        sel.points.map((p) => p.tick),
      ),
    )
    autoPick = null
  }

  function onContextMenu(event: MouseEvent) {
    // The field that types a point's value keeps the browser's menu.
    if (keepsBrowserMenu(event.target) || event.defaultPrevented) return
    const { x, y } = local(event)
    const row = y >= RULER_HEIGHT ? rowAt(layout, y, y0) : null
    const ref = row?.kind === 'lane' && row.lane ? laneRefs.get(row.lane) : undefined
    if (row && !(ref && row.lane)) return clipContext(event, row, x, y)
    if (!row || !ref || !row.lane) return
    event.preventDefault()
    const hit = hitPoint(pointsOf(ref.info.lane), ref.info.axis, x, y, shown, geoOf(row))
    if (!hit) {
      menu = null
      return
    }
    if (!(autoPick?.lane === row.lane && autoPick.ticks.has(hit.tick))) autoPick = { lane: row.lane, ticks: new Set([hit.tick]) }
    menu = { x: event.clientX, y: event.clientY }
  }

  /**
   * The right click on a clip (#524): the clip joins the selection when it is not part of it, and its actions open. On the
   * free part of a MIDI track's row: creating a clip there. Anywhere else (ruler, master, audio track) nothing opens.
   */
  function clipContext(event: MouseEvent, where: Row, x: number, y: number) {
    autoPick = null
    const cell = (event.target as Element | null)?.closest?.<HTMLElement>('[data-block]')
    const block = cell?.dataset.block ? current(cell.dataset.block) : hitBlock(blocks, layout, x, y, shown, y0)?.block
    const items: MenuItem[] = []
    if (block) {
      if (!picked.has(block.id)) picked = new Set([block.id])
      select(block.track)
      ui.editClip(block.clip, block.id)
      const mod = (key: string[]) => chordText(['Mod', ...key])
      items.push(
        {
          id: 'open',
          label: t('arrangement.openInRoll'),
          shortcut: chordText(['Enter']),
          run: () => {
            ui.editClip(block.clip, block.id)
            ui.showPanel('piano-roll')
          },
        },
        { id: 'duplicate', label: t('arrangement.duplicate'), shortcut: mod(['D']), separator: true, run: () => duplicate(false) },
        { id: 'duplicate-linked', label: t('arrangement.duplicateLinked'), shortcut: mod(['Shift', 'D']), run: () => duplicate(true) },
        { id: 'repeat', label: t('arrangement.repeatUntil'), shortcut: mod(['Shift', 'R']), disabled: picked.size !== 1, run: openRepeat },
        { id: 'split', label: t('arrangement.split'), shortcut: mod(['E']), disabled: cuts.length === 0, run: splitSelected },
        {
          id: 'move-up',
          label: t('arrangement.moveUp'),
          shortcut: chordText(['Alt', '↑']),
          separator: true,
          disabled: !('moves' in trackTargets(selection, -1, trackList)),
          run: () => sendToTracks(selection, -1, 0, block.id),
        },
        {
          id: 'move-down',
          label: t('arrangement.moveDown'),
          shortcut: chordText(['Alt', '↓']),
          disabled: !('moves' in trackTargets(selection, 1, trackList)),
          run: () => sendToTracks(selection, 1, 0, block.id),
        },
        { id: 'delete', label: t('arrangement.delete'), shortcut: chordText(['Delete']), separator: true, run: removeSelected },
      )
    } else {
      const entry = where.kind === 'track' ? entries[where.track] : undefined
      const start = Math.floor(snapTick(xToTick(x, shown), stepOf(event)) / bar) * bar
      if (entry && isMidi(entry.track) && isFree(blocks, where.track, start, start + bar)) {
        items.push({
          id: 'create',
          label: t('pianoRoll.createClip'),
          run: () => {
            select(entry.id)
            run(createClipAt(entry.id, start, bar))
          },
        })
      }
    }
    ui.openMenu(event, items, t('arrangement.menu'))
  }

  /** Opens the field to type the value of the point under a double click. */
  function editPoint(row: Row, x: number, y: number): boolean {
    const ref = row.lane ? laneRefs.get(row.lane) : undefined
    if (!ref || !row.lane) return false
    const hit = hitPoint(pointsOf(ref.info.lane), ref.info.axis, x, y, shown, geoOf(row))
    if (!hit) return false
    limitNote = null
    autoPick = { lane: row.lane, ticks: new Set([hit.tick]) }
    editing = { lane: row.lane, tick: hit.tick, x: tickToX(hit.tick, shown), y: yOfPos(ref.info.axis.pos(hit.value), geoOf(row)) }
    return true
  }

  /** What the field of a typed value does with its text: one `automation.points.update`, or nothing when the value is the same. */
  function commitValue(text: string): boolean {
    const e = editing
    const ref = e ? laneRefs.get(e.lane) : undefined
    const point = ref && e ? pointAtTick(pointsOf(ref.info.lane), e.tick) : null
    if (!e || !ref || !point) return true
    const parsed = ref.info.axis.parse(text)
    if (parsed === null) return false
    const value = ref.info.axis.clamp(parsed.value)
    if (value !== point.value) run(updatePoints(e.lane, [asPoint({ ...point, value })]))
    // A text past the range is brought back to the bound, and the editor says so once the field has closed (#421, as #410 for the controls).
    limitNote = null
    if (parsed.limit !== undefined) sayLimited(ref.info.axis.format(ref.info.axis.clamp(parsed.limit)), e.x, e.y)
    return true
  }

  /** The note an entry limited to the range leaves where the field was, for about two seconds. */
  let limitNote = $state.raw<{ text: string; x: number; y: number } | null>(null)
  let limitTimer: ReturnType<typeof setTimeout> | undefined
  function sayLimited(value: string, x: number, y: number) {
    limitNote = { text: t('controls.limited', { value }), x, y }
    clearTimeout(limitTimer)
    limitTimer = setTimeout(() => (limitNote = null), 2000)
  }
  onDestroy(() => clearTimeout(limitTimer))

  /** Copies the given clips (the selection by default), one bar group right after them. */
  function duplicate(linked: boolean, from: readonly Block[] = selection) {
    const by = copyShift(from)
    if (by === null) return
    const keyboard = (document.activeElement as HTMLElement | null)?.dataset?.block !== undefined
    const copied = copyAndSelect(
      from.map((b) => b.id),
      linked,
      by,
    )
    // From the keyboard the focus follows the copies, which are the selection: a repeated Ctrl+D copies them again and
    // Shift+arrow starts from them, never from the original left behind (#528).
    const first = [...picked]
      .map((id) => blocks.find((b) => b.id === id))
      .filter((b): b is Block => !!b)
      .sort((x, y) => x.start - y.start)[0]
    if (copied && keyboard && first) void focusItem(first.id)
  }

  // --- repeat until (#555) ------------------------------------------------------------------------------------------
  /** The clip being repeated, and the first bar the field accepts (the one after the clip's end). */
  let repeating = $state<{ id: string; min: number } | null>(null)
  /** Opens the field for the one clip selected, or the one the keyboard focus is on when nothing is selected. */
  function openRepeat() {
    const focusedId = (document.activeElement as HTMLElement | null)?.dataset?.block
    const block = selection.length === 1 ? selection[0] : selection.length === 0 && focusedId ? blocks.find((b) => b.id === focusedId) : undefined
    if (!block) return announce(t('arrangement.saidRepeatOne'))
    repeating = { id: block.id, min: Math.floor(block.end / bar) + 1 }
  }
  /**
   * The "More" menu of the toolbar (#575): the actions that are not needed at every step of making a track, with their
   * shortcuts. It opens under its button (also from the keyboard, where the click has no position) in the shared menu.
   */
  function openMore(event: MouseEvent) {
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
    const mod = (key: string[]) => chordText(['Mod', ...key])
    ui.openMenuAt(
      box.left,
      box.bottom + 2,
      [
        {
          id: 'more-duplicate-linked',
          label: t('arrangement.duplicateLinked'),
          shortcut: mod(['Shift', 'D']),
          disabled: selection.length === 0,
          run: () => duplicate(true),
        },
        { id: 'more-repeat', label: t('arrangement.repeatUntil'), shortcut: mod(['Shift', 'R']), disabled: selection.length !== 1, run: openRepeat },
        { id: 'more-insert-bars', label: t('arrangement.insertBars'), separator: true, run: () => (inserting = true) },
        { id: 'more-delete-bars', label: t('arrangement.deleteBars'), disabled: !timeSel, run: deleteSelectedTime },
        { id: 'more-marker', label: t('arrangement.addMarker'), shortcut: chordText(['Alt', 'K']), separator: true, run: addMarkerHere },
        { id: 'more-curve-linear', label: t('automation.curveLinear'), disabled: !autoSel, separator: true, run: () => setCurve('linear') },
        { id: 'more-curve-step', label: t('automation.curveStep'), disabled: !autoSel, run: () => setCurve('step') },
      ],
      t('arrangement.more'),
    )
  }
  function repeatUpTo(barNumber: number) {
    const target = repeating
    repeating = null
    if (!target) return
    // A refusal (an audio clip, the limit, no audio started) shows in the page's own banner; only a success is said here.
    if (run(repeatPlacement(target.id, barNumber * bar))) announce(t('arrangement.saidRepeated', { bar: barNumber }))
  }

  function removeSelected() {
    if (selection.length === 0) return
    const n = selection.length
    run(deletePlacements(selection.map((b) => b.id)))
    picked = new Set()
    announce(t('arrangement.saidDeleted', { n }))
  }

  /** The tick of the play position, whole. */
  const position = $derived(shownTick === null ? null : Math.round(shownTick))
  const cuts = $derived(position === null ? [] : splittable(selection, position))

  function splitSelected() {
    if (position === null || cuts.length === 0) return
    inGroup(cuts.map((b) => splitPlacement(b.id, position)))
    announce(t('arrangement.saidSplit', { n: cuts.length }))
  }

  function onDoubleClick(event: MouseEvent) {
    const { x, y } = local(event)
    if (y < RULER_HEIGHT) return
    const where = rowAt(layout, y, y0)
    // In a lane, a double click on a point opens the field to type its value (the clicks before it added or selected it).
    if (where?.kind === 'lane') {
      editPoint(where, x, y)
      return
    }
    if (where?.kind === 'add') return
    const hit = hitBlock(blocks, layout, x, y, shown, y0)
    if (hit) {
      // The clip opens in the piano roll below.
      ui.editClip(hit.block.clip, hit.block.id)
      select(hit.block.track)
      ui.showPanel('piano-roll')
      return
    }
    const row = where ? where.track : null
    // The master row has no clips and is not a real track: skip it.
    const entry = row === null ? null : entries[row]
    if (!entry || row === null || !isMidi(entry.track)) return
    const start = Math.floor(snapTick(xToTick(x, shown), stepOf(event)) / bar) * bar
    if (!isFree(blocks, row, start, start + bar)) return
    select(entry.id)
    run(createClipAt(entry.id, start, bar))
  }

  // --- a clip dropped from the Clips panel (#473) ----------------------------------------------------------------
  /** The clip a drag from the pool carries, when it is one; only its own track's lane takes it. */
  function poolDrag(event: DragEvent): { clip: string; track: string } | null {
    const raw = event.dataTransfer?.getData(CLIP_DRAG_TYPE)
    if (!raw) return null
    try {
      const v = JSON.parse(raw) as { clip?: unknown; track?: unknown }
      return typeof v.clip === 'string' && typeof v.track === 'string' ? { clip: v.clip, track: v.track } : null
    } catch {
      return null
    }
  }
  /** The track of the clip row under a drop, when it is a MIDI track's. */
  function dropRow(event: DragEvent): string | null {
    const { y } = local(event)
    if (y < RULER_HEIGHT) return null
    const where = rowAt(layout, y, y0)
    if (!where || where.kind !== 'track') return null
    const entry = entries[where.track]
    return entry && isMidi(entry.track) ? entry.id : null
  }
  function onDragOver(event: DragEvent) {
    // The data is hidden until the drop, its types are not: only the clip's own track takes it.
    const track = dropRow(event)
    if (!event.dataTransfer || track === null || !event.dataTransfer.types.includes(clipTrackType(track))) return
    event.preventDefault()
    event.dataTransfer.dropEffect = event.altKey ? 'link' : 'copy'
  }
  function onDrop(event: DragEvent) {
    const dragged = poolDrag(event)
    const track = dropRow(event)
    if (!dragged || track === null || dragged.track !== track || !project.view) return
    event.preventDefault()
    const entry = poolOf(project.view.project, track).find((c) => c.id === dragged.clip)
    if (!entry) return
    const { x } = local(event)
    const start = Math.max(0, snapTick(xToTick(x, shown), step))
    const command = placeCommand(entry, start, event.altKey)
    if (command) {
      select(track)
      run(command)
    }
  }

  function onKeyDown(event: KeyboardEvent) {
    if (isTextTarget(event.target)) return
    const mod = (event.ctrlKey || event.metaKey) && !event.altKey
    if (event.key === 'Delete' || event.key === 'Backspace') {
      if (autoSel) removePicked()
      else removeSelected()
    } else if (event.key === 'Escape') {
      if (menu) menu = null
      else if (gesture) cancel()
      else {
        picked = new Set()
        autoPick = null
      }
    } else if (mod && event.key.toLowerCase() === 'a') {
      // With points selected, or a lane pointed at, all of that lane's points; else all the clips.
      const lane = autoPick?.lane ?? hover?.lane
      const ref = lane ? laneRefs.get(lane) : undefined
      if (lane && ref) autoPick = { lane, ticks: new Set(pointsOf(ref.info.lane).map((p) => p.tick)) }
      else picked = new Set(blocks.map((b) => b.id))
    } else if (mod && event.key.toLowerCase() === 'd') {
      // With nothing selected, Ctrl+D copies the clip the keyboard focus is on (a Tab, Escape). A selection is always copied whole,
      // as the Duplicate button's count says (#528).
      const id = (event.target as HTMLElement | null)?.dataset?.block
      const focused = id && selection.length === 0 ? blocks.find((b) => b.id === id) : undefined
      if (focused) picked = new Set([focused.id])
      duplicate(event.shiftKey, focused ? [focused] : selection)
    } else if (mod && !event.shiftKey && event.key.toLowerCase() === 'e') splitSelected()
    else return
    event.preventDefault()
  }

  /** Gives the keyboard back to the grid (after a menu or a field closed), so that Delete and the shortcuts keep working. */
  function focusGrid() {
    stage?.querySelector<HTMLElement>('[data-testid="timegrid"]')?.focus()
  }

  function toggleOpen(id: string) {
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const next = new Set(open)
    if (!next.delete(id)) next.add(id)
    open = next
    // The lanes of a folded track are out of reach: their points are no longer selected.
    if (autoPick && !next.has(laneRefs.get(autoPick.lane)?.trackId ?? '')) autoPick = null
  }

  /** The parameters of a track (or the master) that have no lane yet, for the menu that adds one. */
  function freeOptions(trackId: string) {
    const entry = allEntries.find((e) => e.id === trackId)
    if (!entry) return []
    const taken = new Set(shownLanes.get(trackId)?.map((l) => targetKey(l.lane.target)) ?? [])
    return optionsOf(entry.track, labels).filter((o) => !taken.has(o.key))
  }
  function addLane(trackId: string, key: string) {
    const option = freeOptions(trackId).find((o) => o.key === key)
    if (!option) return
    if (trackId === MASTER) run(createMasterLane(option.target))
    else run(createLane(trackId, option.target))
  }

  // The mixer or the devices panel requests, once, that an owner's lanes be unfolded (`ui.showLanes()`).
  // The request is a one-shot value: this effect reads and clears it, adding the owner to `open` exactly once
  // (an owner that no longer exists, a deleted track, is ignored and the request is cleared all the same).
  // It fires both when the request arrives while the view is alive and when the view mounts with a pending
  // request (Automate clicked elsewhere, then navigation brought the user here). Because the request is
  // consumed, a user who folds the owner and comes back is not overridden.
  $effect(() => {
    if (ui.openLanes === null) return
    const owner = untrack(() => ui.takeLanesRequest())
    if (owner !== null && untrack(() => allEntries.some((e) => e.id === owner)) && !open.has(owner)) open = new Set([...open, owner])
  })

  // --- the clips from the keyboard and for assistive technology (#471) ---------------------------------------------
  // The canvas has no children: a layer of named cells lies over it, one per clip in sight (and the focused one), in a
  // grid whose rows are the tracks. One cell is in the tab order (roving focus); the arrows move it (the selection
  // follows), Shift+arrows extend the selection, Alt+Left and Alt+Right move the selected clips by the snap step, Enter
  // opens the clip in the piano roll, Delete removes the selection. Every edit is a command; a polite live region says
  // what it did.
  let focusId = $state<string | null>(null)
  let itemsEl: HTMLDivElement | undefined = $state()
  /** What the live region says; a counter makes a repeated message a new one. */
  let said = $state({ text: '', n: 0 })
  function announce(text: string) {
    // The same words twice are still a new message: an invisible space every other time changes the text.
    said = { text: said.n % 2 === 0 ? `${text}\u200b` : text, n: said.n + 1 }
  }

  const items = $derived(blocks.map((b) => ({ id: b.id, row: b.row, start: b.start, end: b.end })))
  /** The cell in the tab order: the focused clip when it still exists, else the first selected, else the first clip. */
  const tabId = $derived((focusId && blocks.some((b) => b.id === focusId) ? focusId : null) ?? selection[0]?.id ?? blocks[0]?.id ?? null)
  const barsOf = (b: Block) => ({ from: Math.floor(b.start / bar) + 1, to: Math.max(Math.floor(b.start / bar) + 1, Math.ceil(b.end / bar)) })
  function clipText(b: Block): string {
    const entry = entries[b.row]
    const name = b.name || t('arrangement.clipDefault')
    const where = barsOf(b)
    const parts = [t('arrangement.clipItem', { name, track: entry ? trackLabel(entry, t) : '', from: where.from, to: where.to })]
    // A linked clip and a repeated content are said (#473), as they are drawn.
    if (b.uses > 1) parts.push(t('arrangement.clipLinked'))
    const passes = passCount(b)
    if (passes > 1) parts.push(t('arrangement.clipPasses', { n: passes }))
    if (selectedIds.has(b.id)) parts.push(t('arrangement.clipSelected'))
    return parts.join(', ')
  }
  /** The cells drawn: the clips in sight, and the one in the tab order wherever it is (so that Tab still finds it). */
  const shownItems = $derived.by(() => {
    if (stageW === 0) return []
    const [from, to] = visibleRange(shown, stageW)
    return blocks
      .filter((b) => b.id === tabId || (b.end >= from && b.start <= to))
      .map((b) => {
        const top = laneTop(layout, b.row, y0) + BLOCK_INSET
        const height = Math.max(4, laneHeight(layout, b.row) - 2 * BLOCK_INSET)
        const left = tickToX(b.start, shown)
        return { block: b, left, top, width: Math.max(4, tickToX(b.end, shown) - left), height }
      })
  })

  /** Brings a clip into the window (time and rows), then focuses its cell. */
  async function focusItem(id: string) {
    const b = blocks.find((x) => x.id === id)
    if (!b) return
    focusId = id
    onCursor = false
    const [from, to] = visibleRange(shown, stageW)
    if (b.start < from || b.start > to) {
      follow = false
      view = clampView({ pxPerTick: shown.pxPerTick, scrollTick: b.start - (stageW / shown.pxPerTick) * PAGE_LEAD }, stageW, totalTicks)
    }
    const top = layout.trackTop[b.row] ?? 0
    const height = laneHeight(layout, b.row)
    const room = stageH - RULER_HEIGHT
    if (top < y0) scrollY = clampScrollY(top, layout.total, stageH)
    else if (top + height > y0 + room) scrollY = clampScrollY(top + height - room, layout.total, stageH)
    await tick()
    itemsEl?.querySelector<HTMLElement>(`[data-block="${id}"]`)?.focus()
  }

  /** The step of a keyboard move: the snap step, else a bar (the snap off moves by a bar, as a nudge). */
  const nudge = $derived(step ?? bar)

  function onItemKey(event: KeyboardEvent) {
    const id = (event.target as HTMLElement).dataset.block
    const current = id ? blocks.find((b) => b.id === id) : undefined
    // Ctrl or Cmd with plus or minus zooms the timeline from a cell too (the page's zoom would take it otherwise).
    const zoom = zoomKey(event)
    if (zoom !== null) {
      event.preventDefault()
      event.stopPropagation()
      zoomBy(zoom > 0 ? ZOOM_STEP : 1 / ZOOM_STEP)
      if (current) void focusItem(current.id)
      else if ((event.target as HTMLElement).dataset.cursor !== undefined) void focusCursor()
      return
    }
    // Shift+Enter on the free cursor puts the playhead there (#554); the roving keys have no Shift+Enter.
    if (
      event.key === 'Enter' &&
      event.shiftKey &&
      !event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      (event.target as HTMLElement).dataset.cursor !== undefined
    ) {
      event.preventDefault()
      event.stopPropagation()
      return putHeadAtCursor()
    }
    const key = rovingKey(event)
    if (key && (event.target as HTMLElement).dataset.cursor !== undefined) return onCursorKey(event, key)
    // Page Up and Page Down mean a velocity: a clip has none.
    if (!current || !key || key.kind === 'velocity') return
    event.preventDefault()
    event.stopPropagation()
    if (key.kind === 'free') {
      // Escape frees the cursor at the clip's start on its track (#494); the selection stays (Escape again on the cursor clears it).
      cursorAt = { row: current.row, tick: current.start }
      void focusCursor()
      return
    }
    if (key.kind === 'open') {
      ui.editClip(current.clip, current.id)
      select(current.track)
      ui.showPanel('piano-roll')
      announce(t('arrangement.saidOpened', { name: current.name || t('arrangement.clipDefault') }))
      return
    }
    if (key.kind === 'delete') {
      const chosen = selection.length > 0 && selectedIds.has(current.id) ? selection : [current]
      // The neighbour that stays: the clips deleted with this one are skipped.
      const gone = new Set(chosen.map((b) => b.id))
      const left = items.filter((i) => i.id === current.id || !gone.has(i.id))
      const next = stepItem(left, current, 'right') ?? stepItem(left, current, 'left') ?? stepItem(left, current, 'down') ?? stepItem(left, current, 'up')
      run(deletePlacements(chosen.map((b) => b.id)))
      picked = new Set()
      announce(t('arrangement.saidDeleted', { n: chosen.length }))
      if (next && blocks.some((b) => b.id === next.id)) void focusItem(next.id)
      else {
        // No clip left to go to: the cursor takes the focus where this one was, so the keyboard stays in the grid.
        cursorAt = { row: current.row, tick: current.start }
        void focusCursor()
      }
      return
    }
    if (key.kind === 'move') {
      const chosen = selectedIds.has(current.id) ? selection : [current]
      if (!selectedIds.has(current.id)) picked = new Set([current.id])
      // Up and Down move the clips to the track above or below (`arrangement.move_to_track`).
      if (key.direction === 'up' || key.direction === 'down') {
        const landed = sendToTracks(chosen, key.direction === 'up' ? -1 : 1, 0, current.id)
        const now = landed?.[current.id]
        if (now) void focusItem(now.id)
        return
      }
      const sign = key.direction === 'left' ? -1 : 1
      const delta = moveDelta(chosen, current, current.start + sign * nudge, current.start, null)
      if (delta === 0) return announce(t('arrangement.saidCannotMove'))
      // A move over other clips carves them (`arrangement.move`): what it did to them is said too.
      const ids = new Set(chosen.map((b) => b.id))
      const others = new Map(blocks.filter((b) => !ids.has(b.id)).map((b) => [b.id, `${b.start}:${b.end}`]))
      run(movePlacements([...ids], delta))
      const after = blocks.filter((b) => !ids.has(b.id))
      const carved = [...others].filter(([bid, span]) => {
        const now = after.find((b) => b.id === bid)
        return !now || `${now.start}:${now.end}` !== span
      }).length
      const moved = blocks.find((b) => b.id === current.id)
      if (moved) {
        const said = t('arrangement.saidMoved', { n: chosen.length, bar: barsOf(moved).from })
        announce(carved > 0 ? `${said}. ${t('arrangement.saidCarved', { n: carved })}` : said)
      }
      void focusItem(current.id)
      return
    }
    const target = stepItem(items, current, key.direction as Direction)
    if (!target) {
      // Shift+arrow with no neighbour that way still selects the focused clip, so a lone clip is reachable (#553).
      if (key.kind === 'extend' && !selectedIds.has(current.id)) {
        picked = new Set([current.id])
        select(current.track)
        ui.editClip(current.clip, current.id)
        announce(t('arrangement.saidSelected', { name: current.name || t('arrangement.clipDefault') }))
      }
      return
    }
    const block = blocks.find((b) => b.id === target.id)!
    // Shift+arrow extends the selection the focused clip is in; from a clip outside it (after a copy, a click elsewhere) it
    // starts a new one from that clip, so the old selection is never added to (#528).
    if (key.kind === 'extend') picked = new Set(selectedIds.has(current.id) ? [...selectedIds, target.id] : [current.id, target.id])
    else picked = new Set([target.id])
    select(block.track)
    ui.editClip(block.clip, block.id)
    void focusItem(target.id)
  }

  function onItemsFocus(event: FocusEvent) {
    itemsHadFocus = true
    const target = event.target as HTMLElement
    if (target.dataset.block) {
      focusId = target.dataset.block
      onCursor = false
    } else if (target.dataset.cursor !== undefined && cursorPos) {
      // Pinned where it was found: it no longer follows the view.
      cursorAt ??= cursorPos
      onCursor = true
    }
  }

  // --- the free cursor (#494) -----------------------------------------------------------------------------------------
  // A track and a tick where no clip needs to be: the only cell in the tab order of an empty timeline, and where Escape on a
  // clip leaves the focus. The arrows move it by the snap step (a beat at least) and a track (Shift+Left and Right: a bar); Enter creates a
  // clip of one bar there (`clip.create`, placed), or goes to the clip already under it.
  /** Where the cursor was left; null until it is first used (it then stands on the selected track, where the view looks). */
  let cursorAt = $state<Cursor | null>(null)
  /** Whether the cursor holds the tab stop rather than a clip. */
  let onCursor = $state(false)
  const cursorTab = $derived(entries.length > 0 && (onCursor || blocks.length === 0))
  const cursorPos = $derived.by<Cursor | null>(() => {
    if (entries.length === 0) return null
    const last = Math.max(0, totalBars * bar - 1)
    // Row -1 is the ruler (#554): where Enter puts the playhead.
    if (cursorAt) return { row: Math.max(-1, Math.min(cursorAt.row, entries.length - 1)), tick: Math.min(cursorAt.tick, last) }
    const row = Math.max(
      0,
      entries.findIndex((e) => e.id === project.selected?.id),
    )
    return { row, tick: Math.min(Math.ceil(Math.max(0, shown.scrollTick) / bar) * bar, last) }
  })
  /** The cursor's cell: one bar wide (the clip Enter would create), on its track's row. */
  const cursorCell = $derived.by(() => {
    if (!cursorPos || cursorPos.row < 0) return null
    const top = laneTop(layout, cursorPos.row, y0) + BLOCK_INSET
    const height = Math.max(4, laneHeight(layout, cursorPos.row) - 2 * BLOCK_INSET)
    const left = tickToX(cursorPos.tick, shown)
    return { left, top, width: Math.max(4, tickToX(cursorPos.tick + bar, shown) - left), height }
  })
  /** The ruler's cell (#554): the cursor's tick, over the ruler, one bar wide. */
  const rulerCell = $derived.by(() => {
    if (cursorPos?.row !== -1 || stageW === 0) return null
    const left = tickToX(cursorPos.tick, shown)
    return { left, width: Math.max(4, tickToX(cursorPos.tick + bar, shown) - left) }
  })
  const cursorText = $derived.by(() => {
    if (cursorPos?.row === -1) {
      const { bar: at, beat } = barBeat(cursorPos.tick)
      return cursorPos.tick % bar === 0 ? t('arrangement.rulerBar', { bar: at }) : t('arrangement.rulerBeat', { bar: at, beat })
    }
    const entry = cursorPos ? entries[cursorPos.row] : undefined
    if (!cursorPos || !entry) return ''
    const into = cursorPos.tick % bar
    const vars = { track: trackLabel(entry, t), bar: Math.floor(cursorPos.tick / bar) + 1, beat: Number((into / PPQ + 1).toFixed(2)) }
    return into === 0 ? t('arrangement.cursorBar', vars) : t('arrangement.cursorBeat', vars)
  })

  /** Brings the cursor into the window (time and rows), then gives it the focus. */
  async function focusCursor() {
    const at = cursorPos
    if (!at) return
    onCursor = true
    const [from, to] = visibleRange(shown, stageW)
    if (at.tick < from || at.tick > to) {
      follow = false
      view = clampView({ pxPerTick: shown.pxPerTick, scrollTick: at.tick - (stageW / shown.pxPerTick) * PAGE_LEAD }, stageW, totalTicks)
    }
    // The ruler row does not scroll with the tracks.
    if (at.row >= 0) {
      const top = layout.trackTop[at.row] ?? 0
      const height = laneHeight(layout, at.row)
      const room = stageH - RULER_HEIGHT
      if (top < y0) scrollY = clampScrollY(top, layout.total, stageH)
      else if (top + height > y0 + room) scrollY = clampScrollY(top + height - room, layout.total, stageH)
    }
    await tick()
    stage?.querySelector<HTMLElement>('[data-testid="arr-cursor"]')?.focus()
  }

  /** The playhead goes to the free cursor's tick (`transport.seek`), and the live region says where. */
  function putHeadAtCursor() {
    const at = cursorPos
    if (!at) return
    placeHead(at.tick)
    announce(t('arrangement.saidPlayhead', barBeat(at.tick)))
  }

  function onCursorKey(event: KeyboardEvent, key: NonNullable<RovingKey>) {
    const at = cursorPos
    // Delete and Escape go on to the timeline (they remove or clear the selection); a velocity means nothing here.
    if (!at || key.kind === 'delete' || key.kind === 'free' || key.kind === 'velocity') return
    event.preventDefault()
    event.stopPropagation()
    if (key.kind === 'focus' || key.kind === 'extend') {
      // Up from the first track reaches the ruler row, Down from it comes back (#554).
      if (key.direction === 'up' && at.row <= 0 && key.kind === 'focus') {
        cursorAt = { row: -1, tick: at.tick }
        void focusCursor()
        return
      }
      if (key.direction === 'down' && at.row === -1) {
        cursorAt = { row: 0, tick: at.tick }
        void focusCursor()
        return
      }
      if (at.row === -1 && (key.direction === 'up' || key.direction === 'down')) return
      cursorAt = moveCursor(
        at,
        key.direction,
        { tick: key.kind === 'extend' ? bar : timelineCursorStep(step, beatTicks(signature), bar), row: 1 },
        entries.length,
        totalBars * bar,
      )
      void focusCursor()
      return
    }
    if (key.kind !== 'open') return
    // Enter on the ruler row puts the playhead at the cursor's tick (#554); Shift+Enter does it on a track's row (`onItemKey`).
    if (at.row === -1) return putHeadAtCursor()
    const under = itemAt(items, at)
    const entry = entries[at.row]
    if (under) {
      const block = blocks.find((b) => b.id === under.id)!
      picked = new Set([block.id])
      select(block.track)
      ui.editClip(block.clip, block.id)
      void focusItem(block.id)
      return
    }
    if (!entry) return
    if (!isMidi(entry.track)) return announce(t('arrangement.saidNotMidi'))
    if (!isFree(blocks, at.row, at.tick, at.tick + bar)) return announce(t('arrangement.saidNoRoom'))
    select(entry.id)
    run(createClipAt(entry.id, at.tick, bar))
    const made = blocks.find((b) => b.row === at.row && b.start === at.tick)
    if (!made) return
    picked = new Set([made.id])
    ui.editClip(made.clip, made.id)
    announce(t('arrangement.saidCreated', { bar: Math.floor(at.tick / bar) + 1 }))
    void focusItem(made.id)
  }

  // Undo and redo say what they did here too (#494), from wherever they were asked; what was said before the view opened is not said again.
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
  /** Whether the keyboard was last in the clips' grid (a focus that leaves it for another element clears it; a removed cell does not). */
  let itemsHadFocus = false
  function onItemsFocusOut(event: FocusEvent) {
    const to = event.relatedTarget as Node | null
    if (to && !itemsEl?.contains(to)) itemsHadFocus = false
  }
  /** An undo or a redo can take away the focused clip: the focus goes back to the grid's tab stop. */
  async function keepItemsFocus() {
    await tick()
    const active = document.activeElement
    if (active && active !== document.body) return
    itemsEl?.querySelector<HTMLElement>('[tabindex="0"]')?.focus()
  }

  // The window loses the pointer: nothing may stay half done.
  const silence = () => cancel()

  // A plain vertical wheel scrolls the lanes, over the grid or over the track headers; the grid keeps Shift and Ctrl for time and zoom.
  $effect(() => {
    const elements = [stage, heads].filter((e): e is HTMLDivElement => !!e)
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey || event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return
      event.preventDefault()
      const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? stageH : 1
      scrollY = clampScrollY(y0 + event.deltaY * scale, layout.total, stageH)
    }
    for (const element of elements) element.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      for (const element of elements) element.removeEventListener('wheel', onWheel)
    }
  })

  // The headers column never scrolls on its own: a focus or a `scrollIntoView` that moves it is turned into the view's one vertical scroll.
  let heads: HTMLDivElement | undefined = $state()
  function onHeadsScroll() {
    const element = heads
    if (!element || element.scrollTop === 0) return
    scrollY = clampScrollY(y0 + element.scrollTop, layout.total, stageH)
    element.scrollTop = 0
  }

  // A focus in the headers brings its row into view, up as well as down (the column cannot scroll up on its own: it is moved by an offset).
  function onHeadsFocus(event: FocusEvent) {
    const target = event.target
    if (!(target instanceof HTMLElement)) return
    // After the frame: the column's own scroll (a focus below the window) has been folded into `scrollY` by then.
    requestAnimationFrame(() => {
      const column = heads
      if (!column || !target.isConnected) return
      const row = target.closest('.row, .lane-head, .add-head') ?? target
      const box = row.getBoundingClientRect()
      const area = column.getBoundingClientRect()
      const top = area.top + RULER_HEIGHT
      // Whole pixels: the scrollbar beside the rows holds whole pixels, and the rows must stay level with it.
      if (box.top < top) scrollY = clampScrollY(Math.floor(y0 - (top - box.top)), layout.total, stageH)
      else if (box.bottom > area.bottom) scrollY = clampScrollY(Math.ceil(y0 + (box.bottom - area.bottom)), layout.total, stageH)
    })
  }

  // The vertical scrollbar is a native one beside the grid; its content is the rows, so its range is the view's.
  let vbar: HTMLDivElement | undefined = $state()
  const overflowing = $derived(layout.total > stageH - RULER_HEIGHT)
  $effect(() => {
    const element = vbar
    if (!element) return
    // To the pixel: the rows are 44 px high (#471), so a focus can land a pixel off, which the bar must still show.
    if (Math.abs(element.scrollTop - y0) >= 0.5) element.scrollTop = y0
  })
  function onVBarScroll() {
    const element = vbar
    if (!element) return
    const expected = Math.min(y0, element.scrollHeight - element.clientHeight)
    if (Math.abs(element.scrollTop - expected) <= SYNC_PX) return
    scrollY = clampScrollY(element.scrollTop, layout.total, stageH)
  }
</script>

{#snippet fold(id: string, lanes: number, label: string)}
  <button
    class="fold"
    class:has={lanes > 0}
    aria-expanded={open.has(id)}
    aria-label={label}
    title={lanes > 0 ? `${label} · ${t('automation.lanes', { n: lanes })}` : undefined}
    data-testid="arr-automation-toggle"
    onclick={() => toggleOpen(id)}
  >
    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"
      ><path
        d={open.has(id) ? 'M2 4 L 6 8 L 10 4' : 'M4 2 L 8 6 L 4 10'}
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
      /></svg
    >
    {#if lanes > 0}<span class="n">{lanes}</span>{/if}
  </button>
{/snippet}

<svelte:window
  onblur={silence}
  onpointerdown={(event) => {
    if (menu && !(event.target as Element | null)?.closest?.('[data-point-menu]')) menu = null
  }}
/>
<svelte:document onvisibilitychange={silence} />

<div
  class="arr"
  data-testid="arrangement"
  data-block-count={blocks.length}
  data-selected-count={selection.length}
  data-blocks={blocks.length <= 48
    ? JSON.stringify(blocks.map(({ id, track, row, clip, start, length, offset }) => ({ id, track, row, clip, start, length, offset })))
    : undefined}
  data-selection={JSON.stringify(selection.map((b) => b.id))}
  data-loop={transport?.loop_region ? `${transport.loop_region.start}-${transport.loop_region.end}` : ''}
  data-loop-on={loopOn}
  data-position={position ?? ''}
  data-follow={follow}
>
  <div class="toolbar" role="toolbar" aria-label={t('arrangement.tools')}>
    <label class="field">
      <span>{t('pianoRoll.snap')}</span>
      <select aria-label={t('pianoRoll.snap')} data-testid="arr-snap" bind:value={snap}>
        {#each SNAP_SETTINGS as setting (setting)}
          <option value={setting}>{t(`pianoRoll.snap.${setting}`)}</option>
        {/each}
      </select>
    </label>
    <button title={t('arrangement.splitHint')} disabled={cuts.length === 0} data-testid="arr-split" onclick={splitSelected}>{t('arrangement.split')}</button>
    <button title={t('arrangement.duplicateHint')} disabled={selection.length === 0} data-testid="arr-duplicate" onclick={() => duplicate(false)}
      >{t('arrangement.duplicate')}{#if selection.length > 1}
        ({selection.length}){/if}</button
    >
    <button disabled={selection.length === 0 && !autoSel} data-testid="arr-delete" onclick={() => (autoSel ? removePicked() : removeSelected())}
      >{t('arrangement.delete')}</button
    >
    <span class="zoom" role="group" aria-label={t('arrangement.zoom')}>
      <button
        title={t('arrangement.zoomOut')}
        aria-label={t('arrangement.zoomOut')}
        disabled={!canZoomOut}
        data-testid="arr-zoom-out"
        onclick={() => zoomBy(1 / ZOOM_STEP)}>−</button
      >
      <button
        title={t('arrangement.zoomIn')}
        aria-label={t('arrangement.zoomIn')}
        disabled={!canZoomIn}
        data-testid="arr-zoom-in"
        onclick={() => zoomBy(ZOOM_STEP)}>+</button
      >
      <button title={t('arrangement.fitSongHint')} aria-label={t('arrangement.fitSong')} data-testid="arr-fit-song" onclick={fitSong}
        >{t('arrangement.fitSongShort')}</button
      >
    </span>
    <button class="follow" title={t('arrangement.followHint')} aria-pressed={follow} data-testid="arr-follow" onclick={followAgain}
      >{t('arrangement.follow')}</button
    >
    <button
      class="more"
      title={t('arrangement.moreHint')}
      aria-label={t('arrangement.more')}
      aria-haspopup="menu"
      aria-expanded={ui.contextMenu?.label === t('arrangement.more')}
      data-testid="arr-more"
      onclick={openMore}>⋯</button
    >
    <span class="count" data-testid="arr-count"
      >{t('arrangement.count', { n: blocks.length })}{#if selection.length > 0}
        · {t('arrangement.selected', { n: selection.length })}{/if}{#if autoSel}
        · {t('automation.selectedPoints', { n: autoSel.points.length })}{/if}</span
    >
  </div>

  <div class="board" class:scrolls={overflowing} style:--head="276px" style:--ruler="{RULER_HEIGHT}px">
    {#if blocks.length === 0}
      <!-- Nothing to arrange yet. No track: the master row stays visible and the welcome message sits in the empty tracks area below it.
           A track and no clip: the same guide, one step shorter (#474). Under the last row, out of the lanes' way. -->
      <div
        class="empty"
        data-testid={entries.length === 0 ? 'arrangement-empty' : 'arrangement-no-clips'}
        style:top="{Math.min(RULER_HEIGHT + layout.total - y0, Math.max(RULER_HEIGHT, stageH - 140))}px"
      >
        <p class="lead">{t(entries.length === 0 ? 'arrangement.empty' : 'timeline.guide.noClips')}</p>
        <ol class="guide">
          {#if !engine.started}<li>{t('timeline.guide.step0')}</li>{/if}
          {#if entries.length === 0}<li>{t('timeline.guide.step1')}</li>{/if}
          <li>{t('timeline.guide.step2')}</li>
          <li>{t('timeline.guide.step3')}</li>
        </ol>
        <button type="button" class="guide-keys" data-testid="guide-shortcuts" onclick={() => (ui.helpOpen = true)}>{t('timeline.guide.shortcuts')}</button>
      </div>
    {/if}
    <div class="heads" bind:this={heads} onscroll={onHeadsScroll} onfocusin={onHeadsFocus} data-testid="arr-heads">
      <div class="slide" style:top="{-y0}px">
        {#each layout.rows as row (row.kind === 'lane' ? `lane:${row.lane}` : `${row.kind}:${row.trackId}`)}
          {#if row.kind === 'track'}
            {@const entry = allEntries[row.track]!}
            {@const isMaster = entry.id === MASTER}
            {@const lanes = Object.keys(entry.track.automation).length}
            {#if isMaster}
              <!-- The master row: pinned at the bottom, no back-track button, no colour swatch, cannot be deleted or moved. -->
              <div class="row master-row" style:height="{row.height}px" data-testid="arr-master-row">
                <div class="head master-head">
                  <span class="swatch" style:background="var(--fk-text-muted)"></span>
                  <span class="label">{t('mixer.master')}</span>
                </div>
                <div class="lane-meter">
                  <LevelMeter
                    snap={meters.snap(MASTER_METER)}
                    orientation="horizontal"
                    label={t('meter.master')}
                    clipLabel={t('meter.clip')}
                    resetLabel={t('meter.reset', { name: t('mixer.master') })}
                    onreset={() => meters.reset(MASTER_METER)}
                    testid="arr-master-meter"
                  />
                </div>
                {@render fold(MASTER, lanes, t(open.has(MASTER) ? 'automation.hideMaster' : 'automation.showMaster'))}
              </div>
            {:else}
              {@const trackEntry = entries[row.track]!}
              <LaneHeader
                entry={trackEntry}
                height={row.height}
                selected={project.selected?.id === trackEntry.id}
                soloed={project.soloed.includes(trackEntry.id)}
                armed={project.armed.includes(trackEntry.id)}
                implied={project.silencedBySolo(trackEntry.id)}
                detached={detached.has(trackEntry.id)}
                {drag}
                dragging={dragId === trackEntry.id}
                dropTarget={dragId !== null && overId === trackEntry.id && dragId !== trackEntry.id}
              >
                {#if detached.has(trackEntry.id)}
                  <button
                    class="back"
                    title={t('arrangement.backTrackHint', { name: trackLabel(trackEntry, t) })}
                    aria-label={t('arrangement.backTrack') + ' · ' + trackLabel(trackEntry, t)}
                    data-testid="arr-back-track"
                    onclick={() => run(backTrackToArrangement(trackEntry.id))}
                  >
                    <span aria-hidden="true">↩</span>
                  </button>
                {/if}
                {@render fold(trackEntry.id, lanes, t(open.has(trackEntry.id) ? 'automation.hide' : 'automation.show', { name: trackLabel(trackEntry, t) }))}
              </LaneHeader>
            {/if}
          {:else if row.kind === 'lane'}
            {@const ref = laneRefs.get(row.lane ?? '')}
            {#if ref}
              {@const live = shownTick === null ? null : valueAt(pointsOf(ref.info.lane), shownTick, ref.info.lane.target)}
              <div
                class="lane-head"
                style:height="{row.height}px"
                style:--swatch={ref.color}
                role="group"
                aria-label={t('automation.lane', { title: ref.info.title })}
                title={t('automation.laneHint')}
                data-testid="arr-lane"
                data-lane={row.lane}
                data-target={ref.info.title}
                data-points={Object.keys(ref.info.lane.points).length}
              >
                <span class="swatch"></span>
                <span class="lane-name"><span class="group">{ref.info.group}</span><span class="param">{ref.info.label}</span></span>
                <span class="lane-value" title={t('automation.valueNow')} data-testid="arr-lane-value">{live === null ? '' : ref.info.axis.format(live)}</span>
                <button
                  class="lane-delete"
                  aria-label={t('automation.deleteLane', { title: ref.info.title })}
                  title={t('automation.deleteLane', { title: ref.info.title })}
                  data-testid="arr-lane-delete"
                  onclick={() => run(deleteLane(row.lane!))}
                >
                  <svg width="10" height="10" viewBox="0 0 14 14" aria-hidden="true"
                    ><path d="M3 3 L 11 11 M11 3 L 3 11" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg
                  >
                </button>
              </div>
            {/if}
          {:else}
            {@const entry = allEntries[row.track]!}
            {@const realEntry = entries[row.track]}
            {@const free = freeOptions(entry.id)}
            <div class="add-head" style:height="{row.height}px">
              <select
                aria-label={entry.id === MASTER
                  ? t('automation.addLaneForMaster')
                  : t('automation.addLaneFor', { name: realEntry ? trackLabel(realEntry, t) : '' })}
                disabled={free.length === 0}
                data-testid="arr-add-lane"
                value=""
                onchange={(e) => {
                  addLane(entry.id, e.currentTarget.value)
                  e.currentTarget.value = ''
                }}
              >
                <option value="" disabled selected>{t('automation.addLane')}</option>
                {#each [...new Set(free.map((o) => o.group))] as group (group)}
                  <optgroup label={group}>
                    {#each free.filter((o) => o.group === group) as option (option.key)}
                      <option value={option.key}>{option.label}</option>
                    {/each}
                  </optgroup>
                {/each}
              </select>
            </div>
          {/if}
        {/each}
      </div>
    </div>
    <div
      class="stage"
      bind:this={stage}
      bind:clientWidth={stageW}
      bind:clientHeight={stageH}
      role="presentation"
      data-testid="arr-stage"
      id="arr-stage-grid"
      data-scroll-y={y0}
      data-cursor={gesture ? undefined : cursor}
      data-gesture={gesture ? gesture.kind + (gesture.moved ? ' moved' : '') : undefined}
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={() => finish(false)}
      onlostpointercapture={() => gesture && finish(false)}
      ondblclick={onDoubleClick}
      ondragover={onDragOver}
      ondrop={onDrop}
      oncontextmenu={onContextMenu}
      onkeydown={onKeyDown}
      onkeydowncapture={handScroll}
      onwheelcapture={handScroll}
    >
      <TimeGrid bind:this={grid} {signature} {totalBars} bind:view {snap} playhead={position} drawBody={body} label={t('arrangement.grid')} />
      <!-- The section markers on the ruler (#473): named buttons; a click puts the playhead there. -->
      <div class="markers" role="group" aria-label={t('arrangement.markers')} data-testid="arr-markers">
        {#each markers as marker (marker.id)}
          {@const x = tickToX(marker.tick, shown)}
          {#if x >= -200 && x <= stageW}
            {#if renaming?.id === marker.id}
              <input
                class="marker-field"
                style:left="{Math.max(0, x)}px"
                aria-label={t('arrangement.markerRename')}
                data-testid="arr-marker-field"
                bind:value={renaming.text}
                onkeydown={(e) => {
                  if (e.key === 'Enter') commitRename()
                  else if (e.key === 'Escape') renaming = null
                  e.stopPropagation()
                }}
                onblur={commitRename}
                {@attach (el) => el.focus()}
              />
            {:else}
              <button
                class="marker"
                type="button"
                style:left="{x}px"
                title={t('arrangement.markerHint')}
                aria-label={t('arrangement.markerSaid', { name: marker.name, bar: Math.floor(marker.tick / bar) + 1 })}
                data-testid="arr-marker"
                data-marker={marker.id}
                onpointerdown={(e) => e.stopPropagation()}
                onclick={() => {
                  lastMarker = marker.id
                  jumpedTo = null
                  run(seek(marker.tick))
                }}
                ondblclick={(e) => {
                  e.stopPropagation()
                  renaming = { id: marker.id, text: marker.name }
                }}
                onkeydown={(e) => onMarkerKey(e, marker)}>{marker.name}</button
              >
            {/if}
          {/if}
        {/each}
      </div>
      <!-- The clips as named cells over the canvas (#471): the keyboard's and the screen reader's way to them. The pointer goes through to the canvas. -->
      <!-- The cells are the focusable parts of the grid (roving tabindex), as in the launch grid; the grid only listens for their keys. -->
      <!-- svelte-ignore a11y_interactive_supports_focus -->
      <div
        class="items"
        bind:this={itemsEl}
        role="grid"
        aria-label={t('arrangement.clipsGrid')}
        aria-rowcount={entries.length}
        aria-multiselectable="true"
        style:top="{RULER_HEIGHT}px"
        data-testid="arr-items"
        onkeydown={onItemKey}
        onfocusin={onItemsFocus}
        onfocusout={onItemsFocusOut}
      >
        {#each entries as entry, row (entry.id)}
          {@const cells = shownItems.filter((i) => i.block.track === entry.id)}
          {@const here = cursorPos?.row === row && cursorCell && stageW > 0 ? cursorCell : null}
          {#if cells.length > 0 || here}
            <div role="row" aria-rowindex={entry.index + 1} aria-label={trackLabel(entry, t)}>
              {#if here}
                <div
                  class="item cursor"
                  role="gridcell"
                  tabindex={cursorTab ? 0 : -1}
                  aria-selected="false"
                  aria-label={cursorText}
                  data-cursor=""
                  data-testid="arr-cursor"
                  data-row={cursorPos?.row}
                  data-tick={cursorPos?.tick}
                  style:left="{here.left}px"
                  style:top="{here.top - RULER_HEIGHT}px"
                  style:width="{here.width}px"
                  style:height="{here.height}px"
                ></div>
              {/if}
              {#each cells as cell (cell.block.id)}
                <div
                  class="item"
                  role="gridcell"
                  tabindex={cell.block.id === tabId && !cursorTab ? 0 : -1}
                  aria-selected={selectedIds.has(cell.block.id)}
                  aria-label={clipText(cell.block)}
                  data-block={cell.block.id}
                  data-testid="arr-item"
                  style:left="{cell.left}px"
                  style:top="{cell.top - RULER_HEIGHT}px"
                  style:width="{cell.width}px"
                  style:height="{cell.height}px"
                ></div>
              {/each}
            </div>
          {/if}
        {/each}
      </div>
      <!-- The ruler row of the keyboard cursor (#554): one cell, where Enter puts the playhead. -->
      <!-- The group only relays the keys of its one focusable cell, as the clips' grid does. -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        class="ruler-items"
        role="group"
        aria-label={t('arrangement.rulerRow')}
        style:height="{RULER_HEIGHT}px"
        onkeydown={onItemKey}
        onfocusin={onItemsFocus}
        onfocusout={onItemsFocusOut}
      >
        {#if rulerCell}
          <div
            class="item cursor"
            role="button"
            tabindex={cursorTab ? 0 : -1}
            aria-label={cursorText}
            data-cursor=""
            data-testid="arr-cursor"
            data-row="-1"
            data-tick={cursorPos?.tick}
            style:left="{rulerCell.left}px"
            style:top="0px"
            style:width="{rulerCell.width}px"
            style:height="{RULER_HEIGHT}px"
          ></div>
        {/if}
      </div>
      <p class="sr-only" aria-live="polite" data-testid="arr-said" data-n={said.n}>{said.text}</p>
      {#if limitNote}
        <p class="limit-note" role="status" data-testid="value-note" style:left="{limitNote.x}px" style:top="{limitNote.y + 14}px">{limitNote.text}</p>
      {/if}
      {#if editing}
        <div class="anchor" bind:this={editAnchor} style:left="{editing.x}px" style:top="{editing.y}px"></div>
        {#if editAnchor}
          <ValueField
            anchor={editAnchor}
            initial={(() => {
              const ref = laneRefs.get(editing.lane)
              const point = ref ? pointAtTick(pointsOf(ref.info.lane), editing.tick) : null
              return ref && point ? ref.info.axis.format(point.value) : ''
            })()}
            label={t('automation.pointField')}
            commit={commitValue}
            onclose={(refocus) => {
              editing = null
              editAnchor = undefined
              if (refocus) focusGrid()
            }}
          />
        {/if}
      {/if}
    </div>
    {#if overflowing}
      <div
        class="vbar"
        bind:this={vbar}
        onscroll={onVBarScroll}
        data-testid="arr-vscrollbar"
        role="scrollbar"
        aria-controls="arr-stage-grid"
        aria-orientation="vertical"
        aria-valuemin="0"
        aria-valuemax={Math.round(maxScrollY(layout.total, stageH))}
        aria-valuenow={Math.round(y0)}
        tabindex="0"
        aria-label={t('arrangement.scrollRows')}
      >
        <div style:height="{layout.total}px" style:width="1px"></div>
      </div>
    {/if}
    <div class="corner"></div>
    <div
      class="hbar"
      bind:this={scrollbar}
      onscroll={onBarScroll}
      data-testid="arr-scrollbar"
      role="scrollbar"
      aria-controls="arr-stage-grid"
      aria-orientation="horizontal"
      aria-valuemin="0"
      aria-valuemax={totalTicks}
      aria-valuenow={Math.round(shown.scrollTick)}
      tabindex="0"
      aria-label={t('arrangement.scroll')}
    >
      <div style:width="{totalTicks * shown.pxPerTick}px" style:height="1px"></div>
    </div>
  </div>
  {#if menu}
    <div class="menu" role="menu" aria-label={t('automation.menu')} style:left="{menu.x}px" style:top="{menu.y}px" data-point-menu data-testid="point-menu">
      <button
        role="menuitem"
        data-testid="point-menu-linear"
        onclick={() => {
          setCurve('linear')
          menu = null
          focusGrid()
        }}>{t('automation.menuLinear')}</button
      >
      <button
        role="menuitem"
        data-testid="point-menu-step"
        onclick={() => {
          setCurve('step')
          menu = null
          focusGrid()
        }}>{t('automation.menuStep')}</button
      >
      <button
        role="menuitem"
        data-testid="point-menu-delete"
        onclick={() => {
          removePicked()
          menu = null
          focusGrid()
        }}>{t('automation.menuDelete')}</button
      >
    </div>
  {/if}
</div>

<BarDialog
  open={repeating !== null}
  name="repeat"
  title={t('arrangement.repeatTitle')}
  label={t('arrangement.repeatLabel', { min: repeating?.min ?? 1 })}
  invalid={t('arrangement.repeatInvalid', { min: repeating?.min ?? 1, max: MAX_GOTO_BAR })}
  go={t('arrangement.repeatGo')}
  cancel={t('arrangement.repeatCancel')}
  initial={repeating?.min ?? 1}
  min={repeating?.min ?? 1}
  max={MAX_GOTO_BAR}
  onsubmit={repeatUpTo}
  onclose={() => (repeating = null)}
/>
<BarDialog
  open={inserting}
  name="insert"
  title={t('arrangement.insertTitle')}
  label={t('arrangement.insertLabel')}
  invalid={t('arrangement.insertInvalid', { max: MAX_INSERT_BARS })}
  go={t('arrangement.insertGo')}
  cancel={t('arrangement.repeatCancel')}
  initial={barCount}
  min={1}
  max={MAX_INSERT_BARS}
  onsubmit={insertBarsFromDialog}
  onclose={() => (inserting = false)}
/>

<style>
  .empty {
    position: absolute;
    left: calc(var(--head) + var(--fk-space-4));
    width: min(66ch, calc(100% - var(--head) - 2 * var(--fk-space-4)));
    z-index: 1;
    margin: 0;
    padding: var(--fk-space-4);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    color: var(--fk-text-muted);
    pointer-events: none;
  }
  .empty .lead {
    margin: 0 0 var(--fk-space-2);
    color: var(--fk-text);
    font-weight: 600;
  }
  .guide {
    margin: 0 0 var(--fk-space-3);
    padding-left: var(--fk-space-5);
    max-width: 62ch;
    display: grid;
    gap: var(--fk-space-1);
  }
  .guide-keys {
    pointer-events: auto;
  }
  .arr {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    flex-wrap: nowrap;
    align-items: center;
    gap: var(--fk-space-2) var(--fk-space-3);
    padding: var(--fk-space-1) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
    font-size: 12px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
    color: var(--fk-text-muted);
  }
  button,
  select {
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
  .count {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-left: auto;
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
  }
  .board {
    display: grid;
    grid-template-columns: var(--head) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) auto;
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .board.scrolls {
    grid-template-columns: var(--head) minmax(0, 1fr) auto;
  }
  .heads {
    overflow: hidden;
    padding-top: var(--ruler);
    border-right: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  /* Moved by its offset, not a transform: a transform would hold the rows' fixed layers (the volume field) inside the column. */
  .slide {
    position: relative;
  }
  .row {
    display: flex;
    align-items: stretch;
    border-bottom: 1px solid var(--fk-line);
    box-sizing: border-box;
  }
  .lane-meter {
    display: flex;
    flex: none;
    align-items: center;
    padding: 0 var(--fk-space-1);
  }
  .lane-meter :global(.meter) {
    --len: 52px;
  }
  /* The master row: same layout as a track row but without a clickable button head, and with a distinct top border. */
  .master-row {
    border-top: 1px solid var(--fk-text-muted);
  }
  .master-head {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 var(--fk-space-3);
    font-weight: 600;
    color: var(--fk-text-muted);
  }
  .back {
    flex: none;
    align-self: center;
    margin-right: var(--fk-space-1);
    padding: 0 var(--fk-space-1);
    border-color: var(--fk-text);
    color: var(--fk-text);
    background: var(--fk-surface-2);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 var(--fk-space-3);
    border: 0;
    border-radius: 0;
    background: transparent;
    text-align: left;
  }
  .swatch {
    flex: none;
    width: 4px;
    align-self: stretch;
    margin: 6px 0;
    border-radius: 2px;
    background: var(--swatch);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hbar {
    overflow-x: scroll;
    overflow-y: hidden;
    /* The bar's own thickness (12px, below) and the line above it: the room is kept even where scrollbars overlay. */
    height: 13px;
    border-top: 1px solid var(--fk-line);
  }
  /* Always a visible bar, whatever the platform's overlay scrollbars do: styling the bar turns the overlay off. */
  .hbar::-webkit-scrollbar {
    height: 12px;
  }
  .hbar::-webkit-scrollbar-track {
    background: var(--fk-surface);
  }
  .hbar::-webkit-scrollbar-thumb {
    border: 2px solid var(--fk-surface);
    border-radius: 6px;
    background: var(--fk-text-muted);
  }
  @supports not selector(::-webkit-scrollbar) {
    .hbar {
      scrollbar-width: auto;
      scrollbar-color: var(--fk-text-muted) var(--fk-surface);
    }
  }
  /* Beside the grid, under the ruler: always a visible bar, as the horizontal one. */
  .vbar {
    grid-column: 3;
    grid-row: 1;
    overflow-x: hidden;
    overflow-y: scroll;
    width: 13px;
    margin-top: var(--ruler);
    /* Its content (all the rows) must not size the grid row: the bar takes the height of the stage. */
    contain: size;
    border-left: 1px solid var(--fk-line);
  }
  .vbar::-webkit-scrollbar {
    width: 12px;
  }
  .vbar::-webkit-scrollbar-track {
    background: var(--fk-surface);
  }
  .vbar::-webkit-scrollbar-thumb {
    border: 2px solid var(--fk-surface);
    border-radius: 6px;
    background: var(--fk-text-muted);
  }
  @supports not selector(::-webkit-scrollbar) {
    .vbar {
      scrollbar-width: auto;
      scrollbar-color: var(--fk-text-muted) var(--fk-surface);
    }
  }
  .corner {
    border-right: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  .zoom {
    display: inline-flex;
    gap: 2px;
  }
  /* On: a fill in the text colour and a dot before the word, never a hue. */
  .follow[aria-pressed='true'] {
    border-color: var(--fk-text);
    color: var(--fk-bg);
    background: var(--fk-text);
  }
  .follow[aria-pressed='true']::before {
    content: '● ';
  }
  .stage {
    position: relative;
    min-width: 0;
    min-height: 0;
    touch-action: none;
  }
  /* The clips' cells: invisible over the canvas, except the keyboard's focus, which draws its ring around the clip. */
  .markers {
    position: absolute;
    top: 14px;
    left: 0;
    right: 0;
    height: 14px;
    pointer-events: none;
  }
  .marker,
  .marker-field {
    position: absolute;
    top: 0;
    max-width: 160px;
    height: 14px;
    padding: 0 4px 0 5px;
    border: 0;
    border-left: 1px solid var(--fk-text);
    border-radius: 0;
    background: var(--fk-surface-2, var(--fk-bg));
    color: var(--fk-text);
    font-size: 10px;
    line-height: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    pointer-events: auto;
    cursor: pointer;
  }
  .marker-field {
    width: 140px;
    cursor: text;
  }
  .marker:focus-visible {
    outline: 2px solid var(--fk-accent);
    outline-offset: 1px;
  }
  .items {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    overflow: hidden;
    pointer-events: none;
  }
  .ruler-items {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
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
  /* The free cursor (#494): seen only while it has the keyboard, as a tinted bar where Enter would create the clip. */
  .cursor:focus-visible {
    background: var(--fk-accent-tint);
    outline-style: dashed;
  }
  .stage[data-cursor='move'] {
    cursor: grab;
  }
  .stage[data-cursor='resize'] {
    cursor: ew-resize;
  }
  .stage[data-cursor='add'] {
    cursor: crosshair;
  }
  .limit-note {
    position: absolute;
    z-index: 20;
    margin: 0;
    padding: var(--fk-space-1) var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
    font-size: 11px;
    white-space: nowrap;
    pointer-events: none;
  }
  .anchor {
    position: absolute;
    width: 0;
    height: 0;
    pointer-events: none;
  }
  .fold {
    flex: none;
    align-self: center;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 20px;
    margin-right: var(--fk-space-1);
    padding: 0 var(--fk-space-1);
    border-color: transparent;
    background: transparent;
    color: var(--fk-text-muted);
  }
  .fold[aria-expanded='true'],
  .fold.has {
    color: var(--fk-text);
  }
  .fold .n {
    font-family: var(--fk-font-mono);
    font-size: 11px;
  }
  .lane-head {
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
    padding: 0 var(--fk-space-1) 0 var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
    box-sizing: border-box;
    background: color-mix(in srgb, var(--fk-surface) 55%, transparent);
    font-size: 12px;
  }
  .lane-head .swatch {
    align-self: stretch;
    margin: 10px 0;
  }
  .lane-name {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    line-height: 1.25;
  }
  .lane-name .group {
    color: var(--fk-text-muted);
    font-size: 11px;
  }
  .lane-name .group,
  .lane-name .param {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lane-value {
    flex: none;
    font-family: var(--fk-font-mono);
    font-size: 11px;
    color: var(--fk-text-muted);
  }
  .lane-delete {
    flex: none;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border-color: transparent;
    background: transparent;
    color: var(--fk-text-muted);
  }
  .add-head {
    display: flex;
    align-items: center;
    padding: 0 var(--fk-space-2);
    border-bottom: 1px solid var(--fk-line);
    box-sizing: border-box;
  }
  .add-head select {
    width: 100%;
    min-width: 0;
    font-size: 11px;
  }
  .menu {
    position: fixed;
    z-index: 20;
    display: flex;
    flex-direction: column;
    min-width: 140px;
    padding: var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
  }
  .menu button {
    height: 26px;
    border-color: transparent;
    background: transparent;
    text-align: left;
  }
  .menu button:hover {
    background: var(--fk-surface-2);
  }
</style>
