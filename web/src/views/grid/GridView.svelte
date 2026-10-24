<script lang="ts">
  /**
   * The launch grid (GR-3): tracks in columns, scenes in rows, a clip in a cell. A click on a clip
   * launches it, the button of a row launches the scene, the button of a column stops its track,
   * "Stop all" stops everything; each cell shows its state from the engine's report with a glyph
   * as well as a colour. A double-click on an empty cell of a MIDI track creates a clip there and
   * opens it in the piano roll. Every action is a command through the dispatcher; what the grid
   * keeps itself (the cell the keyboard is on, the scene being renamed, the open menu) is view
   * state. The cells are one button each and share the handlers of the board, so a 32 by 64 grid
   * stays light; they redraw only when a state changes, not at each report of the engine.
   */
  import { tick, untrack } from 'svelte'
  import type { ClipState } from '../../lib/engine/feedback'
  import type { Command } from '../../lib/generated/protocol'
  import { deleteClip, duplicateClip, moveClip, renameClip, setClipLoop } from '../../lib/clip-commands'
  import { trackLabel } from '../../lib/project/tracks'
  import { createClipInCell, createScene, deleteScene, launchClip, launchScene, moveScene, renameScene, stopAll, stopTrack } from '../../lib/scene-commands'
  import { useStores } from '../../lib/stores'
  import { beginGroup, endGroup, selectTrack } from '../../lib/track-commands'
  import { activeCells, activeSignature, cellClips, cellKey, clipCounts, clipHomes, clipNumbers, sortedScenes, visibleRange } from './cells'
  import Glyph from './Glyph.svelte'

  type CellClipState = 'idle' | 'restarting' | ClipState

  const { project, engine, ui, dispatcher } = useStores()
  const t = ui.t
  const run = (command: Command) => dispatcher.run(command)

  const NO_SLOTS = { epoch: 0, tracks: {}, scenes: {} }

  /** The sizes the layout is made of (the same numbers as the styles below): the window of drawn cells follows from them. */
  const ROW_H = 40
  const COL_W = 112
  const SCENE_W = 188
  const OVERSCAN_ROWS = 3
  const OVERSCAN_COLS = 2

  const tracks = $derived(project.tracks)
  const scenes = $derived(project.view ? sortedScenes(project.view.project) : [])

  // The grid draws the cells the page shows and a few more around them, not the 2 048 of a 32 by 64 project.
  // Until the scroller is measured, a screen of a typical size.
  let scroller: HTMLElement | undefined = $state()
  let scrollTop = $state(0)
  let scrollLeft = $state(0)
  let viewW = $state(1280)
  let viewH = $state(720)
  const rowFirst = $derived(visibleRange(scrollTop, viewH, ROW_H, scenes.length, OVERSCAN_ROWS, ROW_H).first)
  const rowLast = $derived(visibleRange(scrollTop, viewH, ROW_H, scenes.length, OVERSCAN_ROWS, ROW_H).last)
  const colFirst = $derived(visibleRange(scrollLeft, viewW, COL_W, tracks.length, OVERSCAN_COLS, SCENE_W).first)
  const colLast = $derived(visibleRange(scrollLeft, viewW, COL_W, tracks.length, OVERSCAN_COLS, SCENE_W).last)
  const shownScenes = $derived(scenes.slice(rowFirst, rowLast))
  const shownTracks = $derived(tracks.slice(colFirst, colLast))
  const onScroll = () => {
    scrollTop = scroller!.scrollTop
    scrollLeft = scroller!.scrollLeft
  }
  const clips = $derived(project.view ? cellClips(project.view.project) : new Map())
  const counts = $derived(clipCounts(clips))
  const numbers = $derived(project.view ? clipNumbers(project.view.project) : new Map<string, number>())
  /** Where each clip of the grid sits, by clip. */
  const homes = $derived(clipHomes(clips))
  /** The name of a clip: its own, or "Clip n" as the piano roll calls it; an audio clip has no number. */
  const nameOf = (id: string, clip: { name: string }) => {
    const own = clip.name.trim()
    if (own) return own
    const n = numbers.get(id)
    return n === undefined ? t('grid.clipPlain') : t('grid.clipDefault', { n })
  }
  // The report comes about 30 times a second: the cells depend on this signature, which changes only with a state.
  const signature = $derived(engine.started ? activeSignature(engine.feedback) : '')
  const active = $derived.by(() => {
    void signature
    return activeCells(untrack(() => (engine.started ? engine.feedback : null)), project.view?.slots ?? NO_SLOTS)
  })

  const sceneLabel = (id: string, index: number) => (project.view?.project.scenes[id]?.name.trim() || t('grid.sceneDefault', { n: index + 1 }))
  const stateText = (state: CellClipState) => t(`grid.state.${state}`)

  // --- the cell the keyboard is on --------------------------------------------------------------

  let focusTrack = $state<string | null>(null)
  let focusScene = $state<string | null>(null)
  /** The one cell in the tab order: the one last focused when it still exists and is drawn, else the nearest drawn one. */
  const tabCell = $derived.by(() => {
    const at = (index: number, first: number, last: number) => Math.min(last - 1, Math.max(first, index))
    const ti = at(Math.max(0, tracks.findIndex((entry) => entry.id === focusTrack)), colFirst, colLast)
    const si = at(Math.max(0, scenes.findIndex((entry) => entry.id === focusScene)), rowFirst, rowLast)
    return tracks[ti] && scenes[si] ? cellKey(tracks[ti].id, scenes[si].id) : ''
  })

  function onFocusIn(event: FocusEvent) {
    const cell = (event.target as Element).closest<HTMLElement>('[data-cell]')
    if (!cell) return
    focusTrack = cell.dataset.track ?? null
    focusScene = cell.dataset.scene ?? null
  }

  /**
   * Arrows move between cells. On a clip, Delete and Backspace remove it and F2 renames it; Ctrl (Cmd) + C
   * remembers it and Ctrl + V copies it into the empty cell the keyboard is on. They are not application
   * shortcuts in a field, so a rename keeps them.
   */
  function onBoardKey(event: KeyboardEvent) {
    const cell = (event.target as Element).closest<HTMLElement>('[data-cell]')
    if (!cell) return
    if (event.ctrlKey || event.metaKey) {
      if (event.altKey || event.shiftKey || event.repeat) return
      const key = event.key.toLowerCase()
      if (key === 'c' && cell.dataset.clip && cell.dataset.launchable === 'true') {
        event.preventDefault()
        copied = cell.dataset.clip
      } else if (key === 'v' && !cell.dataset.clip && cell.dataset.midi === 'true') {
        event.preventDefault()
        paste(cell)
      }
      return
    }
    if (event.altKey) return
    if (cell.dataset.clip && cell.dataset.launchable === 'true') {
      if (event.key === 'F2') {
        event.preventDefault()
        void startClipRename(cell.dataset.clip)
        return
      }
      if (event.key === 'Delete' || event.key === 'Backspace') {
        event.preventDefault()
        if (!event.repeat) removeClip(cell.dataset.clip, Number(cell.dataset.ti), Number(cell.dataset.si))
        return
      }
    }
    const dt = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0
    const ds = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0
    if (!dt && !ds) return
    event.preventDefault()
    void focusCell(Number(cell.dataset.ti) + dt, Number(cell.dataset.si) + ds)
  }

  /** Brings a cell into the window and into view, then focuses it: the cell may not be drawn yet. */
  async function focusCell(ti: number, si: number) {
    if (!scroller || ti < 0 || si < 0 || ti >= tracks.length || si >= scenes.length) return
    // The column heads and the row heads stick, and cover the first row and the first column of the view.
    if (si * ROW_H < scroller.scrollTop) scroller.scrollTop = si * ROW_H
    else if ((si + 2) * ROW_H > scroller.scrollTop + viewH) scroller.scrollTop = (si + 2) * ROW_H - viewH
    if (ti * COL_W < scroller.scrollLeft) scroller.scrollLeft = ti * COL_W
    else if (SCENE_W + (ti + 1) * COL_W > scroller.scrollLeft + viewW) scroller.scrollLeft = SCENE_W + (ti + 1) * COL_W - viewW
    onScroll()
    await tick()
    board?.querySelector<HTMLElement>(`[data-cell][data-ti="${ti}"][data-si="${si}"]`)?.focus()
  }

  // --- the cells ---------------------------------------------------------------------------------

  let board: HTMLElement | undefined = $state()

  /** Selects a track (the keyboard plays it) when it is not the selected one. */
  function select(track: string) {
    if (project.selected?.id !== track) run(selectTrack(track))
  }

  /** Points the piano roll at a clip of a track; `show` also brings the roll forward. */
  function open(track: string, clip: string, show: boolean) {
    select(track)
    ui.editedClip = clip
    if (show) ui.showPanel('piano-roll')
  }

  /**
   * Creates a clip in an empty cell of a MIDI track and opens it in the roll. The clip loops over its
   * whole length, as one history entry with its creation: a launched clip that played once and went
   * quiet would read as a broken launch (the roll's Loop button turns it off).
   */
  function create(track: string, scene: string) {
    run(beginGroup())
    run(createClipInCell(track, scene))
    const made = cellClips(project.view!.project).get(cellKey(track, scene))
    const content = made?.clip.content
    if (made && content && 'midi' in content) run(setClipLoop(made.id, { start: 0, end: content.midi.length }))
    run(endGroup())
    if (made) open(track, made.id, true)
  }

  /** A mouse click lets go of the focus, so the space bar keeps toggling the transport; a key press (`detail` 0) keeps it. */
  const release = (event: MouseEvent, element: HTMLElement) => {
    if (event.detail > 0) element.blur()
  }

  function onBoardClick(event: MouseEvent) {
    // The click that ends a drag is not a launch.
    if (suppressClick) {
      suppressClick = false
      return
    }
    const target = event.target as Element
    const edit = target.closest<HTMLElement>('[data-edit]')
    if (edit) return open(edit.dataset.track!, edit.dataset.clip!, true)
    const cell = target.closest<HTMLElement>('[data-cell]')
    if (!cell) return
    const { track, scene, clip } = cell.dataset
    if (clip) {
      if (cell.dataset.launchable !== 'true') return select(track!)
      // The clicks of a double-click (`detail` 2 and more) launch nothing more: the double-click opens the clip.
      if (event.detail >= 2) return
      run(launchClip(clip))
      open(track!, clip, false)
      return release(event, cell)
    }
    // An empty cell: a click selects its track, a key press (`detail` 0) creates the clip.
    if (event.detail === 0 && cell.dataset.midi === 'true') return create(track!, scene!)
    select(track!)
  }

  function onBoardDoubleClick(event: MouseEvent) {
    const cell = (event.target as Element).closest<HTMLElement>('[data-cell]')
    if (!cell) return
    if (cell.dataset.clip) {
      if (cell.dataset.launchable === 'true') open(cell.dataset.track!, cell.dataset.clip, true)
    } else if (cell.dataset.midi === 'true') create(cell.dataset.track!, cell.dataset.scene!)
  }

  // --- clips: delete, duplicate, move, rename ------------------------------------------------------

  /** The clip the keyboard copies (Ctrl + C): view state, not the project's. */
  let copied = $state<string | null>(null)
  /** The clip whose menu is open. */
  let cellMenu = $state<string | null>(null)
  let renamingClip = $state<string | null>(null)
  let clipDraft = $state('')
  let clipField = $state<HTMLInputElement | null>(null)

  /** Gives the keyboard back to a cell once the clip that held it is gone or moved. */
  const refocus = (ti: number, si: number) => void tick().then(() => focusCell(ti, si))

  function removeClip(id: string, ti: number, si: number) {
    cellMenu = null
    if (!homes.has(id)) return
    run(deleteClip(id))
    refocus(ti, si)
  }

  /** The first cell below `si` in a track where no clip sits, or null. */
  function freeBelow(track: string, si: number): string | null {
    for (let i = si + 1; i < scenes.length; i++) if (!clips.has(cellKey(track, scenes[i]!.id))) return scenes[i]!.id
    return null
  }

  /** Copies a clip into a cell of a MIDI track: the same track, or another one (the command names it then). */
  function copyTo(clip: string, track: string, scene: string) {
    const home = homes.get(clip)
    if (!home || clips.has(cellKey(track, scene))) return
    run(duplicateClip(clip, scene, track === home.track ? null : track))
  }

  function paste(cell: HTMLElement) {
    if (copied) copyTo(copied, cell.dataset.track!, cell.dataset.scene!)
    refocus(Number(cell.dataset.ti), Number(cell.dataset.si))
  }

  async function openCellMenu(id: string) {
    menuFor = null
    confirmFor = null
    cellMenu = cellMenu === id ? null : id
    if (cellMenu) {
      await tick()
      board?.querySelector<HTMLElement>(`[data-slot-clip="${id}"] [role="menuitem"]:not(:disabled)`)?.focus()
    }
  }

  function closeCellMenu(restore: boolean) {
    const id = cellMenu
    cellMenu = null
    if (restore && id) void tick().then(() => board?.querySelector<HTMLElement>(`[data-cell][data-clip="${id}"]`)?.focus())
  }

  function onSlotKey(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !cellMenu) return
    event.stopPropagation()
    closeCellMenu(true)
  }

  function onSlotFocusOut(event: FocusEvent) {
    const next = event.relatedTarget as Node | null
    if (next && !(event.currentTarget as HTMLElement).contains(next)) cellMenu = null
  }

  function duplicateFromMenu(id: string) {
    const home = homes.get(id)
    cellMenu = null
    if (!home) return
    const scene = freeBelow(home.track, scenes.findIndex((entry) => entry.id === home.scene))
    if (scene) run(duplicateClip(id, scene, null))
    void tick().then(() => board?.querySelector<HTMLElement>(`[data-cell][data-clip="${id}"]`)?.focus())
  }

  async function startClipRename(id: string) {
    const home = homes.get(id)
    if (!home?.cell.launchable) return
    cellMenu = null
    clipDraft = nameOf(id, home.cell.clip)
    renamingClip = id
    await tick()
    clipField?.focus()
    clipField?.select()
  }

  function finishClipRename(commit: boolean, id: string) {
    if (renamingClip !== id) return
    renamingClip = null
    const home = homes.get(id)
    if (!home) return
    const name = clipDraft.trim()
    const stored = home.cell.clip.name.trim()
    // An unchanged default name stays unnamed; anything else is the user's.
    const unchanged = name === stored || (stored === '' && name === nameOf(id, home.cell.clip))
    if (commit && !unchanged) run(renameClip(id, name))
  }

  function onClipRenameKey(event: KeyboardEvent, id: string) {
    if (event.key !== 'Enter' && event.key !== 'Escape') return
    event.preventDefault()
    finishClipRename(event.key === 'Enter', id)
    void tick().then(() => board?.querySelector<HTMLElement>(`[data-cell][data-clip="${id}"]`)?.focus())
  }

  // --- dragging a clip -----------------------------------------------------------------------------

  /** Pixels the pointer must travel before a press on a clip is a drag and not a click. */
  const DRAG_SLOP = 5
  /** The press on a clip that may become a drag. */
  let press: { clip: string; track: string; x: number; y: number; active: boolean; aborted: boolean } | null = null
  /** The drag, once it started: what the ghost shows, where it is, the cell it would drop on. */
  let dragging = $state<{ clip: string; name: string } | null>(null)
  let dragAt = $state({ x: 0, y: 0 })
  let dragCopy = $state(false)
  /** The cell the clip would land on (`track|scene`), only ever an empty cell of a MIDI track. */
  let dropKey = $state<string | null>(null)
  let suppressClick = false

  function onBoardPointerDown(event: PointerEvent) {
    if (event.button !== 0 || press) return
    const cell = (event.target as Element).closest<HTMLElement>('[data-cell]')
    if (!cell?.dataset.clip || cell.dataset.launchable !== 'true') return
    press = { clip: cell.dataset.clip, track: cell.dataset.track!, x: event.clientX, y: event.clientY, active: false, aborted: false }
  }

  /** The empty cell of a MIDI track under the pointer, or null: nothing else is a place to drop. */
  function dropCell(x: number, y: number, copy: boolean): HTMLElement | null {
    const cell = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-cell]')
    if (!cell || cell.dataset.clip || cell.dataset.midi !== 'true') return null
    // A clip the arrangement plays stays in its track: it can be copied to another one, not moved there.
    if (!copy && press && cell.dataset.track !== press.track && isPlaced(press.track, press.clip)) return null
    return cell
  }

  /** Whether a placement of the arrangement plays the clip. */
  function isPlaced(track: string, clip: string): boolean {
    const placements = project.view?.project.tracks[track]?.placements ?? {}
    return Object.values(placements).some((placement) => placement?.clip === clip)
  }

  /** The cell the clip would land on, from the pointer's last place and the key held. */
  function refreshDrop() {
    const over = dropCell(dragAt.x, dragAt.y, dragCopy)
    dropKey = over ? cellKey(over.dataset.track!, over.dataset.scene!) : null
  }

  function onWindowPointerMove(event: PointerEvent) {
    if (!press || press.aborted) return
    if (!press.active) {
      if (Math.hypot(event.clientX - press.x, event.clientY - press.y) < DRAG_SLOP) return
      const home = homes.get(press.clip)
      if (!home) return endPress()
      press.active = true
      dragging = { clip: press.clip, name: nameOf(press.clip, home.cell.clip) }
    }
    dragAt = { x: event.clientX, y: event.clientY }
    dragCopy = event.altKey
    refreshDrop()
  }

  function endPress() {
    const wasDrag = press?.active ?? false
    press = null
    dragging = null
    dropKey = null
    if (wasDrag) {
      // The click that follows the release is not a launch.
      suppressClick = true
      setTimeout(() => (suppressClick = false), 60)
    }
  }

  /** A drop on an empty cell of a MIDI track moves the clip, or copies it with Alt: one command either way. */
  function onWindowPointerUp(event: PointerEvent) {
    const current = press
    if (!current?.active) return endPress()
    const over = current.aborted ? null : dropCell(event.clientX, event.clientY, event.altKey)
    endPress()
    if (!over || !homes.has(current.clip)) return
    const { track, scene } = over.dataset
    if (event.altKey) return copyTo(current.clip, track!, scene!)
    run(moveClip(current.clip, scene!, track === current.track ? null : track!))
  }

  function onWindowKey(event: KeyboardEvent) {
    // Alt pressed or released with the pointer still: the indicator follows the key the drop will read.
    if (press?.active && !press.aborted) {
      dragCopy = event.altKey
      refreshDrop()
    }
    if (event.key === 'Escape' && press?.active) {
      press.aborted = true
      dropKey = null
      dragging = null
    }
  }

  // --- scenes ------------------------------------------------------------------------------------

  let menuFor = $state<string | null>(null)
  let confirmFor = $state<string | null>(null)
  /** The scene whose menu or confirmation is open: it rises above the rows under it. */
  const popupFor = $derived(menuFor ?? confirmFor)
  let renaming = $state<string | null>(null)
  let draft = $state('')
  let field = $state<HTMLInputElement | null>(null)

  async function addScene() {
    const before = new Set(Object.keys(project.view?.project.scenes ?? {}))
    run(createScene())
    const made = Object.keys(project.view?.project.scenes ?? {}).find((id) => !before.has(id))
    if (!made) return
    await tick()
    board?.querySelector(`[data-scene-head="${made}"]`)?.scrollIntoView({ block: 'nearest' })
  }

  async function startRename(id: string, index: number) {
    menuFor = null
    draft = sceneLabel(id, index)
    renaming = id
    await tick()
    field?.focus()
    field?.select()
  }

  function finishRename(commit: boolean, id: string, index: number) {
    if (renaming !== id) return
    renaming = null
    const name = draft.trim()
    const stored = project.view?.project.scenes[id]?.name.trim() ?? ''
    // An unchanged default name stays unnamed; anything else is the user's.
    const unchanged = name === stored || (stored === '' && name === sceneLabel(id, index))
    if (commit && !unchanged) run(renameScene(id, name))
  }

  function onRenameKey(event: KeyboardEvent, id: string, index: number) {
    if (event.key !== 'Enter' && event.key !== 'Escape') return
    event.preventDefault()
    finishRename(event.key === 'Enter', id, index)
    void tick().then(() => board?.querySelector<HTMLElement>(`[data-scene-head="${id}"] .sname`)?.focus())
  }

  function onNameKey(event: KeyboardEvent, id: string, index: number) {
    if (event.key !== 'F2') return
    event.preventDefault()
    void startRename(id, index)
  }

  async function openMenu(id: string) {
    cellMenu = null
    confirmFor = null
    menuFor = menuFor === id ? null : id
    if (menuFor) {
      await tick()
      board?.querySelector<HTMLElement>(`[data-scene-head="${id}"] [role="menuitem"]:not(:disabled)`)?.focus()
    }
  }

  function closePopups(restore: boolean) {
    const id = menuFor ?? confirmFor
    menuFor = null
    confirmFor = null
    if (restore && id) void tick().then(() => board?.querySelector<HTMLElement>(`[data-scene-head="${id}"] .more`)?.focus())
  }

  function onHeadKey(event: KeyboardEvent) {
    if (event.key !== 'Escape' || (!menuFor && !confirmFor)) return
    event.stopPropagation()
    closePopups(true)
  }

  /**
   * Leaving the row with the keyboard (focus lands elsewhere) closes its menu or confirmation. Focus
   * that lands nowhere is not "leaving": it is the clicked menu item being removed, and a click
   * elsewhere is `onWindowPointer`'s.
   */
  function onHeadFocusOut(event: FocusEvent) {
    const next = event.relatedTarget as Node | null
    if (next && !(event.currentTarget as HTMLElement).contains(next)) closePopups(false)
  }

  function onWindowPointer(event: PointerEvent) {
    if (popupFor && !(event.target as Element).closest(`[data-scene-head="${popupFor}"]`)) closePopups(false)
    if (cellMenu && !(event.target as Element).closest(`[data-slot-clip="${cellMenu}"]`)) cellMenu = null
  }

  function requestDelete(id: string) {
    menuFor = null
    if ((counts.get(id) ?? 0) === 0) return run(deleteScene(id))
    confirmFor = id
    void tick().then(() => board?.querySelector<HTMLElement>(`[data-scene-head="${id}"] .cancel`)?.focus())
  }

  function confirmDelete(id: string) {
    confirmFor = null
    run(deleteScene(id))
  }

  function move(id: string, index: number) {
    menuFor = null
    run(moveScene(id, index))
  }
