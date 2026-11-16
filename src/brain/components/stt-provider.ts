/**
 * T4 SttProvider (4.3, hidden TS brain window).
 *
 * Responsibilities: Local STT by default; Databricks ASR as a probe-gated user opt-in; live
 * demotion, fallback chain.
 */
import type { SegmentRef, Tags, Transcript } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

export interface SttProvider {
  transcribe(seg: SegmentRef, tags: Tags): Promise<Transcript>;
  activeProvider(): Transcript['provider'];
}

const C = 'SttProvider';

export class SttProviderStub implements SttProvider {
  async transcribe(_seg: SegmentRef, _tags: Tags): Promise<Transcript> {
    throw new NotImplementedError(C, 'transcribe', 52); // TODO(#52)
  }
  activeProvider(): Transcript['provider'] {
    throw new NotImplementedError(C, 'activeProvider', 52); // TODO(#52)
  }
}

export function createSttProvider(): SttProvider {
  return new SttProviderStub();
}
