/**
 * Slice 2a: the routing rules of an ⌥G question (plan Amendment 2 A2, R-1 to R-15). A pure
 * function, with no model call and no I/O. It says if a question goes to the live Genie Code
 * session (the agent) or to the direct home path of ADR 0097.
 *
 * - R-1: the agent runs only if Genie Code is ready.
 * - R-2: and only if at least one available workspace is not home.
 * - R-3: and only if the question names an available non-home workspace, or holds a compare word.
 * - R-4: a name matches on the workspace name or the profile name, as whole words, in any case.
 * - R-5: the compare words are fixed (`COMPARE_WORDS`).
 * - R-6: each other question takes the direct path.
 * - R-7: the check adds 5 ms or less: it is a few regular expressions over one short string.
 * - R-8: with R-1 and R-2 true, a direct card offers "Ask other workspaces too" (`offerOthers`).
 * - R-11: with R-3 true and R-1 false, the question takes the direct path, and the card says why.
 * - R-12: a thread follow-up (#606) stays on the direct path.
 * - R-13: while the card of an agent turn is open, or for 10 min after the last agent answer, ⌥G
 *   goes to the session as a follow-up (`createFollowUpWindow`). UX slices 3 and 4 reuse it.
 * - R-14: a follow-up uses the open session (the window closes when the session ends).
 * - R-15: a new agent question uses the open session. Rust starts a new one if it is tainted.
 *
 * R-9 and R-10 are in the chip handler and in Rust: a click on the R-8 chip sends the same
 * question with `mode: 'ask_others'`, and Rust removes home from the catalog of that turn.
 */
import type { AgentTurn, MogId } from '../shared/interfaces';
import type { GenieCodeStatus } from '../shared/ipc';
import { fallbackNotice, whyOfStatus } from './agent-card';
import type { AgentClient } from './agent-run';
import type { ChipView } from './card-chips';
import type { MonotonicClock } from './clock';
import type { DirectSendOptions } from './genie-direct';

/** R-5: the compare words, matched as whole words in any case. */
export const COMPARE_WORDS: readonly string[] = [
  'compare',
  'versus',
  'vs',
  'across',
  'both',
  'each workspace',
  'all workspaces',
  'other workspaces',
];

/** R-13: the follow-up window after the last agent answer (owner default). */
export const FOLLOW_UP_WINDOW_MS = 10 * 60_000;

/** One workspace that the user switched on (or home). */
export interface RouteWorkspace {
  profile: string;
  name: string;
  isHome: boolean;
}

export interface AgentRouteInput {
  question: string;
  /** The last known Genie Code status. Undefined before the first read: not ready. */
  genieCode: Pick<GenieCodeStatus, 'binary' | 'agentProfileState'> | undefined;
  /** The available workspaces, home included. */
  workspaces: readonly RouteWorkspace[];
  /** R-13: the follow-up window is open. */
  followUp?: boolean;
  /** R-12: the question is a follow-up in a hub thread. */
  thread?: boolean;
}

export type AgentRoute =
  | { path: 'agent'; mode: 'ask' | 'follow_up' }
  | {
      path: 'direct';
      /** R-8: show the chip "Ask other workspaces too" on the direct card. */
      offerOthers: boolean;
      /** R-11: a question that wanted the agent, and Genie Code was not ready. */
      reason?: 'not_ready';
    };

/** R-1. */
export function genieCodeReady(s: AgentRouteInput['genieCode']): boolean {
  return s !== undefined && s.binary === 'ready' && s.agentProfileState === 'ready';
}

const WORD = '(?<![\\p{L}\\p{N}_])';
const WORD_END = '(?![\\p{L}\\p{N}_])';
const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
/** A whole-word, case-insensitive match of `needle` in `text`. */
function hasWord(text: string, needle: string): boolean {
  const n = needle.trim();
  if (n === '') return false;
  return new RegExp(`${WORD}${escapeRe(n).replace(/\s+/g, '\\s+')}${WORD_END}`, 'iu').test(text);
}

/** R-3, part 1: the question names an available non-home workspace (R-4). */
export function namesOtherWorkspace(q: string, ws: readonly RouteWorkspace[]): boolean {
  return ws.some((w) => !w.isHome && (hasWord(q, w.name) || hasWord(q, w.profile)));
}

/** R-3, part 2: the question holds a compare word (R-5). */
export function hasCompareWord(q: string): boolean {
  return COMPARE_WORDS.some((w) => hasWord(q, w));
}

export function routeAsk(i: AgentRouteInput): AgentRoute {
  // R-12: a thread follow-up never leaves the direct path, and shows no escalation chip.
  if (i.thread === true) return { path: 'direct', offerOthers: false };
  const ready = genieCodeReady(i.genieCode);
  // R-2.
  const others = i.workspaces.some((w) => !w.isHome);
  // R-13, R-14: a follow-up goes to the open session, whatever it says.
  if (i.followUp === true && ready && others) return { path: 'agent', mode: 'follow_up' };
  const wants =
    others && (namesOtherWorkspace(i.question, i.workspaces) || hasCompareWord(i.question));
  // R-15: a new question uses the open session. Rust restarts it if it is tainted.
  if (wants && ready) return { path: 'agent', mode: 'ask' };
  // R-11: the question wanted the agent but Genie Code is not ready.
  if (wants) return { path: 'direct', offerOthers: false, reason: 'not_ready' };
  // R-6, R-8.
  return { path: 'direct', offerOthers: ready && others };
}

