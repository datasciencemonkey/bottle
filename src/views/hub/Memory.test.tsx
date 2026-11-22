// @vitest-environment jsdom
/**
 * The hub Memory area (local memory plan 9.1, slice 1; ADR 0112): the OpenClaw-style tree
 * (USER.md, MEMORY.md, the daily notes), the items of a file, the rendered view, the "New"
 * badge, thumbs up, and Delete or thumbs down with Undo. The view runs against a local
 * `MemorySource`, so no brain and no store take part. All values are plainly fake.
 */
import { act } from 'react';
import { render } from '../test-tauri';
import { MemoryArea, type MemorySource } from './Memory';
import {
  asHubMemory,
  filesOf,
  MEMORY_UNDO_MS,
  splitFile,
  type HubMemory,
  type MemoryAction,
  type MemoryFileRow,
  type MemoryFileView,
} from '../../shared/hub-memory';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const MOG_A = '00000000-0000-4000-8000-00000000000a';
const MOG_B = '00000000-0000-4000-8000-00000000000b';
const NOW = new Date(2026, 10, 21, 16, 30).getTime();
const TODAY = '/memories/sales/memory/2026-11-21.md';
const YESTERDAY = '/memories/sales/memory/2026-11-20.md';

function mem(n: number, over: Partial<MemoryFileRow> = {}): MemoryFileRow {
  const file = over.file ?? TODAY;
  return {
    itemId: `51111111-2222-4333-8444-${String(n).padStart(12, '0')}`,
    kind: 'fact',
    layer: 'daily',
    file,
    path: `${file}#fake-fact-${n}`,
    title: `Fake fact ${n}`,
    summary: `The fake mart ${n} refreshes at six.`,
    status: 'active',
    pinned: true,
    source: 'you',
    origin: 'pin',
    tags: [],
    rating: 0,
    createdAt: NOW - n * 60_000,
    updatedAt: NOW - n * 60_000,
    ...over,
  };
}

const MODEL: HubMemory = {
  userItems: 0,
  folders: [
    {
      mogId: MOG_A,
      folder: 'sales',
      mogName: 'Sales',
      rows: [
        mem(1),
        mem(2, { kind: 'genie_answer', title: 'Fake churn', source: 'genie' }),
        mem(3, { status: 'quarantined', title: 'Fake odd one' }),
        mem(4, { file: YESTERDAY, rating: 1, reviewedAt: NOW - 1 }),
      ],
    },
    {
      mogId: MOG_B,
      folder: 'finance',
      mogName: 'Finance',
      rows: [mem(5, { file: '/memories/finance/memory/2026-11-21.md' })],
    },
  ],
};

class LocalSource implements MemorySource {
  sent: MemoryAction[] = [];
  onList: ((m: HubMemory) => void) | undefined;
  onFile: ((f: MemoryFileView) => void) | undefined;
  subscribe(onList: (m: HubMemory) => void, onFile: (f: MemoryFileView) => void) {
    this.onList = onList;
    this.onFile = onFile;
    return () => {
      this.onList = undefined;
      this.onFile = undefined;
    };
  }
  send(a: MemoryAction) {
    this.sent.push(a);
  }
  async list(m: HubMemory) {
    await act(async () => this.onList?.(m));
  }
  async file(f: MemoryFileView) {
    await act(async () => this.onFile?.(f));
  }
  actions<K extends MemoryAction['action']>(name: K) {
    return this.sent.filter((a): a is Extract<MemoryAction, { action: K }> => a.action === name);
  }
}

const q = (el: HTMLElement, s: string) => el.querySelector(s) as HTMLElement | null;
const options = (el: HTMLElement) => [...el.querySelectorAll('[role="option"]')] as HTMLElement[];
const click = async (el: HTMLElement | null) =>
  act(async () => {
    el?.click();
  });
const buttonByText = (el: HTMLElement, text: string) =>
  [...el.querySelectorAll('button')].find((b) => b.textContent?.trim() === text) ?? null;
const fileButton = (el: HTMLElement, path: string) => q(el, `[data-file="${path}"] button`);

