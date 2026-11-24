/**
 * The route of one PTT or hands-free utterance after the router step (#59, ADR 0053, ADR 0098).
 *
 * - ADR 0098 (user decision, MVP): with no ⌥G, every utterance is dictation. There is no intent
 *   gate: a question, a command and a plain sentence are all inserted (or go to Copy). Each
 *   dictation also gets the "Ask Genie" chip of the exact dictated text, the fallback for text
 *   that was already dictated. Only a press on that chip sends it.
 * - RTR-01c: a dictation ends at `insert_text` or at Copy. The main agent and Genie take no part:
 *   this module never calls `agent_*` or `mcp_call`.
 * - RTR-17a: ⌥G during the talk-key hold (`hotkey:ptt` up, `ask: true`) forces the ask route.
 *   The router step still runs (RTR-14), but its intent and its rewritten question are not used:
 *   the route holds the exact text that a dictation would insert.
 * - RTR-17b: the ask route has no preview. ⌥G is the user action (B1), so the T3 ask gate
 *   (`ask-gate.ts`) sends the text at once through its one send seam. Nothing is sent here.
 * - RTR-07: a session utterance never gets the ask route. It gets a chip.
 * - RTR-14: an injection-suspected utterance never gets the ask route. It gets the warning chip.
 *
 * The capture lifecycle calls it through `createUtteranceHandler` (`utterance-wiring.ts`).
 */
import type { LeaseId, MogId, RouterDecision } from '../shared/interfaces';
import { call, type CommandArgs, type CommandReturns } from '../shared/ipc';
import { createRouterContextGate } from './components/router-context-gate';
import type { RouterInput } from './components/router-step';
import { DICTATION_CLEANUP_DEADLINE_MS } from './dictation-cleanup';

/** A router confidence mark (D30). The offline heuristic stays below it (RTR-15c). */
export const ASK_CONFIDENCE = 0.7;

export type InsertResult = CommandReturns['insert_text']['result'];

export interface Utterance {
  mogId: MogId;
  /** The STT text (pre-pass). */
  text: string;
  /** ADR 0118: the capture lease of the segment. `insert_text` pastes only into its app. */
  leaseId: LeaseId;
  /** ⌥G was tapped during the talk-key hold (`hotkey:ptt` up, `ask: true`, ADR 0098). */
  forceAsk: boolean;
  /** `app.dictation.insert` (Appendix F): on after the user grants Accessibility. */
  insertionOn: boolean;
  input: RouterInput;
  /** ADR 0113: Rust parsed "remember this …" from its own transcript. The brain only passes it on. */
  remember?: { ticket: string; text: string };
}

export type UtteranceRoute =
  | {
      /** ADR 0113: a "remember this" order. Nothing is pasted, routed or sent to a model. */
      route: 'remember';
      ticket: string;
      text: string;
    }
  | { route: 'inserted'; decision: RouterDecision }
  | {
      route: 'insert_failed';
      reason: Exclude<InsertResult, 'inserted'> | 'host_failed';
      text: string;
    }
  | { route: 'copy'; text: string }
  | {
      /** RTR-17a: ⌥G asks Genie with `question`, the exact text, at once (ADR 0098). */
      route: 'ask';
      question: string;
      decision: RouterDecision;
    }
  | {
      route: 'chip';
      decision: RouterDecision;
      /** RTR-14: the chip shows a warning. */
      warning: 'injection_suspected' | undefined;
      /** The dictation of the same text, done before the chip (inserted, failed, or Copy). */
      dictated?: DictationRoute;
    }
  | { route: 'other'; decision: RouterDecision };

/** The end of a dictation, before its fallback chip. */
export type DictationRoute = Extract<
  UtteranceRoute,
  { route: 'inserted' | 'insert_failed' | 'copy' }
>;

/** Per-call options of the router step. */
export interface RouteOptions {
  /** A dictation paste: not a ⌥G ask and not a session capture. Only it gets the cleanup deadline. */
  dictation: boolean;
}

