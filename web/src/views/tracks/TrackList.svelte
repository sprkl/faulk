<script lang="ts">
  /**
   * The left column (#471): the menu that adds a track (a MIDI track takes the instrument chosen in it, or an audio
   * track), Undo and Redo (a deleted track comes back that way), the musical typing indicator, and the inspector of
   * the selected track. The tracks themselves are listed once, as the arrangement's rows. Every action is a command
   * through the dispatcher.
   */
  import { INSTRUMENTS, OFFER_AUDIO_TRACK } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import { createTrack, redo, selectTrack, undo } from '../../lib/track-commands'
  import { withShortcut } from '../../lib/shortcuts'
  import TrackInspector from './TrackInspector.svelte'

  const { project, engine, ui, dispatcher, keys } = useStores()
  const t = ui.t

  // A project that opens with no selected track selects its first, once: the computer keyboard plays the
  // selected track. Later, deselecting (deleting the selected track) is the user's.
  let selectedFirst = false
  $effect(() => {
    if (selectedFirst || !project.view) return
    selectedFirst = true
    const first = project.tracks[0]
    if (first && !project.selected) dispatcher.run(selectTrack(first.id))
  })

  let menuOpen = $state(false)
  let addButton = $state<HTMLButtonElement | null>(null)

  /** Creates a track and selects it, so the keyboard plays it. */
  function add(kind: 'midi' | 'audio', instrument: string | null) {
    menuOpen = false
    const before = new Set(project.tracks.map((entry) => entry.id))
    dispatcher.run(createTrack(kind, instrument))
    const created = project.tracks.find((entry) => !before.has(entry.id))
    if (created) dispatcher.run(selectTrack(created.id))
    addButton?.focus()
  }

  function onMenuKey(event: KeyboardEvent) {
    if (event.key !== 'Escape') return
    event.stopPropagation()
    menuOpen = false
    addButton?.focus()
  }

  function onFocusOut(event: FocusEvent) {
    const next = event.relatedTarget as Node | null
    if (!next || !(event.currentTarget as HTMLElement).contains(next)) menuOpen = false
  }
</script>

<aside class="tracks" aria-labelledby="fk-tracks-title" data-testid="track-list" data-track-count={project.view ? project.tracks.length : undefined}>
  <div class="head">
    <h2 id="fk-tracks-title" class="label">{t('tracks.title')}</h2>
    <button class="ghost" type="button" disabled={!project.canUndo} title={withShortcut(t('tracks.undo'), ['Mod', 'Z'])} onclick={() => dispatcher.run(undo())}
      >{t('tracks.undo')}</button
    >
    <button
      class="ghost"
      type="button"
      disabled={!project.canRedo}
      title={withShortcut(t('tracks.redo'), ['Mod', 'Shift', 'Z'])}
      onclick={() => dispatcher.run(redo())}>{t('tracks.redo')}</button
    >
  </div>
  <!-- Musical typing (#470): always shown, with the key that toggles it in its hint. -->
  <button
    class="ghost typing"
    type="button"
    data-testid="typing-mode"
    data-mode={ui.typing}
    title={t(ui.typing === 'notes' ? 'keys.typingHintNotes' : 'keys.typingHintShortcuts')}
    onclick={() => keys.toggleTyping()}>{t(ui.typing === 'notes' ? 'keys.typingNotes' : 'keys.typingShortcuts')}</button
  >

  {#if !engine.started && !project.view}
    <p class="empty" data-testid="tracks-waiting">{t('tracks.waiting')}</p>
  {:else}
    <div class="add" role="presentation" onkeydown={onMenuKey} onfocusout={onFocusOut}>
      <button
        class="wide"
        class:primary={engine.started}
        type="button"
        bind:this={addButton}
        disabled={!engine.started}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        data-testid="add-track"
        onclick={() => (menuOpen = !menuOpen)}>+ {t('tracks.add')}</button
      >
      {#if menuOpen}
        <div class="menu" role="menu" data-testid="add-track-menu">
          <p class="label group">{t('tracks.addMidi')}</p>
          {#each INSTRUMENTS as instrument (instrument.kind)}
            <button role="menuitem" type="button" onclick={() => add('midi', instrument.kind)}>{instrument.name}</button>
          {/each}
          {#if OFFER_AUDIO_TRACK}
            <hr />
            <button role="menuitem" type="button" onclick={() => add('audio', null)}>{t('tracks.addAudio')}</button>
          {/if}
        </div>
      {/if}
    </div>

    {#if project.tracks.length === 0}
      <p class="empty" data-testid="tracks-empty">{t('tracks.empty')}</p>
    {:else if project.selected}
      {#key project.selected.id}
        <TrackInspector entry={project.selected} count={project.tracks.length} />
      {/key}
    {:else}
      <p class="empty" data-testid="inspector-empty">{t('tracks.noSelection')}</p>
    {/if}
  {/if}
</aside>

<style>
  .tracks {
    grid-area: tracks;
    min-height: 0;
    min-width: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-3);
    padding: var(--fk-space-3);
    border-right: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
  }
  .typing {
    align-self: flex-start;
  }
  /* Typing notes: the button is filled in the text colour (the words say it too), never a hue. */
  .typing[data-mode='notes'] {
    background: var(--fk-text);
    border-color: var(--fk-text);
    color: var(--fk-bg);
  }
  .head h2 {
    flex: 1;
  }
  h2,
  p {
    margin: 0;
  }
  .empty {
    color: var(--fk-text-muted);
  }
  button {
    min-height: var(--fk-control);
    padding: 0 var(--fk-space-3);
    border-radius: var(--fk-radius-pill);
    border: 1px solid var(--fk-line);
    background: transparent;
    cursor: pointer;
    transition: background var(--fk-dur-fast) ease-out;
  }
  button:hover:not(:disabled) {
    background: var(--fk-surface-2);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button.ghost {
    min-height: var(--fk-row);
    font-size: 12px;
  }
  button.wide {
    width: 100%;
  }
  button.primary {
    width: 100%;
    background: var(--fk-accent);
    border-color: var(--fk-accent);
    color: #fff;
    font-weight: 600;
  }
  button.primary:hover:not(:disabled) {
    background: var(--fk-accent-strong);
  }
  .add {
    position: relative;
  }
  .menu {
    position: absolute;
    z-index: 5;
    top: calc(100% + var(--fk-space-1));
    left: 0;
    right: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
  }
  .menu button {
    border: 0;
    border-radius: var(--fk-radius-panel);
    text-align: left;
  }
  .menu hr {
    width: 100%;
    margin: var(--fk-space-1) 0;
    border: 0;
    border-top: 1px solid var(--fk-line);
  }
  .group {
    padding: var(--fk-space-1) var(--fk-space-3);
  }
  @media (max-width: 760px) {
    .tracks {
      border-right: 0;
      border-top: 1px solid var(--fk-line);
    }
  }
</style>
