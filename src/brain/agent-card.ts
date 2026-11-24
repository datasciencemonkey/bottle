/**
 * Slice 2a: the card of an agent turn, brain side (plan Amendment 2 A2 "What the user sees").
 *
 * - `partOf`: one answered Genie call becomes one part of the turn view (the sections of today's
 *   answer body: answer, key numbers, footer). The reply is verbatim, never rewritten.
 * - `cardChips`: the chips of the compact card, `agent-stop:<turnId>` (Stop, or Remove for a
 *   queued turn), `agent-open:<turnId>` ("Open in Agent") and, for an expired sign-in,
 *   `agent-settings:<turnId>`. UX slice 5 adds `agent-resend:<turnId>` to the same row.
 * - `createAgentCard`: puts the live turn on the card of its MOG. Each update is an arrival, so
 *   it goes through the UX slice 1 arrival gate: no card opens during a hold, and the card opens
 *   1.2 s after the paste. No question, answer or reply text goes to the log.
 * - `fallbackNotice`: the one-line reasons of the fallback to the direct home path.
 */
import type { MogId } from '../shared/interfaces';
import type {
  AgentCallState,
  AgentCallView,
  AgentPartView,
  AgentTurnCall,
  AgentTurnView,
  AgentWsChip,
  GenieCodeStatus,
} from '../shared/ipc';
import { CARD_DONE } from '../shared/genie-status';
import type { ArrivalGate } from './arrival-gate';
import { onViewInput, type BrainBus } from './bus';
import type { ChipView } from './card-chips';
import { buildSections } from './card-sections';
import { keyNumbersOf } from './components/ask-events';

/** The label of the compact card link to the hub area. */
export const OPEN_IN_AGENT_LABEL = 'Open in Agent';
export const STOP_LABEL = 'Stop';
/** A queued turn has nothing to stop: Stop removes it from the queue. */
export const REMOVE_LABEL = 'Remove';
export const SETTINGS_LABEL = 'Open Settings';
/** The text of a part whose answer the core withheld (a quarantine hit). */
export const WITHHELD_TEXT = 'This answer was held back. It looked like it held instructions.';
/** The "Where your data goes" line is not part of a part: the card shows it once. */
const NO_FLOW = '';

export const CHIP = {
  stop: 'agent-stop:',
  more: 'agent-more:',
  resend: 'agent-resend:',
  open: 'agent-open:',
  settings: 'agent-settings:',
} as const;

/** The id of an agent chip, split into its kind and its argument, or undefined. */
export function parseAgentChip(id: string): { kind: keyof typeof CHIP; arg: string } | undefined {
  for (const [kind, prefix] of Object.entries(CHIP) as [keyof typeof CHIP, string][]) {
    if (id.startsWith(prefix) && id.length > prefix.length)
      return { kind, arg: id.slice(prefix.length) };
  }
  return undefined;
}

/** Core call states to the view states of a call row (the same names as the UX proposal 2.1). */
export function callViewState(s: AgentCallState): AgentCallView['state'] {
  switch (s) {
    case 'queued':
      return 'queued';
    case 'asking':
      return 'asking';
    case 'answered':
      return 'ready';
    case 'no_answer':
      return 'no_answer';
    case 'signed_out':
      return 'needs_you';
    case 'stopped':
      return 'stopped';
    case 'failed':
    case 'timeout':
      return 'failed';
  }
}

/** A call that will not change again. `asking` and `queued` still move. */
export const isTerminalCall = (s: AgentCallState): boolean => s !== 'queued' && s !== 'asking';

export interface PartOptions {
  chip: AgentWsChip;
  /** `https://host` of the workspace, so the footer link is trusted for this host only. */
  host?: string | undefined;
  fromEarlier?: boolean;
  /** D1 off and this part is not from the agent workspace. */
  notShownToAgent?: boolean;
}

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

/** The answer markdown of a reply: `final_answer`, else the reply text. */
export function answerOf(call: Pick<AgentTurnCall, 'reply'>): string {
  const sc = call.reply?.structuredContent;
  return (isObj(sc) ? str(sc['final_answer']) : '') || call.reply?.text || '';
}

/** `deep_link`, `conversation_id` and `response_id` of a reply, when it has them. */
export function replyRefs(call: Pick<AgentTurnCall, 'reply'>): {
  deepLink: string;
  conversationId?: string;
  responseId?: string;
} {
  const sc = call.reply?.structuredContent;
  const o = isObj(sc) ? sc : {};
  const conversationId = str(o['conversation_id']);
  const responseId = str(o['response_id']);
  return {
    deepLink: str(o['deep_link']),
    ...(conversationId !== '' ? { conversationId } : {}),
    ...(responseId !== '' ? { responseId } : {}),
  };
}

