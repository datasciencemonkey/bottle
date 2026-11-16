/**
 * The route of one PTT or hands-free utterance after the router step (#59, ADR 0053).
 *
 * - RTR-01b, RTR-01c: an utterance with the intent `dictation` ends at `insert_text`. The main
 *   agent and Genie take no part: this module never calls `agent_*` or `mcp_call`.
 * - RTR-17a: Shift held with the talk key forces the ask route. Only the intent gate is skipped:
 *   the router step still runs (cleaned text, hints), and the route still goes through the preview.
 * - RTR-17b: a forced ask keeps the 1.5 s preview (RTR-04a). The preview, the agent hand-off and
 *   the chip are the caller's (T3 CapturePolicy, TODO(#142)); this module returns the route.
 * - B1: nothing goes to Genie here. A question gives a `preview` or a `chip`, never a send.
 *
 * The capture lifecycle calls `routeUtterance` at key-up with the STT text: TODO(#14).
 */
import type { MogId, RouterDecision } from '../shared/interfaces';
import { call, type CommandReturns } from '../shared/ipc';
import { createRouterContextGate } from './components/router-context-gate';
import type { RouterInput } from './components/router-step';

/** The preview of RTR-04a, "Sending in 1.5 s". */
export const PREVIEW_MS = 1500;
/** A question at or above this confidence gets the preview; below it, the "Ask Genie" chip (D30). */
export const ASK_CONFIDENCE = 0.7;

export type InsertResult = CommandReturns['insert_text']['result'];

export interface Utterance {
  mogId: MogId;
  /** The STT text (pre-pass). */
  text: string;
  /** Shift was held with the talk key (`hotkey:ptt` down, `shift: true`). */
  forceAsk: boolean;
  /** `app.dictation.insert` (Appendix F): on after the user grants Accessibility. */
  insertionOn: boolean;
  input: RouterInput;
}

export type UtteranceRoute =
  | { route: 'inserted'; decision: RouterDecision }
  | {
      route: 'insert_failed';
      reason: Exclude<InsertResult, 'inserted'> | 'host_failed';
      text: string;
    }
  | { route: 'copy'; text: string }
  | {
      route: 'preview';
      question: string;
      forced: boolean;
      previewMs: typeof PREVIEW_MS;
      decision: RouterDecision;
    }
  | { route: 'chip'; decision: RouterDecision }
  | { route: 'other'; decision: RouterDecision };

export interface UtteranceRouteDeps {
  /** The router step (`RouterContextGate.route`). */
  route(text: string, mogId: MogId, input: RouterInput): Promise<RouterDecision>;
  /** `insert_text` (R11, #58). */
  insertText(args: { mogId: MogId; text: string }): Promise<CommandReturns['insert_text']>;
}

/** The production deps: the router step over `model_invoke` and the `insert_text` command. */
export function ipcUtteranceDeps(): UtteranceRouteDeps {
  const gate = createRouterContextGate();
  return {
    route: (text, mogId, input) => gate.route(text, mogId, input),
    insertText: (args) => call('insert_text', args),
  };
}

const firstNonEmpty = (...xs: string[]): string => xs.find((x) => x.trim() !== '') ?? '';

/** Route one utterance. It calls only the router step and, for a dictation, `insert_text`. */
export async function routeUtterance(
  u: Utterance,
  deps: UtteranceRouteDeps,
): Promise<UtteranceRoute> {
  const decision = await deps.route(u.text, u.mogId, u.input);
  const preview = (question: string, forced: boolean): UtteranceRoute => ({
    route: 'preview',
    question,
    forced,
    previewMs: PREVIEW_MS,
    decision,
  });
  // RTR-17a: Shift skips the intent gate only. The preview still guards the send (RTR-17b, B1).
  if (u.forceAsk) {
    return preview(firstNonEmpty(decision.genie_question, decision.cleaned_text, u.text), true);
  }
  switch (decision.intent) {
    case 'dictation': {
      // RTR-01c: the flow ends here. No main agent and no Genie take part.
      const text = firstNonEmpty(decision.cleaned_text, u.text);
      if (!u.insertionOn) return { route: 'copy', text };
      let result: InsertResult | 'host_failed';
      try {
        result = (await deps.insertText({ mogId: u.mogId, text })).result;
      } catch {
        result = 'host_failed';
      }
      return result === 'inserted'
        ? { route: 'inserted', decision }
        : { route: 'insert_failed', reason: result, text };
    }
    case 'question':
      return decision.confidence >= ASK_CONFIDENCE
        ? preview(firstNonEmpty(decision.genie_question, decision.cleaned_text, u.text), false)
        : { route: 'chip', decision };
    default:
      return { route: 'other', decision };
  }
}
