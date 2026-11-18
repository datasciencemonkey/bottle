/**
 * T12 ConsentService (4.3, hidden TS brain window).
 *
 * Responsibilities: Versioned notice, per-session attestation, consent records (13 months).
 *
 * #27 ships the first-run notice (9.3, MB-08a, MB-08b, ADR 0044). The capture gate
 * (`src/brain/capture-gate.ts`) reads `noticeState()` before each `capture_start`.
 *
 * #431 (ADR 0071): the records live only in the Rust local store (`mog_core::consent`, ADR 0052).
 * T12 reaches them through `consent_notice_record` and `consent_records_list` (#430, ADR 0060),
 * which only the brain window may call. The brain sends only the notice version; Rust fills the
 * user, the device and the time (SEC-32a). The brain sees content-free views only. No record is
 * in Web Storage, IndexedDB or a cookie: the hub, MOG and card views share those with the brain.
 * The user comes from the Rust `SignedInUser` seam, not from the brain.
 * TODO(#478): the host gives the signed-in user from T7 ProfileService (#17).
 */
import type { MogId } from '../../shared/interfaces';
import { NOTICE_VERSION, type HubNotice } from '../../shared/hub-notice';
import { call, type ConsentRecordView } from '../../shared/ipc';
import { NotImplementedError } from '../errors';

/** One stored consent record, as `consent_records_list` returns it: content-free. */
export type ConsentRecord = ConsentRecordView;

export interface ConsentService {
  /** The highest accepted notice version, or null before the first-run notice is accepted. */
  noticeAccepted(): Promise<{ version: number } | null>;
  /** The current notice version and whether the user accepted it (MB-08b). */
  noticeState(): Promise<HubNotice>;
  /**
   * Store the record for `version`. Called only on a user click on Continue in the hub.
   * Returns false, and stores nothing, if `version` is not the current version or the store
   * cannot write. A version with a stored record gets no second record.
   */
  acceptNotice(version: number): Promise<boolean>;
  /** Called only on a user action in the attestation card. */
  attest(mogId: MogId, kind: 'only_me' | 'others_agreed'): Promise<{ consentToken: string }>;
  /** The stored consent records, oldest first (SEC-32a). The 13-month purge removes old ones. */
  records(): Promise<ConsentRecord[]>;
}

/** The Rust consent records, as the brain reaches them (#430). */
export interface ConsentRecordsPort {
  recordNotice(noticeVersion: number): Promise<{ recordId: string }>;
  list(): Promise<ConsentRecordView[]>;
}

/** The production port: the #430 IPC commands. */
export function ipcConsentRecords(): ConsentRecordsPort {
  return {
    recordNotice: (noticeVersion) => call('consent_notice_record', { noticeVersion }),
    list: async () => (await call('consent_records_list', {})).records,
  };
}

/**
 * The Web Storage key where the #27 build kept the notice record (ADR 0044). A view could write
 * it, so T12 never reads it: it removes it at start, and the user accepts the notice once more.
 */
export const LEGACY_CONSENT_STORAGE_KEY = 'bottle.consent.notice';

export interface ConsentServiceOptions {
  /** Default: the #430 IPC commands. */
  records?: ConsentRecordsPort;
  /** The current notice version. Default `NOTICE_VERSION`. */
  version?: number;
  /** Where the #27 record may still be. Default: the brain window's Web Storage, if any. */
  legacyStorage?: Pick<Storage, 'removeItem'> | null;
}

const C = 'ConsentService';
const KINDS: readonly string[] = ['notice', 'only_me', 'others_agreed'];

const isPositiveInt = (v: unknown): v is number =>
  typeof v === 'number' && Number.isInteger(v) && v > 0;

/** One `ConsentRecordView` of the #430 contract, or false. */
function isView(v: unknown): v is ConsentRecordView {
  if (typeof v !== 'object' || v === null) return false;
  const r = v as Record<string, unknown>;
  const notice = r['kind'] === 'notice';
  return (
    typeof r['recordId'] === 'string' &&
    typeof r['kind'] === 'string' &&
    KINDS.includes(r['kind']) &&
    isPositiveInt(r['noticeVersion']) &&
    (notice ? r['sessionId'] === null : typeof r['sessionId'] === 'string') &&
    typeof r['acceptedAt'] === 'number' &&
    Number.isFinite(r['acceptedAt'])
  );
}

const STORAGE_FAILED = { error: 'consent_storage_failed' } as const;

export class BrainConsentService implements ConsentService {
  readonly #records: ConsentRecordsPort;
  readonly #version: number;
  #accepting: Promise<boolean> | undefined;

  constructor(o: ConsentServiceOptions = {}) {
    this.#records = o.records ?? ipcConsentRecords();
    this.#version = o.version ?? NOTICE_VERSION;
    dropLegacyRecord(o.legacyStorage === undefined ? brainStorage() : o.legacyStorage);
  }

  /** The stored records. A reply that is not the #430 shape is a storage failure. */
  async #list(): Promise<ConsentRecordView[]> {
    const list: unknown = await this.#records.list();
    if (!Array.isArray(list) || !list.every(isView)) throw STORAGE_FAILED;
    return list;
  }

  /** The accepted notice versions, or none when the store cannot be read: the gate fails closed. */
  async #noticeVersions(): Promise<number[]> {
    try {
      return (await this.#list()).filter((r) => r.kind === 'notice').map((r) => r.noticeVersion);
    } catch {
      return [];
    }
  }

  async noticeAccepted(): Promise<{ version: number } | null> {
    const versions = await this.#noticeVersions();
    return versions.length ? { version: Math.max(...versions) } : null;
  }

  async noticeState(): Promise<HubNotice> {
    const accepted = (await this.#noticeVersions()).includes(this.#version);
    return { version: this.#version, accepted };
  }

  async acceptNotice(version: number): Promise<boolean> {
    if (version !== this.#version || !isPositiveInt(version)) return false;
    // Overlapping calls (a fast double click, two hubs) share one write: one record per version.
    this.#accepting ??= this.#accept(version).finally(() => {
      this.#accepting = undefined;
    });
    return this.#accepting;
  }

  async #accept(version: number): Promise<boolean> {
    if ((await this.noticeState()).accepted) return true;
    try {
      await this.#records.recordNotice(version);
    } catch {
      return false;
    }
    return (await this.noticeState()).accepted;
  }

  async attest(
    _mogId: MogId,
    _kind: 'only_me' | 'others_agreed',
  ): Promise<{ consentToken: string }> {
    throw new NotImplementedError(C, 'attest', 64); // TODO(#64)
  }

  async records(): Promise<ConsentRecord[]> {
    return [...(await this.#list())];
  }
}

/** The brain window's Web Storage, or null where there is none (Node tests). */
function brainStorage(): Pick<Storage, 'removeItem'> | null {
  return typeof window !== 'undefined' && window.localStorage ? window.localStorage : null;
}

function dropLegacyRecord(storage: Pick<Storage, 'removeItem'> | null): void {
  try {
    storage?.removeItem(LEGACY_CONSENT_STORAGE_KEY);
  } catch {
    // Web Storage is off: there is nothing to remove.
  }
}

export function createConsentService(o?: ConsentServiceOptions): ConsentService {
  return new BrainConsentService(o);
}
