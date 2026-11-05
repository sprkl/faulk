<script lang="ts">
  /**
   * The euclidean rhythm generator's fields (#694), a strip under the piano roll's toolbar. Not a modal dialog: the grid
   * below stays in sight, because it shows the notes the numbers give while they change. The roll holds the values and
   * turns them into notes; this strip only edits them. Enter places the notes, Escape closes it with nothing placed, from
   * any field; Tab goes through the fields in reading order. The focus is given to the first field when it opens and goes
   * back to what opened it when it closes (the roll does that).
   */
  import { MAX_VELOCITY, MIN_VELOCITY, QUANTIZE_GRIDS } from './roll'
  import { MAX_GATE, MAX_STEPS, MIN_GATE, MIN_STEPS, type EuclideanDraft, type EuclideanFault } from './euclidean'
  import { useStores } from '../../lib/stores'

  let {
    draft = $bindable(),
    sounds,
    fault,
    summary,
    onsubmit,
    oncancel,
  }: {
    draft: EuclideanDraft
    /** The sounds of a drum kit by key; null for a pitched instrument, where the pitch is typed. */
    sounds: readonly { key: number; name: string }[] | null
    fault: EuclideanFault | null
    summary: string
    onsubmit: () => void
    oncancel: () => void
  } = $props()

  const { ui } = useStores()
  const t = ui.t

  let first = $state<HTMLInputElement | null>(null)
  /** Gives the first field the focus; the roll calls it once the strip is on screen. */
  export function focusFirst() {
    first?.focus()
    first?.select()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.ctrlKey || event.metaKey || event.altKey) return
    // The roll's and the page's keys must not see what is typed here (a letter on a tick box would play a note).
    event.stopPropagation()
    if (event.key === 'Escape') {
      event.preventDefault()
      oncancel()
    } else if (event.key === 'Enter' && !(event.target instanceof HTMLButtonElement)) {
      event.preventDefault()
      onsubmit()
    }
  }
</script>

<!-- The keys are those of the fields inside (Enter places, Escape closes); the group itself takes none. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<form
  class="euclid"
  role="group"
  aria-label={t('pianoRoll.euclideanTitle')}
  data-testid="roll-euclid"
  onkeydown={onKeyDown}
  onsubmit={(event) => {
    event.preventDefault()
    onsubmit()
  }}
>
  <label class="field">
    <span>{t('pianoRoll.euclideanSteps')}</span>
    <input
      bind:this={first}
      type="number"
      min={MIN_STEPS}
      max={MAX_STEPS}
      step="1"
      bind:value={draft.steps}
      aria-invalid={fault === 'steps' ? 'true' : undefined}
      data-testid="roll-euclid-steps"
    />
  </label>
  <label class="field">
    <span>{t('pianoRoll.euclideanHits')}</span>
    <input
      type="number"
      min="1"
      max={draft.steps ?? MAX_STEPS}
      step="1"
      bind:value={draft.hits}
      aria-invalid={fault === 'hits' ? 'true' : undefined}
      data-testid="roll-euclid-hits"
    />
  </label>
  <label class="field">
    <span>{t('pianoRoll.euclideanRotation')}</span>
    <input
      type="number"
      min="0"
      max={Math.max(0, (draft.steps ?? 1) - 1)}
      step="1"
      bind:value={draft.rotation}
      aria-invalid={fault === 'rotation' ? 'true' : undefined}
      data-testid="roll-euclid-rotation"
    />
  </label>
  <label class="field">
    <span>{t('pianoRoll.euclideanGrid')}</span>
    <select bind:value={draft.grid} data-testid="roll-euclid-grid">
      {#each QUANTIZE_GRIDS as grid (grid.id)}
        <option value={grid.id}>{t(`pianoRoll.qgrid.${grid.id}`)}</option>
      {/each}
    </select>
  </label>
  <label class="field">
    <span>{sounds ? t('pianoRoll.euclideanSound') : t('pianoRoll.euclideanPitch')}</span>
    {#if sounds}
      <select bind:value={draft.pitch} data-testid="roll-euclid-pitch" aria-invalid={fault === 'pitch' ? 'true' : undefined}>
        {#each sounds as sound (sound.key)}
          <option value={String(sound.key)}>{sound.name}</option>
        {/each}
      </select>
    {:else}
      <input
        type="text"
        class="pitch"
        autocomplete="off"
        spellcheck="false"
        bind:value={draft.pitch}
        aria-invalid={fault === 'pitch' ? 'true' : undefined}
        data-testid="roll-euclid-pitch"
      />
    {/if}
  </label>
  <label class="field">
    <span>{t('pianoRoll.euclideanVelocity')}</span>
    <input
      type="number"
      min={MIN_VELOCITY}
      max={MAX_VELOCITY}
      step="1"
      bind:value={draft.velocity}
      aria-invalid={fault === 'velocity' ? 'true' : undefined}
      data-testid="roll-euclid-velocity"
    />
  </label>
  <label class="field">
    <span>{t('pianoRoll.euclideanGate')}</span>
    <input
      type="number"
      min={MIN_GATE}
      max={MAX_GATE}
      step="1"
      bind:value={draft.gate}
      aria-invalid={fault === 'gate' ? 'true' : undefined}
      data-testid="roll-euclid-gate"
    />
  </label>
  <label class="check"><input type="checkbox" bind:checked={draft.fill} data-testid="roll-euclid-fill" /><span>{t('pianoRoll.euclideanFill')}</span></label>
  <label class="check"><input type="checkbox" bind:checked={draft.keep} data-testid="roll-euclid-keep" /><span>{t('pianoRoll.euclideanKeep')}</span></label>
  <button type="submit" class="primary" disabled={fault !== null} data-testid="roll-euclid-submit">{t('pianoRoll.euclideanPlace')}</button>
  <button type="button" data-testid="roll-euclid-cancel" onclick={oncancel}>{t('pianoRoll.euclideanCancel')}</button>
  <span class={fault ? 'summary error' : 'summary'} role="status" data-testid="roll-euclid-summary"
    >{fault ? t(`pianoRoll.euclideanFault.${fault}`) : summary}</span
  >
</form>

<style>
  .euclid {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--fk-space-2) var(--fk-space-3);
    padding: var(--fk-space-1) var(--fk-space-3);
    border-bottom: 1px solid var(--fk-line);
    background: var(--fk-surface);
    font-size: 12px;
  }
  .field,
  .check {
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
  }
  input[type='number'] {
    width: 4.5em;
  }
  .pitch {
    width: 5em;
    font-family: var(--fk-font-mono);
  }
  input[aria-invalid='true'],
  select[aria-invalid='true'] {
    outline: 1px solid var(--fk-danger);
  }
  .summary {
    color: var(--fk-text-muted);
  }
  .error {
    color: var(--fk-danger);
  }
</style>
