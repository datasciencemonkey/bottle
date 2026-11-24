/**
 * The hub Agent panel: the view the brain sends and the actions the panel sends back (slice 2a,
 * the MOG agent on one live Genie Code session).
 *
 * The brain (`src/brain/hub-agent-wiring.ts`) and the hub view (`AgentPanel.tsx`) both read this
 * file. It holds names and shape checks only, no state. It is not a contract file, but it is
 * frozen for the slice: change it only through the orchestrator.
 *
 * The panel content lives only in memory (the brain). A new session, quit or Wipe clears it.
 */
import type {
  AgentCallView,
  AgentCitation,
  AgentPartView,
  AgentSessionState,
  AgentTurnView,
  AgentWsChip,
  GenieCodeStatus,
} from './ipc';

/** The brain-to-view event that carries the Agent panel to the hub. */
export const HUB_AGENT_EVENT = 'hub:agent';

/** The `mogId` of a hub-level `view_emit`, shared with the hub notice. */
export { HUB_SCOPE } from './hub-notice';

/** What the Agent panel shows. */
export interface HubAgentView {
  session: AgentSessionState;
  /** The live turn, if any. */
  live?: AgentTurnView;
  /** At most 3, in send order. */
  waiting: AgentTurnView[];
  /** Newest first. */
  earlier: AgentTurnView[];
  genieCode: GenieCodeStatus;
  /** The thread of each turn in the panel, so Conversations can show it ("Open in Conversations"). */
  threads: { turnId: string; threadId: string }[];
  /**
   * @deprecated The jump to the Agent area is withdrawn: "Open" sends `hub:conversations.open`
   * with `{ threadId }`. The field stays only for the existing wiring and the hub notice.
   */
  notice?: { kind: 'error' | 'info'; text: string };
}

/** The hub Agent actions, sent as `view_emit` kind `click` with mogId `HUB_SCOPE`. */
export type AgentAction =
  | { action: 'agent.ready' }
  | { action: 'agent.stop'; turnId: string }
  | { action: 'agent.remove_queued'; turnId: string }
  | { action: 'agent.new_session' }
  | { action: 'agent.open_settings' };

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
const isStr = (v: unknown): v is string => typeof v === 'string';
const isBool = (v: unknown): v is boolean => typeof v === 'boolean';
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
/** A count: a whole number, 0 or more. */
const isCount = (v: unknown): v is number => isNum(v) && Number.isInteger(v) && v >= 0;
const isProfile = (v: unknown): v is string => isStr(v) && v.length > 0 && v.length <= 128;
const TURN_ID = /^[0-9a-f]{32}$/;
const isTurnId = (v: unknown): v is string => isStr(v) && TURN_ID.test(v);
const CALL_ID = /^c-[0-9a-f]{8}$/;
const isCallId = (v: unknown): v is string => isStr(v) && CALL_ID.test(v);

function oneOf<T extends string>(list: readonly T[]): (v: unknown) => v is T {
  return (v): v is T => list.includes(v as T);
}

const isBinary = oneOf(['ready', 'not_installed', 'wrong_version', 'installing', 'unsupported_os']);
const isProfileState = oneOf([
  'ready',
  'not_u2m',
  'signed_out',
  'missing',
  'not_eligible',
  'no_home',
]);
const isSessionState = oneOf(['none', 'starting', 'idle', 'busy', 'ending']);
const isTurnState = oneOf([
  'queued',
  'planning',
  'running',
  'slow',
  'done',
  'stopped',
  'timed_out',
  'offline',
  'failed',
  'needs_you',
]);
const isCallState = oneOf([
  'queued',
  'asking',
  'ready',
  'no_answer',
  'needs_you',
  'failed',
  'stopped',
]);
const isSummaryKind = oneOf(['note', 'combined']);

/** The action of one hub Agent click, or undefined. Unknown fields are dropped. */
export function asAgentAction(p: unknown): AgentAction | undefined {
  if (!isObj(p)) return undefined;
  switch (p['action']) {
    case 'agent.ready':
    case 'agent.new_session':
    case 'agent.open_settings':
      return { action: p['action'] };
    case 'agent.stop':
    case 'agent.remove_queued':
      return isTurnId(p['turnId']) ? { action: p['action'], turnId: p['turnId'] } : undefined;
    default:
      return undefined;
  }
}

