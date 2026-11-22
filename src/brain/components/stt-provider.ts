/**
 * T4 SttProvider (4.3, hidden TS brain window; #52, ADR 0088).
 *
 * Responsibilities: Local STT by default; Databricks ASR as a probe-gated user opt-in; live
 * demotion, fallback chain.
 *
 * - Providers (CAP-47a, CAP-47b): local Parakeet (`local_parakeet`, the default, CAP-57c),
 *   Databricks ASR (`dbx_gemini`) only after the opt-in, and the Whisper endpoint
 *   (`dbx_whisper`) only when the admin configures it. Every decode is one `stt_transcribe`.
 * - The 8.3 PTT chains (CAP-60a). Default: local Parakeet → Whisper (if configured) →
 *   "Could not transcribe. Retry". With the ASR opt-in: Databricks ASR (hedged at 1.2 s) →
 *   local Parakeet → Whisper (if configured) → "Could not transcribe. Retry". A rejected or
 *   invalid reply goes to the next step. The 8.3 Sessions chain (local Parakeet → Databricks ASR
 *   if allowed → queue) is TODO(#116), with the meeting STT path.
 * - `SttPolicy=local_only` removes every cloud step: ASR and Whisper (CAP-60b). An unknown
 *   value or a failed policy read is `local_only`. Offline (ARC-F7) also removes them.
 * - The hedge (CAP-59a, CAP-59b): with no cloud reply in 1.2 s, the local decode of the same
 *   segment starts. The first valid result wins. A late reply changes nothing.
 * - Languages (CAP-64a, CAP-64b, D59): Parakeet v3 covers 25 European languages. A known
 *   uncovered language goes to Databricks ASR when policy permits it, else to the Whisper
 *   endpoint, else T4 rejects with the "not available" text. T4 never resolves an empty
 *   transcript for an uncovered language.
 *
 * Nothing here logs. A failure holds a fixed code, a fixed user text and the segment ID only:
 * never transcript text, audio, a profile or a token. The Rust side adds the token; the user
 * names the ASR profile (Bottle never picks one).
 */
import { call } from '../../shared/ipc';
import type { CommandArgs } from '../../shared/ipc';
import type { MogId, ProfileName, SegmentRef, Tags, Transcript } from '../../shared/interfaces';
import type { NetworkStatus } from './embedding-queue';
import { DEFAULT_ASR_MODEL, modelPolicyFrom, type SttPolicy } from './model-access';
import type { SttUse } from './mog-badges';

type Provider = Transcript['provider'];

/** CAP-59a: the local decode starts when no cloud reply came in this time. */
export const HEDGE_MS = 1_200;

/**
 * A cloud step (ASR or Whisper) with no reply in this time fails, so the chain goes on.
 * TODO(#49): the ASR client sets its own request timeout; this is the T4 backstop.
 */
export const CLOUD_STEP_DEADLINE_MS = 8_000;

/** A local decode with no reply in this time fails, so the chain ends with a message. */
export const LOCAL_STEP_DEADLINE_MS = 15_000;

/** 8.3: Retry works this long after "Could not transcribe" (the audio stays in RAM). */
export const RETRY_WINDOW_MS = 60_000;

/** 8.3: the end of each PTT chain. */
export const COULD_NOT_TRANSCRIBE = 'Could not transcribe. Retry';

/** CAP-64b: no permitted provider covers the language. */
export const LANGUAGE_NOT_AVAILABLE =
  'Transcription of this language is not available on this device.';

/**
 * The languages of Parakeet TDT 0.6B v3 (CAP-47a, COL-64): 25 European languages, as ISO 639-1
 * primary subtags.
 */
// prettier-ignore
export const PARAKEET_LANGUAGES: ReadonlySet<string> = new Set([
  'bg', 'hr', 'cs', 'da', 'nl', 'en', 'et', 'fi', 'fr', 'de', 'el', 'hu', 'it',
  'lv', 'lt', 'mt', 'pl', 'pt', 'ro', 'sk', 'sl', 'es', 'sv', 'ru', 'uk',
]);

/** The primary subtag of a BCP 47 tag, in lower case. Undefined for an empty or bad value. */
function primaryLanguage(tag: unknown): string | undefined {
  if (typeof tag !== 'string') return undefined;
  const p = tag.trim().split(/[-_]/)[0]?.toLowerCase() ?? '';
  return /^[a-z]{2,3}$/.test(p) ? p : undefined;
}

