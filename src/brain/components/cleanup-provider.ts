/**
 * T5 CleanupProvider (4.3, hidden TS brain window).
 *
 * Responsibilities: Pre-pass, SLM (non-streaming, 900 ms deadline), number/entity check, raw
 * fallback.
 */
import type { Tags } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

export interface CleanupProvider {
  /** `usedRaw` is true when the raw text is kept (deadline or a failed meaning check). */
  clean(raw: string, tags: Tags): Promise<{ text: string; usedRaw: boolean }>;
}

export class CleanupProviderStub implements CleanupProvider {
  async clean(_raw: string, _tags: Tags): Promise<{ text: string; usedRaw: boolean }> {
    throw new NotImplementedError('CleanupProvider', 'clean', 55); // TODO(#55)
  }
}

export function createCleanupProvider(): CleanupProvider {
  return new CleanupProviderStub();
}
