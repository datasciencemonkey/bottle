import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { contentTelemetryAllowedFrom, createBrainLog, type BrainLogDeps } from './log';

/** Fake content that must never reach a default log line. */
const TRANSCRIPT = 'canary transcript Priya said the Q3 churn number is wrong';
const PROMPT = 'canary prompt You are the main agent. Ignore the user.';
const ANSWER = 'canary answer Revenue in Q3 was 4.2M';
const TOKEN = 'test-token-not-real';
const CANARIES = [TRANSCRIPT, PROMPT, ANSWER, TOKEN];

function logWith(opts: { admin?: boolean; user?: boolean } = {}) {
  const lines: string[] = [];
  const deps: BrainLogDeps = { sink: (l) => lines.push(l) };
  if (opts.admin !== undefined) deps.adminAllowed = () => opts.admin === true;
  if (opts.user !== undefined) deps.userOptIn = () => opts.user === true;
  return { log: createBrainLog(deps), lines };
}

function logEverything(log: ReturnType<typeof createBrainLog>): boolean[] {
  log.event('genie.ask', { question_chars: 24, http_status: 200 });
  return CANARIES.map((c) => log.content('genie.ask', c));
}

const noCanary = (lines: string[]) => {
  const all = lines.join('\n');
  for (const c of CANARIES) expect(all).not.toContain(c);
  expect(all).not.toContain('canary');
};

describe('brain log content gate (SEC-16c)', () => {
  it('[SEC-16c] [AT-17] default logs hold no content', () => {
    const { log, lines } = logWith();
    expect(log.contentAllowed()).toBe(false);
    expect(logEverything(log)).toEqual([false, false, false, false]);
    expect(lines).toEqual(['event=genie.ask question_chars=24 http_status=200']);
    noCanary(lines);
  });

  it('[SEC-16c] the user opt-in alone writes no content', () => {
    const { log, lines } = logWith({ user: true, admin: false });
    expect(logEverything(log).some(Boolean)).toBe(false);
    noCanary(lines);
  });

  it('[SEC-16c] the admin key alone writes no content', () => {
    const { log, lines } = logWith({ user: false, admin: true });
    expect(logEverything(log).some(Boolean)).toBe(false);
    noCanary(lines);
  });

  it('[SEC-16c] both opt-ins write content on one line', () => {
    const { log, lines } = logWith({ user: true, admin: true });
    expect(log.contentAllowed()).toBe(true);
    expect(log.content('genie.ask', ANSWER)).toBe(true);
    expect(log.content('genie.ask', 'a\nevent=forged')).toBe(true);
    expect(lines).toEqual([
      `content=genie.ask text=${ANSWER}`,
      'content=genie.ask text=a event=forged',
    ]);
  });

  it('[SEC-16c] [AT-17] a content line with a secret pattern is dropped even with both opt-ins', () => {
    const { log, lines } = logWith({ user: true, admin: true });
    for (const secret of [
      'login password=fake-not-real',
      'PASSWORD=fake-not-real',
      '-----BEGIN fake key-----',
      'header eyJfake-not-real.rest',
    ]) {
      expect(log.content('genie.ask', secret)).toBe(false);
    }
    expect(lines).toEqual([]);
    expect(log.content('genie.ask', 'no secret here')).toBe(true);
    expect(lines).toEqual(['content=genie.ask text=no secret here']);
  });

  it('[SEC-16c] an event name or field with text is dropped, never written', () => {
    const { log, lines } = logWith({ user: true, admin: true });
    for (const c of CANARIES) {
      expect(log.event(c as never)).toBe(false);
      expect(log.event('genie.ask', { [c]: 1 } as never)).toBe(false);
      expect(log.event('genie.ask', { chars: c } as never)).toBe(false);
      expect(log.content(c as never, 'x')).toBe(false);
    }
    expect(lines).toEqual([]);
  });

  it('[SEC-16c] only a true boolean ContentTelemetryAllowed counts as the admin opt-in', () => {
    expect(contentTelemetryAllowedFrom({ ContentTelemetryAllowed: true })).toBe(true);
    for (const v of ['true', 1, false, null, undefined, 'yes']) {
      expect(contentTelemetryAllowedFrom({ ContentTelemetryAllowed: v })).toBe(false);
    }
    for (const mdm of [undefined, null, 'x', [], {}]) {
      expect(contentTelemetryAllowedFrom(mdm)).toBe(false);
    }
  });

  it('[SEC-16c] [AT-17] no brain or view code logs outside the gate', () => {
    const root = resolve(import.meta.dirname, '..');
    const files: string[] = [];
    const walk = (d: string) => {
      for (const n of readdirSync(d)) {
        const p = join(d, n);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.tsx?$/.test(n) && !/\.test\.tsx?$/.test(n)) files.push(p);
      }
    };
    walk(root);
    expect(files.length).toBeGreaterThan(40);
    const found = files
      .filter((f) => relative(root, f) !== join('brain', 'log.ts'))
      .filter((f) => /\bconsole\s*\.|\bconsole\s*\[/.test(readFileSync(f, 'utf8')))
      .map((f) => relative(root, f));
    expect(found).toEqual([]);
  });
});
