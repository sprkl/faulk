<script lang="ts">
  import type { Text } from './types'

  let { t }: { t: Text } = $props()
</script>

<section id="boost" class="boost" aria-labelledby="boost-title">
  <div class="lp-wrap lp-section inner">
    <div class="lp-meta lp-mono meta">
      <span>{t('boost.metaLeft')}</span><span>{t('boost.metaRight')}</span>
    </div>
    <h2 id="boost-title" class="lp-display">{t('boost.title')}</h2>
    <div class="grid">
      <p>{t('boost.body')}</p>
      <div class="figure">
        <span class="value">{t('boost.figure')}</span>
        <span class="caption">{t('boost.figureCaption')}</span>
      </div>
      <div class="downloads" role="group" aria-label={t('boost.downloads')}>
        <button class="lp-pill dl" type="button" disabled>{t('boost.windows')}</button>
        <button class="lp-pill dl" type="button" disabled>{t('boost.macos')}</button>
      </div>
    </div>
  </div>
</section>

<style>
  .boost {
    background: var(--fk-ink);
    color: var(--fk-ink-text);
  }
  .inner {
    display: flex;
    flex-direction: column;
    gap: 40px;
  }
  .meta {
    color: var(--fk-ink-muted-2);
    border-top-color: var(--fk-text-soft);
  }
  h2 {
    font-size: clamp(64px, 14vw, 220px);
    line-height: 0.82;
    letter-spacing: -0.04em;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(300px, 100%), 1fr));
    gap: 32px;
    align-items: end;
  }
  p {
    margin: 0;
    font-size: 22px;
    font-weight: 300;
    line-height: 1.4;
  }
  .figure {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-family: var(--fk-font-mono);
  }
  .value {
    font-size: 56px;
    font-weight: 500;
    line-height: 1;
  }
  .caption {
    font-size: 13px;
    color: var(--fk-ink-muted);
  }
  .downloads {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }
  .dl {
    padding: 0 26px;
    background: transparent;
    border-color: var(--fk-ink-muted);
    color: var(--fk-ink-muted-2);
  }
</style>
