<script lang="ts">
  // Transport controls: placeholders, disabled, until the transport epic (TR-4) builds them here.
  import { useStores } from '../../lib/stores'

  const { ui } = useStores()
  const t = ui.t
</script>

<div class="transport" role="group" aria-label={t('transport.label')} title={t('layout.soon')}>
  <button class="tool" disabled aria-label={t('transport.play')}>
    <svg width="13" height="13" viewBox="0 0 14 14" aria-hidden="true"><path d="M3 1.5 L12 7 L3 12.5 Z" fill="var(--fk-play)" /></svg>
  </button>
  <button class="tool" disabled aria-label={t('transport.stop')}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect width="10" height="10" rx="1" fill="currentColor" /></svg>
  </button>
  <button class="tool" disabled aria-label={t('transport.record')}>
    <svg width="11" height="11" viewBox="0 0 12 12" aria-hidden="true"><circle cx="6" cy="6" r="5.5" fill="var(--fk-rec)" /></svg>
  </button>
  <button class="tool" disabled aria-label={t('transport.loop')}>
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><path d="M3 6 V5 a2 2 0 0 1 2-2 h7 M10 1 l2 2 -2 2 M13 10 v1 a2 2 0 0 1-2 2 H4 M6 15 l-2-2 2-2" /></svg>
  </button>
  <dl class="readouts">
    <div><dt class="sr-only">{t('transport.position')}</dt><dd class="value">---.-.-</dd></div>
    <div><dt class="sr-only">{t('transport.tempo')}</dt><dd class="value">---.--</dd></div>
    <div><dt class="sr-only">{t('transport.signature')}</dt><dd class="value">-/-</dd></div>
  </dl>
</div>

<style>
  .transport {
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
  }
  .tool {
    width: 34px;
    height: 34px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: var(--fk-radius-panel);
    background: var(--fk-surface-2);
    color: var(--fk-text);
  }
  .tool:disabled {
    opacity: 0.45;
  }
  .readouts {
    display: flex;
    gap: var(--fk-space-4);
    margin: 0 0 0 var(--fk-space-3);
    color: var(--fk-text-muted);
  }
  .readouts dd {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