export interface UtteranceRouteDeps {
  /** The router step (`RouterContextGate.route`). */
  route(
    text: string,
    mogId: MogId,
    input: RouterInput,
    opts: RouteOptions,
  ): Promise<RouterDecision>;
  /**
   * The exact text that a ⌥G ask sends (RTR-17a, ADR 0122): the deterministic pre-pass text,
   * never the model `cleaned_text`. Without it the ask sends the decision `cleaned_text`.
   */
  askText?(text: string): string;
  /** `insert_text` (R11, #58). */
  insertText(args: CommandArgs['insert_text']): Promise<CommandReturns['insert_text']>;
}

/** The production deps: the router step over `model_invoke` and the `insert_text` command. */
export function ipcUtteranceDeps(opts: { deadlineMs?: number } = {}): UtteranceRouteDeps {
  // ADR 0122: Rust runs the router step on the orchestrator model of the user's setting, so the
  // brain sends no model id. The dictation deadline is CAP-67a. Only a dictation call has the
  // deadline and no retry. `withDictationCleanup` falls back to the pre-pass text at once when
  // the deadline passes.
  const dictationGate = createRouterContextGate({
    deadlineMs: opts.deadlineMs ?? DICTATION_CLEANUP_DEADLINE_MS,
  });
  const plainGate = createRouterContextGate();
  return {
    route: (text, mogId, input, o) =>
      (o.dictation ? dictationGate : plainGate).route(text, mogId, input),
    insertText: (args) => call('insert_text', args),
  };
}

const firstNonEmpty = (...xs: string[]): string => xs.find((x) => x.trim() !== '') ?? '';

/** Route one utterance. It calls only the router step and, for a dictation, `insert_text`. */
export async function routeUtterance(
  u: Utterance,
  deps: UtteranceRouteDeps,
): Promise<UtteranceRoute> {
  // ADR 0113: Rust decides. A remember order is saved, never pasted, and its text never goes to
  // the router model. The brain has no "remember this" grammar of its own.
  if (u.remember) return { route: 'remember', ticket: u.remember.ticket, text: u.remember.text };
  const dictation = !u.forceAsk && u.input.captureMode !== 'session';
  const decision = await deps.route(u.text, u.mogId, u.input, { dictation });
  const warning = decision.injection_suspected ? ('injection_suspected' as const) : undefined;
  // The exact text: what a dictation inserts, and what ⌥G or the fallback chip sends.
  const exact = firstNonEmpty(decision.cleaned_text, u.text);
  // RTR-07, B1: in a session nothing sends without a click, so an ask is a chip. A session is
  // never dictated into the focused app.
  if (u.input.captureMode === 'session') {
    // ADR 0122: no ask path sends model text. The chip holds the pre-pass text.
    const asked = deps.askText ? deps.askText(u.text) : undefined;
    const chipDecision =
      asked === undefined ? decision : { ...decision, cleaned_text: asked, genie_question: asked };
    return u.forceAsk || decision.intent === 'question'
      ? { route: 'chip', decision: chipDecision, warning }
      : { route: 'other', decision };
  }
  if (u.forceAsk) {
    // RTR-17a: an ask with no text sends nothing. RTR-14: a flagged one is the warning chip.
    // ADR 0122: the ask sends the pre-pass text, never the model `cleaned_text`.
    const asked = deps.askText ? deps.askText(u.text) : exact;
    if (asked === '') return { route: 'other', decision };
    if (warning) return { route: 'chip', decision, warning };
    return { route: 'ask', question: asked, decision };
  }
  // ADR 0098: no intent gate. The utterance is dictated, then gets the chip of the same text.
  let dictated: DictationRoute;
  if (!u.insertionOn) dictated = { route: 'copy', text: exact };
  else {
    let result: InsertResult | 'host_failed';
    try {
      result = (await deps.insertText({ mogId: u.mogId, text: exact, leaseId: u.leaseId })).result;
    } catch {
      result = 'host_failed';
    }
    dictated =
      result === 'inserted'
        ? { route: 'inserted', decision }
        : { route: 'insert_failed', reason: result, text: exact };
  }
  return {
    route: 'chip',
    decision: { ...decision, genie_question: exact },
    warning,
    dictated,
  };
}
