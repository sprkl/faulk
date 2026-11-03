<script lang="ts">
  /**
   * The engine's dialog (#472), opened by the engine pill of the top bar: what the Instrument view used to
   * show. The audio (mode, estimated or measured latency, load when the engine reports it, voices, output level),
   * the helper (detection, pairing, Boost or Web) and the MIDI input. A native modal dialog: the page behind
   * is inert, Escape closes it and the focus goes back to the pill. Nothing here writes the project: the
   * actions are the dispatcher's (`midi.select_input`, pairing, mode switch) and `synth.all_notes_off`.
   */
  import LevelMeter from '../../lib/components/LevelMeter.svelte'
  import { incompatibleMessage } from '../../lib/engine/helper-client'
  import { absentHelperMessages } from '../../lib/engine/local-access'
  import { MASTER_METER, useStores } from '../../lib/stores'
  import { allNotesOff } from '../../lib/synth-commands'
  import { QUANTIZE_GRIDS, DEFAULT_QUANTIZE_STRENGTH } from '../piano-roll/roll'

  const { engine, ui, dispatcher, meters, project } = useStores()
  const t = ui.t

  let dialog = $state<HTMLDialogElement | null>(null)
  let code = $state('')

  // The store says whether it is open; the element follows (and tells the store when the browser closed it: Escape).
  $effect(() => {
    if (!dialog) return
    if (ui.engineOpen && !dialog.open) dialog.showModal()
    else if (!ui.engineOpen && dialog.open) dialog.close()
  })

  const latency = $derived(
    engine.status?.estimatedLatencyMs != null ? t('engine.latency', { ms: Math.round(engine.status.estimatedLatencyMs) }) : t('engine.latencyUnknown'),
  )
  const load = $derived(engine.feedback?.cpuLoad != null ? `${Math.round(engine.feedback.cpuLoad * 100)} %` : null)

  /** The manual latency offset of recording (#470): session state, `record.set_settings`. */
  const offset = $derived(project.view?.session.record?.offset_ms ?? 0)
  let offsetDraft = $state('')
  let offsetFocus = $state(false)
  $effect(() => {
    if (!offsetFocus) offsetDraft = String(offset)
  })
  function commitOffset() {
    const value = Math.round(Number(offsetDraft.trim()))
    if (offsetDraft.trim() !== '' && Number.isFinite(value) && value !== offset)
      dispatcher.run({ type: 'record.set_settings', offset_ms: Math.max(-500, Math.min(500, value)) })
    offsetDraft = String(offset)
  }

  /** Replace mode and quantization on input (#500): session state too, `record.set_settings`. */
  const replace = $derived(project.view?.session.record?.replace ?? false)
  const inputGrid = $derived(project.view?.session.record?.quantize_on_input ?? 'none')
  const strength = $derived(project.view?.session.record?.quantize_strength ?? DEFAULT_QUANTIZE_STRENGTH)
  function onStrength(event: Event) {
    const input = event.currentTarget as HTMLInputElement
    const value = Math.round(Number(input.value))
    if (input.value.trim() !== '' && Number.isFinite(value) && value !== strength)
      dispatcher.run({ type: 'record.set_settings', quantize_strength: Math.max(0, Math.min(100, value)) })
    input.value = String(strength)
  }

  /** A click on the backdrop (the dialog element itself, outside its box) closes it. */
  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) ui.engineOpen = false
  }
</script>

<!-- svelte-ignore a11y_no_redundant_roles -->
<dialog
  bind:this={dialog}
  class="engine-dialog"
  role="dialog"
  aria-modal={ui.engineOpen ? 'true' : undefined}
  aria-labelledby="engine-dialog-title"
  data-testid="engine-dialog"
  onclose={() => (ui.engineOpen = false)}
  onclick={onBackdrop}
