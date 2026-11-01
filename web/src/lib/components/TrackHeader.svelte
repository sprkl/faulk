<script lang="ts">
  /**
   * A track's header: colour, name (renamed from the keyboard with F2, Enter to confirm), kind and
   * instrument, mute, solo, arm, volume, pan and delete. Shared by the track list, and later the grid
   * and the timeline (the mixer draws its own vertical strips from the same commands). It holds no project state: it reads the stores and every action
   * is a command (`track-commands.ts`) through the dispatcher. An armed track receives the live
   * input (MIDI controller) besides the selected one; recording itself comes with RE-2.
   */
  import { tick } from 'svelte'
  import GainControl from './GainControl.svelte'
  import PanControl from './PanControl.svelte'
  import { copyName, formatGain, formatPan, instrumentName, NO_COLOR, trackLabel, type DragHooks, type TrackEntry } from '../project/tracks'
  import { useStores } from '../stores'
  import { armTrack, deleteTrack, duplicateTrack, moveTrack, renameTrack, selectTrack, setMute, setSolo, setTrackColor, setVolume } from '../track-commands'

  let {
    entry,
    selected = false,
    soloed = false,
    armed = false,
    compact = false,
    count = 1,
    drag,
    dragging = false,
    dropTarget = false,
  }: {
    entry: TrackEntry
    selected?: boolean
    soloed?: boolean
    armed?: boolean
    compact?: boolean
    /** How many tracks the list holds: the first cannot move up, the last cannot move down. */
    count?: number
    /** Set by the track list: the header can then be dragged by its grip. */
    drag?: DragHooks
    dragging?: boolean
    dropTarget?: boolean
  } = $props()

  const { project, ui, dispatcher, automated } = useStores()
  const t = ui.t

  const track = $derived(entry.track)
  const label = $derived(trackLabel(entry, t))
  const instrument = $derived(instrumentName(track))
  const mixer = $derived(track.mixer)
  // A lane that drives the volume or the pan moves the control with the play position, and marks it (AM-4, #388).
  const shownGain = $derived(automated.shownGain(entry.id, mixer.gain_db))
  const shownPan = $derived(automated.shownPan(entry.id, mixer.pan))

  // The controls whose readouts open the field to type a value.
  let gainControl: ReturnType<typeof GainControl> | undefined = $state()
  let panControl: ReturnType<typeof PanControl> | undefined = $state()

  let renaming = $state(false)
  let draft = $state('')
  let field = $state<HTMLInputElement | null>(null)
  let nameButton = $state<HTMLButtonElement | null>(null)

  async function startRename() {
    draft = label
    renaming = true
    await tick()
    field?.focus()
    field?.select()
  }

  function finishRename(commit: boolean) {
    if (!renaming) return
    renaming = false
    const name = draft.trim()
    // An unchanged default name stays unnamed; anything else is the user's.
    const stored = track.name.trim()
    const unchanged = name === stored || (stored === '' && name === label)
    if (commit && !unchanged) dispatcher.run(renameTrack(entry.id, name))
  }

  function onRenameKey(event: KeyboardEvent) {
    if (event.key !== 'Enter' && event.key !== 'Escape') return
    event.preventDefault()
    finishRename(event.key === 'Enter')
    // Back on the name, so the keyboard stays on the track.
    void tick().then(() => nameButton?.focus())
  }

  function onNameKey(event: KeyboardEvent) {
    if (event.key === 'F2') {
      event.preventDefault()
      void startRename()
    }
  }

  // --- the actions menu: rename, move, duplicate, delete (each one command, one history entry) ---------
  let menuOpen = $state(false)
  let root = $state<HTMLElement | null>(null)

  const focusMore = () => void tick().then(() => root?.querySelector<HTMLElement>('.more')?.focus())

  async function openMenu() {
    menuOpen = !menuOpen
    if (!menuOpen) return
    await tick()
    root?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')?.focus()
  }

  function closeMenu(restore: boolean) {
    if (!menuOpen) return
    menuOpen = false
    if (restore) focusMore()
  }

  function onRootKey(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !menuOpen) return
    event.stopPropagation()
    closeMenu(true)
  }

  /** Leaving the header with the keyboard closes the menu; a click elsewhere is `onWindowPointer`'s. */
  function onRootFocusOut(event: FocusEvent) {
    const next = event.relatedTarget as Node | null
    if (menuOpen && next && !(event.currentTarget as HTMLElement).contains(next)) closeMenu(false)
  }

  function onWindowPointer(event: PointerEvent) {
    if (menuOpen && !root?.contains(event.target as Node)) closeMenu(false)
  }

  function move(index: number) {
    closeMenu(false)
    dispatcher.run(moveTrack(entry.id, index))
    focusMore()
  }

  /** The copy sits right below the source under a free name, and is selected so the keyboard plays it. */
  function duplicate() {
    closeMenu(false)
    const before = new Set(project.tracks.map((e) => e.id))
    // The names on screen once the copy is in: an unnamed track shows its position, which the copy shifts.
    const after = project.tracks.map((e) => trackLabel({ track: e.track, index: e.index > entry.index ? e.index + 1 : e.index }, t))
    const name = copyName(label, after)
    dispatcher.run(duplicateTrack(entry.id, name))
    const made = project.tracks.find((e) => !before.has(e.id))
    if (made) dispatcher.run(selectTrack(made.id))
  }

  function remove() {
    closeMenu(false)
    dispatcher.run(deleteTrack(entry.id))
  }

  function onGripDragStart(event: DragEvent) {
    if (!drag || !event.dataTransfer) return
    event.dataTransfer.effectAllowed = 'move'
    // Firefox starts no drag without data.
    event.dataTransfer.setData('text/plain', entry.id)
    if (root) event.dataTransfer.setDragImage(root, 12, 12)
    drag.start(entry.id)
  }

  function onDragOver(event: DragEvent) {
    if (!drag) return
    // Allows the drop; the list ignores a drag that is not one of its own headers.
    event.preventDefault()
    drag.over(entry.id)
  }

  function onDrop(event: DragEvent) {
    if (!drag) return
    event.preventDefault()
    drag.drop(entry.id)
  }

  /** Only the name, the kind line and the background select: a click on a control acts on its track without moving the keyboard. */
  function select(event: MouseEvent) {
    if ((event.target as Element).closest('.strip, .swatch, .icon, .grip, .pop')) return
    if (!selected) dispatcher.run(selectTrack(entry.id))
  }
