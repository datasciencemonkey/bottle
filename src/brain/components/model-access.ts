/**
 * The model service access check (#20, ADR 0074; ADM-04a, ADM-04b, ADM-09, AT-36).
 *
 * The admin gives access to 7 model services on the home workspace (ADM-04a, checklist item 5):
 * ASR, cleanup, router, writer, judge, embedding and the main agent model. Bottle checks each
 * one through a `ModelServiceProbe` and applies a fallback when access is missing:
 *
 * - ASR without access, blocked by cross-geography routing (ADM-04b) or with inference tables on
 *   (ADM-09): the effective `SttPolicy` is `local_only`, so STT runs on this device.
 * - The main agent model without access: the fallback classifier routes (RTR-15b, B4).
 * - Cleanup: the pre-pass text only (ARC-F7). Router step: the heuristic fallback (RTR-13d).
 *   Embeddings: the outbox queue (MEM-19d). Writer and judge: off.
 *
 * The check probes ASR only when Databricks ASR is selected by the user or by MDM `cloud_only`.
 * With local STT (the default, CAP-57c) or MDM `local_only` (CAP-60b) no ASR probe leaves the
 * device, and local STT needs no admin action (ADM-04b).
 *
 * The report is content-free: fixed codes and fixed status texts, never a probe error message,
 * a URL or a token. The check never throws for a probe failure. Bottle never picks a profile:
 * the caller's probe is bound to the home profile.
 */
import type { SttUse } from './mog-badges';

/** The model services of ADM-04a, in the order of the report. */
export const MODEL_SERVICES = [
  'asr',
  'cleanup',
  'router',
  'writer',
  'judge',
  'embed',
  'agent',
] as const;

export type ModelService = (typeof MODEL_SERVICES)[number];

/** The default Databricks ASR model (CAP-62a). MDM `AsrModelService` replaces it. */
export const DEFAULT_ASR_MODEL = 'system.ai.gemini-3-1-flash-lite';

/** The MDM `SttPolicy` key (Appendix C). */
export type SttPolicy = 'cloud_allowed' | 'local_only' | 'cloud_only';

/**
 * One probe result. `no_access`: the service refused the home profile (no permission, or the
 * service does not exist). `geo_blocked`: cross-geography routing or "Enforce data processing
 * within workspace Geography" blocks the service (ADM-04b, CAP-63). `unreachable`: offline or
 * another failure. `not_checked`: no probe ran.
 */
export type ServiceAccess = 'ok' | 'no_access' | 'geo_blocked' | 'unreachable' | 'not_checked';

/** The inference-table state of the ASR service config (ADM-09). */
export type InferenceTables = 'off' | 'on' | 'unknown';

/** What one probe checks. `model` is the MDM `*ModelService` name; unset means the default. */
export interface ProbeTarget {
  service: ModelService;
  model?: string;
}

/**
 * The probe of the model services on the home profile. Each method may throw; the check treats
 * a throw as `unreachable` (or `unknown`). A probe sends no user content.
 */
export interface ModelServiceProbe {
  access(t: ProbeTarget): Promise<ServiceAccess>;
  /** Reads the ASR service config. Called only when the ASR access probe passed. */
  asrInferenceTables(t: ProbeTarget): Promise<InferenceTables>;
}

/** The STT and model keys of Appendix C. */
export interface ModelPolicy {
  sttPolicy: SttPolicy;
  models: Partial<Record<ModelService, string>>;
}

export interface AccessCheckInput {
  policy: ModelPolicy;
  /** True when the user selected Databricks ASR (CAP-57e). Local STT is the default. */
  asrSelected: boolean;
}

/** What Bottle does instead of the service. */
export type ServiceFallback =
  | 'none'
  | 'local_stt'
  | 'prepass_only'
  | 'heuristic_router'
  | 'writer_off'
  | 'judge_off'
  | 'embedding_queue'
  | 'fallback_classifier';

/** Why the effective `SttPolicy` is `local_only`. */
export type SttReason =
  'policy' | 'no_access' | 'geo_blocked' | 'unreachable' | 'inference_tables_on' | 'not_verified';

export interface ServiceStatus {
  service: ModelService;
  access: ServiceAccess;
  fallback: ServiceFallback;
  /** A fixed status text for the user and the admin. */
  text: string;
}

