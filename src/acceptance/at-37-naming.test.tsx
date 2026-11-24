// @vitest-environment jsdom
/**
 * AT-37 "Licences, brand and supply chain" (#71), section 15: the naming part. It stops later PRs
 * from bringing back a former product name, from losing the experimental label, or from showing "MOG" as
 * anything other than a UI noun. The scope (what counts as UI text and package metadata) is in
 * ADR 0030. The bundle-ID and crate rules also run in `src-tauri/tests/skeleton.rs` (MB-10c).
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { relative, resolve } from 'node:path';
import { act } from 'react';
import { render, tauri } from '../views/test-tauri';
import { Hub } from '../views/hub/Hub';
import { About } from '../views/hub/About';
import { MogView } from '../views/mog/MogView';
import { CardView } from '../views/card/CardView';
import type { MascotRenderer } from '../views/mog/mascot';
import {
  EXPERIMENTAL_LABEL,
  PRODUCT_NAME,
  bannedNameHits,
  htmlTitle,
  markdownProse,
  mogNounHits,
  rustWindowTitles,
  visibleTexts,
} from '../test-support/naming-rules';

vi.mock('@tauri-apps/api/core', async () => (await import('../views/test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../views/test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../views/test-tauri')).windowModule);

const root = resolve(import.meta.dirname, '../..');
const read = (p: string): string => readFileSync(resolve(root, p), 'utf8');
const json = (p: string): Record<string, unknown> => JSON.parse(read(p)) as Record<string, unknown>;

/** Every file under `dir` (repository-relative), without tests and build output. */
function walk(dir: string): string[] {
  return readdirSync(resolve(root, dir)).flatMap((name) => {
    const p = `${dir}/${name}`;
    if (['node_modules', 'target', 'dist', 'icons', 'gen'].includes(name)) return [];
    if (statSync(resolve(root, p)).isDirectory()) return walk(p);
    return /\.test\.tsx?$/.test(name) || /\.(png|icns|ico|riv|wasm)$/.test(name) ? [] : [p];
  });
}

const VIEW_HTML = ['hub.html', 'mog.html', 'card.html', 'brain.html'];
const CRATES = readdirSync(resolve(root, 'crates')).map((c) => `crates/${c}`);

/** UI text and package metadata (ADR 0030). */
const SCANNED = [
  ...VIEW_HTML,
  'package.json',
  'Cargo.toml',
  'THIRD_PARTY_NOTICES.md',
  ...walk('src'),
  ...walk('src-tauri/src'),
  ...walk('src-tauri/capabilities'),
  ...walk('src-tauri/permissions'),
  'src-tauri/tauri.conf.json',
  'src-tauri/Cargo.toml',
  ...CRATES.flatMap((c) => [`${c}/Cargo.toml`, ...walk(`${c}/src`)]),
  ...walk('docs/user'),
];

const ID = '00000000-0000-4000-8000-000000000001';
const overlays = { attention: false, recording: false, offline: false, localOnly: false };
const mascot: MascotRenderer = {
  mount: () => undefined,
  setMood: () => undefined,
  setLevel: () => undefined,
  setAttention: () => undefined,
  setHover: () => undefined,
  poke: () => undefined,
  destroy: () => undefined,
};

async function renderedViewText(): Promise<string[]> {
  const hub = await render(<Hub />);
  await tauri.emit('mog:view_state', { mogId: ID, mood: 'idle', overlays });
  const mog = await render(<MogView mogId={ID} createMascot={() => mascot} />);
  const card = await render(<CardView mogId={ID} />);
  await tauri.emit('card:update', {
    mogId: ID,
    blurbs: [{ id: 'b1', text: 'Revenue was 4.2M' }],
    chips: [{ id: 'c1', label: 'Ask Genie' }],
    answerMd: 'ok',
  });
  const text = [hub.el, mog.el, card.el].flatMap(visibleTexts);
  await act(async () => [hub, mog, card].forEach((r) => r.root.unmount()));
  return text;
}

