/**
 * T12 ConsentService (4.3, hidden TS brain window).
 *
 * Responsibilities: Versioned notice, per-session attestation, consent records (13 months).
 *
 * #27 ships the first-run notice (9.3, MB-08a, MB-08b, ADR 0044): the accepted versions and the
 * record of each acceptance (version, time, user, device). The capture gate
 * (`src/brain/capture-gate.ts`) reads `noticeState()` before each `capture_start`.
 */
import type { MogId } from '../../shared/interfaces';
import { NOTICE_VERSION, type HubNotice } from '../../shared/hub-notice';
import { NotImplementedError } from '../errors';

export interface ConsentRecord {
  kind: 'notice' | 'only_me' | 'others_agreed';
  version: number;
  at: string;
}

/** One acceptance of the first-run notice (9.3 step 5). It holds no content. */
export interface NoticeRecord {
  kind: 'notice';
  version: number;
  /** ISO 8601 time of the Continue click. */
  at: string;
  /** The signed-in user, or null when no profile is signed in yet (first run). */
  user: string | null;
  /** A random ID of this device, made once. */
  device: string;
}

export interface ConsentService {
  /** The highest accepted notice version, or null before the first-run notice is accepted. */
  noticeAccepted(): Promise<{ version: number } | null>;
  /** The current notice version and whether the user accepted it (MB-08b). */
  noticeState(): Promise<HubNotice>;
  /**
   * Store the record for `version`. Called only on a user click on Continue in the hub.
   * Returns false, and stores nothing, if `version` is not the current version or the store
   * cannot write.
   */
  acceptNotice(version: number): Promise<boolean>;
  /** The stored notice records, oldest first. */
  noticeRecords(): Promise<NoticeRecord[]>;
  /** Called only on a user action in the attestation card. */
  attest(mogId: MogId, kind: 'only_me' | 'others_agreed'): Promise<{ consentToken: string }>;
  records(): Promise<ConsentRecord[]>;
}

/**
 * Where the notice records live: one text value. TODO(#431): move the records to the local store
 * (SEC-32a fields, 13-month purge) once `mem_local_exec` is routed.
 */
export interface ConsentStore {
  read(): string | null;
  write(text: string): void;
}

/** A store in memory, for tests and for a brain with no Web Storage. */
export function memoryConsentStore(initial: string | null = null): ConsentStore {
  let text = initial;
  return {
    read: () => text,
    write: (t) => {
      text = t;
    },
  };
}

/** The Web Storage key of the notice records (ADR 0044). */
export const CONSENT_STORAGE_KEY = 'bottle.consent.notice';

/**
 * A store in the Web Storage of the app origin: local to this device and this OS user.
 * It is NOT private to the brain: the hub, MOG and card views load from the same origin and
 * share this Web Storage, so code in a view could write the record without a Continue click.
 * The views run only bundled code (CSP `script-src 'self'`, no HTML from Genie), which limits this.
 * TODO(#431): before MVP-D, move the record behind the Rust local store, out of view reach.
 */
export function webConsentStore(storage: Storage): ConsentStore {
  return {
    read: () => storage.getItem(CONSENT_STORAGE_KEY),
    write: (t) => storage.setItem(CONSENT_STORAGE_KEY, t),
  };
}

interface StoreData {
  device: string;
  notices: NoticeRecord[];
}

function isRecord(v: unknown): v is NoticeRecord {
  if (typeof v !== 'object' || v === null) return false;
  const r = v as Record<string, unknown>;
  return (
    r['kind'] === 'notice' &&
    typeof r['version'] === 'number' &&
    Number.isInteger(r['version']) &&
    typeof r['at'] === 'string' &&
    (r['user'] === null || typeof r['user'] === 'string') &&
    typeof r['device'] === 'string'
  );
}

/** Parse the stored text. Anything else reads as "no records": the gate fails closed. */
function parse(text: string | null): StoreData | null {
  if (text === null) return null;
  try {
    const v: unknown = JSON.parse(text);
    if (typeof v !== 'object' || v === null || Array.isArray(v)) return null;
    const { device, notices } = v as Record<string, unknown>;
    if (typeof device !== 'string' || !Array.isArray(notices) || !notices.every(isRecord))
      return null;
    return { device, notices };
  } catch {
    return null;
  }
}

export interface ConsentServiceOptions {
  store?: ConsentStore;
  /** The current notice version. Default `NOTICE_VERSION`. */
  version?: number;
  now?: () => Date;
  newId?: () => string;
  /** The signed-in user. TODO(#431): ask T7 ProfileService. */
  user?: () => string | null;
}

const C = 'ConsentService';

export class BrainConsentService implements ConsentService {
  readonly #store: ConsentStore;
  readonly #version: number;
  readonly #now: () => Date;
  readonly #newId: () => string;
  readonly #user: () => string | null;

  constructor(o: ConsentServiceOptions = {}) {
    this.#store = o.store ?? defaultStore();
    this.#version = o.version ?? NOTICE_VERSION;
    this.#now = o.now ?? (() => new Date());
    this.#newId = o.newId ?? (() => crypto.randomUUID());
    this.#user = o.user ?? (() => null);
  }

  #data(): StoreData | null {
    try {
      return parse(this.#store.read());
    } catch {
      return null;
    }
  }

  async noticeAccepted(): Promise<{ version: number } | null> {
    const versions = (this.#data()?.notices ?? []).map((r) => r.version);
    return versions.length ? { version: Math.max(...versions) } : null;
  }

  async noticeState(): Promise<HubNotice> {
    const accepted = (this.#data()?.notices ?? []).some((r) => r.version === this.#version);
    return { version: this.#version, accepted };
  }

  async acceptNotice(version: number): Promise<boolean> {
    if (version !== this.#version) return false;
    const data = this.#data() ?? { device: this.#newId(), notices: [] };
    if (data.notices.some((r) => r.version === version)) return true;
    const record: NoticeRecord = {
      kind: 'notice',
      version,
      at: this.#now().toISOString(),
      user: this.#user(),
      device: data.device,
    };
    try {
      this.#store.write(JSON.stringify({ ...data, notices: [...data.notices, record] }));
    } catch {
      return false;
    }
    return (await this.noticeState()).accepted;
  }

  async noticeRecords(): Promise<NoticeRecord[]> {
    return [...(this.#data()?.notices ?? [])];
  }

  async attest(
    _mogId: MogId,
    _kind: 'only_me' | 'others_agreed',
  ): Promise<{ consentToken: string }> {
    throw new NotImplementedError(C, 'attest', 64); // TODO(#64)
  }

  async records(): Promise<ConsentRecord[]> {
    throw new NotImplementedError(C, 'records', 431); // TODO(#431)
  }
}

/** The brain window's Web Storage, or memory where there is none (Node tests). */
function defaultStore(): ConsentStore {
  if (typeof window !== 'undefined' && window.localStorage)
    return webConsentStore(window.localStorage);
  return memoryConsentStore();
}

export function createConsentService(o?: ConsentServiceOptions): ConsentService {
  return new BrainConsentService(o);
}
