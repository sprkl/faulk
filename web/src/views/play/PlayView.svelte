<script lang="ts">
  import { onMount } from 'svelte'
  import Meter from '../../lib/components/Meter.svelte'
  import PianoStrip from '../../lib/components/PianoStrip.svelte'
  import { incompatibleMessage } from '../../lib/engine/helper-client'
  import { absentHelperMessages } from '../../lib/engine/local-access'
  import { clampOctave, defaultOctave, ignoreKeyEvent, noteForKey, octaveStep } from '../../lib/keyboard'
  import { instrumentKind, isMidi, trackLabel } from '../../lib/project/tracks'
  import { useStores } from '../../lib/stores'
  import { allNotesOff, inputNoteOff, inputNoteOn } from '../../lib/synth-commands'

  const { project, engine, ui, dispatcher } = useStores()
  const t = ui.t

  let code = $state('')
  /** Notes held on the computer keyboard, by physical key, so a change of octave releases the right note. */
  // Not reactive on purpose: nothing renders from it.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const held = new Map<string, number>()

  /** The selected track, when it is a MIDI track: the live input goes to it and to the armed tracks. */
  const target = $derived(project.selected && isMidi(project.selected.track) ? project.selected : null)
  /** An armed MIDI track other than the target: the command layer plays only MIDI tracks, whatever a command armed. */
  const armedMidi = $derived(project.armed.filter((id) => project.tracks.some((entry) => entry.id === id && isMidi(entry.track))))
  const armedElsewhere = $derived(armedMidi.some((id) => id !== target?.id))
  const targetHint = $derived(
    target ? t(armedElsewhere ? 'keys.targetArmed' : 'keys.target', { name: trackLabel(target, t) }) : t(armedElsewhere ? 'keys.armedOnly' : 'keys.noTarget'),
  )
  /** The kind of the target's instrument; the octave starts where its sounds are when it changes (drums or not). */
  const targetKind = $derived(target ? instrumentKind(target.track) : null)
  /** Changes with the kind only: switching between tracks of the same kind keeps the octave the user chose. */
  let octave = $derived(defaultOctave(targetKind))

  function releaseAllKeys() {
    for (const note of held.values()) dispatcher.run(inputNoteOff(note))
    held.clear()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (!engine.started || ignoreKeyEvent(event)) return
    const step = octaveStep(event.code)
    if (step !== 0) {
      releaseAllKeys()
      octave = clampOctave(octave + step)
      return
    }
    const note = noteForKey(event.code, octave)
    if (note === null || held.has(event.code)) return
    // The command layer routes the note: the selected MIDI track and the armed ones (none: nothing plays).
    // It sends the note off where the note on went, so a track change while the key is held ends it.
    if (!target && armedMidi.length === 0) return
    held.set(event.code, note)
    dispatcher.run(inputNoteOn(note, 100))
  }

  function onKeyUp(event: KeyboardEvent) {
    const note = held.get(event.code)
    if (note === undefined) return
    held.delete(event.code)
    dispatcher.run(inputNoteOff(note))
  }

  onMount(() => {
    const release = () => releaseAllKeys()
    const onVisibility = () => document.hidden && releaseAllKeys()
    window.addEventListener('blur', release)
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      window.removeEventListener('blur', release)
      document.removeEventListener('visibilitychange', onVisibility)
      // Leaving the view while keys are held: their notes would never end.
      releaseAllKeys()
    }
  })
</script>

<svelte:window onkeydown={onKeyDown} onkeyup={onKeyUp} />

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
        <span class="value">{t('keys.octave', { n: octave })}</span>
        <span class="muted target" data-testid="keys-target">{targetHint}</span>
        <button class="ghost" onclick={() => dispatcher.run(allNotesOff())}>{t('panic')}</button>
      </div>
      <PianoStrip low={(octave + 1) * 12} active={engine.activeNotes} />
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
