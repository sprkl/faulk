<script lang="ts">
  /**
   * A track's one row in the arrangement (#471): its colour bar (also the grip that reorders it), its name in normal
   * case with its kind and instrument, M, S and In, its meter, and the handle that sets its height. This row is the list
   * of tracks; the left column is the inspector of the selected one (`TrackInspector`), where volume, pan and the
   * devices are. The volume shows here on hover and on focus, and is typed here: on the selected track's head, a digit
   * opens the field with that digit, Enter or F2 on the current value (`lane-keys.ts`). Every action is a command.
   */
  import { tick, untrack, type Snippet } from 'svelte'
  import LevelMeter from '../../lib/components/LevelMeter.svelte'
  import ValueField from '../../lib/components/ValueField.svelte'
  import { formatGain, instrumentName, MAX_GAIN_DB, trackColor, trackLabel, type DragHooks, type TrackEntry } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import { dropTarget as browserDrop } from '../../lib/browser/dnd'
  import { putItem } from '../../lib/browser/put'
  import { trackMenuItems } from '../../lib/track-menu'
  import { armTrack, renameTrack, selectTrack, setMute, setSolo, setTrackHeight, setVolume } from '../../lib/track-commands'
  import { parseGainEntry } from '../../lib/value-entry'
  import { CLIP_ROW_MAX, CLIP_ROW_MIN, clampClipRow } from './automation/layout'
  import { heightKey, volumeKey } from './lane-keys'

  let {
    entry,
    height,
    selected,
    soloed,
    armed,
    implied,
    detached,
    drag,
    dragging = false,
    dropTarget = false,
    children,
  }: {
    entry: TrackEntry
    /** The row's height, in px (the user's, else the `lane` token). */
    height: number
    selected: boolean
    soloed: boolean
    armed: boolean
    /** Silenced by another track's solo. */
    implied: boolean
    /** The launch grid holds the track. */
    detached: boolean
    drag: DragHooks
    dragging?: boolean
    dropTarget?: boolean
    /** The buttons at the end of the row (back to the arrangement, the automation fold): the view's. */
    children?: Snippet
  } = $props()

  const stores = useStores()
  const { ui, dispatcher, meters, automated } = stores
  const t = ui.t

  const track = $derived(entry.track)
  const name = $derived(trackLabel(entry, t))
  const color = $derived(trackColor(track.color, entry.index))
  const instrument = $derived(instrumentName(track))
  const kind = $derived(`${track.kind === 'midi' ? t('tracks.kind.midi') : t('tracks.kind.audio')}${instrument ? ` · ${instrument}` : ''}`)
  const muted = $derived(track.mixer.mute)
  const gain = $derived(automated.shownGain(entry.id, track.mixer.gain_db))
  const volumeText = $derived(`${formatGain(gain)} dB`)
  // Two lines (name, then kind) need the comfort height; a lower row keeps the name and gives the kind to the name of the button.
  const roomy = $derived(height >= 40)

  let head = $state<HTMLButtonElement | null>(null)

  // --- the volume, typed on the row ----------------------------------------------------------------
  /** The open field and its first text: the typed key, or the value shown. The row is one track's, so is the field. */
  let typing = $state<{ text: string; typed: boolean } | null>(null)
  let note = $state<{ text: string; left: number; top: number } | null>(null)
  let noteTimer: ReturnType<typeof setTimeout> | undefined

  function onHeadKey(event: KeyboardEvent) {
    if (renaming || typing) return
    const action = volumeKey(event, selected)
    if (!action) return
    // The key opens the field and is not typed a second time into it, nor does Enter press the button.
    event.preventDefault()
    discard = false
    typing = action.open === 'typed' ? { text: action.text, typed: true } : { text: volumeText, typed: false }
  }

  // A field opened on the selected row closes when the selection moves away: the row's keys are the selected track's.
  // What is typed there is dropped, not applied by the blur that closing the field fires.
  let discard = false
  $effect(() => {
    if (!selected && untrack(() => typing)) {
      discard = true
      typing = null
    }
  })

  function commitVolume(text: string): boolean {
    if (discard) return true
    const parsed = parseGainEntry(text)
    if (!parsed) return false
    // The same gain is no command, and no empty history entry.
    if ((track.mixer.gain_db ?? null) !== parsed.gain) dispatcher.run(setVolume(entry.id, parsed.gain))
    if (parsed.limited) say(t('controls.limited', { value: `${MAX_GAIN_DB} dB` }))
    return true
  }

  function say(text: string) {
    // Beside the row, on the timeline's side: under it, the note would cover the next row's name (the master's, on the last track).
    const row = head?.closest('[data-testid="track-header"]')?.getBoundingClientRect()
    if (!row) return
    note = { text, left: Math.min(row.right + 8, window.innerWidth - 160), top: Math.max(4, Math.min(row.top + row.height / 2 - 12, window.innerHeight - 32)) }
    clearTimeout(noteTimer)
    noteTimer = setTimeout(() => (note = null), 2000)
  }
  $effect(() => () => clearTimeout(noteTimer))

  // --- rename, by a double click on the name (the inspector's F2 and menu do it from the keyboard) -----------
  let renaming = $state(false)
  let draft = $state('')
  let field = $state<HTMLInputElement | null>(null)

  async function startRename() {
    // A volume field still open gives way, unapplied: one field at a time on a row.
    if (typing) discard = true
    typing = null
    draft = name
    renaming = true
    await tick()
    field?.focus()
    field?.select()
  }

  function finishRename(commit: boolean) {
    if (!renaming) return
    renaming = false
    const next = draft.trim()
    const stored = track.name.trim()
    const unchanged = next === stored || (stored === '' && next === name)
    if (commit && !unchanged) dispatcher.run(renameTrack(entry.id, next))
  }

  function onRenameKey(event: KeyboardEvent) {
    event.stopPropagation()
    if (event.key !== 'Enter' && event.key !== 'Escape') return
    event.preventDefault()
    finishRename(event.key === 'Enter')
    void tick().then(() => head?.focus())
  }

  function select() {
    if (!selected) dispatcher.run(selectTrack(entry.id))
  }

  /** The right click on the row (#524): the track's actions. The rename field keeps the browser's menu. */
  function onContextMenu(event: MouseEvent) {
    if (renaming || event.defaultPrevented) return
    ui.openMenu(
      event,
      trackMenuItems(stores, entry, { x: event.clientX, y: event.clientY }, () => void startRename()),
      t('tracks.actions', { name }),
    )
  }

  // --- the height handle: a drag, the arrows, or a double click for the default -----------------------------------
  let resize: { id: number; y: number; from: number } | null = null

  function onHandleDown(event: PointerEvent) {
    if (event.button !== 0) return
    event.preventDefault()
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
    resize = { id: event.pointerId, y: event.clientY, from: height }
  }
  // While the pointer moves the row follows locally (header and timeline); the release sends one command (#490).
  function onHandleMove(event: PointerEvent) {
    if (!resize || event.pointerId !== resize.id) return
    ui.laneHeightPreview = { track: entry.id, height: clampClipRow(resize.from + event.clientY - resize.y) }
  }
  function onHandleUp(event: PointerEvent) {
    if (!resize || event.pointerId !== resize.id) return
    const from = resize.from
    resize = null
    const preview = ui.laneHeightPreview
    ui.laneHeightPreview = null
    // A click, or a drag back to where it started, changes nothing: no explicit height is written for a jitter.
    if (event.type === 'pointercancel' || preview?.track !== entry.id || preview.height === from) return
    // A dragged height is the producer's own, even when it equals the density's default: only Enter or a double click give the default back.
    setHeight(preview.height)
  }
  /** The height is the track's, in the project: the same height is no command. */
  function setHeight(next: number | null) {
    if ((track.height ?? null) !== next) dispatcher.run(setTrackHeight(entry.id, next))
  }
  function onHandleKey(event: KeyboardEvent) {
    const next = heightKey(event, height)
    if (next === null) return
    event.preventDefault()
    setHeight(next === 'default' ? null : next)
  }
  // A row that leaves (the track deleted by an agent, a view switch) mid-drag takes its preview with it.
  $effect(() => {
    const id = entry.id
    return () => {
      if (ui.laneHeightPreview?.track === id) ui.laneHeightPreview = null
    }
  })

  // --- reordering: the colour bar is the grip; a drop on another row puts the track at that row's place ---------
  function onGripDragStart(event: DragEvent) {
    if (!event.dataTransfer) return
    event.dataTransfer.effectAllowed = 'move'
    // Firefox starts no drag without data.
    event.dataTransfer.setData('text/plain', entry.id)
    // A reorder starts: a browser drag that never ended (its source left the page) is over.
    ui.dragged = null
    drag.start(entry.id)
  }
  // A device or a preset dragged out of the browser (#190) is not a reorder: `dropTarget` below takes it.
  function onDragOver(event: DragEvent) {
    if (ui.dragged) return
    event.preventDefault()
    drag.over(entry.id)
  }
  function onDrop(event: DragEvent) {
    if (ui.dragged) return
    event.preventDefault()
    drag.drop(entry.id)
  }