/** True when local Parakeet covers the language (a BCP 47 tag). */
export function coversLanguage(tag: string): boolean {
  const p = primaryLanguage(tag);
  return p !== undefined && PARAKEET_LANGUAGES.has(p);
}

const uncovered = (lang: string | undefined): boolean =>
  lang !== undefined && !PARAKEET_LANGUAGES.has(lang);

// ---------- failures ----------

export type SttFailureKind = 'could_not_transcribe' | 'language_not_available' | 'model_missing';

/**
 * #413, #45: the hub's Local speech step (`SttModelStep`) owns the Download button; this text
 * only points there. It never names a file path or a folder (SEC-03).
 */
export const MODEL_MISSING_TEXT =
  'Download the speech model (487 MB) for private, on-device dictation. Open the Bottle hub, then Local speech.';

const USER_TEXT: Readonly<Record<SttFailureKind, string>> = {
  could_not_transcribe: COULD_NOT_TRANSCRIBE,
  language_not_available: LANGUAGE_NOT_AVAILABLE,
  model_missing: MODEL_MISSING_TEXT,
};

/**
 * Why T4 gives no transcript. The message is the fixed code; `userText` is the fixed text that
 * the MOG shows. It holds no transcript text, no audio and no profile.
 */
export class SttFailure extends Error {
  override readonly name = 'SttFailure';
  readonly kind: SttFailureKind;
  readonly userText: string;
  readonly segmentId: string;
  /** True while `retry` can run the chain again for this segment (8.3, 60 s). */
  readonly retryable: boolean;

  constructor(kind: SttFailureKind, segmentId: string, retryable: boolean) {
    super(`stt_${kind}`);
    this.kind = kind;
    this.userText = USER_TEXT[kind];
    this.segmentId = segmentId;
    this.retryable = retryable;
  }
}

// ---------- config ----------

/** One cloud STT target. The user or the admin names the profile; Bottle never picks one. */
export interface CloudTarget {
  profile: ProfileName;
  model: string;
}

/** What T4 may use for one utterance. */
export interface SttConfig {
  /** The MDM `SttPolicy` (Appendix C). `local_only` removes every cloud step. */
  sttPolicy: SttPolicy;
  /**
   * Databricks ASR. Present only after the user (or MDM `cloud_only`) selected it and the
   * probe passed (CAP-57e).
   */
  asr?: CloudTarget;
  /** The admin Whisper endpoint (CAP-47b). Present only when the admin configures it. */
  whisper?: CloudTarget;
}

export interface SttConfigSource {
  read(): Promise<SttConfig>;
}

/** The Databricks ASR opt-in from #51: the profile the user named, and an optional model. */
export interface AsrOptIn {
  profile: ProfileName;
  model?: string;
}

export interface IpcSttConfigDeps {
  /**
   * The ASR opt-in, or undefined. TODO(#51): the probe (CAP-57a, CAP-57b), the user's choice
   * (CAP-57e), MDM `cloud_only`, live demotion (CAP-58) and the geography check (CAP-63)
   * decide it. Default: undefined, so Databricks ASR stays off.
   */
  asr?: () => Promise<AsrOptIn | undefined>;
  /**
   * The admin Whisper endpoint, or undefined. TODO(#530): Appendix C has no key for it yet.
   * Default: undefined, so no Whisper call occurs (CAP-47b).
   */
  whisper?: () => Promise<CloudTarget | undefined>;
}

const LOCAL_ONLY: SttConfig = { sttPolicy: 'local_only' };

async function optional<T>(f: (() => Promise<T | undefined>) | undefined): Promise<T | undefined> {
  if (!f) return undefined;
  try {
    return await f();
  } catch {
    return undefined;
  }
}

/**
 * The production config: `SttPolicy` and `AsrModelService` from `policy_get`, the opt-in and
 * the Whisper endpoint from their sources. A failed policy read is `local_only` (CAP-60b).
 */
