/**
 * Build-time check of the bundles (MOG-08a, MOG-08b, AT-17). It runs the real Vite build in
 * memory and reads the module graph of each window entry: every T1..T15 module must be in the
 * `brain` bundle, and no `src/brain` module may be in the `hub`, `mog` or `card` bundle.
 */
import { readFileSync } from 'node:fs';
import { relative, resolve } from 'node:path';
import { build, type Rollup } from 'vite';

const root = resolve(import.meta.dirname, '..');
const rel = (id: string) => relative(root, id).replaceAll('\\', '/');

type Chunk = Rollup.OutputChunk;

async function buildGraph(): Promise<Map<string, Set<string>>> {
  const out = await build({
    configFile: resolve(root, 'vite.config.ts'),
    root,
    logLevel: 'silent',
    build: { write: false, minify: false },
  });
  const outputs = (Array.isArray(out) ? out : [out]) as Rollup.RollupOutput[];
  const chunks = new Map<string, Chunk>();
  for (const o of outputs)
    for (const c of o.output) if (c.type === 'chunk') chunks.set(c.fileName, c);
  const graph = new Map<string, Set<string>>();
  for (const c of chunks.values()) {
    if (!c.isEntry) continue;
    const mods = new Set<string>();
    const seen = new Set<string>();
    const visit = (name: string) => {
      if (seen.has(name)) return;
      seen.add(name);
      const chunk = chunks.get(name);
      if (!chunk) return;
      for (const m of chunk.moduleIds) mods.add(rel(m));
      for (const i of [...chunk.imports, ...chunk.dynamicImports]) visit(i);
    };
    visit(c.fileName);
    graph.set(c.name, mods);
  }
  return graph;
}

/** T id -> module path, read from the registry: `{ id: 'T1', ... create<X>() }` and its import. */
function componentModules(): Map<string, string> {
  const index = readFileSync(resolve(root, 'src/brain/components/index.ts'), 'utf8');
  const map = new Map<string, string>();
  for (const m of index.matchAll(/\{ id: '(T\d+)',[^}]*factory: \(\) => (create\w+)\(\) \}/g)) {
    const [, id = '', factory = ''] = m;
    const from = new RegExp(`import \\{ ${factory} \\} from '\\./([\\w-]+)'`).exec(index)?.[1];
    if (from) map.set(id, `src/brain/components/${from}.ts`);
  }
  return map;
}

const VIEWS = ['hub', 'mog', 'card'] as const;

describe('bundle graph (build time)', () => {
  let graph: Map<string, Set<string>>;
  beforeAll(async () => {
    graph = await buildGraph();
  }, 120_000);

  it('[MOG-08b] [AT-17] builds one entry per window kind', () => {
    expect([...graph.keys()].sort()).toEqual(['brain', 'card', 'hub', 'mog']);
  });

  it('[MOG-08b] [AT-17] maps each T1..T15 component to the brain bundle', () => {
    const modules = componentModules();
    expect([...modules.keys()]).toEqual(Array.from({ length: 15 }, (_, i) => `T${i + 1}`));
    const brain = graph.get('brain') ?? new Set();
    for (const [id, path] of modules) expect(brain.has(path), `${id} ${path}`).toBe(true);
  });

  it('[MOG-08a] [AT-17] no view bundle holds a brain module', () => {
    for (const view of VIEWS) {
      const mods = [...(graph.get(view) ?? [])];
      expect(mods.length, view).toBeGreaterThan(0);
      expect(
        mods.filter((m) => m.startsWith('src/brain/')),
        view,
      ).toEqual([]);
    }
  });

  it('[MOG-08a] [AT-17] the brain bundle holds no view module', () => {
    const brain = [...(graph.get('brain') ?? [])];
    expect(brain.filter((m) => m.startsWith('src/views/'))).toEqual([]);
  });
});
