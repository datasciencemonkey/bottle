/**
 * ADR 0122: the old session host is gone. No shared, brain, view, test-support or test file names
 * it: not in code, not in comments, not in UI text, not in fixtures.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
/** The old name, in any spelling: words, snake_case, kebab-case or camelCase. */
const OLD_NAME = new RegExp(['gen', 'ie'].join('') + '[ _-]?' + ['co', 'de'].join(''), 'i');

/** Folders that this rule covers. */
const FOLDERS = ['src/shared', 'src/brain', 'src/views', 'src/test-support', 'tests'];
/** No file is left out: the pattern above is built from pieces. */
const SKIP: string[] = [];

function files(dir: string): string[] {
  return readdirSync(join(root, dir)).flatMap((name) => {
    const rel = `${dir}/${name}`;
    if (SKIP.some((s) => rel === s || rel.startsWith(`${s}/`))) return [];
    if (statSync(join(root, rel)).isDirectory()) return files(rel);
    return /\.(ts|tsx|css|json|md)$/.test(name) ? [rel] : [];
  });
}

describe('[ADR-0122] no mention of the old session host', () => {
  it('no shared, brain, view, test-support or test file mentions it, comments and tests included', () => {
    const hits: string[] = [];
    for (const rel of FOLDERS.flatMap(files)) {
      readFileSync(join(root, rel), 'utf8')
        .split('\n')
        .forEach((line, i) => {
          if (OLD_NAME.test(line)) hits.push(`${rel}:${i + 1}`);
        });
    }
    expect(hits).toEqual([]);
  });
});