/** One answered call as a part, or undefined when the call has nothing to show. */
export function partOf(call: AgentTurnCall, o: PartOptions): AgentPartView | undefined {
  if (call.state !== 'answered') return undefined;
  const part: AgentPartView = { callId: call.callId, workspace: o.chip, sections: [] };
  if (call.withheld !== undefined) {
    part.sections = [{ id: 'answer', md: WITHHELD_TEXT }];
  } else {
    const md = answerOf(call);
    if (md.trim() === '') return undefined;
    const refs = replyRefs(call);
    part.sections = buildSections({
      answerMd: md,
      keyNumbers: keyNumbersOf(md).map((k) => k.text),
      ...(refs.deepLink !== '' ? { deepLink: refs.deepLink } : {}),
      ...(o.host ? { workspaceHost: o.host } : {}),
      dataFlowText: NO_FLOW,
    }).filter((s) => s.id !== 'dataFlow');
  }
  if (o.fromEarlier) part.fromEarlier = true;
  if (o.notShownToAgent) part.notShownToAgent = true;
  return part;
}

const LIVE: ReadonlySet<AgentTurnView['state']> = new Set(['planning', 'running', 'slow']);

/** True while the turn waits or runs. */
export const isOpenTurn = (t: Pick<AgentTurnView, 'state'>): boolean =>
  t.state === 'queued' || LIVE.has(t.state);

/** The chips of the compact card. */
export function cardChips(t: AgentTurnView): ChipView[] {
  const chips: ChipView[] = [];
  if (t.state === 'queued') chips.push({ id: `${CHIP.stop}${t.turnId}`, label: REMOVE_LABEL });
  else if (LIVE.has(t.state)) chips.push({ id: `${CHIP.stop}${t.turnId}`, label: STOP_LABEL });
  if (t.state === 'needs_you')
    chips.push({ id: `${CHIP.settings}${t.turnId}`, label: SETTINGS_LABEL });
  chips.push({ id: `${CHIP.open}${t.turnId}`, label: OPEN_IN_AGENT_LABEL });
  return chips;
}

/** The fixed one-line fallback reasons (plan section 1, "Fallback notice"). */
export type FallbackWhy =
  | 'not_installed'
  | 'installing'
  | 'unsupported_os'
  | 'wrong_version'
  | 'not_available'
  | 'signed_out'
  | 'timeout'
  | 'offline'
  | 'failed';

export function fallbackNotice(why: FallbackWhy, ws: { agent: string; home: string }): string {
  const asked = `Asked ${ws.home} only.`;
  switch (why) {
    case 'not_installed':
      return `Genie Code is not installed. ${asked}`;
    case 'installing':
      return `Genie Code is still installing. ${asked}`;
    case 'unsupported_os':
      return `Genie Code is not available on this Mac yet. ${asked}`;
    case 'wrong_version':
      return `Genie Code is not the version that Bottle needs. ${asked}`;
    case 'not_available':
      return `Genie Code is not available in ${ws.agent}. ${asked}`;
    case 'signed_out':
      return `Your sign-in to ${ws.agent} expired. ${asked}`;
    case 'timeout':
      return `The agent timed out. ${asked}`;
    case 'offline':
      return `Offline. ${asked}`;
    case 'failed':
      return `The agent could not run. ${asked}`;
  }
}

/** R-11: why Genie Code is not ready, from its last known status. */
export function whyOfStatus(
  s: Pick<GenieCodeStatus, 'binary' | 'agentProfileState'> | undefined,
): FallbackWhy {
  if (s === undefined) return 'failed';
  switch (s.binary) {
    case 'not_installed':
      return 'not_installed';
    case 'installing':
      return 'installing';
    case 'unsupported_os':
      return 'unsupported_os';
    case 'wrong_version':
      return 'wrong_version';
    case 'ready':
      return s.agentProfileState === 'signed_out' ? 'signed_out' : 'not_available';
  }
}

/** The fallback reason of a refused `agent_turn_submit` (an IPC code), or undefined for none. */
export function whyOfRefusal(code: string): FallbackWhy | undefined {
  switch (code) {
    case 'genie_code_missing':
      return 'not_installed';
    case 'genie_code_version':
      return 'wrong_version';
    case 'agent_profile_unsupported':
    case 'agent_not_eligible':
      return 'not_available';
    case 'agent_signed_out':
      return 'signed_out';
    case 'agent_session_unavailable':
    case 'agent_failed':
      return 'failed';
    case 'agent_timeout':
    case 'timeout':
      return 'timeout';
    case 'agent_offline':
    case 'offline':
      return 'offline';
    default:
      return undefined;
  }
}