export interface AccessReport {
  /** The effective `SttPolicy` after the check. */
  sttPolicy: SttPolicy;
  /** Why `sttPolicy` is `local_only`; `null` when the MDM policy applies unchanged. */
  sttReason: SttReason | null;
  agentRoute: 'main_agent' | 'fallback_classifier';
  asrInferenceTables: InferenceTables | 'not_checked';
  services: ServiceStatus[];
  /** The `MogStatus.localOnly` input of the MoodMapper. */
  localOnly: boolean;
  /** The STT badge input (UX-38). Local means "On this device" (CAP-63). */
  stt: SttUse;
}

// ---------- the MDM keys (Appendix C) ----------

const STT_POLICIES: readonly SttPolicy[] = ['cloud_allowed', 'local_only', 'cloud_only'];

/** The Appendix C key of each model service name. Judge and the agent have no key. */
const MODEL_KEYS: Readonly<Partial<Record<ModelService, string>>> = {
  asr: 'AsrModelService',
  cleanup: 'CleanupModelService',
  router: 'RouterModelService',
  writer: 'WriterModelService',
  embed: 'EmbeddingModelService',
};

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

/**
 * The STT and model keys of the MDM policy (`policy_get`). No `SttPolicy` means the default
 * `cloud_allowed`. A value Bottle does not know means `local_only`, the safe side (CAP-60b).
 */
export function modelPolicyFrom(mdm: unknown): ModelPolicy {
  const p = isRecord(mdm) ? mdm : {};
  const raw = p['SttPolicy'];
  const sttPolicy: SttPolicy =
    raw === undefined
      ? 'cloud_allowed'
      : STT_POLICIES.includes(raw as SttPolicy)
        ? (raw as SttPolicy)
        : 'local_only';
  const models: Partial<Record<ModelService, string>> = {};
  for (const [service, key] of Object.entries(MODEL_KEYS) as [ModelService, string][]) {
    const v = p[key];
    if (typeof v === 'string' && v.trim() !== '') models[service] = v.trim();
  }
  return { sttPolicy, models };
}

// ---------- status texts ----------

const ADMIN = 'Ask your admin to give access (see the admin prerequisites).';

/** The fallback and the text of a small-model or agent service without access. */
const NO_ACCESS: Readonly<Record<Exclude<ModelService, 'asr'>, [ServiceFallback, string]>> = {
  cleanup: ['prepass_only', 'Cleanup is not available. Bottle keeps the text without cleanup.'],
  router: ['heuristic_router', 'The router model is not available. Bottle uses a simple rule.'],
  writer: ['writer_off', 'The meeting writer model is not available. Meeting notes are off.'],
  judge: ['judge_off', 'The judge model is not available. Answer checks are off.'],
  embed: ['embedding_queue', 'The embedding model is not available. New memory waits in a queue.'],
  agent: [
    'fallback_classifier',
    'The main agent model is not available. Bottle routes with the fallback classifier.',
  ],
};

const SERVICE_OK = 'Available.';
const LOCAL = 'Speech to text runs on this device.';

const ASR_TEXT: Readonly<Record<SttReason, string>> = {
  policy: `${LOCAL} Your admin set local-only speech to text.`,
  no_access: `Databricks speech to text is not available. ${LOCAL} ${ADMIN}`,
  geo_blocked:
    `Databricks speech to text is blocked in this region. ${LOCAL} Ask your admin to turn on ` +
    'cross-geography routing for the ASR service.',
  unreachable: `Databricks speech to text cannot be reached. ${LOCAL}`,
  inference_tables_on: `Inference tables are on for the ASR service. ${LOCAL} Ask your admin to turn them off.`,
  not_verified: `Databricks speech to text is not verified yet. ${LOCAL}`,
};

const ASR_TABLES_UNKNOWN =
  'Available. The ASR inference tables are not verified; your admin must keep them off.';

// ---------- the check ----------

async function probeAccess(probe: ModelServiceProbe, t: ProbeTarget): Promise<ServiceAccess> {
  try {
    return await probe.access(t);
  } catch {
    return 'unreachable';
  }
}

async function probeTables(probe: ModelServiceProbe, t: ProbeTarget): Promise<InferenceTables> {
  try {
    return await probe.asrInferenceTables(t);
  } catch {
    return 'unknown';
  }
}

