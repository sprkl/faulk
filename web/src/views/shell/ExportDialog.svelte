<script lang="ts">
  /**
   * Exporting the song as a WAV file (issue #297). The project menu (or an agent's
   * `project.export_audio`) opens it with the settings to check; Export asks where the file goes,
   * then shows the render's progress with a way to cancel it; the end says what was written, how
   * loud, and whether the mix was pushed into the output limiter (#402).
   */
  import type { ExportChoices } from '../../lib/export/audio-export.svelte'
  import { LIMITER_PUSHED_DB } from '../../lib/export/audio-export.svelte'
  import { useStores } from '../../lib/stores'

  const { ui, audioExport: exporter } = useStores()
  const t = ui.t

  const FORMATS = ['pcm24', 'pcm16', 'float32'] as const
  const RATES = [
    [48_000, '48 kHz'],
    [44_100, '44.1 kHz'],
    [96_000, '96 kHz'],
  ] as const

  const percent = $derived(Math.floor(exporter.fraction * 100))
  const result = $derived(exporter.result)
  const duration = $derived.by(() => {
    if (!result) return ''
    const total = Math.round(result.seconds)
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
  })
  const megabytes = $derived(result ? (result.bytes / 1e6).toFixed(1) : '')

  let first = $state<HTMLInputElement | null>(null)
  let primary = $state<HTMLButtonElement | null>(null)
  $effect(() => {
    if (!exporter.open) return
    if (exporter.phase === 'options') first?.focus()
    else if (exporter.phase === 'done' || exporter.phase === 'failed') primary?.focus()
  })

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === 'Escape' && exporter.open) {
      event.preventDefault()
      exporter.close()
    }
  }

  const rate = (value: ExportChoices['sampleRate']) => () => (exporter.choices.sampleRate = value)
</script>

<svelte:window onkeydown={onKeyDown} />

