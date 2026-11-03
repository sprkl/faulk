// The cap on the size of `src/lib/generated/schema.json` (#559), checked by `gen-protocol.mjs`
// before it writes the file. Kept apart so its rule is unit tested (`schema-size.test.ts`).
//
// The file reached 55 MB in November 2026 because every MCP tool of the catalog carried every
// definition of the contract (its size grew with the square of the number of commands); trimmed,
// it was 1.4 MB for 136 commands, about 8 KB more per command. 5 MB leaves room for hundreds of
// commands and stops the quadratic growth at once. GitHub warns above 50 MB and refuses 100 MB.

/** The largest `schema.json` the generator writes, in bytes. */
export const SCHEMA_MAX_BYTES = 5 * 1024 * 1024

/**
 * The size of each top-level section of the bundle, as written (two-space indentation), largest first.
 * @param {Record<string, unknown>} bundle
 * @returns {[string, number][]}
 */
export const sectionSizes = (bundle) =>
  Object.entries(bundle)
    .map(([key, value]) => /** @type {[string, number]} */ ([key, Buffer.byteLength(JSON.stringify(value, null, 2) ?? '')]))
    .sort((a, b) => b[1] - a[1])

/**
 * Throws when `text`, the content of `schema.json`, is larger than `max` bytes, naming the size of each section.
 * @param {string} text
 * @param {Record<string, unknown>} bundle
 * @param {number} [max]
 */
export function checkSchemaSize(text, bundle, max = SCHEMA_MAX_BYTES) {
  const size = Buffer.byteLength(text)
  if (size <= max) return size
  const mb = (/** @type {number} */ n) => `${(n / 1024 / 1024).toFixed(2)} MB`
  const sections = sectionSizes(bundle)
    .map(([key, bytes]) => `  ${key}: ${mb(bytes)}`)
    .join('\n')
  throw new Error(
    `schema.json would be ${mb(size)}, above its cap of ${mb(max)} (#559). Look for definitions copied into every command or tool instead of referenced. Size of each section:\n${sections}`,
  )
}
