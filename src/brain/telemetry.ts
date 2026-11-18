/**
 * The brain client of the R13 telemetry gate (#184, SEC-16b, D42, ADR 0079).
 *
 * A span is a name from `SPAN_NAMES`, times, and whole counts under keys from `SPAN_ATTR_KEYS`.
 * The client checks the same closed sets as `mog_core::telemetry` and sends `telemetry_span`.
 * It never takes a text field, and errors never echo a value. Rust drops the span when the
 * admin has not set `OtlpTable` (SEC-16a), so the brain does not read the policy here.
 *
 * TODO(#502): `telemetry_span` reaches the Rust gate once the host routes it.
 */
import { call, type CommandArgs, type CommandReturns } from '../shared/ipc';
import { SIZE_KEYS } from './audit-log';

/** The span names. The same closed set as `mog_core::telemetry::SPAN_NAMES`. */
export const SPAN_NAMES = [
  'agent.tool',
  'agent.turn',
  'app.start',
  'card.render',
  'cleanup.run',
  'context.assemble',
  'genie.ask',
  'genie.poll',
  'insert.text',
  'memory.recall',
  'memory.write',
  'page.propose',
  'ptt.capture',
  'ptt.preview',
  'router.run',
  'session.capture',
  'stt.transcribe',
  'wipe.run',
] as const;
export type SpanName = (typeof SPAN_NAMES)[number];

/** The attribute keys: the audit size names (#140) and `EXTRA_ATTR_KEYS` of the Rust gate. */
export const SPAN_ATTR_KEYS = [
  ...SIZE_KEYS,
  'attempt',
  'http_status',
  'ok',
  'queue_ms',
  'retries',
] as const;
export type SpanAttrKey = (typeof SPAN_ATTR_KEYS)[number];

export interface SpanEvent {
  name: SpanName;
  startMs: number;
  endMs: number;
  /** Whole counts only. */
  attrs?: Partial<Record<SpanAttrKey, number>>;
}

export type SpanRejectCode =
  'unknown_field' | 'unknown_name' | 'unknown_attr_key' | 'bad_attr_value' | 'bad_times';

/** A refused span. The message is the code only; it never holds the refused value. */
export class SpanRejectedError extends Error {
  override readonly name = 'SpanRejectedError';
  readonly code: SpanRejectCode;

  constructor(code: SpanRejectCode) {
    super(`the span was refused: ${code}`);
    this.code = code;
  }
}

const FIELDS = new Set(['name', 'startMs', 'endMs', 'attrs']);

const has = <T extends string>(list: readonly T[], v: unknown): v is T =>
  typeof v === 'string' && (list as readonly string[]).includes(v);

const isCount = (v: unknown): v is number =>
  typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;

/** Check a span and build its `telemetry_span` args. Throws `SpanRejectedError` on any text. */
export function buildSpan(e: SpanEvent): CommandArgs['telemetry_span'] {
  const reject = (code: SpanRejectCode): never => {
    throw new SpanRejectedError(code);
  };
  if (Object.keys(e).some((k) => !FIELDS.has(k))) reject('unknown_field');
  if (!has(SPAN_NAMES, e.name)) reject('unknown_name');
  if (!isCount(e.startMs) || !isCount(e.endMs) || e.endMs < e.startMs) reject('bad_times');
  const raw: unknown = e.attrs ?? {};
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) reject('unknown_attr_key');
  const attrs: Record<string, number> = {};
  for (const [k, v] of Object.entries(raw as object)) {
    if (!has(SPAN_ATTR_KEYS, k)) reject('unknown_attr_key');
    if (!isCount(v)) reject('bad_attr_value');
    attrs[k] = v as number;
  }
  return { name: e.name, startMs: e.startMs, endMs: e.endMs, attrs };
}

export interface TelemetryDeps {
  /** The `telemetry_span` call. Default: the IPC `call`. */
  invoke?: (
    name: 'telemetry_span',
    args: CommandArgs['telemetry_span'],
  ) => Promise<CommandReturns['telemetry_span']>;
}

export interface TelemetryClient {
  /** Check the span and send `telemetry_span`. A refused span sends nothing. */
  span(e: SpanEvent): Promise<void>;
}

export function createTelemetry(deps: TelemetryDeps = {}): TelemetryClient {
  const send = deps.invoke ?? call;
  return {
    async span(e) {
      await send('telemetry_span', buildSpan(e));
    },
  };
}
