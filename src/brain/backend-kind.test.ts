/**
 * G-21a (inspection, AT-09): UI, mascot and memory code never branch on the backend kind. They
 * read only the normalised AskEvents of GL-1 (#33, ADR 0084). Only the GenieBridge, the MOG
 * config and the backend label badge (UX-15) name a backend kind.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const src = resolve(import.meta.dirname, '..');

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? files(p) : /\.(ts|tsx)$/.test(f) ? [p] : [];
  });
}

/** The UI, mascot, card and memory code of G-21a. */
const GUARDED = [
  ...files(join(src, 'views')),
  ...[
    'blurb-stack.ts',
    'card-chips.ts',
    'card-sections.ts',
    'card-stack.ts',
    'outcome-blurb.ts',
    'mood-publisher.ts',
    'notification-service.ts',
  ].map((f) => join(src, 'brain', f)),
  ...[
    'mood-mapper.ts',
    'memory-service.ts',
    'memory-store.ts',
    'exchange-summary.ts',
    'embedding-queue.ts',
  ].map((f) => join(src, 'brain', 'components', f)),
].filter((f) => !/\.test\.tsx?$/.test(f) && !f.endsWith('test-tauri.ts'));

/** The backend-kind branches that G-21a forbids. Each returns the offending text. */
export function backendBranches(text: string): string[] {
  const re =
    /\b(?:genie_one_mcp|genie_agent|custom_agent|BackendKind|BackendBinding|backendFor)\b|\bbackend\??\.kind\b|\bbinding\??\.kind\b/g;
  return [...text.matchAll(re)].map((m) => m[0]);
}

describe('G-21a: no branch on the backend kind', () => {
  it('[G-21a] [AT-09] the matcher finds a backend-kind branch', () => {
    expect(backendBranches("if (req.binding.kind === 'genie_agent') show();")).toEqual([
      'binding.kind',
      'genie_agent',
    ]);
    expect(backendBranches('const b = bridge.backendFor(x); if (b.kind) {}')).toEqual([
      'backendFor',
    ]);
    expect(backendBranches("switch (e.t) { case 'answer': }")).toEqual([]);
  });

  it('[G-21a] [AT-09] UI, mascot, card and memory code never name a backend kind', () => {
    // The card render of the outcomes exists and is in the list (#33).
    expect(GUARDED.some((f) => f.endsWith('outcome-blurb.ts'))).toBe(true);
    for (const f of GUARDED) {
      expect(backendBranches(readFileSync(f, 'utf8')), relative(src, f)).toEqual([]);
    }
  });
});