/** The Genie Code status of a `hub:agent` or `hub:workspaces` payload, or undefined. */
export function asGenieCodeStatus(v: unknown): GenieCodeStatus | undefined {
  if (!isObj(v)) return undefined;
  const { binary, version, pinned, agentProfile, agentProfileState } = v;
  if (!isBinary(binary) || !isStr(pinned) || !isProfileState(agentProfileState)) return undefined;
  if (agentProfile !== null && !isProfile(agentProfile)) return undefined;
  if (version !== undefined && !isStr(version)) return undefined;
  return {
    binary,
    ...(version !== undefined ? { version } : {}),
    pinned,
    agentProfile,
    agentProfileState,
  };
}

function asSession(v: unknown): AgentSessionState | undefined {
  if (!isObj(v)) return undefined;
  const { state, sessionId, startedAt, agentProfile, turns, queued, tokens } = v;
  if (!isSessionState(state) || !isCount(turns) || !isCount(queued) || !isCount(tokens)) {
    return undefined;
  }
  if (agentProfile !== null && !isProfile(agentProfile)) return undefined;
  if (sessionId !== undefined && !isTurnId(sessionId)) return undefined;
  if (startedAt !== undefined && !isNum(startedAt)) return undefined;
  return {
    state,
    ...(sessionId !== undefined ? { sessionId } : {}),
    ...(startedAt !== undefined ? { startedAt } : {}),
    agentProfile,
    turns,
    queued,
    tokens,
  };
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function asThreadLinks(v: unknown): HubAgentView['threads'] | undefined {
  if (!Array.isArray(v)) return undefined;
  const out: HubAgentView['threads'] = [];
  for (const t of v) {
    if (!isObj(t) || !isTurnId(t['turnId']) || !isStr(t['threadId']) || !UUID.test(t['threadId'])) {
      return undefined;
    }
    out.push({ turnId: t['turnId'], threadId: t['threadId'] });
  }
  return out;
}

function asChip(v: unknown): AgentWsChip | undefined {
  if (!isObj(v) || !isStr(v['name']) || !isStr(v['colorVariant'])) return undefined;
  return { name: v['name'], colorVariant: v['colorVariant'] };
}

function asCall(v: unknown): AgentCallView | undefined {
  if (!isObj(v)) return undefined;
  const { callId, question, state, startedAt, endedAt } = v;
  const workspace = asChip(v['workspace']);
  if (!isCallId(callId) || !workspace || !isStr(question) || !isCallState(state)) return undefined;
  if (startedAt !== undefined && !isNum(startedAt)) return undefined;
  if (endedAt !== undefined && !isNum(endedAt)) return undefined;
  return {
    callId,
    workspace,
    question,
    state,
    ...(startedAt !== undefined ? { startedAt } : {}),
    ...(endedAt !== undefined ? { endedAt } : {}),
  };
}

function asPart(v: unknown): AgentPartView | undefined {
  if (!isObj(v)) return undefined;
  const workspace = asChip(v['workspace']);
  const sections = v['sections'];
  if (!isCallId(v['callId']) || !workspace || !Array.isArray(sections)) return undefined;
  // Each section is a `CardSection`: the card view checks its fields when it draws them.
  if (!sections.every((s) => isObj(s) && isStr(s['id']))) return undefined;
  for (const k of ['fromEarlier', 'notShownToAgent', 'folded']) {
    if (v[k] !== undefined && !isBool(v[k])) return undefined;
  }
  return {
    callId: v['callId'],
    workspace,
    sections: sections as AgentPartView['sections'],
    ...(v['fromEarlier'] !== undefined ? { fromEarlier: v['fromEarlier'] as boolean } : {}),
    ...(v['notShownToAgent'] !== undefined
      ? { notShownToAgent: v['notShownToAgent'] as boolean }
      : {}),
    ...(v['folded'] !== undefined ? { folded: v['folded'] as boolean } : {}),
  };
}

/** One turn view, or undefined. Unknown fields are dropped. */
export function asAgentTurnView(v: unknown): AgentTurnView | undefined {
  if (!isObj(v)) return undefined;
  const { turnId, question, state, position, used, budget, notice, tokens, seenAt } = v;
  const agentWorkspace = asChip(v['agentWorkspace']);
  if (!isTurnId(turnId) || !isStr(question) || !isTurnState(state) || !agentWorkspace) {
    return undefined;
  }
  if (!isCount(used) || !isCount(budget)) return undefined;
  if (position !== undefined && !isCount(position)) return undefined;
  if (notice !== undefined && !isStr(notice)) return undefined;
  if (tokens !== undefined && !isCount(tokens)) return undefined;
  if (seenAt !== undefined && !isNum(seenAt)) return undefined;
  if (!Array.isArray(v['calls']) || !Array.isArray(v['parts'])) return undefined;
  const calls: AgentCallView[] = [];
  for (const c of v['calls']) {
    const call = asCall(c);
    if (!call) return undefined;
    calls.push(call);
  }
  const parts: AgentPartView[] = [];
  for (const q of v['parts']) {
    const part = asPart(q);
    if (!part) return undefined;
    parts.push(part);
  }
  const out: AgentTurnView = {
    turnId,
    question,
    state,
    used,
    budget,
    agentWorkspace,
    calls,
    parts,
  };
  if (position !== undefined) out.position = position;
  if (notice !== undefined) out.notice = notice;
  if (tokens !== undefined) out.tokens = tokens;
  if (seenAt !== undefined) out.seenAt = seenAt;
  const s = v['summary'];
  if (s !== undefined) {
    if (!isObj(s) || !isStr(s['md']) || !isSummaryKind(s['kind'])) return undefined;
    if (!Array.isArray(s['workspaces'])) return undefined;
    const workspaces: AgentWsChip[] = [];
    for (const w of s['workspaces']) {
      const chip = asChip(w);
      if (!chip) return undefined;
      workspaces.push(chip);
    }
    out.summary = { md: s['md'], kind: s['kind'], workspaces };
    const rawCitations = s['citations'];
    if (rawCitations !== undefined) {
      if (!Array.isArray(rawCitations)) return undefined;
      const citations: AgentCitation[] = [];
      for (const c of rawCitations) {
        if (!isObj(c) || !isCount(c['n']) || c['n'] < 1 || !isCallId(c['callId'])) {
          return undefined;
        }
        const chip = asChip(c['workspace']);
        if (!chip) return undefined;
        const deepLink = c['deepLink'];
        const workspaceHost = c['workspaceHost'];
        if (deepLink !== undefined && !isStr(deepLink)) return undefined;
        if (workspaceHost !== undefined && !isStr(workspaceHost)) return undefined;
        citations.push({
          n: c['n'],
          callId: c['callId'],
          workspace: chip,
          ...(deepLink === undefined ? {} : { deepLink }),
          ...(workspaceHost === undefined ? {} : { workspaceHost }),
        });
      }
      out.summary.citations = citations;
    }
  }
  return out;
}

function asTurnList(v: unknown): AgentTurnView[] | undefined {
  if (!Array.isArray(v)) return undefined;
  const out: AgentTurnView[] = [];
  for (const t of v) {
    const turn = asAgentTurnView(t);
    if (!turn) return undefined;
    out.push(turn);
  }
  return out;
}

/** The Agent panel from one `hub:agent` event, or undefined. Unknown fields are dropped. */
export function asHubAgentView(p: unknown): HubAgentView | undefined {
  if (!isObj(p)) return undefined;
  const session = asSession(p['session']);
  const genieCode = asGenieCodeStatus(p['genieCode']);
  const waiting = asTurnList(p['waiting']);
  const earlier = asTurnList(p['earlier']);
  const threads = asThreadLinks(p['threads']);
  if (!session || !genieCode || !waiting || !earlier || !threads) return undefined;
  // The queue holds at most 3 waiting turns.
  if (waiting.length > 3) return undefined;
  const out: HubAgentView = { session, waiting, earlier, genieCode, threads };
  if (p['live'] !== undefined) {
    const live = asAgentTurnView(p['live']);
    if (!live) return undefined;
    out.live = live;
  }
  // `notice` is a hint for the view: a bad value is dropped, not fatal.
  const n = p['notice'];
  if (isObj(n) && (n['kind'] === 'error' || n['kind'] === 'info') && isStr(n['text'])) {
    out.notice = { kind: n['kind'], text: n['text'] };
  }
  return out;
}
