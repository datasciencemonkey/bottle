import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import eslintConfig from '../eslint.config.js';

const root = resolve(import.meta.dirname, '..');
const prettierIgnore = readFileSync(resolve(root, '.prettierignore'), 'utf8')
  .split('\n')
  .map((l) => l.trim())
  .filter((l) => l !== '' && !l.startsWith('#'));
const eslintIgnores = (eslintConfig as { ignores?: string[] }[]).flatMap((c) => c.ignores ?? []);

// App code that `pnpm lint` and `pnpm format:check` must keep in scope.
const appPaths = ['src/', 'src/brain/', 'src/views/', 'src-tauri/src/', 'crates/', 'tests/'];

describe('lint and format scope (#300)', () => {
  it('[#300] prettier and eslint skip the throwaway spikes/ folders', () => {
    expect(prettierIgnore).toContain('spikes/');
    expect(eslintIgnores).toContain('spikes/**');
  });

  it('[#300] no app code folder is excluded from prettier or eslint', () => {
    for (const p of appPaths) {
      expect(prettierIgnore.some((i) => p.startsWith(i.replace(/\*\*$/, '')))).toBe(false);
      expect(eslintIgnores.some((i) => p.startsWith(i.replace(/\*\*$/, '')))).toBe(false);
    }
  });
});