{#if exporter.open}
  <div class="overlay">
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="export-title"
      data-testid="export-dialog"
      data-phase={exporter.phase}
      data-render-ms={exporter.result?.renderMs ?? ''}
      data-silenced-frames={exporter.result?.silencedFrames ?? ''}
    >
      <h2 id="export-title">{t('export.title')}</h2>

      {#if exporter.phase === 'options'}
        <label class="name">
          <span>{t('export.fileName')}</span>
          <input type="text" maxlength="100" bind:value={exporter.choices.fileName} data-testid="export-file-name" />
          <span class="ext">.wav</span>
        </label>
        <fieldset>
          <legend>{t('export.range')}</legend>
          <label>
            <input bind:this={first} type="radio" name="export-range" value="song" bind:group={exporter.choices.region} data-testid="export-song" />
            {t('export.range.song')}
          </label>
          <label class:off={!exporter.hasLoop}>
            <input type="radio" name="export-range" value="loop" bind:group={exporter.choices.region} disabled={!exporter.hasLoop} data-testid="export-loop" />
            {t(exporter.hasLoop ? 'export.range.loop' : 'export.range.noLoop')}
          </label>
          <div class="bars">
            <label>
              <input type="radio" name="export-range" value="bars" bind:group={exporter.choices.region} data-testid="export-bars" />
              {t('export.range.bars')}
            </label>
            <label class="number">
              {t('export.range.from')}
              <input
                type="number"
                min="1"
                step="1"
                bind:value={exporter.choices.fromBar}
                disabled={exporter.choices.region !== 'bars'}
                aria-invalid={exporter.choices.region === 'bars' && !exporter.barsValid}
                data-testid="export-from-bar"
              />
            </label>
            <label class="number">
              {t('export.range.to')}
              <input
                type="number"
                min="1"
                step="1"
                bind:value={exporter.choices.toBar}
                disabled={exporter.choices.region !== 'bars'}
                aria-invalid={exporter.choices.region === 'bars' && !exporter.barsValid}
                data-testid="export-to-bar"
              />
            </label>
          </div>
          {#if exporter.choices.region === 'bars' && !exporter.barsValid}
            <p class="warn" role="status" data-testid="export-bad-bars">{t('export.range.badBars')}</p>
          {/if}
        </fieldset>
        <fieldset>
          <legend>{t('export.format')}</legend>
          {#each FORMATS as format (format)}
            <label>
              <input type="radio" name="export-format" value={format} bind:group={exporter.choices.bitDepth} data-testid={`export-${format}`} />
              {t(`export.format.${format}`)}
            </label>
          {/each}
        </fieldset>
        <fieldset>
          <legend>{t('export.rate')}</legend>
          <div class="row">
            {#each RATES as [value, label] (value)}
              <label>
                <input
                  type="radio"
                  name="export-rate"
                  {value}
                  checked={exporter.choices.sampleRate === value}
                  onchange={rate(value)}
                  data-testid={`export-rate-${value}`}
                />
                {label}
              </label>
            {/each}
          </div>
        </fieldset>
        <p class="note">{t('export.note')}</p>
        {#if !exporter.canChoose}<p class="note" data-testid="export-download-note">{t('export.noChoice')}</p>{/if}
        {#if !exporter.hasContent}<p class="warn" role="status" data-testid="export-empty">{t('export.empty')}</p>{/if}
        <div class="actions">
          <button onclick={() => exporter.close()} data-testid="export-cancel">{t('export.cancel')}</button>
          <button class="primary" disabled={!exporter.canStart} onclick={() => exporter.start()} data-testid="export-start">{t('export.start')}</button>
        </div>
      {:else if exporter.phase === 'picking'}
        <p role="status" data-testid="export-status">{t('export.picking')}</p>
        <div class="actions">
          <button onclick={() => exporter.cancel()} data-testid="export-abort">{t('export.cancel')}</button>
        </div>
      {:else if exporter.phase === 'rendering'}
        <p role="status" data-testid="export-status">{exporter.tail ? t('export.tail') : t('export.rendering', { percent })}</p>
        <progress max="100" value={exporter.tail ? 100 : percent} aria-label={t('export.title')} data-testid="export-progress"></progress>
        <div class="actions">
          <button onclick={() => exporter.cancel()} data-testid="export-abort">{t('export.cancel')}</button>
        </div>
      {:else if exporter.phase === 'done' && result}
        <p role="status" data-testid="export-status">
          {t(result.kind === 'file' ? 'export.done' : 'export.downloaded', { name: result.name, size: megabytes, duration })}
        </p>
        <p class="note" data-testid="export-peak">
          {result.peakDb === -Infinity ? t('export.silent') : t('export.peak', { db: result.peakDb.toFixed(1) })}
        </p>
        {#if result.reductionDb >= LIMITER_PUSHED_DB}<p class="warn" role="alert" data-testid="export-limited">
            {t('export.limited', { db: result.reductionDb.toFixed(1) })}
          </p>{/if}
        {#if result.tailCut}<p class="warn" role="status" data-testid="export-tail-cut">{t('export.tailCut')}</p>{/if}
        {#if result.missing.length > 0}<p class="warn" role="alert" data-testid="export-missing">
            {t('export.missing', { names: result.missing.join(', ') })}
          </p>{/if}
        <div class="actions">
          <button bind:this={primary} class="primary" onclick={() => exporter.close()} data-testid="export-close">{t('export.close')}</button>
        </div>
      {:else}
        <p class="warn" role="alert" data-testid="export-error">{exporter.error ? t(exporter.error) : t('export.failed')}</p>
        <div class="actions">
          <button bind:this={primary} class="primary" onclick={() => exporter.close()} data-testid="export-close">{t('export.close')}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: var(--fk-scrim);
  }
  .dialog {
    width: min(460px, calc(100vw - 32px));
    max-height: calc(100dvh - 32px);
    overflow: auto;
    padding: var(--fk-space-5, 20px);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface);
  }
  h2 {
    margin: 0 0 var(--fk-space-3);
    font-size: 16px;
  }
  fieldset {
    margin: 0 0 var(--fk-space-3);
    padding: 0;
    border: 0;
  }
  legend {
    padding: 0;
    margin-bottom: var(--fk-space-1);
    font-size: 12px;
    font-weight: 600;
    color: var(--fk-text-muted);
  }
  label {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    padding: 2px 0;
    font-size: 13px;
  }
  label.off {
    color: var(--fk-text-muted);
  }
  .bars {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--fk-space-1) var(--fk-space-3);
  }
  label.name {
    margin-bottom: var(--fk-space-3);
  }
  label.name input {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding: 0 var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: var(--fk-surface-2);
    color: var(--fk-text);
    font: inherit;
  }
  .ext {
    color: var(--fk-text-muted);
  }
  .number input {
    width: 4.5rem;
    height: 24px;
    padding: 0 var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: var(--fk-surface-2);
    color: var(--fk-text);
    font: inherit;
  }
  .number input[aria-invalid='true'] {
    border-color: var(--fk-accent);
  }
  .number input:disabled {
    opacity: 0.5;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--fk-space-2) var(--fk-space-4);
  }
  p {
    margin: 0 0 var(--fk-space-3);
  }
  .note {
    font-size: 12px;
    color: var(--fk-text-muted);
  }
  .warn {
    color: var(--fk-accent);
    font-size: 13px;
  }
  progress {
    width: 100%;
    height: 8px;
    margin-bottom: var(--fk-space-3);
    accent-color: var(--fk-accent);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
  }
  button {
    height: 28px;
    padding: 0 var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text);
    font-size: 12px;
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    background: var(--fk-surface-2);
  }
  button:disabled {
    color: var(--fk-text-muted);
    cursor: default;
  }
  button.primary {
    border-color: var(--fk-accent);
    background: var(--fk-accent);
    color: var(--fk-on-accent);
    font-weight: 600;
  }
  button.primary:hover:not(:disabled) {
    background: var(--fk-accent-strong);
  }
  button.primary:disabled {
    opacity: 0.5;
  }
</style>
