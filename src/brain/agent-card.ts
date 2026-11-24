/**
 * Slice 2a: the card of an agent turn, brain side (plan Amendment 2 A2 "What the user sees").
 *
 * - `partOf`: one answered Genie call becomes one part of the turn view (the sections of today's
 *   answer body: answer, key numbers, footer). The reply is verbatim, never rewritten.
 * - `cardChips`: the chips of the compact card, `agent-stop:<turnId>` (Stop, or Remove for a
 *   queued turn), `agent-open:<turnId>` ("Open in Conversations") and, for an expired sign-in,
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
  OrchestratorStatus,
} from '../shared/ipc';
import { CARD_DONE } from '../shared/genie-status';
import type { ArrivalGate } from './arrival-gate';
import { onViewInput, type BrainBus } from './bus';
import type { ChipView } from './card-chips';
import { buildSections } from './card-sections';
import { keyNumbersOf } from './components/ask-events';

/** The label of the compact card link to the thread in hub Conversations. */
export const OPEN_IN_CONVERSATIONS_LABEL = 'Open in Conversations';
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
  resend: 'agent-resend:',
  open: 'agent-open:',
  settings: 'agent-settings:',
  /** A waiting turn under the live one on the card: the label is its question. */
  queued: 'agent-queued:',
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
    part.notShownToAgent = true;
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

/** The chips of the compact card (the turn that owns it). */
export function cardChips(t: AgentTurnView): ChipView[] {
  const chips: ChipView[] = [];
  if (t.state === 'needs_you')
    chips.push({ id: `${CHIP.settings}${t.turnId}`, label: SETTINGS_LABEL });
  else if (t.state === 'queued') chips.push({ id: `${CHIP.stop}${t.turnId}`, label: REMOVE_LABEL });
  else if (LIVE.has(t.state)) chips.push({ id: `${CHIP.stop}${t.turnId}`, label: STOP_LABEL });
  chips.push({ id: `${CHIP.open}${t.turnId}`, label: OPEN_IN_CONVERSATIONS_LABEL });
  return chips;
}

/** The fixed one-line fallback reasons (ADR 0122: the orchestrator model is not ready). */
export type FallbackWhy =
  'no_home' | 'signed_out' | 'no_model' | 'unreachable' | 'timeout' | 'offline' | 'failed';

export function fallbackNotice(why: FallbackWhy, ws: { agent: string; home: string }): string {
  const asked = ` Asked ${ws.home} only.`;
  switch (why) {
    case 'no_home':
      return 'No home workspace is set.';
    case 'signed_out':
      return `Your sign-in to ${ws.home} expired.${asked}`;
    case 'no_model':
      return `The orchestrator model is not available in ${ws.home}.${asked}`;
    case 'unreachable':
      return `The orchestrator model is not reachable.${asked}`;
    case 'timeout':
      return `The agent timed out.${asked}`;
    case 'offline':
      return `Offline.${asked}`;
    case 'failed':
      return `The agent could not run.${asked}`;
  }
}

/** Why the orchestrator model is not ready, from its last known status. Undefined: unknown. */
export function whyOfStatus(s: Pick<OrchestratorStatus, 'state'> | undefined): FallbackWhy {
  if (s === undefined) return 'failed';
  switch (s.state) {
    case 'ready':
      return 'failed';
    case 'no_home':
    case 'signed_out':
    case 'no_model':
    case 'unreachable':
      return s.state;
  }
}

