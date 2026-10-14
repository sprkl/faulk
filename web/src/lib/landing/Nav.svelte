<script lang="ts">
  import { resolve } from '$app/paths'
  import Monogram from '../components/Monogram.svelte'
  import type { Lang } from '../i18n/lang'
  import LangSwitch from './LangSwitch.svelte'
  import type { Text } from './types'

  let { t, lang }: { t: Text; lang: Lang } = $props()
</script>

<header class="lp-wrap nav">
  <a class="brand" href={resolve('/[lang=lang]', { lang })} aria-label={t('nav.home')}>
    <Monogram size={24} />
    <span class="lp-display word" aria-hidden="true">FAULK</span>
  </a>
  <nav aria-label={t('nav.label')}>
    <a href="#modes">{t('nav.modes')}</a>
    <a href="#boost">{t('nav.boost')}</a>
    <a href="#modules">{t('nav.modules')}</a>
  </nav>
  <LangSwitch {t} {lang} />
  <a class="open" href={resolve('/app')} data-sveltekit-reload>{t('nav.open')}</a>
</header>

<style>
  .nav {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 24px;
    padding-block: 20px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    text-decoration: none;
  }
  .word {
    font-size: 18px;
    letter-spacing: -0.01em;
  }
  nav {
    display: flex;
    flex-wrap: wrap;
    flex-grow: 1;
    gap: 22px;
    font-size: 14px;
    font-weight: 500;
  }
  nav a {
    text-decoration: none;
  }
  .open {
    display: inline-flex;
    align-items: center;
    height: 40px;
    padding: 0 20px;
    border-radius: var(--fk-radius-pill);
    background: var(--fk-text);
    color: var(--fk-bg) !important;
    font-size: 14px;
    font-weight: 500;
    text-decoration: none;
  }
  .open:hover {
    background: var(--fk-accent);
  }
  @media (max-width: 560px) {
    .nav {
      gap: 16px;
    }
    nav {
      order: 3;
      flex-basis: 100%;
    }
    .brand {
      margin-right: auto;
    }
  }
</style>
