<script lang="ts">
  /**
   * The arrangement (AR-4): the clips placed on the tracks along the shared time grid. A gesture is previewed
   * locally and sent as ONE command when it ends, so a drag is one history entry and one undo, and no command
   * goes while the pointer moves. The selection, the snap, the zoom and the scroll are the page's view state,
   * not project data.
   */
  import { untrack } from 'svelte'
  import { createClipAt, deletePlacements, duplicatePlacements, movePlacements, resizePlacement, splitPlacement } from '../../lib/arrangement-commands'
  import type { Command } from '../../lib/generated/protocol'
  import { DRAG_SLOP_PX, fineKey } from '../../lib/continuous'
  import TimeGrid from '../../lib/components/timegrid/TimeGrid.svelte'
  import { DEFAULT_PX_PER_TICK, DEFAULT_TOTAL_BARS, SNAP_SETTINGS, clampView, snapStep, snapTick, totalTicksOf, visibleRange, xToTick, zoomAt, type SnapSetting, type View } from '../../lib/components/timegrid/grid'
  import { RULER_HEIGHT, type BodyFrame } from '../../lib/components/timegrid/render'
  import { isTextTarget } from '../../lib/keyboard'
  import { isMidi, trackLabel } from '../../lib/project/tracks'
  import { offTimeline } from '../../lib/off-timeline'
  import { backTrackToArrangement } from '../../lib/scene-commands'
  import { useStores } from '../../lib/stores'
  import { beginGroup, endGroup, selectTrack } from '../../lib/track-commands'
  import { seek, setLoopRegion, toggleLoop } from '../../lib/transport-commands'
  import { liveTick, settle } from '../piano-roll/playhead'
  import { barTicks } from '../transport/time'
  import {
    LANE_HEIGHT,
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

  const { project, engine, ui, dispatcher } = useStores()
  const t = ui.t
  const run = (command: Command) => dispatcher.run(command)

  /** Bars the grid keeps free after the last clip, so that there is always room to place one. */
  const SPARE_BARS = 16

  type Gesture =
    | { kind: 'move' | 'copy'; grabbed: Block; ids: string[]; snapshot: Block[]; grabTick: number; delta: number; moved: boolean; linked: boolean; startX: number; startY: number }
    | { kind: 'resize'; grabbed: Block; edge: 'start' | 'end'; to: number | null; moved: boolean; startX: number }
    | { kind: 'ruler'; from: number; to: number; moved: boolean; startX: number }

  const entries = $derived(project.tracks)
  // The tracks the launch grid holds: their timeline is silent until they are given back.
  const detached = $derived(engine.started ? offTimeline(engine.feedback, project.view?.slots) : new Set<string>())
  const blocks = $derived(blocksOf(entries))
  const signature = $derived(project.view?.project.meter[0]?.signature ?? { numerator: 4, denominator: 4 })
  const bar = $derived(barTicks(signature))
  const totalBars = $derived(Math.max(DEFAULT_TOTAL_BARS, Math.ceil(lastEnd(blocks) / bar) + SPARE_BARS))
  const transport = $derived(project.view?.project.transport ?? null)

  // View state.
  let snap = $state<SnapSetting>('auto')
  let view = $state<View>({ pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 })
  let scrollY = $state(0)
  let stageW = $state(0)
  let stageH = $state(0)
  let picked = $state.raw<ReadonlySet<string>>(new Set())
  let gesture = $state.raw<Gesture | null>(null)
  let cursor = $state<'default' | 'move' | 'resize'>('default')
  /** Whether the view follows the play position while the transport runs. A hand scroll ends it; Follow or the next Play restores it. */
  let follow = $state(true)

  const shown = $derived(clampView(view, stageW, totalBars * bar))
  const step = $derived(snapStep(snap, signature, shown.pxPerTick))
  const y0 = $derived(clampScrollY(scrollY, entries.length, stageH))
  // The selection is kept to the placements the project still has (an undo, an agent).
  const selection = $derived(blocks.filter((b) => picked.has(b.id)))
  const selectedIds = $derived(new Set(selection.map((b) => b.id)))

  const ghosts = $derived.by<Block[]>(() => {
    const g = gesture
    if (!g || !g.moved) return []
    if (g.kind === 'resize') return g.to === null ? [] : [resized(g.grabbed, g.edge, g.to)]
    if (g.kind === 'ruler') return []
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
    void [shownTick, stageW]
    if (running && follow) untrack(showPosition)
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

  const body = (ctx: CanvasRenderingContext2D, frame: BodyFrame) =>
    drawArrangement(ctx, frame, {
      rows: entries.length,
      scrollY: y0,
      blocks,
      hidden,
      selected: selectedIds,
      ghosts,
      loop: loopNow,
      loopOn,
      fallbackName: t('arrangement.clipDefault'),
    })

  // The grid draws in its own frame and cannot see this state: tell it when something it draws changed.
  $effect(() => {
    void [blocks, selectedIds, ghosts, hidden, loopNow, loopOn, y0, entries.length]
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
    const hit = hitBlock(blocks, x, y, shown, y0)
    if (!hit) {
      const row = rowAt(y, y0, entries.length)
      if (row !== null) select(entries[row]!.id)
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
      gesture = { kind: copy ? 'copy' : 'move', grabbed: block, ids, snapshot: blocks.filter((b) => picked.has(b.id)), grabTick: xToTick(x, shown), delta: 0, moved: false, linked: copy && event.shiftKey, startX: x, startY: y }
    } else {
      gesture = { kind: 'resize', grabbed: block, edge: zone, to: null, moved: false, startX: x }
    }
  }

  function onPointerMove(event: PointerEvent) {
    const { x, y } = local(event)
    const g = gesture
    if (!g) {
      const hit = y >= RULER_HEIGHT ? hitBlock(blocks, x, y, shown, y0) : null
      cursor = hit ? (hit.zone === 'body' ? 'move' : 'resize') : 'default'
      return
    }
    if (g.kind === 'ruler') {
      const to = snapTick(xToTick(x, shown), stepOf(event))
      gesture = { ...g, to, moved: g.moved || Math.abs(x - g.startX) >= DRAG_SLOP_PX }
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

  function onPointerUp() {
    finish(true)
  }

  /** Ends the gesture: its one command is sent when `commit` and the arrangement is still what the gesture started on. */
  function finish(commit: boolean) {
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

  function duplicate(linked: boolean) {
    const by = copyShift(selection)
    if (by !== null) copyAndSelect(selection.map((b) => b.id), linked, by)
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
    const hit = hitBlock(blocks, x, y, shown, y0)
    if (hit) {
      // The clip opens in the piano roll below.
      ui.editedClip = hit.block.clip
      select(hit.block.track)
      ui.showPanel('piano-roll')
      return
    }
    const row = rowAt(y, y0, entries.length)
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
    if (event.key === 'Delete' || event.key === 'Backspace') removeSelected()
    else if (event.key === 'Escape') {
      if (gesture) cancel()
      else picked = new Set()
    } else if (mod && event.key.toLowerCase() === 'a') picked = new Set(blocks.map((b) => b.id))
    else if (mod && event.key.toLowerCase() === 'd') duplicate(event.shiftKey)
    else if (mod && !event.shiftKey && event.key.toLowerCase() === 'e') splitSelected()
    else return
    event.preventDefault()
  }

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
      scrollY = clampScrollY(y0 + event.deltaY * scale, entries.length, stageH)
    }
    element.addEventListener('wheel', onWheel, { passive: false })
    return () => element.removeEventListener('wheel', onWheel)
  })
</script>

<svelte:window onblur={silence} />
<svelte:document onvisibilitychange={silence} />

{#if entries.length === 0}
  <p class="empty" data-testid="arrangement-empty">{t('arrangement.empty')}</p>
{:else}
  <div
    class="arr"
    data-testid="arrangement"
    data-block-count={blocks.length}
    data-selected-count={selection.length}
    data-blocks={blocks.length <= 48 ? JSON.stringify(blocks.map(({ id, track, row, clip, start, length, offset }) => ({ id, track, row, clip, start, length, offset }))) : undefined}
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
      <button title={t('arrangement.duplicateHint')} disabled={selection.length === 0} data-testid="arr-duplicate" onclick={() => duplicate(false)}>{t('arrangement.duplicate')}</button>
      <button title={t('arrangement.duplicateLinkedHint')} disabled={selection.length === 0} data-testid="arr-duplicate-linked" onclick={() => duplicate(true)}>{t('arrangement.duplicateLinked')}</button>
      <button disabled={selection.length === 0} data-testid="arr-delete" onclick={removeSelected}>{t('arrangement.delete')}</button>
      <span class="zoom" role="group" aria-label={t('arrangement.zoom')}>
        <button title={t('arrangement.zoomOut')} aria-label={t('arrangement.zoomOut')} disabled={!canZoomOut} data-testid="arr-zoom-out" onclick={() => zoomBy(1 / ZOOM_STEP)}>−</button>
        <button title={t('arrangement.zoomIn')} aria-label={t('arrangement.zoomIn')} disabled={!canZoomIn} data-testid="arr-zoom-in" onclick={() => zoomBy(ZOOM_STEP)}>+</button>
      </span>
      <button class="follow" title={t('arrangement.followHint')} aria-pressed={follow} data-testid="arr-follow" onclick={() => (follow ? (follow = false) : followAgain())}>{t('arrangement.follow')}</button>
      <span class="help" title={t('arrangement.help')}>?</span>
      <span class="count" data-testid="arr-count">{t('arrangement.count', { n: blocks.length })}{#if selection.length > 0} · {t('arrangement.selected', { n: selection.length })}{/if}</span>
    </div>

    <div class="board" style:--head="148px" style:--ruler="{RULER_HEIGHT}px">
      <div class="heads" data-testid="arr-heads">
        <div class="slide" style:transform="translateY({-y0}px)">
          {#each entries as entry (entry.id)}
            <div class="row" title={detached.has(entry.id) ? t('arrangement.offTimeline') : undefined} style:height="{LANE_HEIGHT}px" class:off={detached.has(entry.id)} data-detached={detached.has(entry.id) ? 'true' : undefined}>
              <button
                class="head"
                class:on={project.selected?.id === entry.id}
                style:--swatch={entry.track.color ?? 'var(--fk-text-muted)'}
                aria-pressed={project.selected?.id === entry.id}
                aria-label={t('arrangement.lane', { name: trackLabel(entry, t) })}
                data-testid="arr-track"
                onclick={() => select(entry.id)}
              >
                <span class="swatch"></span>
                <span class="label">{trackLabel(entry, t)}</span>
              </button>
              {#if detached.has(entry.id)}
                <button
                  class="back"
                  title={t('arrangement.backTrackHint', { name: trackLabel(entry, t) })}
                  aria-label={t('arrangement.backTrack') + ' · ' + trackLabel(entry, t)}
                  data-testid="arr-back-track"
                  onclick={() => run(backTrackToArrangement(entry.id))}
                >
                  <span aria-hidden="true">↩</span>
                </button>
              {/if}
            </div>
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
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={() => finish(false)}
        onlostpointercapture={() => gesture && finish(false)}
        ondblclick={onDoubleClick}
        onkeydown={onKeyDown}
        onkeydowncapture={handScroll}
        onwheelcapture={handScroll}
      >
        <TimeGrid bind:this={grid} {signature} {totalBars} bind:view {snap} playhead={position} drawBody={body} label={t('arrangement.grid')} />
      </div>
      <div class="corner"></div>
      <div class="hbar" bind:this={scrollbar} onscroll={onBarScroll} data-testid="arr-scrollbar" role="scrollbar" aria-controls="arr-stage-grid" aria-orientation="horizontal" aria-valuemin="0" aria-valuemax={totalTicks} aria-valuenow={Math.round(shown.scrollTick)} tabindex="0" aria-label={t('arrangement.scroll')}>
        <div style:width="{totalTicks * shown.pxPerTick}px" style:height="1px"></div>
      </div>
    </div>
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
    /* Always a visible bar, whatever the platform's overlay scrollbars do. */
    height: 16px;
    scrollbar-width: auto;
    scrollbar-color: var(--fk-text-muted) var(--fk-surface);
    border-top: 1px solid var(--fk-line);
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
</style>
