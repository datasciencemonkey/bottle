/**
 * T3 CapturePolicy (4.3, hidden TS brain window).
 *
 * Responsibilities: Mode rules, D30 thresholds (chip or preview; no auto-ask, B1), the 1.5 s
 * preview timer, directed segments.
 */
import type { CaptureMode, Intent, MogId, SegmentRef } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

/** What the brain does with one utterance. It never asks Genie on its own (B1). */
export type CaptureOutcome = 'preview' | 'chip' | 'insert' | 'drop';

export interface CapturePolicy {
  decide(u: { mode: CaptureMode; intent: Intent; confidence: number }): CaptureOutcome;
  directedTo(seg: SegmentRef): MogId | undefined;
}

const C = 'CapturePolicy';

export class CapturePolicyStub implements CapturePolicy {
  decide(_u: { mode: CaptureMode; intent: Intent; confidence: number }): CaptureOutcome {
    throw new NotImplementedError(C, 'decide', 142); // TODO(#142)
  }
  directedTo(_seg: SegmentRef): MogId | undefined {
    throw new NotImplementedError(C, 'directedTo', 119); // TODO(#119)
  }
}

export function createCapturePolicy(): CapturePolicy {
  return new CapturePolicyStub();
}
