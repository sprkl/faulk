<script lang="ts">
  import Meter from '../../lib/components/Meter.svelte'
  import PianoStrip from '../../lib/components/PianoStrip.svelte'
  import { incompatibleMessage } from '../../lib/engine/helper-client'
  import { absentHelperMessages } from '../../lib/engine/local-access'
  import { useStores } from '../../lib/stores'
  import { allNotesOff } from '../../lib/synth-commands'

  const stores = useStores()
  const { engine, ui, dispatcher } = stores
  const t = ui.t

  let code = $state('')
  // The computer keyboard plays in every view: `keys` (the shell's) holds the octave and the target this view shows.
  const keys = stores.keys
</script>

{#if !engine.started}
  <div class="start">
    <section class="card">
      <h2>{t('start.title')}</h2>
      <p>{t('start.body')}</p>
      {#if engine.pendingPairCode}
        <p class="muted" role="status" data-testid="pair-pending">{t('start.pairPending')}</p>
      {/if}
      <button class="primary" disabled={engine.starting} onclick={() => dispatcher.start()}>{t('start.button')}</button>
    </section>
  </div>
{:else}
  <div class="grid">
    <section class="panel helper">
      <h2 class="label">{t('helper.title')}</h2>
      {#if engine.helper === 'checking'}
        <p class="muted">{t('helper.checking')}</p>
      {:else if engine.helper === 'absent'}
        {#each absentHelperMessages(engine.localAccess) as key (key)}
          <p class="muted" data-testid={key}>{t(key)}</p>
        {/each}
        <button class="ghost" onclick={() => dispatcher.checkHelper()}>{t('helper.recheck')}</button>
      {:else if engine.helper === 'incompatible'}
        <p class="muted" data-testid="helper-incompatible">
          {t(incompatibleMessage(engine.helperProtocol ?? 0), { v: engine.helperProtocol ?? '?' })}
        </p>
      {:else if engine.mode === 'native'}
        <button class="ghost" onclick={() => dispatcher.enterWeb()}>{t('helper.useWeb')}</button>
      {:else if engine.paired}
        <button class="primary" disabled={engine.helper === 'connecting'} onclick={() => dispatcher.enterBoost()}>{t('helper.useBoost')}</button>
      {:else}
        <form
          class="pair"
          onsubmit={(e) => {
            e.preventDefault()
            void dispatcher.pair(code)
          }}
        >
          <label>
            <span class="muted">{t('helper.pairPrompt')}</span>
            <input class="value" bind:value={code} inputmode="numeric" autocomplete="off" placeholder="0000 0000" aria-label={t('helper.code')} />
          </label>
          <button class="primary" type="submit">{t('helper.pair')}</button>
        </form>
        {#if engine.pairingMessage}
          <p class="muted" role="status">{t(engine.pairingMessage.key, { n: engine.pairingMessage.n ?? 0 })}</p>
        {/if}
      {/if}
    </section>

    <section class="panel midi">
      <h2 class="label">{t('midi.title')}</h2>
      {#if engine.mode === 'web' && engine.midiState === 'unsupported'}
        <p class="muted">{t('midi.unsupported')}</p>
      {:else if engine.mode === 'web' && engine.midiState === 'denied'}
        <p class="muted">{t('midi.denied')}</p>
      {:else if engine.inputs.length === 0}
        <p class="muted" data-testid="midi-empty">{t('midi.empty')}</p>
      {:else}
        <ul class="inputs" data-testid="midi-inputs">
          <li>
            <label><input type="radio" name="midi" checked={engine.selectedInput === null} onchange={() => dispatcher.run({ type: 'midi.select_input', id: null })} /> {t('midi.none')}</label>
          </li>
          {#each engine.inputs as input (input.id)}
            <li>
              <label>
                <input type="radio" name="midi" checked={engine.selectedInput === input.id} onchange={() => dispatcher.run({ type: 'midi.select_input', id: input.id })} />
                {input.name}
                {#if engine.selectedInput === input.id && engine.activeNotes.length > 0}<span class="led" aria-hidden="true"></span>{/if}
              </label>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="panel level">
      <h2 class="label">{t('meter.label')}</h2>
      <div class="meter-box">
        <Meter peak={engine.peak} label={t('meter.label')} clipLabel={t('meter.clip')} />
      </div>
    </section>

    <section class="panel keys">
      <div class="keys-head">
        <h2 class="label">{t('keys.title')}</h2>
        <span class="value">{t('keys.octave', { n: keys.octave })}</span>
        <span class="muted target" data-testid="keys-target">{keys.targetHint}</span>
        <button class="ghost" onclick={() => dispatcher.run(allNotesOff())}>{t('panic')}</button>
      </div>
      <PianoStrip low={(keys.octave + 1) * 12} active={engine.activeNotes} />
      <p class="muted">{t('keys.help')}</p>
    </section>
  </div>
{/if}

<style>
  .muted {
    color: var(--fk-text-muted);
  }
  .start {
    display: grid;
    place-items: center;
    padding: var(--fk-space-12) var(--fk-space-4);
  }
  .card {
    max-width: 420px;
    padding: var(--fk-space-8);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    border: 1px solid var(--fk-line);
  }
  .card h2 {
    margin: 0 0 var(--fk-space-2);
    font-size: 15px;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 2fr);
    grid-template-areas:
      'helper level'
      'midi level'
      'keys keys';
    gap: var(--fk-space-4);
    padding: var(--fk-space-6);
  }
  @media (max-width: 760px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
      grid-template-areas: 'helper' 'midi' 'level' 'keys';
      padding: var(--fk-space-4);
    }
  }
  .panel {
    padding: var(--fk-space-4);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface);
    border: 1px solid var(--fk-line);
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-3);
  }
  .panel h2 {
    margin: 0;
  }
  .helper {
    grid-area: helper;
  }
  .midi {
    grid-area: midi;
  }
  .level {
    grid-area: level;
  }
  .keys {
    grid-area: keys;
  }
  .keys-head {
    display: flex;
    align-items: center;
    gap: var(--fk-space-3);
  }
  .target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .keys-head button {
    margin-left: auto;
  }
  p {
    margin: 0;
  }
  button {
    min-height: var(--fk-control);
    padding: 0 var(--fk-space-4);
    border-radius: var(--fk-radius-pill);
    border: 1px solid var(--fk-line);
    background: transparent;
    cursor: pointer;
    transition: background var(--fk-dur-fast) ease-out;
  }
  button:hover {
    background: var(--fk-surface-2);
  }
  button.primary {
    background: var(--fk-accent);
    border-color: var(--fk-accent);
    color: #fff;
    font-weight: 600;
  }
  button.primary:hover {
    background: var(--fk-accent-strong);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .pair {
    display: flex;
    gap: var(--fk-space-2);
    align-items: flex-end;
    flex-wrap: wrap;
  }
  .pair label {
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-1);
  }
  .pair input {
    height: var(--fk-control);
    width: 140px;
    padding: 0 var(--fk-space-3);
    border-radius: var(--fk-radius-panel);
    border: 1px solid var(--fk-line);
    background: var(--fk-bg);
    font-size: 15px;
    letter-spacing: 0.1em;
  }
  .inputs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-1);
  }
  .inputs label {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-height: var(--fk-row);
  }
  .led {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--fk-play);
  }
  .meter-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--fk-space-1);
    margin-left: auto;
  }
</style>
