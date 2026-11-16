/**
 * The route of one PTT or hands-free utterance after the router step (#59, ADR 0049).
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
import type { CommandReturns } from '../shared/ipc';
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
  | { route: 'insert_failed'; reason: Exclude<InsertResult, 'inserted'> | 'host_failed'; text: string }
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

export async function routeUtterance(
  _u: Utterance,
  _deps: UtteranceRouteDeps,
): Promise<UtteranceRoute> {
  throw new Error('not built');
}
