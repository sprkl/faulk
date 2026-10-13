<script lang="ts">
  import type { LandingKey } from '../i18n/landing'
  import type { Text } from './types'

  let { t }: { t: Text } = $props()

  const PARAMS = ['pitch', 'decay', 'click', 'drive'] as const
</script>

<section class="atelier" aria-labelledby="atelier-title">
  <div class="lp-wrap lp-section inner">
    <div class="copy">
      <span class="lp-mono lp-reveal">{t('atelier.eyebrow')}</span>
      <h2 id="atelier-title" class="lp-display lp-reveal">
        {t('atelier.title1')}<br />{t('atelier.title2')}<br />{t('atelier.title3')}
      </h2>
      <p class="lp-reveal">{t('atelier.body')}</p>
    </div>
    <figure class="card lp-reveal">
      <figcaption class="lp-mono tag">{t('atelier.example')}</figcaption>
      <blockquote>{t('atelier.prompt')}</blockquote>
      <div class="module">
        <div class="object lp-grain" aria-hidden="true"></div>
        <div class="specs">
          <span class="lp-display name">{t('atelier.module')}</span>
          <dl>
            {#each PARAMS as p (p)}
              <div>
                <dt>{t(`atelier.${p}` as LandingKey)}</dt>
                <dd>{t(`atelier.${p}Value` as LandingKey)}</dd>
              </div>
            {/each}
          </dl>
        </div>
      </div>
    </figure>
  </div>
</section>

<style>
  .atelier {
    background: var(--fk-accent);
    color: #fff;
  }
  .inner {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(480px, 100%), 1fr));
    gap: 48px;
    align-items: center;
  }
  .copy {
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  h2 {
    font-size: clamp(44px, 6.3vw, 112px);
    line-height: 0.86;
    letter-spacing: -0.035em;
  }
  p {
    margin: 0;
    max-width: 520px;
    font-size: 19px;
    font-weight: 300;
    line-height: 1.4;
  }
  .card {
    margin: 0;
    padding: 24px;
    border-radius: 24px;
    background: var(--fk-ink);
    color: var(--fk-ink-text);
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .tag {
    font-size: 12px;
    color: var(--fk-ink-muted-2);
  }
  blockquote {
    margin: 0;
    padding: 14px 16px;
    border-radius: 12px;
    background: var(--fk-ink-surface);
    border: 1px solid var(--fk-ink-line);
    font-size: 16px;
    line-height: 1.4;
  }
  .module {
    display: flex;
    gap: 18px;
    align-items: center;
  }
  .object {
    width: 120px;
    height: 120px;
    flex-shrink: 0;
    border-radius: 50%;
    background: radial-gradient(circle at 42% 40%, #ffb27a 0%, #ff6a3d 30%, #c2185b 62%, #2a0e5c 100%);
  }
  /* The example kick thumps like the DRY KICK card: a quick swell every 1.2 s. */
  @keyframes thump {
    0%,
    100% {
      transform: scale(1);
      filter: brightness(1);
    }
    6% {
      transform: scale(1.09);
      filter: brightness(1.22);
    }
    35% {
      transform: scale(1);
      filter: brightness(1);
    }
  }
  @media (prefers-reduced-motion: no-preference) {
    .object {
      animation: thump 1.2s var(--lp-ease) infinite;
    }
  }
  .specs {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex-grow: 1;
    min-width: 0;
  }
  .name {
    font-size: 22px;
  }
  dl {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px;
    margin: 0;
    font-family: var(--fk-font-mono);
    font-size: 12px;
  }
  dl div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  dt {
    color: var(--fk-ink-muted);
  }
  dd {
    margin: 0;
  }
  @media (max-width: 480px) {
    .object {
      width: 88px;
      height: 88px;
    }
    dl {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
