// The runtime catalogue of the commands the page lists (the command palette, #108): one compact entry per command of the
// Rust catalogue (`bundle.command.catalog`), with what the palette needs to find it (name, summary) and to ask for its
// input (the fields, each reduced to a kind a text field can take). Pure: `gen-protocol.mjs` writes its result into
// `protocol.ts`, so the page never reads `schema.json` (a guard test refuses that import).

/** @typedef {Record<string, any>} Schema */

/** The longest summary kept, in characters: the palette shows one line. */
export const SUMMARY_MAX = 120

/**
 * The first sentence of a command's description, on one line: up to the first full stop followed by a space or the end
 * of the text, or up to the first blank line, whichever comes first.
 * @param {string} description
 * @returns {string}
 */
export function summaryOf(description) {
  const paragraph = description
    .trim()
    .split(/\n\s*\n/)[0]
    .replace(/\s+/g, ' ')
  // A full stop that ends a sentence: followed by a space and a capital letter or the end, not "e.g." nor "0.5".
  const stop = paragraph.search(/[.!?](?=\s+[A-Z`]|$)/)
  const sentence = stop === -1 ? paragraph : paragraph.slice(0, stop + 1)
  return sentence.length > SUMMARY_MAX ? sentence.slice(0, SUMMARY_MAX - 1).trimEnd() + '…' : sentence
}

/**
 * Follows `$ref`s into the command's own `$defs`, keeping the name of the first definition met.
 * @param {Schema} schema
 * @param {Schema} defs
 * @returns {{ name: string | null, node: Schema }}
 */
function resolve(schema, defs) {
  /** @type {string | null} */
  let name = null
  /** @type {Schema} */
  let node = schema
  for (let guard = 0; guard < 8 && node && node.$ref; guard++) {
    const target = node.$ref.split('/').pop()
    name ??= target
    node = defs[target]
  }
  return { name, node: node ?? {} }
}

/**
 * The string values of a schema that is a closed list of constants (`oneOf` or `enum`), else `null`.
 * @param {Schema} node
 * @returns {string[] | null}
 */
function optionsOf(node) {
  if (Array.isArray(node.enum) && node.enum.every((/** @type {unknown} */ v) => typeof v === 'string')) return node.enum
  const list = node.oneOf ?? node.anyOf
  if (Array.isArray(list) && list.length > 0 && list.every((/** @type {Schema} */ v) => typeof v.const === 'string'))
    return list.map((/** @type {Schema} */ v) => v.const)
  return null
}

/**
 * One field of a command's input as the palette asks for it.
 * - `string`, `integer`, `number`, `boolean`: typed as is (a number outside the integers or the range is the core's to refuse);
 * - `enum`: one of `options`;
 * - `json`: anything else (lists, objects, snapshots), typed as JSON.
 * `ref` is the name of the definition an identifier or a value belongs to (`TrackId`, `Tick`), which lets the page pre-fill
 * the field; `nullable` says `null` is an answer (an empty field then means `null`).
 * @param {string} name
 * @param {Schema} schema
 * @param {boolean} required
 * @param {Schema} defs
 * @returns {Schema}
 */
export function fieldOf(name, schema, required, defs) {
  const { name: ref, node } = resolve(schema, defs)
  /** @type {Schema} */
  const field = { name, required }
  if (ref) field.ref = ref
  let type = node.type
  if (Array.isArray(type)) {
    if (type.includes('null')) field.nullable = true
    type = type.filter((/** @type {string} */ t) => t !== 'null')
    type = type.length === 1 ? type[0] : undefined
  }
  const list = node.anyOf ?? node.oneOf
  if (type === undefined && Array.isArray(list) && list.some((/** @type {Schema} */ v) => v.type === 'null')) {
    // `Option<T>`: an `anyOf` of `T` and `null`.
    const rest = list.filter((/** @type {Schema} */ v) => v.type !== 'null')
    if (rest.length === 1) return { ...fieldOf(name, rest[0], required, defs), nullable: true }
  }
  const options = optionsOf(node)
  if (options) return { ...field, kind: 'enum', options }
  if (type === 'string' || type === 'integer' || type === 'number' || type === 'boolean') return { ...field, kind: type }
  return { ...field, kind: 'json' }
}

/**
 * The palette's entry of one command of the catalogue.
 * @param {Schema} command
 * @returns {Schema}
 */
export function entryOf(command) {
  const schema = command.input_schema
  const defs = schema.$defs ?? {}
  const required = new Set(schema.required ?? [])
  const fields = Object.entries(schema.properties ?? {}).map(([name, property]) => fieldOf(name, property, required.has(name), defs))
  // Required fields first, then the optional ones, each group in the schema's order (the properties are sorted by name).
  fields.sort((a, b) => Number(b.required) - Number(a.required))
  return {
    name: command.name,
    kind: command.kind,
    summary: summaryOf(command.description),
    ...(command.destructive ? { destructive: true } : {}),
    fields,
  }
}

/**
 * Every command of the catalogue except the queries (they answer, they change nothing: an agent's, not a user's), in the catalogue's order.
 * @param {Schema[]} catalog
 * @returns {Schema[]}
 */
export function commandCatalog(catalog) {
  return catalog.filter((command) => command.kind !== 'query').map(entryOf)
}