export function ipcSttConfig(deps: IpcSttConfigDeps = {}): SttConfigSource {
  return {
    async read() {
      let mdm: unknown;
      try {
        mdm = await call('policy_get', {});
      } catch {
        return LOCAL_ONLY;
      }
      const policy = modelPolicyFrom(mdm);
      if (policy.sttPolicy === 'local_only') return LOCAL_ONLY;
      const [optIn, whisper] = await Promise.all([optional(deps.asr), optional(deps.whisper)]);
      const cfg: SttConfig = { sttPolicy: policy.sttPolicy };
      if (optIn && typeof optIn.profile === 'string' && optIn.profile !== '') {
        cfg.asr = {
          profile: optIn.profile,
          model: optIn.model ?? policy.models.asr ?? DEFAULT_ASR_MODEL,
        };
      }
      if (whisper && whisper.profile !== '' && whisper.model !== '') cfg.whisper = whisper;
      return cfg;
    },
  };
}

// ---------- the provider ----------

export interface TranscribeOptions {
  /** The language of the utterance (BCP 47), when the caller knows it. */
  language?: string;
}

export interface SttProvider {
  /** Runs the 8.3 chain for one final segment. Rejects only with an `SttFailure`. */
  transcribe(seg: SegmentRef, tags: Tags, opts?: TranscribeOptions): Promise<Transcript>;
  /** "Retry" after "Could not transcribe": the chain again, for 60 s after the failure. */
  retry(seg: SegmentRef, tags: Tags, opts?: TranscribeOptions): Promise<Transcript>;
  /** The head of the current chain. */
  activeProvider(): Provider;
  /**
   * The STT badge source of one MOG (UX-38): the provider of its last transcript, or the
   * selected provider before one.
   */
  sttUse(mogId: MogId): SttUse;
}

export type SttInvoke = (args: CommandArgs['stt_transcribe']) => Promise<Transcript>;

export interface SttProviderDeps {
  /** Default: `ipcSttConfig()` (no ASR opt-in, no Whisper). */
  config?: SttConfigSource;
  /** Default: `stt_transcribe` over IPC. */
  invoke?: SttInvoke;
  /**
   * Offline (ARC-F7) removes the cloud steps. Default: online, so a cloud step fails and the
   * chain goes on. TODO(#550): the brain passes the `net.status` NetworkStatus.
   */
  net?: Pick<NetworkStatus, 'online'>;
  /** Default: `Date.now`. */
  now?: () => number;
}

/** The most failed segments that wait for a Retry. */
const MAX_RETRY_ENTRIES = 32;

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

/** The `{ error: <code> }` rejection shape of a refused IPC call; `''` for anything else. */
const errorCode = (e: unknown): string => {
  const c = isRecord(e) ? e['error'] : undefined;
  return typeof c === 'string' ? c : '';
};

/** A reply is valid for the segment and the provider it asked; cloud text is never empty. */
function validReply(t: unknown, seg: SegmentRef, provider: Provider): Transcript | undefined {
  if (!isRecord(t)) return undefined;
  if (t['segmentId'] !== seg.segmentId || t['provider'] !== provider) return undefined;
  const { text, raw, model, runtime, latencyMs, lang } = t;
  if (typeof text !== 'string' || typeof raw !== 'string') return undefined;
  if (provider !== 'local_parakeet' && text.trim() === '') return undefined;
  const out: Transcript = {
    segmentId: seg.segmentId,
    text,
    raw,
    provider,
    model: typeof model === 'string' ? model : '',
    runtime: typeof runtime === 'string' ? runtime : '',
    latencyMs: typeof latencyMs === 'number' && Number.isFinite(latencyMs) ? latencyMs : 0,
  };
  if (typeof lang === 'string' && lang !== '') out.lang = lang;
  // ADR 0113: Rust parsed "remember this" and minted the ticket. Keep both, or neither.
  const r = t['remember'];
  if (isRecord(r) && typeof r['ticket'] === 'string' && typeof r['text'] === 'string') {
    if (r['ticket'] !== '' && r['text'].trim() !== '') {
      out.remember = { ticket: r['ticket'], text: r['text'] };
    }
  }
  return out;
}

class ChainProvider implements SttProvider {
  private readonly config: SttConfigSource;
  private readonly invoke: SttInvoke;
  private readonly net: Pick<NetworkStatus, 'online'> | undefined;
  private readonly now: () => number;
  private active: Provider = 'local_parakeet';
  private activeModel = '';
  /** #413, #45: the local decode of the current `run` failed with `stt_model_missing`. */
  private modelMissing = false;
  /** MOG → the provider of its last transcript (UX-38). Ids only; never text. */
  private readonly last = new Map<MogId, SttUse>();
  /** segmentId → the running Retry, so a second click starts no second chain. */
  private readonly retrying = new Map<string, Promise<Transcript>>();
  /** segmentId → the time of its "Could not transcribe". */
  private readonly failedAt = new Map<string, number>();