</script>

<svelte:window onpointerdown={onWindowPointer} onpointermove={onWindowPointerMove} onpointerup={onWindowPointerUp} onpointercancel={endPress} onkeydown={onWindowKey} onkeyup={onWindowKey} onblur={endPress} />

{#if !project.view}
  <p class="note" data-testid="grid-waiting">{t('tracks.waiting')}</p>
{:else if tracks.length === 0}
  <p class="note" data-testid="grid-no-tracks">{t('grid.noTracks')}</p>
{:else}
  <div class="scroll" style:--scene-w="{SCENE_W}px" style:--col-w="{COL_W}px" style:--row-h="{ROW_H}px" bind:this={scroller} bind:clientWidth={viewW} bind:clientHeight={viewH} onscroll={onScroll}>
    <!-- The cells are the focusable parts of the grid (roving tabindex); the board only listens for their events. -->
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div
      class="board"
      role="grid"
      aria-label={t('grid.label')}
      aria-rowcount={scenes.length + 1}
      aria-colcount={tracks.length + 1}
      data-testid="grid"
      style:--tracks={tracks.length}
      bind:this={board}
      class:dragging={dragging !== null}
      onclick={onBoardClick}
      ondblclick={onBoardDoubleClick}
      onpointerdown={onBoardPointerDown}
      onkeydown={onBoardKey}
      onfocusin={onFocusIn}
    >
      <div class="row" role="row">
        <div class="corner" role="columnheader">
          <button class="stopall" type="button" title={t('grid.stopAllHint')} data-testid="stop-all" onclick={() => run(stopAll())}>
            <Glyph kind="stop" size={12} />{t('grid.stopAll')}
          </button>
        </div>
        {#if colFirst > 0}<div class="gap" style:grid-column="span {colFirst}"></div>{/if}
        {#each shownTracks as entry (entry.id)}
          {@const label = trackLabel(entry, t)}
          <div class="colhead" role="columnheader" aria-colindex={entry.index + 2} style:--track-color={entry.track.color ?? undefined} aria-current={project.selected?.id === entry.id ? 'true' : undefined} data-testid="grid-track" data-track={entry.id}>
            <button class="tname" type="button" title={label} onclick={() => select(entry.id)}>{label}</button>
            <button class="stop" type="button" aria-label={t('grid.stopTrack', { name: label })} title={t('grid.stopTrack', { name: label })} data-testid="stop-track" onclick={() => run(stopTrack(entry.id))}>
              <Glyph kind="stop" size={12} />
            </button>
          </div>
        {/each}
      </div>
      {#if rowFirst > 0}<div class="skip" style:height="{rowFirst * ROW_H}px"></div>{/if}

      {#each shownScenes as sc (sc.id)}
        {@const si = sc.index}
        {@const name = sceneLabel(sc.id, si)}
        {@const held = counts.get(sc.id) ?? 0}
        <div class="row" role="row" aria-rowindex={si + 2} data-testid="scene-row" data-scene={sc.id}>
          <!-- svelte-ignore a11y_interactive_supports_focus -->
          <div class="rowhead" class:open={popupFor === sc.id} role="rowheader" data-scene-head={sc.id} data-testid="scene-head" onkeydown={onHeadKey} onfocusout={onHeadFocusOut}>
            <button class="launch" type="button" aria-label={t('grid.launchScene', { name })} title={t('grid.launchScene', { name })} data-testid="launch-scene" onclick={(event) => { run(launchScene(sc.id)); release(event, event.currentTarget) }}>
              <Glyph kind="launch" />
            </button>
            {#if renaming === sc.id}
              <input
                class="rename"
                type="text"
                maxlength="200"
                bind:this={field}
                bind:value={draft}
                aria-label={t('grid.sceneName')}
                onkeydown={(event) => onRenameKey(event, sc.id, si)}
                onblur={() => finishRename(true, sc.id, si)}
              />
            {:else}
              <button class="sname" type="button" title={t('grid.renameHint')} data-testid="scene-name" onkeydown={(event) => onNameKey(event, sc.id, si)} ondblclick={() => startRename(sc.id, si)}>{name}</button>
            {/if}
            <button class="more" type="button" aria-haspopup="menu" aria-expanded={menuFor === sc.id} aria-label={t('grid.sceneActions', { name })} title={t('grid.sceneActions', { name })} data-testid="scene-more" onclick={() => openMenu(sc.id)}>⋯</button>
            {#if menuFor === sc.id}
              <div class="pop" role="menu" data-testid="scene-menu">
                <button role="menuitem" type="button" onclick={() => startRename(sc.id, si)}>{t('grid.rename')}</button>
                <button role="menuitem" type="button" disabled={si === 0} onclick={() => move(sc.id, si - 1)}>{t('grid.moveUp')}</button>
                <button role="menuitem" type="button" disabled={si === scenes.length - 1} onclick={() => move(sc.id, si + 1)}>{t('grid.moveDown')}</button>
                <button role="menuitem" type="button" data-testid="scene-delete" onclick={() => requestDelete(sc.id)}>{t('grid.delete')}</button>
              </div>
            {:else if confirmFor === sc.id}
              <div class="pop confirm" role="group" aria-label={t('grid.deleteConfirm')} data-testid="scene-confirm">
                <p>{held === 1 ? t('grid.deleteOne', { name }) : t('grid.deleteMany', { name, n: held })}</p>
                <div class="actions">
                  <button class="cancel" type="button" onclick={() => closePopups(true)}>{t('grid.cancel')}</button>
                  <button class="danger" type="button" data-testid="scene-confirm-delete" onclick={() => confirmDelete(sc.id)}>{t('grid.delete')}</button>
                </div>
              </div>
            {/if}
          </div>
          {#if colFirst > 0}<div class="gap" style:grid-column="span {colFirst}"></div>{/if}
          {#each shownTracks as entry (entry.id)}
            {@const key = cellKey(entry.id, sc.id)}
            {@const cell = clips.get(key)}
            {@const midi = entry.track.kind === 'midi'}
            {@const label = trackLabel(entry, t)}
            {@const here = tabCell === key}
            {#if cell}
              {@const clipName = nameOf(cell.id, cell.clip)}
              {@const reported = active.get(key)}
              {@const state = reported?.restarting ? 'restarting' : (reported?.state ?? 'idle')}
              <div class="slot" class:open={cellMenu === cell.id} role="gridcell" aria-colindex={entry.index + 2} style:--track-color={entry.track.color ?? undefined} data-slot-clip={cell.id} onkeydown={onSlotKey} onfocusout={onSlotFocusOut}>
                <button
                  class="cell"
                  type="button"
                  tabindex={here ? 0 : -1}
                  data-cell
                  data-testid="cell"
                  data-state={state}
                  data-track={entry.id}
                  data-scene={sc.id}
                  data-clip={cell.id}
                  data-launchable={cell.launchable}
                  data-copied={copied === cell.id ? 'true' : undefined}
                  data-lifted={dragging?.clip === cell.id ? 'true' : undefined}
                  data-ti={entry.index}
                  data-si={si}
                  disabled={!cell.launchable}
                  aria-label={cell.launchable ? t('grid.cell', { track: label, scene: name, clip: clipName, state: stateText(state) }) : t('grid.cellAudio', { track: label, scene: name, clip: clipName })}
                  title={cell.launchable ? `${stateText(state)}. ${t('grid.clipHint')}` : undefined}
                >
                  <Glyph kind={state} />
                  <span class="cname">{clipName}</span>
                </button>
                {#if renamingClip === cell.id}
                  <input
                    class="crename"
                    type="text"
                    maxlength="200"
                    bind:this={clipField}
                    bind:value={clipDraft}
                    aria-label={t('grid.clipName')}
                    data-testid="clip-rename"
                    onkeydown={(event) => onClipRenameKey(event, cell.id)}
                    onblur={() => finishClipRename(true, cell.id)}
                  />
                {/if}
                {#if cell.launchable}
                  <button class="edit" type="button" tabindex={here ? 0 : -1} data-edit data-track={entry.id} data-clip={cell.id} data-testid="edit-clip" aria-label={t('grid.editClip', { clip: clipName })} title={t('grid.editClip', { clip: clipName })}>✎</button>
                  <button class="cmore" type="button" tabindex={here ? 0 : -1} aria-haspopup="menu" aria-expanded={cellMenu === cell.id} data-testid="clip-more" aria-label={t('grid.clipActions', { clip: clipName })} title={t('grid.clipActions', { clip: clipName })} onclick={() => openCellMenu(cell.id)}>⋯</button>
                  {#if cellMenu === cell.id}
                    <div class="pop cpop" role="menu" data-testid="clip-menu">
                      <button role="menuitem" type="button" data-testid="clip-menu-rename" onclick={() => startClipRename(cell.id)}>{t('grid.rename')}</button>
                      <button role="menuitem" type="button" data-testid="clip-menu-duplicate" disabled={freeBelow(entry.id, si) === null} onclick={() => duplicateFromMenu(cell.id)}>{t('grid.duplicate')}</button>
                      <button role="menuitem" type="button" data-testid="clip-menu-delete" onclick={() => removeClip(cell.id, entry.index, si)}>{t('grid.delete')}</button>
                    </div>
                  {/if}
                {/if}
              </div>
            {:else}
              <div class="slot" role="gridcell" aria-colindex={entry.index + 2}>
                <button
                  class="cell"
                  type="button"
                  tabindex={here ? 0 : -1}
                  data-cell
                  data-testid="cell"
                  data-state="empty"
                  data-drop={dropKey === key ? (dragCopy ? 'copy' : 'move') : undefined}
                  data-track={entry.id}
                  data-scene={sc.id}
                  data-midi={midi}
                  data-ti={entry.index}
                  data-si={si}
                  aria-label={midi ? t('grid.cellEmptyCreate', { track: label, scene: name }) : t('grid.cellEmpty', { track: label, scene: name })}
                ></button>
              </div>
            {/if}
          {/each}
        </div>
      {/each}
      {#if rowLast < scenes.length}<div class="skip" style:height="{(scenes.length - rowLast) * ROW_H}px"></div>{/if}

      <div class="row" role="row">
        <div class="addrow" role="rowheader">
          <button class="add" type="button" data-testid="add-scene" onclick={addScene}>+ {t('grid.addScene')}</button>
        </div>
      </div>
    </div>
    {#if dragging}
      <div class="ghost" style:left="{dragAt.x + 12}px" style:top="{dragAt.y + 12}px" data-testid="clip-ghost" data-mode={dragCopy ? 'copy' : 'move'}>
        {dragCopy ? t('grid.dragCopy', { clip: dragging.name }) : t('grid.dragMove', { clip: dragging.name })}
      </div>
    {/if}
    {#if scenes.length === 0}
      <p class="note" data-testid="grid-no-scenes">{t('grid.noScenes')}</p>
    {/if}
  </div>
{/if}

<style>
  .note {
    margin: 0;
    padding: var(--fk-space-4);
    color: var(--fk-text-muted);
  }
  .scroll {
    /* The rooms of the rows not drawn change as the window moves: the browser must not move the scroll to follow them. */
    overflow-anchor: none;
    height: 100%;
    min-height: 0;
    overflow: auto;
  }
  .board {
    display: grid;
    grid-template-columns: var(--scene-w) repeat(var(--tracks), var(--col-w));
    width: max-content;
    min-width: 100%;
    align-content: start;
  }
  .row {
    display: contents;
  }
  /* The rows and columns the grid does not draw keep their room. */
  .skip {
    grid-column: 1 / -1;
  }
  .gap {
    height: 0;
  }
  button {
    font: inherit;
    color: var(--fk-text);
    cursor: pointer;
  }
  button:disabled {
    cursor: default;
  }

  /* Headers: the column heads stick to the top, the row heads to the left. */
  .corner,
  .colhead,
  .rowhead,
  .addrow {
    /* A row of the grid is not full when its cells beyond the window are not drawn: the first item of a row says where it starts. */
    background: var(--fk-surface);
    border-bottom: 1px solid var(--fk-line);
    border-right: 1px solid var(--fk-line);
  }
  .corner,
  .rowhead,
  .addrow {
    grid-column: 1;
  }
  .corner {
    position: sticky;
    top: 0;
    left: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    padding: 0 var(--fk-space-2);
    height: var(--row-h);
  }
  .colhead {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
    min-width: 0;
    height: var(--row-h);
    padding: 0 var(--fk-space-1) 0 var(--fk-space-2);
    border-top: 3px solid var(--track-color, var(--fk-line));
    box-sizing: border-box;
  }
  .colhead[aria-current='true'] {
    background: var(--fk-surface-2);
  }
  .tname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    border: 0;
    background: none;
    font-weight: 600;
    font-size: 12px;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rowhead {
    position: sticky;
    left: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
    height: var(--row-h);
    padding: 0 var(--fk-space-1) 0 var(--fk-space-2);
    box-sizing: border-box;
  }
  .rowhead.open {
    z-index: 4;
  }
  .sname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    border: 0;
    background: none;
    font-size: 12px;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rename {
    flex: 1;
    min-width: 0;
    height: 24px;
    border: 1px solid var(--fk-accent);
    border-radius: var(--fk-radius-work);
    background: var(--fk-bg);
    color: var(--fk-text);
    font: inherit;
    font-size: 12px;
  }
  .launch,
  .stop,
  .more,
  .stopall,
  .add {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--fk-space-1);
    min-width: 26px;
    height: 26px;
    padding: 0 var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: var(--fk-surface-2);
    font-size: 12px;
  }
  .launch,
  .stop,
  .more {
    flex: none;
  }
  .launch:hover,
  .stop:hover,
  .more:hover,
  .stopall:hover,
  .add:hover {
    background: var(--fk-bg);
  }
  .addrow {
    position: sticky;
    left: 0;
    z-index: 2;
    padding: var(--fk-space-2);
    border-bottom: 0;
  }
  .add {
    width: 100%;
  }

  /* The cells. */
  .slot {
    position: relative;
    height: var(--row-h);
    border-bottom: 1px solid var(--fk-line);
    border-right: 1px solid var(--fk-line);
    box-sizing: border-box;
  }
  .cell {
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
    width: 100%;
    height: 100%;
    padding: 0 var(--fk-space-2);
    border: 0;
    background: none;
    color: var(--track-color, var(--fk-text-muted));
    text-align: left;
  }
  .cname {
    min-width: 0;
    overflow: hidden;
    color: var(--fk-text);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cell[data-state='empty'] {
    cursor: cell;
  }
  .cell[data-state='empty']:hover {
    background: var(--fk-surface);
  }
  .cell:not([data-state='empty']) {
    background: color-mix(in srgb, var(--track-color, var(--fk-text-muted)) 20%, var(--fk-bg));
  }
  .cell[data-state='idle']:hover {
    background: color-mix(in srgb, var(--track-color, var(--fk-text-muted)) 34%, var(--fk-bg));
  }
  .cell[data-state='playing'],
  .cell[data-state='restarting'] {
    background: var(--track-color, var(--fk-accent));
    color: #141414;
  }
  .cell[data-state='playing'] .cname,
  .cell[data-state='restarting'] .cname {
    color: #141414;
  }
  .cell[data-state='queued'],
  .cell[data-state='restarting'] {
    box-shadow: inset 0 0 0 2px var(--track-color, var(--fk-accent));
  }
  .cell[data-state='stopping'] {
    background: color-mix(in srgb, var(--track-color, var(--fk-text-muted)) 60%, var(--fk-bg));
    outline: 2px dashed var(--track-color, var(--fk-accent));
    outline-offset: -3px;
  }
  .cell[data-state='armed'] {
    box-shadow: inset 0 0 0 1px var(--track-color, var(--fk-accent));
  }
  .cell[data-state='queued'] :global(.glyph),
  .cell[data-state='restarting'] :global(.glyph),
  .cell[data-state='stopping'] :global(.glyph),
  .cell[data-state='armed'] :global(.glyph) {
    animation: pulse 0.9s ease-in-out infinite alternate;
  }
  @keyframes pulse {
    from {
      opacity: 1;
    }
    to {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cell :global(.glyph) {
      animation: none !important;
    }
  }
  .cell:disabled {
    opacity: 0.55;
  }
  .edit {
    position: absolute;
    top: 2px;
    right: 2px;
    display: none;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: var(--fk-bg);
    font-size: 11px;
    line-height: 1;
  }
  .cmore {
    position: absolute;
    top: 2px;
    right: 24px;
    display: none;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: var(--fk-bg);
    font-size: 11px;
    line-height: 1;
  }
  .slot:hover .edit,
  .slot:focus-within .edit,
  .slot:hover .cmore,
  .slot:focus-within .cmore,
  .slot.open .cmore {
    display: block;
  }
  .slot.open {
    z-index: 4;
  }
  .cpop {
    top: calc(100% - 2px);
    left: 4px;
  }
  .crename {
    position: absolute;
    inset: 6px 4px;
    z-index: 1;
    min-width: 0;
    border: 1px solid var(--fk-accent);
    border-radius: var(--fk-radius-work);
    background: var(--fk-bg);
    color: var(--fk-text);
    font: inherit;
    font-size: 12px;
  }
  /* Dragging a clip: the cells it can land on show it, the clip stays where it is until the drop. */
  .cell {
    user-select: none;
  }
  .board.dragging {
    cursor: grabbing;
  }
  .board.dragging .cell {
    cursor: inherit;
  }
  .cell[data-drop] {
    outline: 2px solid var(--fk-accent);
    outline-offset: -3px;
    background: var(--fk-surface-2);
  }
  .cell[data-drop='copy'] {
    outline-style: dashed;
  }
  .cell[data-lifted='true'] {
    opacity: 0.5;
  }
  .cell[data-copied='true'] {
    outline: 1px dashed var(--fk-text-muted);
    outline-offset: -3px;
  }
  .ghost {
    position: fixed;
    z-index: 20;
    max-width: 220px;
    padding: 2px var(--fk-space-2);
    overflow: hidden;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
    font-size: 12px;
    pointer-events: none;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Menu and confirmation of a scene. */
  .pop {
    position: absolute;
    z-index: 6;
    top: calc(100% - 2px);
    left: var(--fk-space-2);
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 150px;
    padding: var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
  }
  .pop button {
    height: 26px;
    padding: 0 var(--fk-space-2);
    border: 0;
    border-radius: var(--fk-radius-work);
    background: none;
    font-size: 12px;
    text-align: left;
  }
  .pop button:hover:not(:disabled) {
    background: var(--fk-surface-2);
  }
  .pop button:disabled {
    opacity: 0.45;
  }
  .confirm {
    width: 232px;
    gap: var(--fk-space-2);
    padding: var(--fk-space-3);
  }
  .confirm p {
    margin: 0;
    font-size: 12px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
  }
  .actions button {
    border: 1px solid var(--fk-line);
    text-align: center;
  }
  .actions .danger {
    border-color: var(--fk-rec);
    color: var(--fk-rec);
  }

  :where(button, input):focus-visible {
    outline: 2px solid var(--fk-accent);
    outline-offset: -2px;
  }
</style>
