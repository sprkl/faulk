<script lang="ts">
  /**
   * The piano roll (PN-1): the notes of one clip of the selected MIDI track on the shared time grid,
   * with a keyboard on the left and a velocity lane under it. A gesture is previewed locally and
   * sent as ONE command when it ends (one batch is one history entry, so one undo takes the whole
   * drag back, and no command is sent while the pointer moves). Selecting notes, the clip being
   * edited, the tool and the zoom are the page's view state, not project data.
   */
  import { onDestroy, untrack } from 'svelte'
  import type { Command, Note } from '../../lib/generated/protocol'
  import { addNotes, auditionOff, auditionOn, createClipAtStart, removeNotes, setClipLength, setClipLoop, updateNotes } from '../../lib/clip-commands'
  import TimeGrid from '../../lib/components/timegrid/TimeGrid.svelte'
  import { DEFAULT_PX_PER_TICK, SNAP_SETTINGS, clampView, snapStep, snapTick, xToTick, type SnapSetting, type View } from '../../lib/components/timegrid/grid'
  import { RULER_HEIGHT, type BodyFrame } from '../../lib/components/timegrid/render'
  import { isTextTarget } from '../../lib/keyboard'
  import { isMidi } from '../../lib/project/tracks'
  import { beginGroup, endGroup } from '../../lib/track-commands'
  import { useStores } from '../../lib/stores'
  import { barTicks, beatTicks } from '../transport/time'
  import { clipLabel, editedClip, midiClips } from './clips'
  import { drawKeys, drawLane, drawRoll } from './draw'
  import {
    DEFAULT_ROW_HEIGHT,
    DEFAULT_VELOCITY,
    acceptable,
    clampScrollY,
    drawnNote,
    hitNote,
    hitVelocity,
    initialScrollY,
    moveNotes,
    noteKey,
    notesInRect,
    pitchAt,
    rectBetween,
    resizeNotes,
    shiftVelocity,
    stillApplies,
    fits,
    toggled,
    velocityAt,
    afterChanges,
    type Change,
    type Layout,
    type Rect,
  } from './roll'
  import Surface from './Surface.svelte'

  const { project, ui, dispatcher } = useStores()
  const t = ui.t
  const run = (command: Command) => dispatcher.run(command)

  /** The most bars a clip can be given from the length field. */
  const MAX_BARS = 256
  /** How long an audition sounds at least, so that a click is heard. */
  const MIN_AUDITION_MS = 150
  const KEYS_WIDTH = 56
  const LANE_HEIGHT = 56
  const ROW = DEFAULT_ROW_HEIGHT

  type Tool = 'draw' | 'select'
  type Gesture =
    | { kind: 'draw'; clip: string; pressTick: number; pitch: number; note: Note | null }
    | { kind: 'move' | 'resize'; clip: string; grabbed: Note; key: string; startX: number; startY: number; changes: Change[] }
    | { kind: 'velocity'; clip: string; grabbed: Note; key: string; changes: Change[] }
    | { kind: 'band'; clip: string; x0: number; y0: number; base: ReadonlySet<string>; rect: Rect }
    | { kind: 'loop'; clip: string; from: number; to: number }

  // What is edited: the selected MIDI track's clip.
  const track = $derived(project.selected && isMidi(project.selected.track) ? project.selected : null)
  const clips = $derived(track ? midiClips(track.track) : [])
  const edited = $derived(editedClip(clips, ui.editedClip))
  const notes = $derived<readonly Note[]>(edited?.notes ?? [])
  const signature = $derived(project.view?.project.meter[0]?.signature ?? { numerator: 4, denominator: 4 })
  const bar = $derived(barTicks(signature))
  const clipLength = $derived(edited?.length ?? bar)
  const totalBars = $derived(Math.max(1, Math.ceil(clipLength / bar)))
  const clipName = $derived(edited ? clipLabel(edited, clips.indexOf(edited), (n) => t('pianoRoll.clipDefault', { n })) : '')

  // View state.
  let tool = $state<Tool>('draw')
  let snap = $state<SnapSetting>('auto')
  let view = $state<View>({ pxPerTick: DEFAULT_PX_PER_TICK, scrollTick: 0 })
  let scrollY = $state(0)
  let stageW = $state(0)
  let stageH = $state(0)
  let laneH = $state(LANE_HEIGHT)
  let lastLength = $state(0)
  let lastVelocity = $state(DEFAULT_VELOCITY)
  let picked = $state.raw<ReadonlySet<string>>(new Set())
  let gesture = $state.raw<Gesture | null>(null)
  let cursor = $state<'draw' | 'move' | 'resize'>('draw')
  let activePitches = $state.raw<ReadonlySet<number>>(new Set())

  const shown = $derived(clampView(view, stageW, totalBars * bar))
  const step = $derived(snapStep(snap, signature, shown.pxPerTick))
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
  const loopShown = $derived(gesture?.kind === 'loop' ? (gesture.from === gesture.to ? edited?.looping ?? null : { start: Math.min(gesture.from, gesture.to), end: Math.max(gesture.from, gesture.to) }) : (edited?.looping ?? null))

  let grid: ReturnType<typeof TimeGrid> | undefined = $state()
  let stage: HTMLDivElement | undefined = $state()

  const body = (ctx: CanvasRenderingContext2D, frame: BodyFrame) =>
    drawRoll(ctx, frame, { layout, notes, hidden, selected: chosenKeys, ghosts, band, clipLength, loop: loopShown })

  // The grid draws in its own frame and cannot see this state: tell it when something it draws changed.
  $effect(() => {
    void [notes, chosenKeys, ghosts, hidden, band, layout, clipLength, loopShown]
    grid?.invalidate()
  })

  // A new clip opens with a clean selection, around its notes, and its last drawn length at one beat.
  let openedFor: string | null = null
  $effect(() => {
    const id = edited?.id ?? null
    if (id === openedFor) return
    if (id !== null && stageH === 0) return
    openedFor = id
    untrack(() => {
      picked = new Set()
      gesture = null
      scrollY = initialScrollY(notes, stageH, { rowHeight: ROW, top: RULER_HEIGHT })
      lastLength = beatTicks(signature)
    })
  })

  // --- audition ---------------------------------------------------------------------------------

  let sounding: { track: string; pitch: number } | null = null
  let soundSeq = 0
  let soundSince = 0

  /** Sounds `pitch` on the edited track (or silences with `null`), one note at a time. */
  function sound(pitch: number | null, velocity = lastVelocity) {
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

  onDestroy(() => sound(null))

  // --- gestures ---------------------------------------------------------------------------------

  const local = (event: { clientX: number; clientY: number }, element: HTMLElement | undefined) => {
    const box = element!.getBoundingClientRect()
    return { x: event.clientX - box.left, y: event.clientY - box.top }
  }

  /** Drops the gesture and the note it sounds, sending nothing. */
  function cancel() {
    gesture = null
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
      const note = drawnNote(xToTick(x, shown), null, pitch, lastVelocity, step, lastLength, clipLength)
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
      return
    }
    if (!edited || edited.id !== g.clip) return cancel()
    if (g.kind === 'loop') {
      gesture = { ...g, to: Math.min(clipLength, snapTick(xToTick(x, shown), step)) }
    } else if (g.kind === 'draw') {
      const end = snapTick(xToTick(x, shown), step)
      const pitch = g.note?.pitch ?? g.pitch
      gesture = { ...g, note: drawnNote(g.pressTick, end, pitch, lastVelocity, step, lastLength, clipLength) }
    } else if (g.kind === 'band') {
      const rect = rectBetween(g.x0, g.y0, x, y)
      picked = new Set([...g.base, ...notesInRect(notes, rect, shown, layout)])
      gesture = { ...g, rect }
    } else if (g.kind === 'move' || g.kind === 'resize') {
      // The notes are read again at each move: a change of the clip under the drag (undo, an agent) ends it.
      if (!chosenKeys.has(g.key)) return cancel()
      let changes: Change[]
      if (g.kind === 'move') {
        const dTick = snapTick(g.grabbed.start + (x - g.startX) / shown.pxPerTick, step) - g.grabbed.start
        changes = moveNotes(chosenNotes, dTick, pitchAt(y, layout) - pitchAt(g.startY, layout), clipLength)
      } else {
        const end = snapTick(xToTick(x, shown), step)
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
      if (grabbed && g.kind === 'velocity') lastVelocity = grabbed.to.velocity
    } else if (g.kind === 'loop') {
      if (g.from === g.to || Math.max(g.from, g.to) > clipLength) return
      run(setClipLoop(g.clip, { start: Math.min(g.from, g.to), end: Math.max(g.from, g.to) }))
    }
  }

  function removeSelected() {
    if (!edited || chosenNotes.length === 0) return
    run(removeNotes(edited.id, chosenNotes))
    picked = new Set()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (isTextTarget(event.target)) return
    if (event.key === 'Delete' || event.key === 'Backspace') {
      removeSelected()
    } else if (event.key === 'Escape') {
      if (gesture) cancel()
      else picked = new Set()
    } else if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === 'a') {
      picked = new Set(notes.map(noteKey))
    } else return
    event.preventDefault()
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
    const { x } = local(event, lane)
    const index = hitVelocity(notes, x, shown)
    if (index === null) return
    const grabbed = notes[index]!
    const key = noteKey(grabbed)
    if (event.shiftKey) {
      const next = toggled(picked, key)
      picked = next.picked
      if (!next.on) return
    } else if (!picked.has(key)) picked = new Set([key])
    lane!.setPointerCapture(event.pointerId)
    gesture = { kind: 'velocity', clip: edited.id, grabbed, key, changes: [] }
    sound(grabbed.pitch, grabbed.velocity)
  }

  function laneMove(event: PointerEvent) {
    const g = gesture
    if (!g || g.kind !== 'velocity') return
    if (!edited || edited.id !== g.clip || !chosenKeys.has(g.key)) return cancel()
    const { y } = local(event, lane)
    const velocity = velocityAt(y, 2, laneH - 4)
    gesture = { ...g, changes: shiftVelocity(chosenNotes, velocity - g.grabbed.velocity) }
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
    // A loop over the whole clip stays over the whole clip when the clip grows (a shorter clip trims it itself): one history entry.
    const whole = edited.looping?.start === 0 && edited.looping.end === clipLength
    if (!whole || length < clipLength) return run(setClipLength(edited.id, length))
    run(beginGroup())
    run(setClipLength(edited.id, length))
    run(setClipLoop(edited.id, { start: 0, end: length }))
    run(endGroup())
  }

  function onLengthChange(event: Event & { currentTarget: HTMLInputElement }) {
    const input = event.currentTarget
    // An empty field is not a length: Number('') is 0, which would shorten the clip to one bar.
    const value = input.value.trim() === '' ? Number.NaN : Number(input.value)
    if (Number.isFinite(value)) setBars(value)
    input.value = String(totalBars)
  }

  const toggleLoop = () => edited && run(setClipLoop(edited.id, edited.looping ? null : { start: 0, end: clipLength }))
