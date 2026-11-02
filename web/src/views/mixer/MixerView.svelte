<script lang="ts">
  /**
   * The mixer, on the whole height of the main area (#472): one fixed-width strip per track in the order of the
   * track list, left aligned; a ghost "+ Add track" strip fills the free room; the master's pinned at the right
   * and never leaves the screen. The faders grow with the height. The meters come from the shared meter store
   * (`stores/meters.svelte.ts`). Every control is a command (`MixerStrip`, `createTrack`).
   */
  import { INSTRUMENTS, nextColor } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import { createTrack, selectTrack } from '../../lib/track-commands'
  import MixerStrip from './MixerStrip.svelte'

  const { project, ui, dispatcher } = useStores()
  const t = ui.t

  let menuOpen = $state(false)
  let addButton = $state<HTMLButtonElement | null>(null)

  /** A MIDI track with this instrument, selected so that the keyboard plays it (the track list's way). */
  function add(instrument: string) {
    menuOpen = false
    const before = new Set(project.tracks.map((entry) => entry.id))
    dispatcher.run(createTrack('midi', instrument, nextColor(project.tracks.length)))
    const created = project.tracks.find((entry) => !before.has(entry.id))
    if (created) dispatcher.run(selectTrack(created.id))
    addButton?.focus()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !menuOpen) return
    event.stopPropagation()
    menuOpen = false
    addButton?.focus()
  }
</script>

<div class="mixer" data-testid="mixer">
  <ul class="strips" aria-label={t('mixer.strips')}>
    {#each project.tracks as entry (entry.id)}
      <MixerStrip {entry} selected={project.selected?.id === entry.id} soloed={project.soloed.includes(entry.id)} armed={project.armed.includes(entry.id)} />
    {/each}
    <li class="ghost-slot" role="presentation" onkeydown={onKeyDown}>
      <button
        class="ghost"
        type="button"
        bind:this={addButton}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        data-testid="mixer-add-track"
        onclick={() => (menuOpen = !menuOpen)}
      >
        <span class="plus" aria-hidden="true">+</span>
        {t('tracks.add')}
      </button>
      {#if menuOpen}
        <div class="menu" role="menu" data-testid="mixer-add-track-menu">
          <p class="group">{t('tracks.addMidi')}</p>
          {#each INSTRUMENTS as instrument (instrument.kind)}
            <button role="menuitem" type="button" onclick={() => add(instrument.kind)}>{instrument.name}</button>
          {/each}
        </div>
      {/if}
    </li>
  </ul>
  {#if project.tracks.length === 0}
    <p class="empty" data-testid="mixer-empty">{t('mixer.empty')}</p>
  {/if}
  <ul class="master" aria-label={t('mixer.master')}>
    <MixerStrip />
  </ul>
</div>

<style>
  /* The area's own height drives the faders (`--fader-len`, read by the strips): container units, not a media query. */
  .mixer {
    container-type: size;
    --fader-len: clamp(168px, calc(100cqh - 300px), 520px);
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--fk-space-3);
    height: 100%;
    min-height: 0;
    padding: var(--fk-space-3);
    overflow: hidden;
  }
  .strips {
    display: flex;
    align-items: stretch;
    gap: var(--fk-space-2);
    min-width: 0;
    margin: 0;
    padding: 0;
    overflow-x: auto;
    overflow-y: hidden;
  }
  .master {
    display: flex;
    margin: 0;
    padding: 0;
    padding-left: var(--fk-space-3);
    border-left: 1px solid var(--fk-line);
  }
  .ghost-slot {
    list-style: none;
    position: relative;
    flex: 1 0 112px;
    display: flex;
  }
  .ghost {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--fk-space-2);
    border: 1px dashed var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: transparent;
    color: var(--fk-text-muted);
    cursor: pointer;
    transition:
      background var(--fk-dur-fast) ease-out,
      color var(--fk-dur-fast) ease-out;
  }
  .ghost:hover,
  .ghost[aria-expanded='true'] {
    background: var(--fk-surface);
    color: var(--fk-text);
  }
  .plus {
    font-size: 20px;
    line-height: 1;
  }
  .menu {
    position: absolute;
    top: var(--fk-space-2);
    left: var(--fk-space-2);
    z-index: 5;
    min-width: 168px;
    display: flex;
    flex-direction: column;
    padding: var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface);
    box-shadow: var(--fk-shadow-float);
  }
  .group {
    margin: 0;
    padding: var(--fk-space-1) var(--fk-space-2);
    color: var(--fk-text-muted);
    font-size: 11px;
  }
  .menu button {
    min-height: var(--fk-row);
    padding: 0 var(--fk-space-3);
    border: 0;
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text);
    text-align: left;
    cursor: pointer;
  }
  .menu button:hover {
    background: var(--fk-surface-2);
  }
  .empty {
    position: absolute;
    top: 50%;
    left: var(--fk-space-6);
    margin: 0;
    color: var(--fk-text-muted);
    pointer-events: none;
  }
</style>