  constructor(deps: SttProviderDeps) {
    this.config = deps.config ?? ipcSttConfig();
    this.invoke = deps.invoke ?? ((args) => call('stt_transcribe', args));
    this.net = deps.net;
    this.now = deps.now ?? (() => Date.now());
  }

  activeProvider(): Provider {
    return this.active;
  }

  sttUse(mogId: MogId): SttUse {
    return this.last.get(mogId) ?? { provider: this.active, model: this.activeModel };
  }

  async transcribe(seg: SegmentRef, tags: Tags, opts?: TranscribeOptions): Promise<Transcript> {
    return this.run(seg, tags, opts);
  }

  async retry(seg: SegmentRef, tags: Tags, opts?: TranscribeOptions): Promise<Transcript> {
    this.prune();
    const at = this.failedAt.get(seg.segmentId);
    if (at === undefined) throw new SttFailure('could_not_transcribe', seg.segmentId, false);
    const running = this.retrying.get(seg.segmentId);
    if (running) return running;
    // TODO(#531): R4 keeps the audio of a failed decode in RAM for 60 s, so this decodes it again.
    // Until then the host has no audio for a Retry, and the Retry control (#60) waits for #531.
    const p = this.run(seg, tags, opts).finally(() => this.retrying.delete(seg.segmentId));
    this.retrying.set(seg.segmentId, p);
    return p;
  }

  private prune(): void {
    const t = this.now();
    for (const [id, at] of this.failedAt) {
      if (t - at > RETRY_WINDOW_MS) this.failedAt.delete(id);
    }
  }

  private async readConfig(): Promise<SttConfig> {
    try {
      const c = await this.config.read();
      if (c.sttPolicy === 'local_only') return LOCAL_ONLY;
      if (this.net && !this.net.online()) return { sttPolicy: c.sttPolicy };
      return c;
    } catch {
      return LOCAL_ONLY;
    }
  }

  /** One decode. Undefined for a rejection or an invalid reply; it never throws. */
  private async attempt(
    provider: Provider,
    seg: SegmentRef,
    tags: Tags,
    target: CloudTarget | undefined,
    lang: string | undefined,
  ): Promise<Transcript | undefined> {
    const args: CommandArgs['stt_transcribe'] = { segmentId: seg.segmentId, provider, tags };
    if (target) {
      args.profile = target.profile;
      args.model = target.model;
    }
    if (lang !== undefined && provider !== 'local_parakeet') args.language = lang;
    const deadline =
      provider === 'local_parakeet' ? LOCAL_STEP_DEADLINE_MS : CLOUD_STEP_DEADLINE_MS;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<undefined>((resolve) => {
      timer = setTimeout(() => resolve(undefined), deadline);
    });
    try {
      const reply = this.invoke(args).then(
        (t) => validReply(t, seg, provider),
        (e: unknown) => {
          // #413, #45: R4 has no installed, checked model. The hub's Download step (not a
          // Retry here) is the way out, so the local failure is marked for `run` to report.
          if (provider === 'local_parakeet' && errorCode(e) === 'stt_model_missing') {
            this.modelMissing = true;
          }
          return undefined;
        },
      );
      // A step with no reply by its deadline fails; a late reply changes nothing.
      return await Promise.race([reply, timeout]);
    } catch {
      return undefined;
    } finally {
      clearTimeout(timer);
    }
  }

