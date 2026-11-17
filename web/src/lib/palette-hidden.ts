/**
 * The commands the palette does not list by default (#926): they have no meaning for a musician, who never asks for them by
 * name. They stay in the catalogue (an agent calls them, the history runs them) and a `>` typed first in the search shows the
 * whole catalogue, these included. The list is written, one group per reason, and a test holds it against the catalogue (no
 * name that does not exist, none left out of a group by mistake).
 */

/** The inverses the history applies to undo an edit: they carry the snapshot of what was removed, never typed by hand. */
const HISTORY_INVERSES = [
  'track.restore',
  'track.restore_order',
  'track.restore_instrument',
  'device.restore',
  'device.restore_order',
  'preset.restore',
  'preset.restore_device',
  'clip.restore',
  'clip.restore_audio',
  'scene.restore',
  'scene.restore_order',
  'arrangement.restore',
  'arrangement.restore_time',
  'arrangement.marker.restore',
  'automation.lane.restore',
  'automation.points.restore',
  'record.restore',
] as const

/** The grouping of a history entry (a fader drag is one entry): the page opens and closes it around a gesture. */
const HISTORY_GROUPS = ['history.begin_group', 'history.end_group', 'history.abort_group'] as const

/** What the host does for the page (a folder it staged, a banner's button, a prompt it raises, an import or export it runs). */
const HOST_DRIVEN = [
  'project.open',
  'project.keep_mine',
  'project.load_folder_version',
  'project.allow_access',
  'project.choose_helper_folder',
  'project.cancel_export_pack',
  'project.cancel_import_pack',
  'project.open_recent',
  'project.open_browser_project',
  'project.delete_browser_project',
  'clip.cancel_audio_import',
  'clip.import_audio_file',
  'record.discard_take',
  'midi.select_input',
] as const

/** Commands whose input is structured data or an internal reference (lists of notes, points, takes; a media hash; a preset reference). */
const AGENT_INPUT = [
  'clip.set_content',
  'clip.notes.add',
  'clip.notes.remove',
  'clip.notes.update',
  'clip.import_audio',
  'arrangement.move_to_track',
  'automation.points.add',
  'automation.points.remove',
  'automation.points.update',
  'record.commit_take',
  'preset.load',
  'preset.delete',
  'preset.rename',
  'preset.copy',
  'view.select_track',
] as const

/** Live input that a played key or a MIDI keyboard sends: sounding a note by name or a controller by number is not a thing to look for. */
const LIVE_INPUT = ['live.note_on', 'live.note_off', 'live.input_note_on', 'live.input_note_off', 'live.control_change'] as const

/** Every hidden command, by group. */
export const HIDDEN_GROUPS = {
  historyInverses: HISTORY_INVERSES,
  historyGroups: HISTORY_GROUPS,
  hostDriven: HOST_DRIVEN,
  agentInput: AGENT_INPUT,
  liveInput: LIVE_INPUT,
} as const

/** The names of the hidden commands. */
export const HIDDEN_COMMANDS: ReadonlySet<string> = new Set(Object.values(HIDDEN_GROUPS).flat())
