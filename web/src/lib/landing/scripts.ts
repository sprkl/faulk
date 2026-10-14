/**
 * The landing page ships no client-side JavaScript bundle (`csr = false`); these inline scripts
 * are its only code, besides the animation module they load lazily. The language scripts are
 * built from `defaultLang`'s own source, so the rule the DAW uses and the rule the landing page
 * uses are one function.
 */

import { defaultLang, LANG_STORAGE_KEY, type Lang } from '../i18n/lang'

/** JSON for a `<script>` body: `<` is escaped so a value can never close the tag. */
function literal(value: string): string {
  return JSON.stringify(value).replace(/</g, '\\u003c')
}

/**
 * For `/` only: moves the reader to the French page when the stored choice, or else the
 * browser's language, is French. `/en` and `/fr` never redirect, so no loop is possible.
 */
export function detectScript(frenchHref: string): string {
  return (
    `(function(){var s=null;try{s=localStorage.getItem(${literal(LANG_STORAGE_KEY)})}catch(e){}` +
    `var pick=${defaultLang.toString()};` +
    `if(pick(s,navigator.language)==='fr')location.replace(${literal(frenchHref)})})()`
  )
}

/**
 * Stores the language of a clicked (or middle-clicked) `a[data-lang]` link, shared with the DAW. With `app`
 * (on `/en` and `/fr`, never on `/`, whose language is detected), a click on a link to the
 * DAW (`href` exactly `app.href`) also stores the page's language, so the DAW opens in the
 * language the reader chose by opening that page. The stored choice then also steers `/`
 * (`detectScript`), like a choice made with the EN / FR switch.
 */
export function rememberScript(app?: { href: string; lang: Lang }): string {
  const key = literal(LANG_STORAGE_KEY)
  const store = (value: string) => `try{localStorage.setItem(${key},${value})}catch(_){}`
  return (
    `(function(){function f(e){var t=e.target;if(!t||!t.closest)return;` +
    `var a=t.closest('a[data-lang]');if(a){${store("a.getAttribute('data-lang')")}return}` +
    (app
      ? `a=t.closest('a[href]');if(a&&a.getAttribute('href')===${literal(app.href)}){${store(literal(app.lang))}}`
      : '') +
    // A middle click opens the link in a new tab without a `click` event.
    `}document.addEventListener('click',f);document.addEventListener('auxclick',f)})()`
  )
}

/**
 * Loads the animation module (`anim/index.ts`) once a sound object (`[data-fk-anim]`) comes within
 * 400 px of the viewport, and only where it can run: WebGL2, no data-saver. The module is a
 * progressive enhancement: the page is complete without it, and a failed load changes nothing.
 * Returned as the body of a `<script type="module">`, which runs after the document is parsed.
 */
export function animBootScript(moduleUrl: string): string {
  return (
    `(function(){if(!window.WebGL2RenderingContext||!window.IntersectionObserver||` +
    `(navigator.connection&&navigator.connection.saveData))return;` +
    `var els=document.querySelectorAll('[data-fk-anim]');if(!els.length)return;` +
    `var io=new IntersectionObserver(function(es){if(!es.some(function(e){return e.isIntersecting}))return;` +
    `io.disconnect();import(${literal(moduleUrl)}).then(function(m){m.mount()}).catch(function(){})},` +
    `{rootMargin:'400px'});els.forEach(function(el){io.observe(el)})})()`
  )
}

/** A `<script>` element with the given body, for `{@html}` in `<svelte:head>`. */
export function inlineScript(body: string, type?: 'module'): string {
  return `<script${type ? ` type="${type}"` : ''}>${body}</` + `script>`
}
