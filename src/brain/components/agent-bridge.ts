/**
 * T15 AgentBridge (4.3, hidden TS brain window). Implements the 4.7 `AgentBridge`.
 *
 * Responsibilities: Main agent turn input (cleaned question, router decision, recall hits), the
 * UI tool handlers, the guardrail checks of RTR-29a..e and the fallback classifier route (B4).
 *
 * `start` and `send` (#112) go to the Rust R16 AgentHost through `agent_start` and `agent_send`.
 * The core runs one shared harness runtime with one session per MOG (U11, RTR-21a) and keeps
 * every token on its side of the secret boundary (ARC-7). A rejected `send` means the session is
 * unavailable (ARC-F14); the caller routes with the fallback classifier (RTR-15b).
 *
 * Guardrails (#141, ADR 0085):
 * - `buildAgentTurn` makes the 4.7 `AgentTurn` from the `preview` dispatch outcome. The brain
 *   sends it only after the preview gate lets it go (B1, TODO(#142)); the core then approves
 *   that one question for the agent's `genie_ask` (RTR-29b).
 * - `onTool` and `handleToolCall` take the agent's UI tool calls (`agent.tool_call`). Only
 *   `show_blurb`, `set_mood`, `propose_page` and `ask_user` reach a handler, with checked
 *   arguments. Text in the arguments is data: it goes to the handler as it is and is never read
 *   as a command (RTR-03c). The bridge has no capture, context approval, Page create or
 *   publish, Genie or profile handle, so no agent call reaches one (RTR-29a..d).
 *   `propose_page` returns the review ID only (RTR-29c). The Rust core writes the action log
 *   entry of each call (RTR-30), so the bridge writes none.
 * - A hop proposal goes to `HopChips` (`agent-hop.ts`), never to `profile_select` (RTR-29d).
 */
import type {
  AgentBridge,
  AgentTurn,
  BottleTool,
  HarnessKind,
  MogId,
} from '../../shared/interfaces';
import { call, type CommandArgs, type CommandName, type JsonObject } from '../../shared/ipc';
import type { DispatchOutcome } from './router-dispatch';

/** The UI tools that the core relays to the brain. The local tools run in the core. */
export const UI_TOOLS = ['show_blurb', 'set_mood', 'propose_page', 'ask_user'] as const;
export type UiTool = (typeof UI_TOOLS)[number];

/** The moods the agent may set (the same set as `bottle_mcp::AGENT_MOODS`). No `listening`. */
export const AGENT_MOODS = ['idle', 'processing', 'answering', 'success', 'error'] as const;
export type AgentMood = (typeof AGENT_MOODS)[number];

/** The handlers of the UI tools. The app host wires the card, the mood and the Page review. */
export interface AgentToolHandlers {
  showBlurb(mogId: MogId, a: { text: string; title?: string }): Promise<unknown>;
  setMood(mogId: MogId, a: { mood: AgentMood }): Promise<unknown>;
  /** Opens the Page review (PageService.propose). It never creates or publishes a Page. */
  proposePage(mogId: MogId, a: { title: string; body: string }): Promise<{ reviewId: string }>;
  askUser(mogId: MogId, a: { question: string; choices?: string[] }): Promise<unknown>;
}

export type ToolResult = { ok: true; result: unknown } | { ok: false; error: string };

/** The `agent.tool_call` payload of the core. */
export interface AgentToolCallEvent {
  mogId: MogId;
  callId: string;
  tool: string;
  args: unknown;
}

type Invoke = <K extends CommandName>(name: K, args: CommandArgs[K]) => Promise<unknown>;

export interface AgentBridgeDeps {
  /** The IPC call. Default: the IPC `call`. */
  invoke?: Invoke;
  /** The UI tool handlers. TODO(#563): the app host wires them; a missing one answers not_available. */
  handlers?: Partial<AgentToolHandlers>;
}

type Args = Record<string, unknown>;
type Check<T> = { ok: true; v: T } | { ok: false };

const isObj = (v: unknown): v is Args => typeof v === 'object' && v !== null && !Array.isArray(v);
const has = <T extends string>(list: readonly T[], v: unknown): v is T =>
  typeof v === 'string' && (list as readonly string[]).includes(v);

/** A non-blank string of `max` code points or less (the `bottle_mcp` bounds). */
function text(a: Args, key: string, max: number, required: boolean): Check<string | undefined> {
  const v = a[key];
  if (v === undefined || v === null) return required ? { ok: false } : { ok: true, v: undefined };
  if (typeof v !== 'string' || v.trim() === '' || [...v].length > max) return { ok: false };
  return { ok: true, v };
}

