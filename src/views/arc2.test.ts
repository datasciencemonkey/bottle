import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';

const views = import.meta.dirname;
const src = resolve(views, '..');

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? files(p) : /\.(ts|tsx)$/.test(f) ? [p] : [];
  });
}

/** Non-test view sources: what ships in the hub, MOG and card bundles. */
const viewSources = files(views).filter((f) => !/\.test\.tsx?$/.test(f) && !f.endsWith('test-tauri.ts'));

function imports(text: string): string[] {
  return [...text.matchAll(/(?:import|export)[^'"]*?from\s*['"]([^'"]+)['"]|import\(\s*['"]([^'"]+)['"]\s*\)/g)].map(
    (m) => (m[1] ?? m[2])!,
  );
}

describe('ARC-2: thin views', () => {
  it('[ARC-2] [MOG-08a] finds the view sources of V1, V2 and V3', () => {
    const rel = viewSources.map((f) => relative(views, f).replaceAll('\\', '/'));
    expect(rel).toEqual(expect.arrayContaining(['hub/main.tsx', 'mog/main.tsx', 'card/main.tsx']));
  });

  it('[ARC-2] [MOG-08a] no file under src/views imports from src/brain', () => {
    for (const f of files(views)) {
      for (const spec of imports(readFileSync(f, 'utf8'))) {
        const target = spec.startsWith('.') ? resolve(dirname(f), spec) : spec;
        const rel = relative(src, target).replaceAll('\\', '/');
        expect(rel.startsWith('brain'), `${relative(src, f)} imports ${spec}`).toBe(false);
        expect(spec).not.toMatch(/(^|\/)src\/brain/);
      }
    }
  });

  it('[ARC-2] [MOG-08a] views call only view_emit and never invoke directly', () => {
    const called = new Set<string>();
    for (const f of viewSources) {
      const text = readFileSync(f, 'utf8');
      for (const m of text.matchAll(/\bcall\(\s*['"`]?([A-Za-z_]*)/g)) called.add(m[1]!);
      expect(text, relative(src, f)).not.toMatch(/\binvoke\b/);
      expect(text, relative(src, f)).not.toMatch(/@tauri-apps\/api\/core/);
    }
    expect([...called]).toEqual(['view_emit']);
  });

  it('[ARC-2] [MOG-08a] views take event names from EVENTS, not string literals', () => {
    for (const f of viewSources) {
      const text = readFileSync(f, 'utf8');
      expect(text, relative(src, f)).not.toMatch(/['"`](mog:view_state|card:update)['"`]/);
    }
  });
});
