// The Rust consent records (#63, #430, ADR 0060) as a test double for T12 ConsentService (#431).
// It answers as `consent_notice_record` and `consent_records_list` do: Rust stamps the record ID
// and the time; the brain sends only the notice version and sees only content-free views.

import type { CommandArgs, CommandReturns, ConsentRecordView } from '../shared/ipc';

export interface ConsentRecordsDouble {
  recordNotice(
    noticeVersion: CommandArgs['consent_notice_record']['noticeVersion'],
  ): Promise<CommandReturns['consent_notice_record']>;
  list(): Promise<ConsentRecordView[]>;
  /** The records in the Rust store, oldest first. */
  readonly stored: ConsentRecordView[];
  /** The version of each `consent_notice_record` call, in order. */
  readonly writes: number[];
  /** SEC-15b: the WipeService purge removes the records accepted at or before `cutoffMs`. */
  purge(cutoffMs: number): void;
  /** Make the next calls fail as `consent_storage_failed`. */
  fail(on: { write?: boolean; read?: boolean }): void;
}

const STORAGE_FAILED = { error: 'consent_storage_failed' };

export function consentRecordsDouble(now: () => number = () => 0): ConsentRecordsDouble {
  const stored: ConsentRecordView[] = [];
  const writes: number[] = [];
  let failWrite = false;
  let failRead = false;
  let n = 0;
  return {
    stored,
    writes,
    async recordNotice(noticeVersion) {
      writes.push(noticeVersion);
      if (failWrite) throw STORAGE_FAILED;
      n += 1;
      const recordId = `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
      stored.push({ recordId, kind: 'notice', noticeVersion, sessionId: null, acceptedAt: now() });
      return { recordId };
    },
    async list() {
      if (failRead) throw STORAGE_FAILED;
      return stored.map((r) => ({ ...r }));
    },
    purge(cutoffMs) {
      for (let i = stored.length - 1; i >= 0; i -= 1) {
        if ((stored[i]?.acceptedAt ?? 0) <= cutoffMs) stored.splice(i, 1);
      }
    },
    fail(on) {
      failWrite = on.write ?? failWrite;
      failRead = on.read ?? failRead;
    },
  };
}
