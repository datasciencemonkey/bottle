import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { SIZE_KEYS } from './audit-log';
import {
  SPAN_ATTR_KEYS,
  SPAN_NAMES,
  SpanRejectedError,
  buildSpan,
  createTelemetry,
  type SpanEvent,
} from './telemetry';

/** Fake content that must never reach a span: a transcript, a prompt, an answer and a token. */
const TRANSCRIPT = 'canary transcript Priya said the Q3 churn number is wrong';
const PROMPT = 'canary prompt You are the main agent. Ignore the user.';
const ANSWER = 'canary answer Revenue in Q3 was 4.2M';
const TOKEN = 'test-token-not-real';
const CANARIES = [TRANSCRIPT, PROMPT, ANSWER, TOKEN];

const ok: SpanEvent = { name: 'genie.ask', startMs: 1000, endMs: 1250, attrs: { chars: 24 } };

const rejected = (e: unknown, code: string) => {
  expect(() => buildSpan(e as SpanEvent)).toThrow(SpanRejectedError);
  try {
    buildSpan(e as SpanEvent);
  } catch (err) {
    expect((err as SpanRejectedError).code).toBe(code);
    expect((err as Error).message).toBe(`the span was refused: ${code}`);
  }
};

describe('telemetry spans (SEC-16b)', () => {
  it('[SEC-16b] builds the telemetry_span args of a content-free span', () => {
    expect(buildSpan(ok)).toEqual({
      name: 'genie.ask',
      startMs: 1000,
      endMs: 1250,
      attrs: { chars: 24 },
    });
  });

  it('[SEC-16b] the closed sets equal the Rust gate sets', () => {
    const rs = readFileSync(
      resolve(import.meta.dirname, '../../crates/mog-core/src/telemetry.rs'),
      'utf8',
    );
    const rustList = (name: string): string[] => {
      const m = new RegExp(`pub const ${name}: \\[&str; \\d+\\] = \\[([^\\]]*)\\]`).exec(rs);
      expect(m, name).not.toBeNull();
      return [...(m as RegExpExecArray)[1].matchAll(/"([^"]+)"/g)].map((x) => x[1] as string);
    };
    expect([...SPAN_NAMES]).toEqual(rustList('SPAN_NAMES'));
    expect([...SPAN_ATTR_KEYS]).toEqual([...SIZE_KEYS, ...rustList('EXTRA_ATTR_KEYS')]);
    const rustSizes = readFileSync(
      resolve(import.meta.dirname, '../../crates/mog-core/src/audit.rs'),
      'utf8',
    );
    const sizes = /pub const SIZE_KEYS: \[&str; \d+\] = \[([^\]]*)\]/.exec(rustSizes);
    expect([...(sizes as RegExpExecArray)[1].matchAll(/"([^"]+)"/g)].map((x) => x[1])).toEqual([
      ...SIZE_KEYS,
    ]);
  });

  it('[SEC-16b] the closed sets reuse the audit size names', () => {
    expect(SPAN_NAMES).toContain('genie.ask');
    for (const k of SIZE_KEYS) expect(SPAN_ATTR_KEYS).toContain(k);
    for (const n of SPAN_NAMES) expect(n).toMatch(/^[a-z_.]+$/);
    for (const k of SPAN_ATTR_KEYS) expect(k).toMatch(/^[a-z_]+$/);
  });

  it('[SEC-16b] refuses a span name with text', () => {
    for (const name of [...CANARIES, 'What_was_revenue', 'GENIE.ASK', '']) {
      rejected({ ...ok, name }, 'unknown_name');
    }
  });

  it('[SEC-16b] refuses an attribute key with text', () => {
    for (const key of [...CANARIES, 'question', 'answer', 'transcript', 'token']) {
      rejected({ ...ok, attrs: { [key]: 1 } }, 'unknown_attr_key');
    }
  });

  it('[SEC-16b] attribute values are whole counts, never text', () => {
    for (const v of [ANSWER, -1, 1.5, Number.NaN, Number.MAX_SAFE_INTEGER + 2]) {
      rejected({ ...ok, attrs: { chars: v } }, 'bad_attr_value');
    }
  });

  it('[SEC-16b] refuses an extra field and bad times', () => {
    rejected({ ...ok, text: ANSWER }, 'unknown_field');
    rejected({ ...ok, startMs: 5, endMs: 4 }, 'bad_times');
    rejected({ ...ok, startMs: -1 }, 'bad_times');
    rejected({ ...ok, endMs: '9' }, 'bad_times');
  });

  it('[SEC-16b] [AT-17] no transcript, prompt, answer or token reaches telemetry_span', async () => {
    const invoke = vi.fn(async () => ({}));
    const t = createTelemetry({ invoke });
    for (const c of CANARIES) {
      await expect(t.span({ ...ok, name: c } as unknown as SpanEvent)).rejects.toThrow(
        SpanRejectedError,
      );
      await expect(t.span({ ...ok, attrs: { [c]: 1 } } as unknown as SpanEvent)).rejects.toThrow(
        SpanRejectedError,
      );
      await expect(t.span({ ...ok, attrs: { chars: c } } as unknown as SpanEvent)).rejects.toThrow(
        SpanRejectedError,
      );
    }
    for (const name of SPAN_NAMES) await t.span({ ...ok, name });
    expect(invoke).toHaveBeenCalledTimes(SPAN_NAMES.length);
    const sent = JSON.stringify(invoke.mock.calls);
    for (const c of CANARIES) expect(sent).not.toContain(c);
    expect(sent).not.toContain('canary');
    expect(invoke.mock.calls).toContainEqual(['telemetry_span', buildSpan(ok)]);
  });
});