</script>

<svelte:window onpointerdown={onWindowPointer} />

<!-- A click anywhere on the header selects the track; each control is also reachable from the keyboard. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<li
  class="header"
  class:selected
  class:open={menuOpen}
  class:dragging
  class:drop-target={dropTarget}
  class:muted={mixer.mute}
  bind:this={root}
  onkeydown={onRootKey}
  onfocusout={onRootFocusOut}
  ondragover={onDragOver}
  ondrop={onDrop}
  style:--track-color={track.color ?? NO_COLOR}
  data-testid="track-header"
  data-track={entry.id}
  aria-current={selected ? 'true' : undefined}
  onclick={select}
>
  <div class="top">
    {#if drag}
      <!-- The grip drags the header; the menu's Move up and Move down are the keyboard's way. -->
      <span
        class="grip"
        role="presentation"
        draggable="true"
        title={t('tracks.drag', { name: label })}
        data-testid="track-grip"
        ondragstart={onGripDragStart}
        ondragend={() => drag.end()}
      >
        <svg width="8" height="12" viewBox="0 0 8 12" aria-hidden="true"
          ><path d="M2 2h.01M6 2h.01M2 6h.01M6 6h.01M2 10h.01M6 10h.01" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg
        >
      </span>
    {/if}
    <label class="swatch" title={t('tracks.color', { name: label })}>
      <input
        type="color"
        value={track.color ?? NO_COLOR}
        aria-label={t('tracks.color', { name: label })}
        onchange={(e) => dispatcher.run(setTrackColor(entry.id, e.currentTarget.value))}
      />
    </label>
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
        class="name"
        type="button"
        bind:this={nameButton}
        title={t('tracks.renameHint')}
        data-testid="track-name"
        onkeydown={onNameKey}
        ondblclick={startRename}>{label}</button
      >
    {/if}
    <button
      class="icon more"
      type="button"
      aria-haspopup="menu"
      aria-expanded={menuOpen}
      aria-label={t('tracks.actions', { name: label })}
      title={t('tracks.actions', { name: label })}
      data-testid="track-more"
      onclick={openMenu}>⋯</button
    >
    {#if menuOpen}
      <div class="pop" role="menu" data-testid="track-menu">
        <button
          role="menuitem"
          type="button"
          onclick={() => {
            closeMenu(false)
            void startRename()
          }}>{t('tracks.menu.rename')}</button
        >
        <button role="menuitem" type="button" disabled={entry.index === 0} onclick={() => move(entry.index - 1)}>{t('tracks.menu.moveUp')}</button>
        <button role="menuitem" type="button" disabled={entry.index >= count - 1} onclick={() => move(entry.index + 1)}>{t('tracks.menu.moveDown')}</button>
        <button role="menuitem" type="button" data-testid="track-duplicate" onclick={duplicate}>{t('tracks.menu.duplicate')}</button>
        <button role="menuitem" type="button" data-testid="track-delete" onclick={remove}>{t('tracks.menu.delete')}</button>
      </div>
    {/if}
  </div>
  <p class="meta">{track.kind === 'midi' ? t('tracks.kind.midi') : t('tracks.kind.audio')}{instrument ? ` · ${instrument}` : ''}</p>
  {#if !compact}
    <div class="strip">
      <button
        class="toggle mute"
        type="button"
        aria-pressed={mixer.mute}
        aria-label={t('tracks.mute', { name: label })}
        title={t('tracks.mute', { name: label })}
        onclick={() => dispatcher.run(setMute(entry.id, !mixer.mute))}>M</button
      >
      <button
        class="toggle solo"
        type="button"
        aria-pressed={soloed}
        aria-label={t('tracks.solo', { name: label })}
        title={t('tracks.solo', { name: label })}
        onclick={() => dispatcher.run(setSolo(entry.id, !soloed))}>S</button
      >
      <!-- Only a MIDI track has an instrument to play: arming an audio track comes with audio recording. -->
      {#if track.kind === 'midi'}
        <button
          class="toggle arm"
          type="button"
          aria-pressed={armed}
          aria-label={t('tracks.arm', { name: label })}
          title={t('tracks.armHint', { name: label })}
          data-testid="track-arm"
          onclick={() => dispatcher.run(armTrack(entry.id, !armed))}>R</button
        >
      {/if}
      <GainControl
        bind:this={gainControl}
        gainDb={shownGain}
        label={t('tracks.volume', { name: label })}
        set={(gain) => setVolume(entry.id, gain)}
        automated={automated.mixerMark(entry.id, 'gain')}
      />
      <button
        class="value readout"
        type="button"
        tabindex="-1"
        title={t('controls.type')}
        data-testid="track-gain-readout"
        onmousedown={(e) => e.preventDefault()}
        onclick={() => gainControl?.edit()}>{formatGain(shownGain)}</button
      >
    </div>
    <div class="strip pan">
      <span class="label">{t('tracks.panShort')}</span>
      <PanControl
        bind:this={panControl}
        track={entry.id}
        pan={shownPan}
        label={t('tracks.pan', { name: label })}
        automated={automated.mixerMark(entry.id, 'pan')}
      />
      <button
        class="value readout"
        type="button"
        tabindex="-1"
        title={t('controls.type')}
        data-testid="track-pan-readout"
        onmousedown={(e) => e.preventDefault()}
        onclick={() => panControl?.edit()}>{formatPan(shownPan)}</button
      >
    </div>
  {/if}
</li>

<style>
  .header {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-1);
    padding: var(--fk-space-2) var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-left: 4px solid var(--track-color);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    cursor: default;
  }
  /* The selection: a 2px light outline and a tint, with secondary text that stays AA on it. */
  .header.selected {
    --fk-text-muted: var(--fk-text-muted-on-tint);
    background: var(--fk-accent-tint);
    outline: 2px solid var(--fk-text);
    outline-offset: -1px;
  }
  .header {
    position: relative;
  }
  /* The header with an open menu rises above the headers under it. */
  .header.open {
    z-index: 6;
  }
  .header.dragging {
    opacity: 0.45;
  }
  .header.drop-target {
    box-shadow: 0 0 0 2px var(--fk-accent);
  }
  .grip {
    flex: none;
    display: grid;
    place-items: center;
    width: 12px;
    height: var(--fk-row);
    margin-left: calc(-1 * var(--fk-space-1));
    color: var(--fk-text-muted);
    cursor: grab;
  }
  .grip:hover {
    color: var(--fk-text);
  }
  .more {
    font-size: 16px;
    line-height: 1;
  }
  .pop {
    position: absolute;
    z-index: 6;
    top: calc(var(--fk-space-2) + var(--fk-row) + 2px);
    right: var(--fk-space-3);
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
    cursor: pointer;
  }
  .pop button:hover:not(:disabled) {
    background: var(--fk-surface-2);
  }
  .pop button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .header.muted .name,
  .header.muted .meta {
    opacity: 0.55;
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-width: 0;
  }
  .swatch {
    position: relative;
    flex: none;
    width: 14px;
    height: 14px;
    border-radius: var(--fk-radius-work);
    background: var(--track-color);
    cursor: pointer;
    overflow: hidden;
  }
  .swatch input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
  }
  .swatch:focus-within {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: 2px;
  }
  .name,
  .rename {
    flex: 1;
    min-width: 0;
    height: var(--fk-row);
    padding: 0 var(--fk-space-1);
    border: 1px solid transparent;
    border-radius: var(--fk-radius-work);
    background: transparent;
    font-weight: 600;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .rename {
    border-color: var(--fk-accent-fg);
    background: var(--fk-bg);
    cursor: text;
  }
  .meta {
    margin: 0;
    padding-left: 22px;
    color: var(--fk-text-muted);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .icon {
    flex: none;
    display: grid;
    place-items: center;
    width: var(--fk-row);
    height: var(--fk-row);
    padding: 0;
    border: 0;
    border-radius: var(--fk-radius-work);
    background: transparent;
    color: var(--fk-text-muted);
    cursor: pointer;
  }
  .icon:hover {
    background: var(--fk-surface-2);
    color: var(--fk-text);
  }
  .strip {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-width: 0;
  }
  .toggle {
    flex: none;
    width: var(--fk-row);
    height: var(--fk-row);
    padding: 0;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: transparent;
    font-weight: 700;
    font-size: 11px;
    cursor: pointer;
  }
  .toggle:hover {
    background: var(--fk-surface-2);
  }
  .mute[aria-pressed='true'] {
    background: var(--fk-mute);
    border-color: var(--fk-mute);
    color: #131211;
  }
  .solo[aria-pressed='true'] {
    background: var(--fk-solo);
    border-color: var(--fk-solo);
    color: #131211;
  }
  .arm[aria-pressed='true'] {
    background: var(--fk-rec);
    border-color: var(--fk-rec);
    color: #131211;
  }
  .readout {
    border: 0;
    padding: 0;
    background: transparent;
    font: inherit;
    cursor: text;
    flex: none;
    width: 5ch;
    text-align: right;
    color: var(--fk-text-muted);
  }
  .strip .label {
    flex: none;
    width: calc(2 * var(--fk-row) + var(--fk-space-2));
    text-align: right;
  }
</style>
