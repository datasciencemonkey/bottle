/**
 * T12 ConsentService (4.3, hidden TS brain window).
 *
 * Responsibilities: Versioned notice, per-session attestation, consent records (13 months).
 */
import type { MogId } from '../../shared/interfaces';
import { NotImplementedError } from '../errors';

export interface ConsentRecord {
  kind: 'notice' | 'only_me' | 'others_agreed';
  version: number;
  at: string;
}

export interface ConsentService {
  /** The accepted notice version, or null before the first-run notice is accepted. */
  noticeAccepted(): Promise<{ version: number } | null>;
  /** Called only on a user action in the attestation card. */
  attest(mogId: MogId, kind: 'only_me' | 'others_agreed'): Promise<{ consentToken: string }>;
  records(): Promise<ConsentRecord[]>;
}

const C = 'ConsentService';

export class ConsentServiceStub implements ConsentService {
  async noticeAccepted(): Promise<{ version: number } | null> {
    throw new NotImplementedError(C, 'noticeAccepted', 27); // TODO(#27)
  }
  async attest(
    _mogId: MogId,
    _kind: 'only_me' | 'others_agreed',
  ): Promise<{ consentToken: string }> {
    throw new NotImplementedError(C, 'attest', 64); // TODO(#64)
  }
  async records(): Promise<ConsentRecord[]> {
    throw new NotImplementedError(C, 'records', 63); // TODO(#63)
  }
}

export function createConsentService(): ConsentService {
  return new ConsentServiceStub();
}