/** The checked arguments of a UI tool, with only the schema's properties. */
function checkArgs(tool: UiTool, raw: unknown): Check<Args> {
  if (!isObj(raw)) return { ok: false };
  const out: Args = {};
  const put = (key: string, c: Check<string | undefined>): boolean => {
    if (!c.ok) return false;
    if (c.v !== undefined) out[key] = c.v;
    return true;
  };
  switch (tool) {
    case 'show_blurb':
      if (!put('text', text(raw, 'text', 4000, true))) return { ok: false };
      if (!put('title', text(raw, 'title', 200, false))) return { ok: false };
      break;
    case 'set_mood':
      if (!has(AGENT_MOODS, raw['mood'])) return { ok: false };
      out['mood'] = raw['mood'];
      break;
    case 'propose_page':
      if (!put('title', text(raw, 'title', 200, true))) return { ok: false };
      if (!put('body', text(raw, 'body', 20000, true))) return { ok: false };
      break;
    case 'ask_user': {
      if (!put('question', text(raw, 'question', 1000, true))) return { ok: false };
      const c = raw['choices'];
      if (c !== undefined && c !== null) {
        if (
          !Array.isArray(c) ||
          c.length > 10 ||
          !c.every((x) => typeof x === 'string' && x.trim() !== '' && [...x].length <= 200)
        )
          return { ok: false };
        out['choices'] = [...(c as string[])];
      }
      break;
    }
  }
  return { ok: true, v: out };
}

export class CoreAgentBridge implements AgentBridge {
  private readonly invoke: Invoke;
  private readonly handlers: Partial<AgentToolHandlers>;

  constructor(deps: AgentBridgeDeps = {}) {
    this.invoke = deps.invoke ?? (call as Invoke);
    this.handlers = deps.handlers ?? {};
  }

  async start(mogId: MogId, h: HarnessKind): Promise<void> {
    await this.invoke('agent_start', { mogId, harness: h });
  }

  async send(t: AgentTurn): Promise<void> {
    await this.invoke('agent_send', { mogId: t.mogId, turn: t as unknown as JsonObject });
  }

  /**
   * One guarded UI tool call. The 4.7 signature has no MOG, so a call without `mogId` runs
   * nothing. A handler error is a content-free `handler_failed`.
   */
  async onTool(c: {
    callId: string;
    tool: BottleTool;
    args: unknown;
    mogId?: MogId;
  }): Promise<ToolResult> {
    if (typeof c.mogId !== 'string' || c.mogId === '') return { ok: false, error: 'unknown_mog' };
    const tool: unknown = c.tool;
    if (!has(UI_TOOLS, tool)) return { ok: false, error: 'tool_not_allowed' };
    const args = checkArgs(tool, c.args);
    if (!args.ok) return { ok: false, error: 'invalid_arguments' };
    try {
      return await this.run(c.mogId, tool, args.v);
    } catch {
      return { ok: false, error: 'handler_failed' };
    }
  }

  private async run(mogId: MogId, tool: UiTool, a: Args): Promise<ToolResult> {
    const h = this.handlers;
    const na: ToolResult = { ok: false, error: 'not_available' };
    switch (tool) {
      case 'show_blurb':
        return h.showBlurb
          ? { ok: true, result: await h.showBlurb(mogId, a as { text: string; title?: string }) }
          : na;
      case 'set_mood':
        return h.setMood
          ? { ok: true, result: await h.setMood(mogId, a as { mood: AgentMood }) }
          : na;
      case 'propose_page': {
        if (!h.proposePage) return na;
        // RTR-29c: the agent learns the review ID only, never a Page state or ID.
        const r = await h.proposePage(mogId, a as { title: string; body: string });
        return { ok: true, result: { reviewId: String(r.reviewId) } };
      }
      case 'ask_user':
        return h.askUser
          ? {
              ok: true,
              result: await h.askUser(mogId, a as { question: string; choices?: string[] }),
            }
          : na;
    }
  }

  /** Answer one `agent.tool_call` of the core with `agent_tool_result`. */
  async handleToolCall(e: AgentToolCallEvent): Promise<void> {
    const result = await this.onTool({
      mogId: e.mogId,
      callId: e.callId,
      tool: e.tool as BottleTool,
      args: e.args,
    });
    await this.invoke('agent_tool_result', { mogId: e.mogId, callId: e.callId, result });
  }
}

/** The 4.7 `AgentTurn` (question, decision, recall, target) of a `preview` outcome (RTR-21b). */
export function buildAgentTurn(
  mogId: MogId,
  p: Extract<DispatchOutcome, { kind: 'preview' }>,
): AgentTurn {
  return { mogId, question: p.question, decision: p.decision, recall: p.recall, target: p.target };
}

export function createAgentBridge(deps: AgentBridgeDeps = {}): CoreAgentBridge {
  return new CoreAgentBridge(deps);
}
