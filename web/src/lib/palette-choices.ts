/**
 * The closed choices of the palette's forms (#935): a field whose answer is one of a few named things is a list of those things,
 * never a text a musician has to guess (a scale's identifier, a tonic's number, a device's type id). Every list comes from the
 * source the interface already uses (the transport's scale picker, the track menu's named colours, the protocol's `DEVICES`) and
 * is named in the language of the interface; no list is copied here. Pure: the page gives `t`.
 */
import { DEVICES, type CatalogEntry, type CommandField } from './generated/protocol'
import type { MessageKey } from './i18n'
import { messages } from './i18n/app'
import { deviceText, effectDescriptors, instrumentDescriptors, labelKey, midiEffectDescriptors } from './devices'
import type { Lang } from './i18n/lang'
import { rootLabel, ROOTS, SCALE_MODES } from './scale'
import { HUE_KEYS, TRACK_COLORS } from './project/tracks'

type T = (key: MessageKey, vars?: Record<string, string | number>) => string

/** One answer of a list: the text the command is built from, and what the musician reads. */
export interface Choice {
  value: string
  label: string
}

/** The text of the "no value" choice of a field that can be `null` (a track without a colour). */
export const NONE = '__none__'

/** The text of the choice that opens a text field next to the list (a colour typed as `#rrggbb`). */
export const OTHER = '__other__'

/** What a list depends on besides the field: the language (a device's parameters are named by the device's own texts) and the device the form is about. */
export interface ChoiceContext {
  lang: Lang
  /** The kind of the device the form is about (`faulk.echo`), `null` when none is chosen yet. */
  deviceKind: string | null
  /** The kind of the selected track: a MIDI effect only goes on a MIDI track (the master and an audio track play no notes). */
  trackKind?: 'midi' | 'audio' | null
}

/** A closed list, and whether a typed value is also welcome after it. */
export interface ClosedChoices {
  choices: readonly Choice[]
  /** The list ends with "Other…": the musician then types the value (a colour in hexadecimal). */
  other?: boolean
}

const ofColors = (t: T): Choice[] => TRACK_COLORS.map((color, i) => ({ value: color, label: t(HUE_KEYS[i]!) }))

/** The lists of the fields the schema leaves as a string or a number, by `<command>.<field>`. */
const LISTS: Readonly<Record<string, (t: T, x: ChoiceContext) => ClosedChoices>> = {
  // The transport's picker: "No scale" first (the schema's `chromatic`), then its scales in its order; the twelve tonics.
  'project.set_scale.mode': (t) => ({
    choices: [{ value: 'chromatic', label: t('scale.none') }, ...SCALE_MODES.map((m) => ({ value: m, label: t(`scale.mode.${m}` as MessageKey) }))],
  }),
  'project.set_scale.root': (t) => ({ choices: ROOTS.map((r) => ({ value: String(r), label: rootLabel(r, t) })) }),
  // The inspector's sixteen named colours; the hexadecimal stays reachable behind "Other".
  'track.create.color': (t) => ({ choices: ofColors(t), other: true }),
  'track.set_color.color': (t) => ({ choices: [{ value: NONE, label: t('palette.noColor') }, ...ofColors(t)], other: true }),
  // The knobs of the device the form is about, named as its panel names them.
  'device.set_param.param': (_t, x) => {
    const d = DEVICES.find((k) => k.kind === x.deviceKind)
    if (!d) return { choices: [] }
    return { choices: d.params.map((p) => ({ value: p.key, label: deviceText(x.lang, labelKey(d, p.key)) ?? p.key })) }
  },
  // The devices the engine registers, by their one English name.
  'track.create.instrument': () => ({ choices: instrumentDescriptors().map((d) => ({ value: d.kind, label: d.name })) }),
  'track.set_instrument.instrument': () => ({ choices: instrumentDescriptors().map((d) => ({ value: d.kind, label: d.name })) }),
  'device.insert.kind': (_t, x) => ({
    choices: [...effectDescriptors(), ...(x.trackKind === 'midi' ? midiEffectDescriptors() : [])].map((d) => ({ value: d.kind, label: d.name })),
  }),
}

/** The enums the interface already names somewhere: the palette says the same words (the key of an option). */
const INTERFACE_OPTIONS: Readonly<Record<string, (option: string) => string>> = {
  Theme: (o) => `theme.${o}`,
  Quantize: (o) => `transport.quantize.${o}`,
  ExportBitDepth: (o) => `export.format.${o}`,
  ExportRegion: (o) => `export.range.${o}`,
  PresetLocation: (o) => `presets.where.${o}`,
}

/** An option of an enum in words: the interface's own, else the palette's (`palette.opt.<ref>.<option>`), else the option with spaces. */
export function optionLabel(t: T, field: CommandField, option: string): string {
  const ref = field.ref ?? field.name
  const key = INTERFACE_OPTIONS[ref]?.(option) ?? `palette.opt.${ref}.${option}`
  return key in messages.en ? t(key as MessageKey) : option.replace(/_/g, ' ')
}

/** The closed list of a field, or `null` when the answer is free (a name, a number, a web address). */
export function closedChoices(entry: CatalogEntry, field: CommandField, t: T, x: ChoiceContext = { lang: 'en', deviceKind: null }): ClosedChoices | null {
  const list = LISTS[`${entry.name}.${field.name}`]
  if (list) return list(t, x)
  if (field.kind === 'enum') return { choices: (field.options ?? []).map((o) => ({ value: o, label: optionLabel(t, field, o) })) }
  if (field.kind === 'boolean') {
    return {
      choices: [
        { value: 'true', label: t('palette.yes') },
        { value: 'false', label: t('palette.no') },
      ],
    }
  }
  return null
}

/** The fields that name a place in a list (`index`): typed from 1, empty means the end. */
export const isListIndex = (field: CommandField): boolean => field.name === 'index' && field.kind === 'integer'

/** What an empty place means: past the end, which every command that takes one reads as last. */
export const END_OF_LIST = 4294967295

/** Whether a field is asked as a list at all, for the test that refuses a closed choice left as free text. */
export const hasList = (entry: CatalogEntry, field: CommandField): boolean => `${entry.name}.${field.name}` in LISTS
