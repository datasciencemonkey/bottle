/**
 * T3 CapturePolicy (4.3, hidden TS brain window).
 *
 * Responsibilities: Mode rules, D30 thresholds (chip or preview; no auto-ask, B1), the 1.5 s
 * preview timer, directed segments.
 *
 * `decide` holds the D30 thresholds (#142, ADR 0086). The 1.5 s preview timer, its controls and
 * the Ask Genie chip are the ask gate of T3 (`../ask-gate.ts`), which `startBrain` wires.
 */
import type { CaptureMode, Intent, MogId, SegmentRef } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';
import { ASK_CONFIDENCE } from '../utterance-route';

/** What the brain does with one utterance. It never asks Genie on its own (B1). */
export type CaptureOutcome = 'preview' | 'chip' | 'insert' | 'drop';

export interface CapturePolicy {
  decide(u: { mode: CaptureMode; intent: Intent; confidence: number }): CaptureOutcome;
  directedTo(seg: SegmentRef): MogId | undefined;
}

const C = 'CapturePolicy';

export class CapturePolicyStub implements CapturePolicy {
  /**
   * D30: a question at 0.7 or more gets the 1.5 s preview (RTR-04a), below it the Ask Genie chip
   * (RTR-05). A memory question that the RecallGate did not answer goes on as a question (F1
   * steps 13-15). In a session a question is a chip only, because the preview sends by itself
   * (RTR-07, B1). A dictation is inserted outside a session (RTR-01c). Anything else drops.
   */
  decide(u: { mode: CaptureMode; intent: Intent; confidence: number }): CaptureOutcome {
    const ask = u.intent === 'question' || u.intent === 'memory_question';
    if (u.mode === 'session') return ask ? 'chip' : 'drop';
    if (u.intent === 'dictation') return 'insert';
    if (!ask) return 'drop';
    return u.confidence >= ASK_CONFIDENCE ? 'preview' : 'chip';
  }
  directedTo(_seg: SegmentRef): MogId | undefined {
    throw new NotImplementedError(C, 'directedTo', 119); // TODO(#119)
  }
}

export function createCapturePolicy(): CapturePolicy {
  return new CapturePolicyStub();
}