/** R-13: when the follow-up window is open. */
/**
 * The MOG of a thread reply. The hub's MOG must own the thread: a mismatch would write a turn
 * under one MOG with the root of another, so it is refused ("Not sent."). `known` is the owner
 * of the thread, if any; with no hub MOG the reply takes it.
 */
export function threadReplyMog(
  hubMogId: string | undefined,
  known: string | undefined,
): { mogId: string | undefined } | 'refused' {
  if (hubMogId !== undefined && known !== undefined && known !== hubMogId) return 'refused';
  return { mogId: hubMogId ?? known };
}

export interface FollowUpWindow {
  /** The card of an agent turn opened (true) or closed (false). */
  card(open: boolean): void;
  /** An agent answer landed. The 10 min start now. */
  answered(): void;
  /** The session ended: no session to follow up. */
  reset(): void;
  isOpen(): boolean;
}

export function createFollowUpWindow(clock: MonotonicClock): FollowUpWindow {
  let cardOpen = false;
  let lastAnswerAt: number | undefined;
  return {
    card: (open) => {
      cardOpen = open;
    },
    answered: () => {
      lastAnswerAt = clock.now();
    },
    reset: () => {
      cardOpen = false;
      lastAnswerAt = undefined;
    },
    isOpen: () =>
      cardOpen || (lastAnswerAt !== undefined && clock.now() - lastAnswerAt < FOLLOW_UP_WINDOW_MS),
  };
}

/** The label of the R-8 chip. */
export const ASK_OTHERS_CHIP = 'Ask other workspaces too';
const MORE_PREFIX = 'agent-more:';
/** The direct questions that wait for an R-8 click. The oldest go first. */
const ESCALATIONS_MAX = 16;

export interface AskRouterDeps {
  /** The last known Genie Code status and the available workspaces (a cache: nothing is awaited). */
  context(): { genieCode: AgentRouteInput['genieCode']; workspaces: readonly RouteWorkspace[] };
  names(): { agent: string; home: string };
  followUp: Pick<FollowUpWindow, 'isOpen'>;
  agent: Pick<AgentClient, 'submit'>;
  direct(turn: AgentTurn, opts?: DirectSendOptions): Promise<void>;
  /** A direct ask takes the card: the card of an agent turn of this MOG leaves it. */
  closeAgentCard(mogId: MogId): void;
  /** The R-8 chip was used: take it off the card. */
  removeChip(mogId: MogId, opId: string): void;
  /** A one-line pop-up on the card. */
  note(mogId: MogId, text: string): void;
  /** Read the Genie Code status again, in the background (a stale "not installed"). */
  refreshGenieCode(): void;
}

export interface AskRouter {
  /** The hand-off seam of the ask gate. */
  send(turn: AgentTurn): Promise<void>;
  /** The `offerOthers` of the direct send (R-8). */
  offerOthers(turn: AgentTurn, opId: string): ChipView | undefined;
  /** A `chip_click` on `agent-more:<opId>` (R-9). `ticket` is the host-minted Ask ticket. */
  askOthers(mogId: MogId, opId: string, ticket: string | undefined): Promise<void>;
}

export function createAskRouter(deps: AskRouterDeps): AskRouter {
  const asked = new Map<string, AgentTurn>();
  return {
    send(turn) {
      const ctx = deps.context();
      const route = routeAsk({
        question: turn.question,
        genieCode: ctx.genieCode,
        workspaces: ctx.workspaces,
        followUp: deps.followUp.isOpen(),
      });
      if (route.path === 'agent') return deps.agent.submit(turn, route.mode);
      deps.closeAgentCard(turn.mogId);
      if (route.reason === 'not_ready') {
        // R-11: the question wanted the agent. The direct card says why. A stale status is read again.
        deps.refreshGenieCode();
        return deps.direct(turn, {
          notice: fallbackNotice(whyOfStatus(ctx.genieCode), deps.names()),
        });
      }
      return deps.direct(turn);
    },
    offerOthers(turn, opId) {
      const ctx = deps.context();
      const r = routeAsk({
        question: turn.question,
        genieCode: ctx.genieCode,
        workspaces: ctx.workspaces,
      });
      if (r.path !== 'direct' || !r.offerOthers) return undefined;
      asked.set(opId, turn);
      if (asked.size > ESCALATIONS_MAX) asked.delete(asked.keys().next().value as string);
      return { id: `${MORE_PREFIX}${opId}`, label: ASK_OTHERS_CHIP };
    },
    async askOthers(mogId, opId, ticket) {
      const turn = asked.get(opId);
      if (!turn || turn.mogId !== mogId) return;
      asked.delete(opId);
      deps.removeChip(mogId, opId);
      // R-9: the same question, with `mode: 'ask_others'`. R-10 is in Rust: home leaves the catalog.
      try {
        await deps.agent.submit(turn, 'ask_others', ticket);
      } catch {
        deps.note(mogId, 'Not sent.');
      }
    },
  };
}
