<script lang="ts">
  /**
   * A track's header: colour, name (renamed from the keyboard with F2, Enter to confirm), kind and
   * instrument, mute, solo, volume, pan and delete. Shared by the track list, and later the grid,
   * the timeline and the mixer. It holds no project state: it reads the stores and every action
   * is a command (`track-commands.ts`) through the dispatcher. Arming comes with recording (RE).
   */
  import { onDestroy, tick } from 'svelte'
  import { createGesture } from '../gesture'
  import {
    faderValue,
    formatGain,
    formatPan,
    gainOf,
    GAIN_STEP_DB,
    instrumentName,
    MAX_GAIN_DB,
    MIN_GAIN_DB,
    NO_COLOR,
    trackLabel,
    type TrackEntry,
  } from '../project/tracks'
  import { useStores } from '../stores'
  import { deleteTrack, renameTrack, selectTrack, setMute, setPan, setSolo, setTrackColor, setVolume } from '../track-commands'

  let { entry, selected = false, soloed = false, compact = false }: { entry: TrackEntry; selected?: boolean; soloed?: boolean; compact?: boolean } = $props()

  const { ui, dispatcher } = useStores()
  const t = ui.t

  const track = $derived(entry.track)
  const label = $derived(trackLabel(entry, t))
  const instrument = $derived(instrumentName(track))
  const mixer = $derived(track.mixer)

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

  function select() {
    if (!selected) dispatcher.run(selectTrack(entry.id))
  }

  const gesture = createGesture((command) => dispatcher.run(command))
  onDestroy(() => gesture.end())
</script>

<!-- A click anywhere on the header selects the track; each control is also reachable from the keyboard. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<li
  class="header"
  class:selected
  class:muted={mixer.mute}
  style:--track-color={track.color ?? NO_COLOR}
  data-testid="track-header"
  data-track={entry.id}
  aria-current={selected ? 'true' : undefined}
  onclick={select}
>
  <div class="top">
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
      class="icon"
      type="button"
      aria-label={t('tracks.delete', { name: label })}
      title={t('tracks.delete', { name: label })}
      onclick={(event) => {
        event.stopPropagation()
        dispatcher.run(deleteTrack(entry.id))
      }}
    >
      <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="M2 2 L10 10 M10 2 L2 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /></svg>
    </button>
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
      <input
        class="fader"
        type="range"
        min={MIN_GAIN_DB}
        max={MAX_GAIN_DB}
        step={GAIN_STEP_DB}
        value={faderValue(mixer.gain_db)}
        aria-label={t('tracks.volume', { name: label })}
        aria-valuetext={`${formatGain(mixer.gain_db)} dB`}
        oninput={(e) => dispatcher.run(setVolume(entry.id, gainOf(Number(e.currentTarget.value))))}
        onpointerdown={gesture.begin}
        onpointerup={gesture.end}
        onpointercancel={gesture.end}
        onchange={gesture.end}
        onblur={gesture.end}
        ondblclick={() => dispatcher.run(setVolume(entry.id, 0))}
      />
      <span class="value readout">{formatGain(mixer.gain_db)}</span>
    </div>
    <div class="strip pan">
      <span class="label">{t('tracks.panShort')}</span>
      <input
        class="fader"
        type="range"
        min="-1"
        max="1"
        step="0.01"
        value={mixer.pan}
        aria-label={t('tracks.pan', { name: label })}
        aria-valuetext={formatPan(mixer.pan)}
        oninput={(e) => dispatcher.run(setPan(entry.id, Number(e.currentTarget.value)))}
        onpointerdown={gesture.begin}
        onpointerup={gesture.end}
        onpointercancel={gesture.end}
        onchange={gesture.end}
        onblur={gesture.end}
        ondblclick={() => dispatcher.run(setPan(entry.id, 0))}
      />
      <span class="value readout">{formatPan(mixer.pan)}</span>
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
  .header.selected {
    background: var(--fk-accent-tint);
    border-color: var(--fk-accent-fg);
    border-left-color: var(--track-color);
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
  .fader {
    flex: 1;
    min-width: 0;
    height: var(--fk-row);
    margin: 0;
    accent-color: var(--fk-accent-fg);
  }
  .readout {
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
