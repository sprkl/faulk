<script lang="ts">
  import type { LandingKey } from '../i18n/landing'
  import type { Text } from './types'

  let { t }: { t: Text } = $props()

  /** Sketches of sound objects: no module exists yet, the cards say so. */
  const CARDS: { name: LandingKey; kind: LandingKey; art: 'kick' | 'haze' | 'crush' | 'echo' }[] = [
    { name: 'atelier.module', kind: 'library.instrument', art: 'kick' },
    { name: 'library.haze', kind: 'library.instrument', art: 'haze' },
    { name: 'library.crush', kind: 'library.effect', art: 'crush' },
    { name: 'library.echo', kind: 'library.effect', art: 'echo' },
  ]
</script>

<section id="modules" class="lp-wrap lp-section library" aria-labelledby="library-title">
  <div class="head">
    <div class="intro">
      <h2 id="library-title" class="lp-display lp-h2 title lp-reveal">{t('library.title')}</h2>
      <p class="lp-reveal">{t('library.lead')}</p>
    </div>
    <button class="lp-pill lp-pill-outline browse lp-reveal" type="button" disabled>{t('library.browse')}</button>
  </div>
  <ul class="cards">
    {#each CARDS as card (card.art)}
      <li class="card lp-reveal">
        <!-- Each sound object moves like its sound: thump, drift, stepped flicker, echoes. -->
        <div class="stage {card.art}" class:lp-grain={card.art === 'kick' || card.art === 'haze'} aria-hidden="true">
          <div class="object" class:lp-grain={card.art === 'crush' || card.art === 'echo'}></div>
          {#if card.art === 'haze'}
            <div class="object second"></div>
          {:else if card.art === 'echo'}
            <span class="ring"></span><span class="ring"></span><span class="ring"></span>
          {/if}
        </div>
        <div class="row">
          <h3 class="lp-display">{t(card.name)}</h3>
          <span class="kind">{t(card.kind)}</span>
        </div>
        <span class="sketch">{t('library.sketch')}</span>
      </li>
    {/each}
  </ul>
</section>

<style>
  .library {
    display: flex;
    flex-direction: column;
    gap: 32px;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    flex-wrap: wrap;
    gap: 16px;
  }
  .intro {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .title {
    /* "BIBLIOTHÈQUE" must fit 358 px at phone width. */
    font-size: clamp(32px, 6.6vw, 96px);
  }
  p {
    margin: 0;
    max-width: 620px;
    font-size: 17px;
    font-weight: 300;
    line-height: 1.4;
  }
  .head .browse {
    height: 44px;
    padding: 0 22px;
    font-size: 15px;
    border-color: var(--fk-text-muted);
    color: var(--fk-text-muted);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(280px, 100%), 1fr));
    gap: 16px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px;
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
  }
  .stage {
    position: relative;
    height: 240px;
    border-radius: 14px;
    background: var(--fk-ink);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .kick .object {
    width: 140px;
    height: 140px;
    border-radius: 50%;
    background: radial-gradient(circle at 42% 40%, #ffb27a 0%, #ff6a3d 30%, #c2185b 62%, #2a0e5c 100%);
  }
  .haze .object {
    position: absolute;
    inset: 20px;
    filter: blur(26px);
    background: radial-gradient(40% 45% at 40% 55%, #9c92ff 0%, rgb(156 146 255 / 0) 100%);
  }
  .haze .second {
    background: radial-gradient(35% 40% at 62% 42%, #3bb9ed 0%, rgb(59 185 237 / 0) 100%);
  }
  .crush .object {
    width: 140px;
    height: 140px;
    border-radius: 22px;
    background: conic-gradient(from 210deg at 50% 50%, #ee3533, #efcc36, #ee3533, #2a0e5c, #ee3533);
  }
  .echo .object {
    width: 170px;
    height: 170px;
    border-radius: 50%;
    background: repeating-radial-gradient(circle at 50% 50%, #00c5b7 0px, #00c5b7 5px, rgb(0 193 213 / 0.15) 13px, #131211 22px);
    mask-image: radial-gradient(circle, #000 35%, transparent 72%);
  }
  .ring {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 200px;
    height: 200px;
    margin: -100px 0 0 -100px;
    border-radius: 50%;
    border: 2px solid #00c5b7;
    opacity: 0;
    pointer-events: none;
  }

  /* DRY KICK: a short thump every 1.2 s, a quick swell whose glow decays. */
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
  /* HAZE: two blurred blobs drifting slowly against each other. */
  @keyframes drift-a {
    from {
      transform: translate(-8%, 6%) scale(1);
    }
    to {
      transform: translate(10%, -6%) scale(1.12);
    }
  }
  @keyframes drift-b {
    from {
      transform: translate(8%, -4%) scale(1.1);
    }
    to {
      transform: translate(-10%, 8%) scale(0.95);
    }
  }
  /* CRUSH 8: quantized, the object jumps between a few states instead of moving smoothly. */
  @keyframes crush {
    0% {
      transform: translate(0, 0) rotate(0deg);
      filter: hue-rotate(0deg);
    }
    25% {
      transform: translate(3px, -2px) rotate(-4deg) scale(0.97);
      filter: hue-rotate(-14deg) contrast(1.1);
    }
    50% {
      transform: translate(-2px, 2px) rotate(3deg) scale(1.02);
      filter: hue-rotate(8deg);
    }
    75% {
      transform: translate(1px, 3px) rotate(-1deg) scale(0.99);
      filter: hue-rotate(-6deg) contrast(1.05);
    }
  }
  /* ECHO: rings leave the centre, grow and fade, one after the other. */
  @keyframes ring {
    0% {
      transform: scale(0.25);
      opacity: 0.9;
    }
    60% {
      opacity: 0.55;
    }
    100% {
      transform: scale(1.15);
      opacity: 0;
    }
  }
  @keyframes pulse {
    0%,
    100% {
      transform: scale(1);
    }
    50% {
      transform: scale(0.94);
    }
  }
  @media (prefers-reduced-motion: no-preference) {
    .kick .object {
      animation: thump 1.2s var(--lp-ease) infinite;
    }
    .haze .object {
      animation: drift-a 9s ease-in-out infinite alternate;
    }
    .haze .second {
      animation: drift-b 11s ease-in-out infinite alternate;
    }
    .crush .object {
      animation: crush 1.6s steps(1, end) infinite;
    }
    .echo .object {
      animation: pulse 3.6s ease-in-out infinite;
    }
    .ring {
      animation: ring 3.6s cubic-bezier(0.2, 0.7, 0, 1) infinite;
    }
    .ring:nth-of-type(2) {
      animation-delay: 1.2s;
    }
    .ring:nth-of-type(3) {
      animation-delay: 2.4s;
    }
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
  }
  h3 {
    font-size: 20px;
  }
  .kind {
    font-family: var(--fk-font-mono);
    font-size: 12px;
    color: var(--fk-text-muted);
  }
  .sketch {
    font-size: 14px;
    color: var(--fk-text-soft);
  }
</style>
