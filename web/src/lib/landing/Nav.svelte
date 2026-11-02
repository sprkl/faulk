<script lang="ts">
  import { resolve } from '$app/paths'
  import Monogram from '../components/Monogram.svelte'
  import type { Lang } from '../i18n/lang'
  import LangSwitch from './LangSwitch.svelte'
  import type { Text } from './types'

  /** `page`: the page showing the bar; on the Journey page the section links go back to the landing page. */
  let { t, lang, page = 'home', langHref: pageLangHref }: { t: Text; lang: Lang; page?: 'home' | 'journey'; langHref?: (l: Lang) => string } = $props()

  const home = $derived(resolve('/[lang=lang]', { lang }))
  const journey = $derived(resolve('/[lang=lang]/journey', { lang }))
  const section = (id: string) => (page === 'home' ? `#${id}` : `${home}#${id}`)
  /** The language switch leads to this page in the other language: `langHref`, else the landing page. */
  const langHref = (l: Lang) => pageLangHref?.(l) ?? resolve('/[lang=lang]', { lang: l })
  const SECTIONS = [
    ['modes', 'nav.modes'],
    ['boost', 'nav.boost'],
    ['modules', 'nav.modules'],
  ] as const
</script>

<header class="lp-wrap nav">
  <a class="brand" href={home} aria-label={t('nav.home')}>
    <Monogram size={24} />
    <span class="lp-display word" aria-hidden="true">FAULK</span>
  </a>
  <nav aria-label={t('nav.label')}>
    {#each SECTIONS as [id, label] (id)}
      <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a fragment, or the resolved landing page plus a fragment -->
      <a href={section(id)}>{t(label)}</a>
    {/each}
    <a href={journey} aria-current={page === 'journey' ? 'page' : undefined}>{t('nav.journey')}</a>
  </nav>
  <LangSwitch {t} {lang} href={langHref} />
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
  nav a[aria-current='page'] {
    text-decoration: underline;
    text-decoration-thickness: 2px;
    text-underline-offset: 6px;
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
