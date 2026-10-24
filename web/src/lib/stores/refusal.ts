import type { CommandError } from '../generated/protocol'
import type { MessageKey } from '../i18n'

/** What the user is told about a command the core refused: a text key, its variables, and how many times in a row. */
export interface Refusal {
  key: MessageKey
  vars: Record<string, string | number>
  /** Name of what a limit counts, as a key; the view resolves it in the user's language. */
  whatKey?: MessageKey
  count: number
}

/** The entities a `limit` refusal names (`crates/commands`), by the English name the core sends. */
const ENTITY_KEYS: Record<string, MessageKey> = {
  tracks: 'refusal.entity.tracks',
  scenes: 'refusal.entity.scenes',
  clips: 'refusal.entity.clips',
  notes: 'refusal.entity.notes',
  'notes per clip': 'refusal.entity.notesPerClip',
  devices: 'refusal.entity.devices',
  'automation points': 'refusal.entity.automationPoints',
  placements: 'refusal.entity.placements',
  'placements per track': 'refusal.entity.placementsPerTrack',
  'clips per track': 'refusal.entity.clipsPerTrack',
  lanes: 'refusal.entity.lanes',
  'export seconds': 'refusal.entity.exportSeconds',
}

/** The text key of a refusal code: one text per code, never one per call site. */
const CODE_KEYS = {
  not_found: 'refusal.not_found',
  invalid: 'refusal.invalid',
  limit: 'refusal.limit',
  unsupported: 'refusal.unsupported',
  folder: 'refusal.folder',
} as const satisfies Record<CommandError['code'], MessageKey>

/** The notice for a refusal of the core. The developer message of the error stays in the console. */
export function refusalOf(error: CommandError): Omit<Refusal, 'count'> {
  // A code a newer core added: the generic text, never a blank toast.
  const key = (CODE_KEYS as Record<string, MessageKey>)[error.code] ?? CODE_KEYS.invalid
  if (error.code !== 'limit') return { key, vars: {} }
  return { key, vars: { max: error.max }, whatKey: ENTITY_KEYS[error.entity] ?? 'refusal.entity.other' }
}
