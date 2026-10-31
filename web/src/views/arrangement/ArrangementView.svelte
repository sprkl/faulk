<script lang="ts">
  /**
   * The arrangement (AR-4): the clips placed on the tracks along the shared time grid. A gesture is previewed
   * locally and sent as ONE command when it ends, so a drag is one history entry and one undo, and no command
   * goes while the pointer moves. The selection, the snap, the zoom and the scroll are the page's view state,
   * not project data.
   */
  import { untrack } from 'svelte'
  import { createClipAt, deletePlacements, duplicatePlacements, movePlacements, resizePlacement, splitPlacement } from '../../lib/arrangement-commands'
  import { addPoints, createLane, createMasterLane, deleteLane, removePoints, updatePoints } from '../../lib/automation-commands'
  import type { Command, Shape } from '../../lib/generated/protocol'
  import { DRAG_SLOP_PX, fineKey } from '../../lib/continuous'
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
  import { isTextTarget } from '../../lib/keyboard'
  import { NO_COLOR, isMidi, trackLabel } from '../../lib/project/tracks'
  import { offTimeline } from '../../lib/off-timeline'
  import { backTrackToArrangement } from '../../lib/scene-commands'
  import { useStores } from '../../lib/stores'
  import { MASTER } from '../../lib/stores/automated.svelte'
  import { beginGroup, endGroup, selectTrack } from '../../lib/track-commands'
  import { seek, setLoopRegion, toggleLoop } from '../../lib/transport-commands'
  import { liveTick, settle } from '../piano-roll/playhead'
  import { barTicks } from '../transport/time'
  import {
    blocksOf,
    clampScrollY,
    copyShift,
    hitBlock,
    isFree,
    lastEnd,
    moveDelta,
    moved,
    resizeTo,
    resized,
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
  import { layoutOf, rowsBetween, type Row } from './automation/layout'
  import { lanesOf, optionsOf, pointAtTick, pointsOf, targetKey, valueAt, type LaneInfo, type LanePoint, type Labels } from './automation/model'

  const { project, engine, ui, dispatcher } = useStores()
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
        moved: boolean
        linked: boolean
        startX: number
        startY: number
      }
    | { kind: 'resize'; grabbed: Block; edge: 'start' | 'end'; to: number | null; moved: boolean; startX: number }
    | { kind: 'ruler'; from: number; to: number; moved: boolean; startX: number }
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
  // The tracks the launch grid holds: their timeline is silent until they are given back.
  const detached = $derived(engine.started ? offTimeline(engine.feedback, project.view?.slots) : new Set<string>())
  const blocks = $derived(blocksOf(entries))
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

  // View state.
  let snap = $state<SnapSetting>('auto')
  let view = $state<View>({ pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 })
  let scrollY = $state(0)
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
  let open = $state.raw<ReadonlySet<string>>(new Set())
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
  const layout = $derived(layoutOf(allEntries, open, (id) => (shownLanes.get(id) ?? []).map((l) => l.id)))
  /** A lane shown, by its id, with its track (or the master). */
  const laneRefs = $derived.by(() => {
    // A new map each time the lanes change: the derived value is what is reactive.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const refs = new Map<string, { trackId: string; info: LaneInfo; color: string }>()
    for (const e of allEntries) {
      const color = e.id === MASTER ? NO_COLOR : (('color' in e.track ? (e.track.color as string | null | undefined) : null) ?? NO_COLOR)
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
    if (g?.kind === 'ruler' && g.moved && g.from !== g.to) return { start: Math.min(g.from, g.to), end: Math.max(g.from, g.to) }
    return transport?.loop_region ?? null
  })
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
      fallbackName: t('arrangement.clipDefault'),
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
    void [blocks, selectedIds, ghosts, hidden, loopNow, loopOn, y0, layout, laneRefs, autoPick, hover, pointPreview, gesture]
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
      gesture = { kind: 'ruler', from: tick, to: tick, moved: false, startX: x }
      return
    }
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
    if (zone === 'body') {
      const ids = blocks.filter((b) => picked.has(b.id)).map((b) => b.id)
      gesture = {
        kind: copy ? 'copy' : 'move',
        grabbed: block,
        ids,
        snapshot: blocks.filter((b) => picked.has(b.id)),
        grabTick: xToTick(x, shown),
        delta: 0,
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
      gesture = { ...g, moved: true, delta: moveDelta(chosen, g.grabbed, xToTick(x, shown), g.grabTick, stepOf(event)) }
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
    if (ids.length !== g.ids.length || g.delta === 0) return
    if (g.kind === 'move') run(movePlacements(ids, g.delta))
    else copyAndSelect(ids, g.linked, g.delta)
  }

  /** Copies, then selects the copies (the placements that were not there before). */
  function copyAndSelect(ids: string[], linked: boolean, by?: number) {
    const before = new Set(blocks.map((b) => b.id))
    run(duplicatePlacements(ids, linked, by))
    const fresh = blocks.filter((b) => !before.has(b.id)).map((b) => b.id)
    if (fresh.length > 0) picked = new Set(fresh)
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
    const { x, y } = local(event)
    const row = y >= RULER_HEIGHT ? rowAt(layout, y, y0) : null
    const ref = row?.kind === 'lane' && row.lane ? laneRefs.get(row.lane) : undefined
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

  /** Opens the field to type the value of the point under a double click. */
  function editPoint(row: Row, x: number, y: number): boolean {
    const ref = row.lane ? laneRefs.get(row.lane) : undefined
    if (!ref || !row.lane) return false
    const hit = hitPoint(pointsOf(ref.info.lane), ref.info.axis, x, y, shown, geoOf(row))
    if (!hit) return false
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
    const value = ref.info.axis.clamp(parsed)
    if (value !== point.value) run(updatePoints(e.lane, [asPoint({ ...point, value })]))
    return true
  }

  function duplicate(linked: boolean) {
    const by = copyShift(selection)
    if (by !== null)
      copyAndSelect(
        selection.map((b) => b.id),
        linked,
        by,
      )
  }

  function removeSelected() {
    if (selection.length === 0) return
    run(deletePlacements(selection.map((b) => b.id)))
    picked = new Set()
  }

  /** The tick of the play position, whole. */
  const position = $derived(shownTick === null ? null : Math.round(shownTick))
  const cuts = $derived(position === null ? [] : splittable(selection, position))

  function splitSelected() {
    if (position === null || cuts.length === 0) return
    inGroup(cuts.map((b) => splitPlacement(b.id, position)))
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
      ui.editedClip = hit.block.clip
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
    } else if (mod && event.key.toLowerCase() === 'd') duplicate(event.shiftKey)
    else if (mod && !event.shiftKey && event.key.toLowerCase() === 'e') splitSelected()
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

  // The mixer or the devices panel requests, once, that the master's lanes be unfolded (`ui.showMasterLanes()`).
  // The request is a one-shot flag: this effect reads and clears it, adding MASTER to `open` exactly once.
  // It fires both when the request arrives while the view is alive and when the view mounts with a pending
  // request (Automate clicked elsewhere, then navigation brought the user here). Because the flag is consumed,
  // a user who folds the master and comes back is not overridden.
  $effect(() => {
    if (ui.openMasterLanes && untrack(() => ui.takeMasterLanesRequest())) {
      if (!open.has(MASTER)) open = new Set([...open, MASTER])
    }
  })

  // The window loses the pointer: nothing may stay half done.
  const silence = () => cancel()

  // A plain vertical wheel scrolls the lanes; the grid keeps Shift and Ctrl for time and zoom.
  $effect(() => {
    const element = stage
    if (!element) return
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey || event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return
      event.preventDefault()
      const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? stageH : 1
      scrollY = clampScrollY(y0 + event.deltaY * scale, layout.total, stageH)
    }
    element.addEventListener('wheel', onWheel, { passive: false })
    return () => element.removeEventListener('wheel', onWheel)
  })
</script>

<svelte:window
  onblur={silence}
  onpointerdown={(event) => {
    if (menu && !(event.target as Element | null)?.closest?.('[data-point-menu]')) menu = null
  }}
/>
<svelte:document onvisibilitychange={silence} />

{#if allEntries.length <= 1 && Object.keys(masterEntry.track.automation).length === 0}
  <p class="empty" data-testid="arrangement-empty">{t('arrangement.empty')}</p>
{:else}
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
        <select data-testid="arr-snap" bind:value={snap}>
          {#each SNAP_SETTINGS as setting (setting)}
            <option value={setting}>{t(`pianoRoll.snap.${setting}`)}</option>
          {/each}
        </select>
      </label>
      <button title={t('arrangement.splitHint')} disabled={cuts.length === 0} data-testid="arr-split" onclick={splitSelected}>{t('arrangement.split')}</button>
      <button title={t('arrangement.duplicateHint')} disabled={selection.length === 0} data-testid="arr-duplicate" onclick={() => duplicate(false)}
        >{t('arrangement.duplicate')}</button
      >
      <button title={t('arrangement.duplicateLinkedHint')} disabled={selection.length === 0} data-testid="arr-duplicate-linked" onclick={() => duplicate(true)}
        >{t('arrangement.duplicateLinked')}</button
      >
      <button disabled={selection.length === 0 && !autoSel} data-testid="arr-delete" onclick={() => (autoSel ? removePicked() : removeSelected())}
        >{t('arrangement.delete')}</button
      >
      <span class="curves" role="group" aria-label={t('automation.curves')}>
        <button title={t('automation.linearHint')} disabled={!autoSel} data-testid="arr-curve-linear" onclick={() => setCurve('linear')}
          >{t('automation.linear')}</button
        >
        <button title={t('automation.stepHint')} disabled={!autoSel} data-testid="arr-curve-step" onclick={() => setCurve('step')}
          >{t('automation.step')}</button
        >
      </span>
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
      </span>
      <button class="follow" title={t('arrangement.followHint')} aria-pressed={follow} data-testid="arr-follow" onclick={followAgain}
        >{t('arrangement.follow')}</button
      >
      <span class="help" title={t('arrangement.help')}>?</span>
      <span class="count" data-testid="arr-count"
        >{t('arrangement.count', { n: blocks.length })}{#if selection.length > 0}
          · {t('arrangement.selected', { n: selection.length })}{/if}{#if autoSel}
          · {t('automation.selectedPoints', { n: autoSel.points.length })}{/if}</span
      >
    </div>

    <div class="board" style:--head="148px" style:--ruler="{RULER_HEIGHT}px">
      <div class="heads" data-testid="arr-heads">
        <div class="slide" style:transform="translateY({-y0}px)">
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
                  <button
                    class="fold"
                    class:has={lanes > 0}
                    aria-expanded={open.has(MASTER)}
                    aria-label={t(open.has(MASTER) ? 'automation.hideMaster' : 'automation.showMaster')}
                    title={lanes > 0
                      ? `${t(open.has(MASTER) ? 'automation.hideMaster' : 'automation.showMaster')} · ${t('automation.lanes', { n: lanes })}`
                      : undefined}
                    data-testid="arr-automation-toggle"
                    onclick={() => toggleOpen(MASTER)}
                  >
                    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"
                      ><path
                        d={open.has(MASTER) ? 'M2 4 L 6 8 L 10 4' : 'M4 2 L 8 6 L 4 10'}
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.6"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      /></svg
                    >
                    {#if lanes > 0}<span class="n">{lanes}</span>{/if}
                  </button>
                </div>
              {:else}
                {@const trackEntry = entries[row.track]!}
                <div
                  class="row"
                  title={detached.has(trackEntry.id) ? t('arrangement.offTimeline') : undefined}
                  style:height="{row.height}px"
                  class:off={detached.has(trackEntry.id)}
                  data-detached={detached.has(trackEntry.id) ? 'true' : undefined}
                >
                  <button
                    class="head"
                    class:on={project.selected?.id === trackEntry.id}
                    style:--swatch={trackEntry.track.color ?? 'var(--fk-text-muted)'}
                    aria-pressed={project.selected?.id === trackEntry.id}
                    aria-label={t('arrangement.lane', { name: trackLabel(trackEntry, t) })}
                    data-testid="arr-track"
                    onclick={() => select(trackEntry.id)}
                  >
                    <span class="swatch"></span>
                    <span class="label">{trackLabel(trackEntry, t)}</span>
                  </button>
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
                  <button
                    class="fold"
                    class:has={lanes > 0}
                    aria-expanded={open.has(trackEntry.id)}
                    aria-label={t(open.has(trackEntry.id) ? 'automation.hide' : 'automation.show', { name: trackLabel(trackEntry, t) })}
                    title={lanes > 0
                      ? `${t(open.has(trackEntry.id) ? 'automation.hide' : 'automation.show', { name: trackLabel(trackEntry, t) })} · ${t('automation.lanes', { n: lanes })}`
                      : undefined}
                    data-testid="arr-automation-toggle"
                    onclick={() => toggleOpen(trackEntry.id)}
                  >
                    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"
                      ><path
                        d={open.has(trackEntry.id) ? 'M2 4 L 6 8 L 10 4' : 'M4 2 L 8 6 L 4 10'}
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.6"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      /></svg
                    >
                    {#if lanes > 0}<span class="n">{lanes}</span>{/if}
                  </button>
                </div>
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
                  <span class="lane-value" title={t('automation.valueNow')} data-testid="arr-lane-value">{live === null ? '' : ref.info.axis.format(live)}</span
                  >
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
        oncontextmenu={onContextMenu}
        onkeydown={onKeyDown}
        onkeydowncapture={handScroll}
        onwheelcapture={handScroll}
      >
        <TimeGrid bind:this={grid} {signature} {totalBars} bind:view {snap} playhead={position} drawBody={body} label={t('arrangement.grid')} />
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
{/if}

<style>
  .empty {
    margin: 0;
    padding: var(--fk-space-4);
    color: var(--fk-text-muted);
  }
  .arr {
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
  .help {
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border: 1px solid var(--fk-line);
    border-radius: 50%;
    color: var(--fk-text-muted);
    cursor: help;
  }
  .count {
    margin-left: auto;
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
  }
  .board {
    display: grid;
    grid-template-columns: var(--head) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) auto;
    flex: 1;
    min-height: 0;
  }
  .heads {
    overflow: hidden;
    padding-top: var(--ruler);
    border-right: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  .slide {
    will-change: transform;
  }
  .row {
    display: flex;
    align-items: stretch;
    border-bottom: 1px solid var(--fk-line);
    box-sizing: border-box;
  }
  /* A track the grid holds: the lane label is dimmed and a return button sits beside it (never the dimming alone). */
  .row.off .head {
    opacity: 0.6;
  }
  /* The master row: same layout as a track row but without a clickable button head, and with a distinct top border. */
  .master-row {
    border-top: 1px solid var(--fk-accent-fg);
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
    border-color: var(--fk-accent-fg);
    color: var(--fk-accent-fg);
    background: var(--fk-accent-tint);
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
  .head.on {
    background: var(--fk-accent-tint);
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
  .corner {
    border-right: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  .zoom {
    display: inline-flex;
    gap: 2px;
  }
  .follow[aria-pressed='true'] {
    border-color: var(--fk-accent-fg);
    color: var(--fk-accent-fg);
    background: var(--fk-accent-tint);
  }
  .stage {
    position: relative;
    min-width: 0;
    min-height: 0;
    touch-action: none;
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
  .curves {
    display: inline-flex;
    gap: 2px;
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
    color: var(--fk-accent-fg);
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