describe('AT-37 naming: Bottle, the experimental label and the MOG noun (#71)', () => {
  beforeEach(() => tauri.reset());

  it('[MB-10a] [AT-37] the scan covers the views, the Tauri host, the manifests and the user docs', () => {
    for (const p of [
      'hub.html',
      'src/views/hub/Hub.tsx',
      'src/views/hub/About.tsx',
      'src-tauri/src/windows.rs',
      'src-tauri/tauri.conf.json',
      'package.json',
      'docs/user/where-your-data-goes.md',
    ]) {
      expect(SCANNED).toContain(p);
    }
    expect(SCANNED.some((p) => p.endsWith('naming-rules.test.ts'))).toBe(false);
  });

  it('[MB-10a] [AT-37] no UI text or package metadata contains a former product name', () => {
    const hits = SCANNED.flatMap((p) => bannedNameHits(read(p)).map((h) => `${p}: ${h}`));
    expect(hits).toEqual([]);
  });

  it('[MB-10a] [AT-37] the product name is Bottle in the Tauri config and every view title', () => {
    expect(json('src-tauri/tauri.conf.json').productName).toBe(PRODUCT_NAME);
    for (const p of VIEW_HTML) expect(htmlTitle(read(p)), p).toMatch(/^Bottle\b/);
    for (const t of rustWindowTitles(read('src-tauri/src/windows.rs'))) {
      expect(t.startsWith(PRODUCT_NAME) || t.startsWith('MOG'), t).toBe(true);
    }
  });

  it('[MB-10b] [AT-37] the hub title bar shows the experimental label', () => {
    expect(htmlTitle(read('hub.html'))).toBe(`${PRODUCT_NAME} (${EXPERIMENTAL_LABEL})`);
    expect(rustWindowTitles(read('src-tauri/src/windows.rs'))).toContain(
      `${PRODUCT_NAME} (${EXPERIMENTAL_LABEL})`,
    );
  });

  it('[MB-10b] [AT-37] the hub and About show the experimental label', async () => {
    const hub = await render(<Hub />);
    expect(hub.el.querySelector('main > p')?.textContent).toBe(EXPERIMENTAL_LABEL);
    const about = await render(<About />);
    expect(about.el.querySelector('[data-label="labs"]')?.textContent).toBe(EXPERIMENTAL_LABEL);
    await act(async () => [hub, about].forEach((r) => r.root.unmount()));
  });

  it('[MB-10c] [AT-37] no package or bundle ID contains MOG', () => {
    const conf = json('src-tauri/tauri.conf.json');
    const ids = [
      conf.identifier,
      conf.productName,
      conf.mainBinaryName,
      json('package.json').name,
      /^name = "([^"]+)"/m.exec(read('src-tauri/Cargo.toml'))?.[1],
    ].filter((v): v is string => typeof v === 'string');
    expect(ids).toContain('com.databricks.bottle');
    for (const id of ids) expect(id.toLowerCase(), id).not.toContain('mog');
  });

  it('[MB-10c] [AT-37] window titles, page titles and metadata use MOG only as a noun', () => {
    const conf = json('src-tauri/tauri.conf.json');
    const bundle = (conf.bundle ?? {}) as Record<string, unknown>;
    const texts = [
      ...VIEW_HTML.map((p) => `${p}: ${htmlTitle(read(p))}`),
      ...rustWindowTitles(read('src-tauri/src/windows.rs')),
      `${String(bundle.shortDescription)} ${String(bundle.longDescription ?? '')}`,
      String(json('package.json').description),
    ];
    expect(texts.flatMap((t) => mogNounHits(t).map((h) => `${t} -> ${h}`))).toEqual([]);
  });

  it('[MB-10c] [AT-37] the user docs use MOG only as a noun', () => {
    const hits = walk('docs/user').flatMap((p) =>
      mogNounHits(markdownProse(read(p))).map((h) => `${relative(root, resolve(root, p))}: ${h}`),
    );
    expect(hits).toEqual([]);
  });

  it('[MB-10a] [MB-10c] [AT-37] the rendered hub, MOG and card views follow the naming rules', async () => {
    const text = await renderedViewText();
    expect(text.join(' ')).toContain('MOG');
    expect(text.flatMap(bannedNameHits)).toEqual([]);
    expect(text.flatMap(mogNounHits)).toEqual([]);
  });
});