  /**
   * CAP-59a, CAP-59b: the cloud call, and the local decode of the same segment if no cloud
   * reply came in 1.2 s (or at once after a cloud failure). The first valid result wins. A local
   * result in an uncovered language is not valid; its language goes back to the chain. An empty
   * local result ("no speech") wins only when the cloud call fails: it never hides cloud text.
   */
  private hedge(
    seg: SegmentRef,
    tags: Tags,
    asr: CloudTarget,
    lang: string | undefined,
    skipLocal: boolean,
  ): Promise<{ t?: Transcript; localTried: boolean; lang?: string }> {
    return new Promise((resolve) => {
      let done = false;
      let cloudDone = false;
      let local: 'idle' | 'running' | 'done' = 'idle';
      let detected: string | undefined;
      let emptyLocal: Transcript | undefined;
      const finish = (t?: Transcript) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        resolve({
          ...(t ? { t } : {}),
          localTried: local !== 'idle',
          ...(detected !== undefined ? { lang: detected } : {}),
        });
      };
      const settleIfBothDone = () => {
        if (cloudDone && (local === 'done' || skipLocal)) finish(emptyLocal);
      };
      const startLocal = () => {
        if (done || skipLocal || local !== 'idle') return;
        local = 'running';
        void this.attempt('local_parakeet', seg, tags, undefined, undefined).then((t) => {
          local = 'done';
          const l = primaryLanguage(t?.lang);
          if (t && !uncovered(l)) {
            if (t.text.trim() !== '') return finish(t);
            // "No speech" waits for the cloud call: it wins only if that call fails.
            emptyLocal = t;
            return settleIfBothDone();
          }
          if (uncovered(l)) detected = l;
          settleIfBothDone();
        });
      };
      const timer = setTimeout(startLocal, HEDGE_MS);
      void this.attempt('dbx_gemini', seg, tags, asr, lang).then((t) => {
        cloudDone = true;
        if (t) return finish(t);
        clearTimeout(timer);
        if (local === 'idle' && !skipLocal) startLocal();
        else settleIfBothDone();
      });
    });
  }

  private async run(seg: SegmentRef, tags: Tags, opts?: TranscribeOptions): Promise<Transcript> {
    const cfg = await this.readConfig();
    const asr = cfg.asr;
    const whisper = cfg.whisper;
    this.active = asr ? 'dbx_gemini' : 'local_parakeet';
    this.activeModel = asr ? asr.model : '';
    // TODO(#531): R4 fills `Transcript.lang`; until then only a caller hint gives the language.
    let lang = primaryLanguage(opts?.language);
    let localTried = false;
    this.modelMissing = false;

    const done = (t: Transcript): Transcript => {
      this.failedAt.delete(seg.segmentId);
      this.last.set(seg.mogId, { provider: t.provider, model: t.model });
      if (t.lang === undefined && lang !== undefined) return { ...t, lang };
      return t;
    };

    // Step 1 with the opt-in: Databricks ASR, hedged with local (8.3, CAP-59a).
    if (asr) {
      const r = await this.hedge(seg, tags, asr, lang, uncovered(lang));
      if (r.lang !== undefined) lang = r.lang;
      if (r.t) return done(r.t);
      localTried = r.localTried;
    }
    // Local Parakeet, unless the language is one it does not cover (CAP-64a).
    if (!localTried && !uncovered(lang)) {
      const t = await this.attempt('local_parakeet', seg, tags, undefined, undefined);
      const l = primaryLanguage(t?.lang);
      if (t && !uncovered(l)) return done(t);
      if (uncovered(l)) lang = l;
    }
    // The admin Whisper endpoint, if configured (CAP-47b).
    if (whisper) {
      const t = await this.attempt('dbx_whisper', seg, tags, whisper, lang);
      if (t) return done(t);
    }
    // CAP-64b: no permitted provider covers the language.
    if (uncovered(lang) && !asr && !whisper) {
      throw new SttFailure('language_not_available', seg.segmentId, false);
    }
    // #413, #45: local Parakeet has no installed, checked model, and nothing else came back
    // with a transcript. Point at the hub's Download step instead of the generic "Could not
    // transcribe. Retry" (a Retry alone cannot fix this); a configured ASR opt-in may still
    // have failed for its own reason, but the local path stays blocked until the model lands.
    if (this.modelMissing && !whisper) {
      throw new SttFailure('model_missing', seg.segmentId, false);
    }
    this.prune();
    if (!this.failedAt.has(seg.segmentId) && this.failedAt.size >= MAX_RETRY_ENTRIES) {
      const oldest = this.failedAt.keys().next().value;
      if (oldest !== undefined) this.failedAt.delete(oldest);
    }
    if (!this.failedAt.has(seg.segmentId)) this.failedAt.set(seg.segmentId, this.now());
    throw new SttFailure('could_not_transcribe', seg.segmentId, true);
  }
}

export function createSttProvider(deps: SttProviderDeps = {}): SttProvider {
  return new ChainProvider(deps);
}
