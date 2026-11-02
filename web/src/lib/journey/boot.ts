/**
 * The Journey page ships no SvelteKit client (`csr = false`). This inline module script loads the
 * players (`live/index.ts`) once the document is parsed. The page is complete without it: every
 * entry keeps the browser's own audio controls, and a failed load changes nothing.
 */
export function journeyBootScript(moduleUrl: string): string {
  const url = JSON.stringify(moduleUrl).replace(/</g, '\\u003c')
  return `import(${url}).then(function(m){m.mount()}).catch(function(){})`
}