/** The fallback reason of a turn that ended before any Genie call, or undefined for none. */
export function whyOfOutcome(outcome: string): FallbackWhy | undefined {
  switch (outcome) {
    case 'timeout':
      return 'timeout';
    case 'offline':
      return 'offline';
    case 'not_eligible':
      return 'not_available';
    case 'failed':
    case 'lockdown':
    case 'refusals':
    case 'session_ended':
      return 'failed';
    default:
      return undefined;
  }
}

/** The text for a refusal that has no fallback: the user can try again. */
export const REFUSAL_TEXT: Readonly<Record<string, string>> = {
  agent_queue_full: 'Your agent has 3 questions waiting. Try again when one is done.',
  agent_rate_limited: 'That is too many questions in a minute. Wait a moment.',
};

/** The card stack calls that the controller needs. */
export interface AgentCardStack {
  setAgentTurn(v: AgentTurnView | undefined): void;
  setControls(owner: string, controls: ChipView[]): void;
  clearControls(owner: string): void;
}

export interface AgentCardDeps {
  stackFor(mogId: MogId): AgentCardStack;
  showCard(mogId: MogId): unknown;
  /** UX slice 1: no card opens during a hold. */
  arrivals?: Pick<ArrivalGate, 'whenClear'> | undefined;
  /** R-13: the card of an agent turn opened (true) or closed (false). */
  onCardOpen?(open: boolean): void;
  /** A fixed code line only. */
  onFault?(): void;
}

export interface AgentCard {
  /** Show this turn on the card of `mogId`. The newest turn of the MOG owns the card. */
  show(mogId: MogId, turn: AgentTurnView): void;
  /** The user closed the card, or a direct ask took it. */
  close(mogId: MogId): void;
  /** The turn on the card of `mogId`, if any. */
  shown(mogId: MogId): string | undefined;
}

export function createAgentCard(deps: AgentCardDeps): AgentCard {
  const onCard = new Map<MogId, string>();
  const run = (fn: () => void) => {
    const safe = () => {
      try {
        fn();
      } catch {
        deps.onFault?.();
      }
    };
    if (deps.arrivals) deps.arrivals.whenClear(safe);
    else safe();
  };
  return {
    show(mogId, turn) {
      onCard.set(mogId, turn.turnId);
      run(() => {
        // An older update that waited behind a hold never covers a newer turn.
        if (onCard.get(mogId) !== turn.turnId) return;
        const stack = deps.stackFor(mogId);
        stack.setAgentTurn(turn);
        stack.setControls('agent', cardChips(turn));
        try {
          void Promise.resolve(deps.showCard(mogId)).catch(() => undefined);
        } catch {
          // The card stays hidden; the state still goes on it.
        }
        deps.onCardOpen?.(true);
      });
    },
    close(mogId) {
      if (!onCard.delete(mogId)) return;
      const stack = deps.stackFor(mogId);
      stack.setAgentTurn(undefined);
      stack.clearControls('agent');
      deps.onCardOpen?.(onCard.size > 0);
    },
    shown: (mogId) => onCard.get(mogId),
  };
}

export interface AgentChipDeps {
  /** Stop the live turn, or remove a waiting one. */
  stop(turnId: string): void;
  /** Show the hub on its Agent area. */
  openAgent(turnId: string): void;
  /** Show the hub on Settings → Workspace. */
  openSettings(): void;
  /** R-9: the click on "Ask other workspaces too". `ticket` is the host's `actionTicket`. */
  askOthers(mogId: MogId, opId: string, ticket: string | undefined): void;
  /** The user closed the card of an agent turn (`card_done`). */
  cardDone(mogId: MogId): void;
}

/**
 * Feed the agent chips of a card from the brain Channel. Only the card of the MOG speaks for its
 * chips. `agent-more:` needs the host ticket, so it reads `input.actionTicket` here.
 */
export function wireAgentChips(bus: BrainBus, deps: AgentChipDeps): () => void {
  return onViewInput(bus, (input) => {
    if (input.from !== `card-${input.mogId}`) return;
    const p = isObj(input.payload) ? input.payload : {};
    if (input.kind === 'click' && p['target'] === CARD_DONE) return deps.cardDone(input.mogId);
    if (input.kind !== 'chip_click' || typeof p['chipId'] !== 'string') return;
    const chip = parseAgentChip(p['chipId']);
    if (!chip) return;
    switch (chip.kind) {
      case 'stop':
        return deps.stop(chip.arg);
      case 'open':
        return deps.openAgent(chip.arg);
      case 'settings':
        return deps.openSettings();
      case 'more':
        return deps.askOthers(input.mogId, chip.arg, input.actionTicket);
      case 'resend':
        // UX slice 5 plugs in here: the kept question goes again with a host-minted ticket.
        return;
    }
  });
}
