import type { AgentResult, Command } from '../generated/protocol'

/**
 * The page's side of an agent's tool call (AG-1). The helper serves MCP and relays each call as
 * `agent.call`: the command an agent sent, with its name. The page runs it like a user's action,
 * through the dispatcher (history author `agent:<name>`, undo, refusals), and answers `agent.result`.
 */
export interface AgentHost {
  /** Runs `command` as `author` through the page's single entry point and says what it did. */
  runForAgent(command: Command, author: string): Promise<AgentResult>
}

/**
 * The commands whose page action opens a picker or a permission prompt: the browser allows those only
 * during a click in the page (spec § SV, #323), which an agent's call never is. They are answered
 * `unavailable` before anything runs, instead of applying in the project while their page action fails.
 * Not listed: `project.open_recent` and `project.reopen_last`, which prompt only when the browser took the
 * folder's permission back, and `project.save`, which asks for a folder only for a project that has none.
 */
export const NEEDS_A_CLICK: ReadonlySet<string> = new Set([
  'project.save_as',
  'project.open_folder',
  'project.allow_access',
  'project.import_pack',
  'project.export_pack',
  'project.export_audio',
])

/** The history's author of an agent: `agent:<name>`, `agent:mcp` without a name. */
export function agentAuthor(agent: string): string {
  const name = agent.trim()
  return `agent:${name === '' ? 'mcp' : name}`
}

/** Answers one agent call: runs `command` through `host` unless it is not a command or needs a click. */
export async function answerAgentCall(host: AgentHost, agent: string, command: unknown): Promise<AgentResult> {
  if (typeof command !== 'object' || command === null || typeof (command as { type?: unknown }).type !== 'string') {
    return { status: 'invalid', message: 'an agent call carries a command object with its `type`' }
  }
  const type = (command as { type: string }).type
  if (NEEDS_A_CLICK.has(type)) {
    return { status: 'unavailable', reason: `${type} opens a picker or a permission prompt: it needs a click in the page` }
  }
  try {
    // The project core checks the command against its schema: anything it does not know is `invalid`.
    return await host.runForAgent(command as Command, agentAuthor(agent))
  } catch (error) {
    console.error(error)
    return { status: 'unavailable', reason: `the page failed to run ${type}: ${String(error)}` }
  }
}