>
  <header>
    <h2 id="engine-dialog-title">{t('engine.settings')}</h2>
    <button
      class="close"
      type="button"
      aria-label={t('engine.close')}
      title={t('engine.close')}
      data-testid="engine-dialog-close"
      onclick={() => (ui.engineOpen = false)}
    >
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"
        ><path d="M3 3 L 11 11 M11 3 L 3 11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg
      >
    </button>
  </header>

  {#if engine.started}
    <section aria-labelledby="engine-audio">
      <h3 id="engine-audio">{t('engine.audio')}</h3>
      <dl>
        <dt>{t('engine.mode')}</dt>
        <dd data-testid="engine-dialog-mode">{engine.mode === 'native' ? t('engine.modeBoost') : t('engine.modeWeb')}</dd>
        <dt>{t('engine.latencyLabel')}</dt>
        <dd data-testid="engine-dialog-latency">{latency}</dd>
        <dt><label for="engine-record-offset">{t('engine.recordOffset')}</label></dt>
        <dd>
          <input
            id="engine-record-offset"
            class="offset"
            type="text"
            inputmode="numeric"
            autocomplete="off"
            size="4"
            title={t('engine.recordOffsetHint')}
            data-testid="engine-record-offset"
            bind:value={offsetDraft}
            onfocus={() => (offsetFocus = true)}
            onblur={() => {
              commitOffset()
              offsetFocus = false
            }}
            onkeydown={(event) => {
              if (event.key === 'Enter') commitOffset()
            }}
          />
          <span class="unit">ms</span>
        </dd>
        <dt><label for="engine-record-replace">{t('engine.recordReplace')}</label></dt>
        <dd>
          <input
            id="engine-record-replace"
            type="checkbox"
            checked={replace}
            title={t('engine.recordReplaceHint')}
            data-testid="engine-record-replace"
            onchange={(event) => dispatcher.run({ type: 'record.set_settings', replace: event.currentTarget.checked })}
          />
        </dd>
        <dt><label for="engine-record-quantize">{t('engine.recordQuantize')}</label></dt>
        <dd>
          <select
            id="engine-record-quantize"
            title={t('engine.recordQuantizeHint')}
            data-testid="engine-record-quantize"
            value={inputGrid}
            onchange={(event) =>
              dispatcher.run({ type: 'record.set_settings', quantize_on_input: event.currentTarget.value as (typeof QUANTIZE_GRIDS)[number]['id'] | 'none' })}
          >
            <option value="none">{t('engine.recordQuantizeNone')}</option>
            {#each QUANTIZE_GRIDS as grid (grid.id)}
              <option value={grid.id}>{t(`pianoRoll.qgrid.${grid.id}`)}</option>
            {/each}
          </select>
          <input
            class="offset"
            type="number"
            min="0"
            max="100"
            step="1"
            value={strength}
            disabled={inputGrid === 'none'}
            aria-label={t('engine.recordQuantizeStrength')}
            title={t('engine.recordQuantizeStrength')}
            data-testid="engine-record-quantize-strength"
            onchange={onStrength}
          />
          <span class="unit">%</span>
        </dd>
        <dt>{t('engine.loadLabel')}</dt>
        <dd data-testid="engine-dialog-load">{load ?? t('engine.loadNone')}</dd>
        <dt>{t('engine.voicesLabel')}</dt>
        <dd data-testid="engine-dialog-voices">{engine.voices}</dd>
        <dt>{t('meter.label')}</dt>
        <dd class="meter-box">
          <LevelMeter
            snap={meters.snap(MASTER_METER)}
            orientation="horizontal"
            label={t('meter.label')}
            clipLabel={t('meter.clip')}
            resetLabel={t('meter.reset', { name: t('mixer.master') })}
            onreset={() => meters.reset(MASTER_METER)}
            readout
            testid="play-meter"
          />
        </dd>
      </dl>
    </section>

    <section aria-labelledby="engine-helper" class="helper">
      <h3 id="engine-helper">{t('helper.title')}</h3>
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

    <section aria-labelledby="engine-midi">
      <h3 id="engine-midi">{t('midi.title')}</h3>
      {#if engine.mode === 'web' && engine.midiState === 'unsupported'}
        <p class="muted">{t('midi.unsupported')}</p>
      {:else if engine.mode === 'web' && engine.midiState === 'denied'}
        <p class="muted">{t('midi.denied')}</p>
      {:else if engine.inputs.length === 0}
        <p class="muted" data-testid="midi-empty">{t('midi.empty')}</p>
      {:else}
        <ul class="inputs" data-testid="midi-inputs">
          <li>
            <label
              ><input
                type="radio"
                name="midi"
                checked={engine.selectedInput === null}
                onchange={() => dispatcher.run({ type: 'midi.select_input', id: null })}
              />
              {t('midi.none')}</label
            >
          </li>
          {#each engine.inputs as input (input.id)}
            <li>
              <label>
                <input
                  type="radio"
                  name="midi"
                  checked={engine.selectedInput === input.id}
                  onchange={() => dispatcher.run({ type: 'midi.select_input', id: input.id })}
                />
                {input.name}
                {#if engine.selectedInput === input.id && engine.activeNotes.length > 0}<span class="led" aria-hidden="true"></span>{/if}
              </label>
            </li>
          {/each}
        </ul>
      {/if}
      <button class="ghost panic" data-testid="panic-dialog" onclick={() => dispatcher.run(allNotesOff())}>{t('panic')}</button>
    </section>
  {/if}
</dialog>

<style>
  .engine-dialog {
    width: min(560px, calc(100vw - 2 * var(--fk-space-4)));
    max-height: calc(100dvh - 2 * var(--fk-space-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
    overflow: auto;
  }
  .engine-dialog::backdrop {
    background: rgb(0 0 0 / 0.5);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--fk-space-4) var(--fk-space-5) var(--fk-space-2);
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  section {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--fk-space-3);
    padding: var(--fk-space-4) var(--fk-space-5);
    border-top: 1px solid var(--fk-line);
  }
  section:first-of-type {
    border-top: 0;
  }
  h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--fk-text-muted);
  }
  p {
    margin: 0;
  }
  .muted {
    color: var(--fk-text-muted);
  }
  input.offset {
    width: 56px;
    height: 24px;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    color: var(--fk-text);
    text-align: right;
    padding: 0 var(--fk-space-1);
    font-variant-numeric: tabular-nums;
  }
  .unit {
    margin-left: 4px;
    color: var(--fk-text-muted);
  }
  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: var(--fk-space-2) var(--fk-space-5);
    align-items: center;
    margin: 0;
    width: 100%;
  }
  dt {
    color: var(--fk-text-muted);
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .meter-box :global(.meter) {
    --len: 220px;
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
  .close {
    display: grid;
    place-items: center;
    width: var(--fk-control);
    padding: 0;
    border-color: transparent;
    color: var(--fk-text-muted);
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
</style>
