<script lang="ts">
  /**
   * The inspector of the selected track (#471), in the left column: colour, name (renamed from the keyboard with F2,
   * Enter to confirm), kind and instrument, the actions menu (rename, move, duplicate, delete), volume, pan and a summary
   * of its devices. The tracks themselves are listed once, as the arrangement's rows (`LaneHeader`), where mute, solo and
   * live input are. It holds no project state: it reads the stores and every action is a command (`track-commands.ts`)
   * through the dispatcher.
   */
  import { tick } from 'svelte'
  import ColorPicker from '../../lib/components/ColorPicker.svelte'
  import GainControl from '../../lib/components/GainControl.svelte'
  import PanControl from '../../lib/components/PanControl.svelte'
  import { chainOf } from '../../lib/project/chain'
  import { copyName, formatGain, formatPan, instrumentName, trackColor, trackLabel, type TrackEntry } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import { deleteTrack, duplicateTrack, moveTrack, renameTrack, selectTrack, setTrackColor, setVolume } from '../../lib/track-commands'

  let {
    entry,
    count = 1,
  }: {
    entry: TrackEntry
    /** How many tracks the project holds: the first cannot move up, the last cannot move down. */
    count?: number
  } = $props()

  const { project, ui, dispatcher, automated } = useStores()
  const t = ui.t

  const track = $derived(entry.track)
  const label = $derived(trackLabel(entry, t))
  const color = $derived(trackColor(track.color, entry.index))
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
  let picking = $state(false)
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

  const devices = $derived(chainOf(track).map((d) => d.descriptor?.name ?? d.kind ?? '?'))

  /** The devices panel, on this track's chain, unfolded. */
  function showDevices() {
    ui.showDevices(false)
    if (ui.panelCollapsed) ui.setPanelCollapsed(false)
  }
</script>

<svelte:window onpointerdown={onWindowPointer} />

<!-- The section only listens for Escape and a focus that leaves it, to close its menu; its controls are the keyboard's way in. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  class="inspector"
  class:open={menuOpen || picking}
  class:muted={mixer.mute}
  bind:this={root}
  onkeydown={onRootKey}
  onfocusout={onRootFocusOut}
  style:--track-color={color}
  aria-label={t('tracks.inspector', { name: label })}
  data-testid="track-inspector"
  data-track={entry.id}
>
  <div class="top">
    <ColorPicker bind:open={picking} {color} stored={track.color ?? null} name={label} onpick={(picked) => dispatcher.run(setTrackColor(entry.id, picked))} />
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
        data-testid="inspector-name"
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
      onclick={openMenu}
      ><svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"
        ><path d="M3 7h.01M7 7h.01M11 7h.01" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" /></svg
      ></button
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
  <div class="strip">
    <span class="label">{t('tracks.volumeShort')}</span>
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
      aria-label={t('controls.valueOf', { name: t('tracks.volume', { name: label }), value: `${formatGain(shownGain)} dB` })}
      title={t('controls.type')}
      data-testid="track-gain-readout"
      onmousedown={(e) => e.preventDefault()}
      onclick={() => gainControl?.edit()}>{formatGain(shownGain)}</button
    >
  </div>
  <div class="strip">
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
      aria-label={t('controls.valueOf', { name: t('tracks.pan', { name: label }), value: formatPan(shownPan) })}
      title={t('controls.type')}
      data-testid="track-pan-readout"
      onmousedown={(e) => e.preventDefault()}
      onclick={() => panControl?.edit()}>{formatPan(shownPan)}</button
    >
  </div>
  <div class="devices" data-testid="inspector-devices">
    <h3 class="label" id="fk-inspector-devices">{t('tracks.devices')}</h3>
    {#if devices.length === 0}
      <p class="none">{t('tracks.noDevices')}</p>
    {:else}
      <ol aria-labelledby="fk-inspector-devices">
        {#each devices as name, i (i)}
          <li>{name}</li>
        {/each}
      </ol>
    {/if}
    <button class="show" type="button" data-testid="inspector-show-devices" onclick={showDevices}>{t('tracks.showDevices')}</button>
  </div>
</section>

<style>
  /* Square, as the work it describes: a 3px bar of the track's colour heads it (DESIGN.md: the track header's bar). */
  .inspector {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-2);
    padding: var(--fk-space-3) 0 0;
    border-top: 3px solid var(--track-color);
  }
  .inspector.open {
    z-index: 6;
  }
  .more {
    line-height: 1;
  }
  .pop {
    position: absolute;
    z-index: 6;
    top: calc(var(--fk-space-3) + var(--fk-row) + 2px);
    right: 0;
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
  .inspector.muted .name,
  .inspector.muted .meta {
    opacity: 0.55;
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-width: 0;
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
    font-size: 15px;
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
    font-size: 13px;
    cursor: text;
  }
  .meta {
    margin: 0;
    color: var(--fk-text-muted);
    font-size: 12px;
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
    border-radius: var(--fk-radius-panel);
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
  .strip .label {
    flex: none;
    width: 3em;
  }
  .readout {
    flex: none;
    width: 5ch;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    text-align: right;
    cursor: text;
  }
  .devices {
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-1);
    padding-top: var(--fk-space-2);
    border-top: 1px solid var(--fk-line);
  }
  h3 {
    margin: 0;
  }
  ol {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    padding: 2px 0;
    font-size: 12px;
  }
  li + li {
    border-top: 1px solid var(--fk-line);
  }
  .none {
    margin: 0;
    color: var(--fk-text-muted);
    font-size: 12px;
  }
  .show {
    align-self: flex-start;
    min-height: var(--fk-row);
    margin-top: var(--fk-space-1);
    padding: 0 var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: transparent;
    font-size: 12px;
    cursor: pointer;
  }
  .show:hover {
    background: var(--fk-surface-2);
  }
</style>