const target = (service: ModelService, policy: ModelPolicy): ProbeTarget => {
  const model = policy.models[service];
  return model === undefined ? { service } : { service, model };
};

/** The status of a non-ASR service. `not_checked` is not a failure: no fallback applies. */
function serviceStatus(
  service: Exclude<ModelService, 'asr'>,
  access: ServiceAccess,
): ServiceStatus {
  if (access === 'ok' || access === 'not_checked') {
    return {
      service,
      access,
      fallback: 'none',
      text: access === 'ok' ? SERVICE_OK : 'Not checked.',
    };
  }
  const [fallback, text] = NO_ACCESS[service];
  return { service, access, fallback, text: access === 'unreachable' ? text : `${text} ${ADMIN}` };
}

/**
 * Checks the 7 model services (ADM-04a) and returns the effective STT policy, the agent route
 * and a status for each service. All probes run in parallel. It never throws.
 */
export async function checkModelServices(
  input: AccessCheckInput,
  probe: ModelServiceProbe,
): Promise<AccessReport> {
  const { policy } = input;
  const others = MODEL_SERVICES.filter((s): s is Exclude<ModelService, 'asr'> => s !== 'asr');
  const wantsAsr =
    policy.sttPolicy !== 'local_only' && (input.asrSelected || policy.sttPolicy === 'cloud_only');

  const asrCheck = (async () => {
    if (!wantsAsr) return { access: 'not_checked' as const, tables: 'not_checked' as const };
    const t = target('asr', policy);
    const access = await probeAccess(probe, t);
    const tables = access === 'ok' ? await probeTables(probe, t) : ('not_checked' as const);
    return { access, tables };
  })();
  const [asr, ...rest] = await Promise.all([
    asrCheck,
    ...others.map((s) => probeAccess(probe, target(s, policy))),
  ]);

  let sttReason: SttReason | null = null;
  if (policy.sttPolicy === 'local_only') sttReason = 'policy';
  else if (wantsAsr) {
    if (asr.access === 'ok') sttReason = asr.tables === 'on' ? 'inference_tables_on' : null;
    else if (asr.access === 'not_checked') sttReason = 'not_verified';
    else sttReason = asr.access;
  }
  const sttPolicy: SttPolicy = sttReason === null ? policy.sttPolicy : 'local_only';
  const cloudAsr = wantsAsr && sttReason === null;

  let asrStatus: ServiceStatus;
  if (sttReason !== null) {
    asrStatus = {
      service: 'asr',
      access: asr.access,
      fallback: 'local_stt',
      text: ASR_TEXT[sttReason],
    };
  } else if (cloudAsr) {
    asrStatus = {
      service: 'asr',
      access: asr.access,
      fallback: 'none',
      text: asr.tables === 'unknown' ? ASR_TABLES_UNKNOWN : SERVICE_OK,
    };
  } else {
    // Local STT is selected: nothing to check and no admin action (ADM-04b).
    asrStatus = { service: 'asr', access: 'not_checked', fallback: 'none', text: LOCAL };
  }

  const services = [asrStatus, ...others.map((s, i) => serviceStatus(s, rest[i] ?? 'unreachable'))];
  const agent = services.find((s) => s.service === 'agent');
  const model = policy.models.asr ?? DEFAULT_ASR_MODEL;
  return {
    sttPolicy,
    sttReason,
    agentRoute: agent?.fallback === 'fallback_classifier' ? 'fallback_classifier' : 'main_agent',
    asrInferenceTables: asr.tables,
    services,
    localOnly: sttPolicy === 'local_only',
    stt: cloudAsr ? { provider: 'dbx_gemini', model } : { provider: 'local_parakeet', model: '' },
  };
}

/**
 * The default probe until the model path is wired. Every service is `not_checked`, so Databricks
 * ASR stays off (CAP-57e) and no other fallback applies.
 * TODO(#19): probe each service through `model_invoke` and the AI Gateway on the home profile.
 * That needs a content-free IPC code for a refused service (access denied, geography blocked)
 * and a route that reads the ASR inference-table config (ADR 0074).
 */
export function createModelServiceProbe(): ModelServiceProbe {
  return {
    access: async () => 'not_checked',
    asrInferenceTables: async () => 'unknown',
  };
}