const view = async (src: LocalSource, path: string, body: string) =>
  src.file({
    mogId: MOG_A,
    path,
    text: `---\nfile: daily\nmog: sales\nitems: 1\n---\n<!-- memory id=x slug=y -->\n${body}\n`,
  });

async function setup() {
  const src = new LocalSource();
  const v = await render(<MemoryArea active source={src} now={() => NOW} />);
  await src.list(MODEL);
  return { src, el: v.el, root: v.root };
}

afterEach(() => {
  vi.useRealTimers();
});

describe('hub Memory area', () => {
  it('asks for the folders each time it shows', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active={false} source={src} />);
    expect(src.actions('memory.ready')).toHaveLength(0);
    await act(async () => v.root.render(<MemoryArea active source={src} />));
    expect(src.actions('memory.ready')).toHaveLength(1);
    await act(async () => v.root.render(<MemoryArea active={false} source={src} />));
    await act(async () => v.root.render(<MemoryArea active source={src} />));
    expect(src.actions('memory.ready')).toHaveLength(2);
  });

  it('shows the loading, empty and unavailable states', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    expect(q(v.el, '[data-memory-loading]')).not.toBeNull();
    await src.list({
      userItems: 0,
      folders: [{ mogId: MOG_A, folder: 'sales', mogName: 'Sales', rows: [] }],
    });
    expect(q(v.el, '[data-memory-empty]')?.textContent).toContain('today’s note');
    await src.list({ userItems: 0, folders: [], unavailable: true });
    expect(q(v.el, '[data-memory-unavailable]')).not.toBeNull();
  });

  it('shows the OpenClaw tree: USER.md, then MEMORY.md and the days of each MOG', async () => {
    const { el } = await setup();
    const files = [...el.querySelectorAll('[data-file]')].map((f) => [
      f.getAttribute('data-file'),
      f.textContent,
    ]);
    expect(files).toEqual([
      ['/memories/USER.md', 'USER.md0'],
      ['/memories/sales/MEMORY.md', 'MEMORY.md0'],
      [TODAY, 'Today3'],
      [YESTERDAY, 'Yesterday1'],
      ['/memories/finance/MEMORY.md', 'MEMORY.md0'],
      ['/memories/finance/memory/2026-11-21.md', 'Today1'],
    ]);
    // The newest note of the first MOG opens first, and its whole file is asked for.
    expect(q(el, `[data-file="${TODAY}"]`)?.getAttribute('aria-selected')).toBe('true');
    expect(options(el)).toHaveLength(3);
  });

  it('asks for the whole file, then for one item section', async () => {
    const { src, el } = await setup();
    expect(src.actions('memory.file').at(-1)).toEqual({
      action: 'memory.file',
      mogId: MOG_A,
      path: TODAY,
    });
    await view(src, TODAY, '# 2026-11-21: Sales\n\n## Fake fact 1');
    expect(q(el, '[data-memory-path]')?.textContent).toBe(TODAY);
    expect(q(el, '.mem-doc-body')?.textContent).toContain('Fake fact 1');
    // The hidden metadata line never shows.
    expect(el.textContent).not.toContain('<!-- memory');
    await click(options(el)[0] ?? null);
    const item = MODEL.folders[0]?.rows[0] as MemoryFileRow;
    expect(src.actions('memory.file').at(-1)).toEqual({
      action: 'memory.file',
      mogId: MOG_A,
      path: item.path,
    });
    expect(q(el, '[data-memory-path]')?.textContent).toBe(`${TODAY}#fake-fact-1`);
    expect(q(el, '[data-memory-file-loading]')).not.toBeNull();
    await view(src, item.path, '## Fake fact 1\nThe mart refreshes at **six**.');
    expect([...el.querySelectorAll('.mem-doc-body strong')].map((s) => s.textContent)).toContain(
      'six',
    );
    expect([...el.querySelectorAll('.mem-field dt')].map((d) => d.textContent)).toEqual([
      'file',
      'mog',
      'items',
    ]);
  });

  it('a file for another path is not shown', async () => {
    const { src, el } = await setup();
    await view(src, YESTERDAY, 'other');
    expect(q(el, '[data-memory-file-loading]')).not.toBeNull();
  });

  it('a MEMORY.md or USER.md click shows its file and its items', async () => {
    const { src, el } = await setup();
    await click(fileButton(el, '/memories/sales/MEMORY.md'));
    expect(src.actions('memory.file').at(-1)?.path).toBe('/memories/sales/MEMORY.md');
    expect(options(el)).toHaveLength(0);
    expect(el.textContent).toContain('Nothing here yet.');
    await click(fileButton(el, '/memories/USER.md'));
    expect(src.actions('memory.file').at(-1)).toEqual({
      action: 'memory.file',
      mogId: '',
      path: '/memories/USER.md',
    });
    expect(el.textContent).toContain('Shared preferences come in a later version.');
  });

  it('marks new, pinned and quarantined items', async () => {
    const { src, el } = await setup();
    const [first, , bad] = options(el);
    expect(first?.querySelector('[data-mark="new"]')).not.toBeNull();
    expect(first?.querySelector('[aria-label="Pinned"]')).not.toBeNull();
    expect(bad?.querySelector('[data-mark="quarantined"]')).not.toBeNull();
    await click(fileButton(el, YESTERDAY));
    expect(options(el)[0]?.querySelector('[data-mark="new"]')).toBeNull();
    await click(fileButton(el, TODAY));
    await click(options(el)[2] ?? null);
    const row = MODEL.folders[0]?.rows[2] as MemoryFileRow;
    await view(src, row.path, '## Fake odd one\n**not markdown**');
    expect(q(el, '[data-memory-quarantine-note]')).not.toBeNull();
    expect(q(el, '.mem-plain')?.textContent).toBe('## Fake odd one\n**not markdown**');
  });

  it('thumbs up sends memory.rate and clears the New badge at once', async () => {
    const { src, el } = await setup();
    await click(options(el)[0] ?? null);
    await click(q(el, '[data-action="thumbs-up"]'));
    expect(src.actions('memory.rate')).toEqual([
      { action: 'memory.rate', mogId: MOG_A, itemId: MODEL.folders[0]?.rows[0]?.itemId },
    ]);
    expect(options(el)[0]?.querySelector('[data-mark="new"]')).toBeNull();
    expect(q(el, '[data-action="thumbs-up"]')?.hasAttribute('disabled')).toBe(true);
  });

  it('thumbs up on a quarantined item releases it in the view', async () => {
    const { src, el } = await setup();
    await click(options(el)[2] ?? null);
    expect(q(el, '[data-memory-quarantine-note]')).not.toBeNull();
    await click(q(el, '[data-action="thumbs-up"]'));
    const row = MODEL.folders[0]?.rows[2] as MemoryFileRow;
    expect(src.actions('memory.rate')).toEqual([
      { action: 'memory.rate', mogId: MOG_A, itemId: row.itemId },
    ]);
    expect(options(el)[2]?.querySelector('[data-mark="quarantined"]')).toBeNull();
    expect(q(el, '[data-memory-quarantine-note]')).toBeNull();
  });

  it('AC-1.8 Delete needs a confirm click and runs after the Undo time', async () => {
    vi.useFakeTimers();
    const { src, el } = await setup();
    await click(options(el)[0] ?? null);
    const row = MODEL.folders[0]?.rows[0] as MemoryFileRow;
    await click(buttonByText(el, 'Delete…'));
    expect(src.actions('memory.delete')).toHaveLength(0);
    await click(buttonByText(el, 'Delete'));
    expect(options(el).map((o) => o.getAttribute('data-item'))).not.toContain(row.itemId);
    expect(q(el, '[data-memory-undo]')?.textContent).toContain('Memory deleted');
    expect(src.actions('memory.delete')).toHaveLength(0);
    await act(async () => {
      vi.advanceTimersByTime(MEMORY_UNDO_MS);
    });
    expect(src.actions('memory.delete')).toEqual([
      { action: 'memory.delete', mogId: MOG_A, itemId: row.itemId },
    ]);
    expect(q(el, '[data-memory-undo]')).toBeNull();
  });

  it('thumbs down takes the delete path with Undo, and Undo keeps the item', async () => {
    vi.useFakeTimers();
    const { src, el } = await setup();
    await click(options(el)[0] ?? null);
    await click(q(el, '[data-action="thumbs-down"]'));
    expect(options(el)).toHaveLength(2);
    await click(buttonByText(el, 'Undo'));
    await act(async () => {
      vi.advanceTimersByTime(MEMORY_UNDO_MS * 2);
    });
    expect(src.actions('memory.delete')).toHaveLength(0);
    expect(options(el)).toHaveLength(3);
  });

  it('Cancel on the confirm deletes nothing', async () => {
    const { el } = await setup();
    await click(options(el)[0] ?? null);
    await click(buttonByText(el, 'Delete…'));
    await click(buttonByText(el, 'Cancel'));
    expect(buttonByText(el, 'Delete…')).not.toBeNull();
    expect(q(el, '[data-memory-undo]')).toBeNull();
  });

  it('a pending delete goes at once when the area goes away', async () => {
    vi.useFakeTimers();
    const { src, el, root } = await setup();
    await click(options(el)[0] ?? null);
    await click(q(el, '[data-action="thumbs-down"]'));
    await act(async () => root.unmount());
    expect(src.actions('memory.delete')).toHaveLength(1);
  });

  it('the arrow keys move in the tree and the list', async () => {
    const { src, el } = await setup();
    const key = async (t: HTMLElement, k: string) =>
      act(async () => {
        t.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
      });
    await key(q(el, '[role="tree"]') as HTMLElement, 'ArrowDown');
    expect(q(el, `[data-file="${YESTERDAY}"]`)?.getAttribute('aria-selected')).toBe('true');
    await key(q(el, '[role="listbox"]') as HTMLElement, 'ArrowDown');
    expect(options(el)[0]?.getAttribute('aria-selected')).toBe('true');
    expect(src.actions('memory.file').at(-1)?.path).toBe(`${YESTERDAY}#fake-fact-4`);
  });
});

