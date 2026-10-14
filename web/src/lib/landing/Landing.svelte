<script lang="ts">
  import { resolve } from '$app/paths'
  import { landingText } from '../i18n/landing'
  import type { Lang } from '../i18n/lang'
  import Atelier from './Atelier.svelte'
  import Boost from './Boost.svelte'
  import Footer from './Footer.svelte'
  import Hero from './Hero.svelte'
  import Library from './Library.svelte'
  import Modes from './Modes.svelte'
  import Nav from './Nav.svelte'
  import { animBootScript, detectScript, inlineScript, rememberScript } from './scripts'
  import { url as animUrl } from 'virtual:landing-anim'
  import './landing.css'

  /** `detect`: this is `/`, which moves French readers to `/fr`. */
  let { lang, detect = false }: { lang: Lang; detect?: boolean } = $props()

  const t = $derived(landingText(lang))
  const hrefs = {
    en: resolve('/[lang=lang]', { lang: 'en' }),
    fr: resolve('/[lang=lang]', { lang: 'fr' }),
  }
  const scripts = $derived(
    (detect ? inlineScript(detectScript(hrefs.fr)) : '') +
      inlineScript(rememberScript(detect ? undefined : { href: resolve('/app'), lang })) +
      inlineScript(animBootScript(animUrl), 'module'),
  )
</script>

<svelte:head>
  <title>{t('meta.title')}</title>
  <meta name="description" content={t('meta.description')} />
  <link rel="canonical" href={hrefs[lang]} />
  <link rel="alternate" hreflang="en" href={hrefs.en} />
  <link rel="alternate" hreflang="fr" href={hrefs.fr} />
  <link rel="alternate" hreflang="x-default" href={resolve('/')} />
  <!-- Generated from tested code (scripts.ts); the page has no other JavaScript besides the
       animation module the last script loads lazily. -->
  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
  {@html scripts}
</svelte:head>

<div class="landing">
  <a class="skip" href="#main">{t('skip')}</a>
  <Nav {t} {lang} />
  <main id="main" tabindex="-1">
    <Hero {t} />
    <Modes {t} />
    <Atelier {t} />
    <Library {t} />
    <Boost {t} />
  </main>
  <Footer {t} {lang} />
</div>
