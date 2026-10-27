// Merges the roots of the schema bundle into one set of definitions, for
// `gen-protocol.mjs`. Kept apart so its rules are unit tested (`schema-merge.test.ts`).

/** @typedef {Record<string, any>} Schema */

/**
 * The value with object keys sorted at every level: key order is not meaningful in a schema.
 * @param {unknown} v
 * @returns {unknown}
 */
const canonical = (v) =>
  Array.isArray(v)
    ? v.map(canonical)
    : v && typeof v === 'object'
      ? Object.fromEntries(
          Object.keys(v)
            .sort()
            .map((k) => [k, canonical(/** @type {Schema} */ (v)[k])]),
        )
      : v

/** @param {Schema} schema @param {string[]} keys @returns {Schema} */
const omit = (schema, keys) => Object.fromEntries(Object.entries(schema).filter(([k]) => !keys.includes(k)))

/**
 * Two copies of a definition are the same when they differ only in key order or in `title`
 * (a root carries its own, an embedded copy is named by its key).
 * @param {Schema} a
 * @param {Schema} b
 */
export const sameDefinition = (a, b) => JSON.stringify(canonical(omit(a, ['title']))) === JSON.stringify(canonical(omit(b, ['title'])))

/**
 * A domain without commands yet is `{ "not": {} }`, a type with no values; the compiler
 * would make it an open object, which breaks narrowing on `type`. It is `never` instead.
 * @param {Schema} def
 * @returns {Schema}
 */
export const neverIfEmpty = (def) => {
  const { description, not, ...rest } = def
  const empty = not && typeof not === 'object' && Object.keys(not).length === 0
  return empty && Object.keys(rest).length === 0 ? { description, tsType: 'never' } : def
}

/**
 * The roots share definitions (`TrackId`, `Author`, ...). Compiling them one by one either
 * declares a shared type twice or leaves a type only one root uses undeclared, so they are
 * compiled as one schema: each root becomes a definition titled by its name (without its
 * `$schema`), next to the union of their definitions. Throws when two roots define one
 * name differently. A root's own copy wins over a copy embedded in another root.
 * @param {Record<string, Schema>} roots
 * @returns {Record<string, Schema>}
 */
export function mergeRoots(roots) {
  /** @type {Record<string, Schema>} */
  const defs = {}
  for (const [name, root] of Object.entries(roots)) {
    /** @type {Record<string, Schema>} */
    const own = { ...root.$defs, [name]: { ...omit(root, ['$defs', '$schema']), title: name } }
    for (const [key, raw] of Object.entries(own)) {
      const def = neverIfEmpty(raw)
      if (key in defs && !sameDefinition(defs[key], def)) {
        throw new Error(`schema definition ${key} differs between roots`)
      }
      if (!(key in defs) || key === name) defs[key] = def
    }
  }
  return defs
}