/** The fallback reason of a refused `agent_turn_submit` (an IPC code), or undefined for none. */
export function whyOfRefusal(code: string): FallbackWhy | undefined {
  switch (code) {
    case 'agent_profile_unsupported':
      return 'no_home';
    case 'agent_not_eligible':
      return 'no_model';
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
    case 'failed':
      return 'failed';
    // The sign-in expired: the turn falls back to direct, with the sign-in notice.
    case 'signed_out':
      return 'signed_out';
    // A session end (New session, Wipe, quit, a setting change) and a refusals stop are never a
    // reason to ask home: nothing falls back.
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
  /** A turn of `mogId` changed. The card shows the live turn, with the waiting turns under it. */
  show(mogId: MogId, turn: AgentTurnView): void;
  /** A turn left (removed, or cleared by a new session). */
  drop(mogId: MogId, turnId: string): void;
  /** The user closed the card: the turn on it never opens it again. */
  close(mogId: MogId): void;
  /**
   * A direct answer takes the card. With `turnId` only that turn (a fallback) stays off it; with
   * none, every turn of this MOG does (a plain direct ask).
   */
  takeover(mogId: MogId, turnId?: string): void;
  /**
   * A one-line note (queue full, "Not sent.") on the open agent card. False when no agent card
   * is open: the caller then draws the note itself.
   */
  note(mogId: MogId, text: string): boolean;
  /** The turn on the card of `mogId`, if any. */
  shown(mogId: MogId): string | undefined;
}

/** The dismissed turns that the card remembers. */
const DISMISSED_MAX = 64;

export function createAgentCard(deps: AgentCardDeps): AgentCard {
  /** The turns of each MOG, in the order they came, with their last view. */
  const turns = new Map<MogId, Map<string, AgentTurnView>>();
  /** The turn that owns the card of each MOG now. */
  const owner = new Map<MogId, string>();
  /** The last turn that owned the card, so a finished answer stays until the user closes it. */
  const lastOwner = new Map<MogId, string>();
  const dismissed = new Set<string>();
  /** A note that waits for the next publish of the card of a MOG. */
  const notes = new Map<MogId, string>();
  const dismiss = (id: string) => {
    dismissed.add(id);
    if (dismissed.size > DISMISSED_MAX) dismissed.delete(dismissed.values().next().value as string);
  };
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
  /** The live turn owns the card. With none, the answer that was on it, then the newest waiting turn. */
  function pick(mogId: MogId): AgentTurnView | undefined {
    const all = [...(turns.get(mogId)?.values() ?? [])].filter((t) => !dismissed.has(t.turnId));
    const live = all.find((t) => isOpenTurn(t) && t.state !== 'queued');
    if (live) return live;
    const last = all.find((t) => t.turnId === lastOwner.get(mogId));
    if (last && !isOpenTurn(last)) return last;
    return [...all].reverse().find((t) => t.state === 'queued');
  }
  const anyOpen = () => [...owner.values()].length > 0;
  function publish(mogId: MogId) {
    // A turn that was dismissed and has ended needs no place in memory.
    for (const [id, t] of turns.get(mogId) ?? [])
      if (dismissed.has(id) && !isOpenTurn(t)) turns.get(mogId)?.delete(id);
    const stack = deps.stackFor(mogId);
    const t = pick(mogId);
    const before = owner.get(mogId);
    if (!t) {
      owner.delete(mogId);
      if (before !== undefined) {
        stack.setAgentTurn(undefined);
        stack.clearControls('agent');
        deps.onCardOpen?.(anyOpen());
      }
      return;
    }
    owner.set(mogId, t.turnId);
    lastOwner.set(mogId, t.turnId);
    const waiting = [...(turns.get(mogId)?.values() ?? [])].filter(
      (w) => w.state === 'queued' && w.turnId !== t.turnId && !dismissed.has(w.turnId),
    );
    const note = notes.get(mogId);
    notes.delete(mogId);
    stack.setAgentTurn(note === undefined ? t : { ...t, notice: note });
    stack.setControls('agent', [
      ...cardChips(t),
      ...waiting.map((w) => ({ id: `${CHIP.queued}${w.turnId}`, label: w.question })),
    ]);
    if (before !== t.turnId) {
      try {
        void Promise.resolve(deps.showCard(mogId)).catch(() => undefined);
      } catch {
        // The card stays hidden; the state still goes on it.
      }
    }
    deps.onCardOpen?.(true);
  }
  const later = (mogId: MogId) => run(() => publish(mogId));
  return {
    show(mogId, turn) {
      if (dismissed.has(turn.turnId)) {
        if (!isOpenTurn(turn)) turns.get(mogId)?.delete(turn.turnId);
        return;
      }
      const m = turns.get(mogId) ?? new Map<string, AgentTurnView>();
      m.set(turn.turnId, turn);
      turns.set(mogId, m);
      later(mogId);
    },
    drop(mogId, turnId) {
      turns.get(mogId)?.delete(turnId);
      if (lastOwner.get(mogId) === turnId) lastOwner.delete(mogId);
      later(mogId);
    },
    close(mogId) {
      const id = owner.get(mogId);
      if (id === undefined) return;
      // The card does not reopen at once for a turn that still runs or waits.
      for (const t of turns.get(mogId)?.values() ?? []) if (isOpenTurn(t)) dismiss(t.turnId);
      dismiss(id);
      // Taking a card away is no arrival: it does not wait for the gate.
      try {
        publish(mogId);
      } catch {
        deps.onFault?.();
      }
    },
    note(mogId, text) {
      if (!owner.has(mogId)) return false;
      notes.set(mogId, text);
      later(mogId);
      return true;
    },
    takeover(mogId, turnId) {
      if (turnId !== undefined) dismiss(turnId);
      else for (const id of turns.get(mogId)?.keys() ?? []) dismiss(id);
      const had = owner.has(mogId);
      if (turnId === undefined || lastOwner.get(mogId) === turnId) lastOwner.delete(mogId);
      if (!had) return;
      try {
        publish(mogId);
      } catch {
        deps.onFault?.();
      }
    },
    shown: (mogId) => owner.get(mogId),
  };
}

export interface AgentChipDeps {
  /** Stop the live turn, or remove a waiting one. */
  stop(turnId: string): void;
  /** Show the hub on Conversations, on the thread of this turn. */
  openAgent(turnId: string): void;
  /** Show the hub on Settings → Workspace. */
  openSettings(): void;
  /** The user closed the card of an agent turn (`card_done`). */
  cardDone(mogId: MogId): void;
}

/**
 * Feed the agent chips of a card from the brain Channel. Only the card of the MOG speaks for its
 * chips.
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
      case 'queued':
        return;
      case 'resend':
        // UX slice 5 plugs in here: the kept question goes again with a host-minted ticket.
        return;
    }
  });
}