</script>

<svelte:window onblur={silence} />
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
    data-testid="piano-roll"
    data-clip={edited.id}
    data-note-count={notes.length}
    data-selected-count={chosenNotes.length}
    data-clip-length={clipLength}
    data-loop={edited.looping ? `${edited.looping.start}-${edited.looping.end}` : ''}
    data-notes={notes.length <= 32 ? JSON.stringify(notes) : undefined}
    data-first-note={notes[0] ? JSON.stringify(notes[0]) : undefined}
    data-selection={chosenNotes.length <= 32 ? JSON.stringify(chosenNotes) : undefined}
  >
    <div class="toolbar">
      {#if clips.length > 1}
        <select aria-label={t('pianoRoll.clip')} data-testid="roll-clip" value={edited.id} onchange={(e) => (ui.editedClip = e.currentTarget.value)}>
          {#each clips as entry, i (entry.id)}
            <option value={entry.id}>{clipLabel(entry, i, (n) => t('pianoRoll.clipDefault', { n }))}</option>
          {/each}
        </select>
      {:else}
        <span class="name" data-testid="roll-clip-name">{clipName}</span>
      {/if}
      <div class="group" role="radiogroup" aria-label={t('pianoRoll.tools')}>
        <button role="radio" aria-checked={tool === 'draw'} title={t('pianoRoll.drawHint')} data-testid="roll-tool-draw" onclick={() => (tool = 'draw')}>{t('pianoRoll.draw')}</button>
        <button role="radio" aria-checked={tool === 'select'} title={t('pianoRoll.selectHint')} data-testid="roll-tool-select" onclick={() => (tool = 'select')}>{t('pianoRoll.select')}</button>
      </div>
      <label class="field">
        <span>{t('pianoRoll.snap')}</span>
        <select data-testid="roll-snap" bind:value={snap}>
          {#each SNAP_SETTINGS as setting (setting)}
            <option value={setting}>{t(`pianoRoll.snap.${setting}`)}</option>
          {/each}
        </select>
      </label>
      <div class="field" role="group" aria-label={t('pianoRoll.length')}>
        <span>{t('pianoRoll.length')}</span>
        <button class="step" aria-label={t('pianoRoll.shorter')} data-testid="roll-shorter" disabled={totalBars <= 1} onclick={() => setBars(totalBars - 1)}>−</button>
        <input type="number" min="1" max={MAX_BARS} step="1" value={totalBars} aria-label={t('pianoRoll.length')} data-testid="roll-length" onchange={onLengthChange} />
        <button class="step" aria-label={t('pianoRoll.longer')} data-testid="roll-longer" disabled={totalBars >= MAX_BARS} onclick={() => setBars(totalBars + 1)}>+</button>
      </div>
      <button aria-pressed={edited.looping !== null} title={t('pianoRoll.loopHint')} data-testid="roll-loop" onclick={toggleLoop}>{t('pianoRoll.loop')}</button>
      <button disabled={chosenNotes.length === 0} data-testid="roll-delete" onclick={removeSelected}>{t('pianoRoll.delete')}</button>
      <span class="count" data-testid="roll-count">{t('pianoRoll.count', { n: notes.length })}{#if chosenNotes.length > 0} · {t('pianoRoll.selected', { n: chosenNotes.length })}{/if}</span>
    </div>

    <div class="board" style:--keys="{KEYS_WIDTH}px" style:--lane="{LANE_HEIGHT}px">
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
        <Surface
          label={t('pianoRoll.keyboard')}
          draw={(ctx, w, h, colors, font) => drawKeys(ctx, w, h, layout, colors, font, activePitches)}
        />
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
        onlostpointercapture={() => gesture && finish(false)}
        onkeydown={onKeyDown}
      >
        <TimeGrid bind:this={grid} {signature} {totalBars} bind:view {snap} drawBody={body} label={t('pianoRoll.grid', { name: clipName })} />
      </div>
      <div class="corner"></div>
      <div
        class="lane"
        bind:this={lane}
        bind:clientHeight={laneH}
        role="presentation"
        data-testid="roll-lane"
        onpointerdown={laneDown}
        onpointermove={laneMove}
        onpointerup={onPointerUp}
        onpointercancel={() => finish(false)}
      >
        <Surface
          label={t('pianoRoll.velocity')}
          draw={(ctx, w, h, colors) => drawLane(ctx, w, h, shown, notes, chosenKeys, ghosts, hidden, clipLength, colors)}
        />
      </div>
    </div>
  </div>
{/if}

<style>
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
  button[aria-pressed='true'],
  button[aria-checked='true'] {
    background: var(--fk-accent-tint);
    border-color: var(--fk-accent-fg);
    color: var(--fk-text);
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
  .count {
    margin-left: auto;
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