</script>

<div
  class="row"
  class:silent={muted || implied}
  class:selected
  class:off={detached}
  class:low={!roomy}
  class:dragging
  class:drop-target={dropTarget}
  style:height="{height}px"
  style:--swatch={color}
  role="group"
  aria-label={name}
  aria-current={selected ? 'true' : undefined}
  title={detached ? t('arrangement.offTimeline') : undefined}
  data-testid="track-header"
  data-track={entry.id}
  data-detached={detached ? 'true' : undefined}
  data-height={height}
  ondragover={onDragOver}
  ondrop={onDrop}
  oncontextmenu={onContextMenu}
  use:browserDrop={{ ui, accepts: () => true, drop: (item) => putItem(stores, item, { type: 'track', track: entry.id }) }}
>
  <span
    class="grip"
    role="presentation"
    draggable="true"
    title={t('tracks.drag', { name })}
    data-testid="track-grip"
    ondragstart={onGripDragStart}
    ondragend={() => drag.end()}
  ></span>
  {#if renaming}
    <input
      class="rename"
      type="text"
      maxlength="200"
      bind:this={field}
      bind:value={draft}
      aria-label={t('tracks.rename')}
      onkeydown={onRenameKey}
      onblur={() => finishRename(true)}
    />
  {:else}
    <button
      class="head"
      class:on={selected}
      type="button"
      bind:this={head}
      aria-pressed={selected}
      aria-label={name}
      aria-describedby="fk-lane-info-{entry.id}"
      title={detached ? `${t('arrangement.offTimeline')}. ${t('arrangement.laneHint')}` : t('arrangement.laneHint')}
      data-testid="arr-track"
      onclick={select}
      ondblclick={startRename}
      onkeydown={onHeadKey}
    >
      <span class="line">
        <span class="track-name" data-testid="track-name">{name}</span>
        <span class="volume" aria-hidden="true" data-testid="lane-volume">{volumeText}</span>
      </span>
      {#if roomy}<span class="kind" aria-hidden="true">{kind}</span>{/if}
    </button>
    <span id="fk-lane-info-{entry.id}" class="sr-only">{t('arrangement.laneInfo', { kind, volume: volumeText })}</span>
  {/if}
  <div class="states" role="group" aria-label={t('arrangement.states', { name })}>
    <button
      class="state mute"
      class:implied
      type="button"
      aria-pressed={muted}
      aria-label={implied && !muted ? `${t('tracks.mute', { name })}. ${t('tracks.implied')}` : t('tracks.mute', { name })}
      title={implied && !muted ? t('tracks.implied') : t('tracks.mute', { name })}
      data-testid="arr-mute"
      data-implied={implied && !muted ? 'true' : undefined}
      onclick={() => dispatcher.run(setMute(entry.id, !muted))}
      >M{#if implied && !muted}<span class="sr-only"> {t('tracks.implied')}</span>{/if}</button
    >
    <button
      class="state solo"
      type="button"
      aria-pressed={soloed}
      aria-label={t('tracks.solo', { name })}
      title={t('tracks.solo', { name })}
      data-testid="arr-solo"
      onclick={() => dispatcher.run(setSolo(entry.id, !soloed))}>S</button
    >
    {#if track.kind === 'midi'}
      <button
        class="state arm"
        type="button"
        aria-pressed={armed}
        aria-label={t('tracks.arm', { name })}
        title={t('tracks.armHint', { name })}
        data-testid="arr-arm"
        onclick={() => dispatcher.run(armTrack(entry.id, !armed))}
        ><svg width="7" height="7" viewBox="0 0 8 8" aria-hidden="true"><circle cx="4" cy="4" r="3.5" fill="var(--fk-rec)" /></svg>Rec</button
      >
    {/if}
  </div>
  <div class="lane-meter">
    <LevelMeter
      snap={meters.snap(entry.id)}
      orientation="horizontal"
      label={t('mixer.level', { name })}
      clipLabel={t('meter.clip')}
      resetLabel={t('meter.reset', { name })}
      onreset={() => meters.reset(entry.id)}
      testid="arr-meter"
    />
  </div>
  {@render children?.()}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <!-- A focusable separator is a widget (it has a value): the bottom panel's handle is the same. The bottom edge sets the row's height: a drag, the arrows (Shift for a big step), Home and End, Enter or a double click for the default. -->
  <div
    class="resize"
    role="separator"
    aria-orientation="horizontal"
    aria-label={t('arrangement.laneHeight', { name })}
    aria-valuemin={CLIP_ROW_MIN}
    aria-valuemax={CLIP_ROW_MAX}
    aria-valuenow={height}
    title={t('arrangement.laneHeightHint')}
    tabindex="0"
    data-testid="lane-resize"
    onpointerdown={onHandleDown}
    onpointermove={onHandleMove}
    onpointerup={onHandleUp}
    onpointercancel={onHandleUp}
    onkeydown={onHandleKey}
    ondblclick={() => setHeight(null)}
  ></div>
</div>

{#if typing && head}
  <ValueField
    anchor={head}
    initial={typing?.text ?? ''}
    typed={typing?.typed ?? false}
    label={t('arrangement.laneVolume', { name })}
    commit={commitVolume}
    onclose={(refocus) => {
      typing = null
      if (refocus) head?.focus()
    }}
  />
{/if}
{#if note}
  <p class="note" role="status" data-testid="value-note" style:left="{note.left}px" style:top="{note.top}px">{note.text}</p>
{/if}

<style>
  .row {
    position: relative;
    display: flex;
    align-items: stretch;
    border-bottom: 1px solid var(--fk-line);
    box-sizing: border-box;
    font-size: 12px;
  }
  .row.dragging {
    opacity: 0.45;
  }
  .row.drop-target {
    box-shadow: inset 0 2px 0 var(--fk-accent-fg);
  }
  /* A device or a preset of the browser over the row (#190). */
  .row:global([data-drop-over]) {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: -2px;
  }
  /* A track the grid holds: the name is dimmed and a return button sits beside it (never the dimming alone). */
  .row.off .head {
    opacity: 0.6;
  }
  /* A muted track, or one a solo silences: the name and the meter at 40 %; its M says which (letters and a dashed frame, never the dimming alone). */
  .row.silent .head,
  .row.silent .lane-meter {
    opacity: 0.4;
  }
  /* The selected track: a 2px light outline inside its row and a tint. */
  .row.selected {
    outline: 2px solid var(--fk-text);
    outline-offset: -2px;
  }
  /* The colour bar (3px, DESIGN.md) inside a wider grip, so that it can be caught. */
  .grip {
    flex: none;
    position: relative;
    width: 12px;
    cursor: grab;
  }
  .grip::before {
    content: '';
    position: absolute;
    left: 4px;
    top: 4px;
    bottom: 4px;
    width: 3px;
    border-radius: 2px;
    background: var(--swatch);
  }
  .head,
  .rename {
    flex: 1;
    min-width: 0;
    margin: 0;
    font: inherit;
    color: var(--fk-text);
  }
  .head {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 1px;
    height: 100%;
    padding: 0 var(--fk-space-1) 0 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .head.on {
    --fk-text-muted: var(--fk-text-muted-on-tint);
    background: var(--fk-accent-tint);
  }
  .rename {
    align-self: center;
    height: 24px;
    padding: 0 var(--fk-space-1);
    border: 1px solid var(--fk-accent-fg);
    border-radius: var(--fk-radius-work);
    background: var(--fk-bg);
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: var(--fk-space-1);
    min-width: 0;
  }
  /* The name the user gave: normal case, the text colour. */
  .track-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .kind {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fk-text-muted);
    font-size: 11px;
  }
  /* The volume: shown on hover and on focus of the row, in mono. */
  .volume {
    display: none;
    flex: none;
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .row:hover .volume,
  .row:focus-within .volume {
    display: inline;
  }
  .states {
    display: flex;
    flex: none;
    align-items: center;
    gap: 2px;
    padding: 0 var(--fk-space-1);
  }
  .state {
    width: 20px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: transparent;
    color: var(--fk-text);
    font-family: var(--fk-font-mono);
    font-size: 11px;
    font-weight: 700;
    cursor: pointer;
  }
  .state:hover {
    background: var(--fk-surface-2);
  }
  .state.mute[aria-pressed='true'] {
    background: var(--fk-mute);
    border-color: var(--fk-mute);
    color: #131211;
    text-decoration: line-through;
  }
  .state.mute.implied:not([aria-pressed='true']) {
    border-style: dashed;
    border-color: var(--fk-mute);
    color: var(--fk-mute);
  }
  .state.solo[aria-pressed='true'] {
    background: var(--fk-solo);
    border-color: var(--fk-solo);
    color: #131211;
    text-decoration: underline;
  }
  /* Armed: the neutral inverted fill of the other strips, with the red dot of recording (#470). */
  .state.arm {
    width: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
  }
  .state.arm[aria-pressed='true'] {
    background: var(--fk-text);
    border-color: var(--fk-text);
    color: var(--fk-bg);
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
  /* The height handle: the bottom 5px of the row; its line shows on hover and on focus. */
  .resize {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -3px;
    height: 6px;
    z-index: 2;
    cursor: ns-resize;
    touch-action: none;
  }
  .resize:hover,
  .resize:focus-visible {
    background: linear-gradient(transparent 2px, var(--fk-text-muted) 2px, var(--fk-text-muted) 4px, transparent 4px);
  }
  .resize:focus-visible {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: 0;
  }
  .note {
    position: fixed;
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
</style>
