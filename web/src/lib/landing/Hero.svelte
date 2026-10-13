<script lang="ts">
  import { resolve } from '$app/paths'
  import type { Text } from './types'

  let { t }: { t: Text } = $props()

  /** A decorative waveform: a decaying sum of partials, drawn once at build time. */
  const wave = (() => {
    const points: string[] = []
    for (let i = 0; i <= 240; i++) {
      const x = i / 240
      const env = Math.exp(-2.2 * x) * (1 - Math.exp(-40 * x))
      const y = env * (Math.sin(x * 80) * 0.7 + Math.sin(x * 211) * 0.25 + Math.sin(x * 23) * 0.3)
      points.push(`${(x * 800).toFixed(1)},${(90 - y * 70).toFixed(1)}`)
    }
    return points.join(' ')
  })()
</script>

<section class="lp-wrap hero" aria-labelledby="hero-title">
  <div class="lp-meta lp-mono lp-enter">
    <span>{t('hero.metaLeft')}</span><span>{t('hero.metaRight')}</span>
  </div>
  <h1 id="hero-title" class="lp-display lp-enter" style="--lp-delay: 80ms">{t('hero.title')}</h1>
  <div class="intro">
    <div class="copy lp-enter" style="--lp-delay: 200ms">
      <p class="lead">{t('hero.lead')}</p>
      <p class="status">{t('hero.status')}</p>
    </div>
    <div class="ctas lp-enter" style="--lp-delay: 300ms">
      <a class="lp-pill lp-pill-accent" href={resolve('/app')} data-sveltekit-reload>{t('hero.cta')}</a>
      <a class="lp-pill lp-pill-outline" href="#boost">{t('hero.secondary')}</a>
    </div>
  </div>
  <div class="stage lp-grain lp-grain-live lp-enter" style="--lp-delay: 420ms">
    <div class="glow" aria-hidden="true"></div>
    <figure class="screen">
      <svg viewBox="0 0 800 180" preserveAspectRatio="none" aria-hidden="true">
        <line x1="0" y1="90" x2="800" y2="90" />
        <polyline points={wave} />
      </svg>
      <figcaption class="lp-mono">{t('hero.preview')}</figcaption>
    </figure>
  </div>
</section>

<style>
  .hero {
    display: flex;
    flex-direction: column;
    gap: 32px;
    padding-top: clamp(24px, 4vw, 56px);
  }
  h1 {
    font-size: clamp(44px, 11.4vw, 176px);
    line-height: 0.86;
    letter-spacing: -0.035em;
    text-wrap: balance;
  }
  .intro {
    display: flex;
    flex-wrap: wrap;
    gap: 24px;
    align-items: flex-end;
    justify-content: space-between;
  }
  .copy {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 620px;
  }
  .lead {
    margin: 0;
    font-size: clamp(19px, 1.8vw, 24px);
    font-weight: 300;
    line-height: 1.35;
    text-wrap: pretty;
  }
  .status {
    margin: 0;
    font-size: 15px;
    color: var(--fk-text-muted);
  }
  .ctas {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }
  .stage {
    height: clamp(360px, 46vw, 640px);
    margin-top: 16px;
    border-radius: 28px;
    background: var(--fk-ink);
  }
  .glow {
    position: absolute;
    inset: -10%;
    filter: blur(60px);
    background:
      radial-gradient(40% 45% at 28% 60%, #00c1d5 0%, rgb(0 193 213 / 0) 70%),
      radial-gradient(42% 48% at 55% 46%, #b060f0 0%, rgb(176 96 240 / 0) 72%),
      radial-gradient(28% 32% at 72% 30%, #ff7a3d 0%, rgb(255 122 61 / 0) 75%);
  }
  /* The colour foci drift slowly, like a sound that keeps sounding. */
  @keyframes drift {
    0% {
      transform: translate(0, 0) rotate(0deg) scale(1);
    }
    50% {
      transform: translate(4%, -3%) rotate(6deg) scale(1.06);
    }
    100% {
      transform: translate(-3%, 2%) rotate(-4deg) scale(1.02);
    }
  }
  /* The waveform swells and settles, slowly. */
  @keyframes breathe {
    0%,
    100% {
      transform: scaleY(1);
      opacity: 0.85;
    }
    50% {
      transform: scaleY(1.18);
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: no-preference) {
    .glow {
      will-change: transform;
      animation: drift 18s ease-in-out infinite alternate;
    }
    polyline {
      transform-box: fill-box;
      transform-origin: 50% 50%;
      animation: breathe 4.8s ease-in-out infinite;
    }
  }
  .screen {
    position: absolute;
    z-index: 4;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -46%);
    width: min(86%, 860px);
    aspect-ratio: 16 / 9;
    margin: 0;
    padding: 24px;
    border-radius: 14px;
    background: rgb(19 18 17 / 0.82);
    border: 1px solid rgb(236 233 230 / 0.18);
    box-shadow: 0 40px 80px rgb(0 0 0 / 0.45);
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 16px;
  }
  svg {
    width: 100%;
    height: 50%;
    overflow: visible;
  }
  line {
    stroke: var(--fk-ink-line);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  polyline {
    fill: none;
    stroke: var(--fk-ink-text);
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }
  figcaption {
    /* Lighter than the usual muted ink: the caption sits on a translucent panel over the glow. */
    color: var(--fk-ink-muted-2);
    text-align: center;
    font-size: 12px;
  }
  @media (max-width: 560px) {
    .stage {
      height: 300px;
    }
    .screen {
      width: 90%;
      padding: 16px;
    }
  }
</style>
