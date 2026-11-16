import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const prettierIgnore = readFileSync(resolve(root, '.prettierignore'), 'utf8')
  .split('\n')
  .map((l) => l.trim())
  .filter((l) => l !== '' && !l.startsWith('#'));
// The first `ignores: [...]` block of eslint.config.js, read as text (the config file has no types).
const eslintText = readFileSync(resolve(root, 'eslint.config.js'), 'utf8');
const eslintIgnores = [
  ...(/ignores:\s*\[([^\]]*)\]/.exec(eslintText)?.[1] ?? '').matchAll(/'([^']+)'/g),
].map((m) => m[1] ?? '');

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
