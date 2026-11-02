<script lang="ts">
  /**
   * The on-screen keyboard, a tab of the bottom panel next to the piano roll: two octaves from the
   * computer keyboard's octave, the notes that sound lit, the octave and the target, and the panic button
   * (the `synth.all_notes_off` command). Display and one command: the keys themselves are played by the shell.
   */
  import PianoStrip from '../../lib/components/PianoStrip.svelte'
  import { useStores } from '../../lib/stores'
  import { allNotesOff } from '../../lib/synth-commands'

  const { engine, ui, dispatcher, keys } = useStores()
  const t = ui.t
</script>

<div class="keyboard" data-testid="keyboard-panel">
  <div class="head">
    <span class="value" data-testid="keys-octave">{t('keys.octave', { n: keys.octave })}</span>
    <span class="muted target" data-testid="keys-target">{keys.targetHint}</span>
    <button class="ghost" data-testid="panic" onclick={() => dispatcher.run(allNotesOff())}>{t('panic')}</button>
  </div>
  <PianoStrip low={(keys.octave + 1) * 12} active={engine.activeNotes} />
  <p class="muted help">{t('keys.help')}</p>
</div>

<style>
  .keyboard {
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-3);
    max-width: 960px;
    padding: var(--fk-space-3) var(--fk-space-4);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--fk-space-3);
  }
  .muted {
    margin: 0;
    color: var(--fk-text-muted);
  }
  .target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .head button {
    margin-left: auto;
    height: var(--fk-control);
    padding: 0 var(--fk-space-4);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: transparent;
    cursor: pointer;
    white-space: nowrap;
  }
  .head button:hover {
    background: var(--fk-surface-2);
  }
  .keyboard :global(.piano) {
    height: 72px;
  }
</style>