describe('hub-memory shapes', () => {
  it('drops bad rows and folders', () => {
    const m = asHubMemory({
      folders: [
        {
          mogId: MOG_A,
          folder: 'sales',
          mogName: 'Sales',
          rows: [
            mem(1),
            { ...mem(2), kind: 'person' },
            { ...mem(3), status: 'proposed' },
            { ...mem(4), file: '/etc/x' },
            {
              ...mem(5),
              file: '/memories/finance/MEMORY.md',
              path: '/memories/finance/MEMORY.md#a',
            },
            { ...mem(6), path: '/memories/other#x' },
            { ...mem(7), rating: 2 },
          ],
        },
        { mogId: MOG_B, folder: '../x', mogName: 'Bad', rows: [] },
        { mogId: 1 },
      ],
    });
    expect(m?.folders).toHaveLength(1);
    expect(m?.folders[0]?.rows.map((r) => r.title)).toEqual(['Fake fact 1']);
    expect(m?.userItems).toBe(0);
    expect(asHubMemory(null)).toBeUndefined();
  });

  it('filesOf puts MEMORY.md first and the days newest first', () => {
    const f = MODEL.folders[0];
    if (!f) throw new Error('fixture');
    expect(filesOf(f).map((e) => [e.label, e.items])).toEqual([
      ['MEMORY.md', 0],
      ['2026-11-21', 3],
      ['2026-11-20', 1],
    ]);
  });

  it('splitFile reads the frontmatter and drops the hidden metadata lines', () => {
    const { fields, body } = splitFile(
      '---\nfile: daily\ntitle: "a: b"\n---\n## X\n<!-- memory id=1 slug=x -->\nBody\n',
    );
    expect(fields).toEqual([
      ['file', 'daily'],
      ['title', 'a: b'],
    ]);
    expect(body).toBe('## X\nBody\n');
    expect(splitFile('no front').body).toBe('no front');
  });
});
