// @vitest-environment jsdom
/**
 * The hub Memory area (local memory plan 9.1, slice 1): the folder tree, the list, the file view
 * and Delete with Undo. The view runs against a local `MemorySource`, so no brain and no store
 * take part. All values are plainly fake.
 */
import { act } from 'react';
import { render } from '../test-tauri';
import { MemoryArea, type MemorySource } from './Memory';
import {
  asHubMemory,
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

function mem(n: number, over: Partial<MemoryFileRow> = {}): MemoryFileRow {
  const slug = `fake-fact-${n}`;
  return {
    itemId: `51111111-2222-4333-8444-${String(n).padStart(12, '0')}`,
    kind: 'fact',
    path: `/memories/sales/facts/${slug}.md`,
    title: `Fake fact ${n}`,
    summary: `The fake mart ${n} refreshes at six.`,
    status: 'active',
    pinned: false,
    source: 'you',
    origin: 'pin',
    tags: [],
    createdAt: 1_795_000_000_000,
    updatedAt: 1_795_000_000_000,
    ...over,
  };
}

const MODEL: HubMemory = {
  folders: [
    {
      mogId: MOG_A,
      folder: 'sales',
      mogName: 'Sales',
      rows: [
        mem(1, { pinned: true }),
        mem(2, {
          kind: 'genie_answer',
          path: '/memories/sales/answers/fake-churn.md',
          title: 'Fake churn',
          source: 'genie',
        }),
        mem(3, { status: 'quarantined', title: 'Fake odd one' }),
      ],
    },
    { mogId: MOG_B, folder: 'finance', mogName: 'Finance', rows: [mem(4)] },
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
  actions(name: MemoryAction['action']) {
    return this.sent.filter((a) => a.action === name);
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

function fileText(row: MemoryFileRow, body: string) {
  return `---\nid: ${row.itemId}\nkind: ${row.kind}\ntitle: ${JSON.stringify(row.title)}\nstatus: ${row.status}\n---\n${body}\n`;
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

  it('shows the empty and the unavailable states', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    expect(q(v.el, '[data-memory-loading]')).not.toBeNull();
    await src.list({ folders: [{ mogId: MOG_A, folder: 'sales', mogName: 'Sales', rows: [] }] });
    expect(q(v.el, '[data-memory-empty]')?.textContent).toContain('Remember this');
    await src.list({ folders: [], unavailable: true });
    expect(q(v.el, '[data-memory-unavailable]')).not.toBeNull();
  });

  it('shows a tree of MOG folders and kind folders with counts', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    const sales = q(v.el, '[data-folder="sales"]');
    expect(sales?.getAttribute('aria-expanded')).toBe('true');
    expect(sales?.getAttribute('aria-selected')).toBe('true');
    const kinds = [...(sales?.querySelectorAll('[data-kind-folder]') ?? [])].map((k) => [
      k.getAttribute('data-kind-folder'),
      k.textContent,
    ]);
    expect(kinds).toEqual([
      ['facts', 'Facts2'],
      ['answers', 'Answers1'],
    ]);
    expect(options(v.el).map((o) => o.getAttribute('data-item'))).toEqual(
      MODEL.folders[0]?.rows.map((r) => r.itemId),
    );
    await click(q(v.el, '[data-kind-folder="answers"] button'));
    expect(options(v.el)).toHaveLength(1);
    await click(q(v.el, '[data-folder="finance"] > button'));
    expect(options(v.el).map((o) => o.textContent)).toEqual([
      expect.stringContaining('Fake fact 4'),
    ]);
  });

  it('AC-1.6 marks a quarantined memory and a pin', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    const [pinned, , bad] = options(v.el);
    expect(pinned?.querySelector('[aria-label="Pinned"]')).not.toBeNull();
    expect(bad?.querySelector('[data-mark="quarantined"]')).not.toBeNull();
    await click(bad ?? null);
    const row = MODEL.folders[0]?.rows[2] as MemoryFileRow;
    await src.file({
      mogId: MOG_A,
      itemId: row.itemId,
      path: row.path,
      text: fileText(row, '**not markdown**'),
    });
    expect(q(v.el, '[data-memory-quarantine-note]')).not.toBeNull();
    // A quarantined body shows as plain text, never as rendered markdown.
    expect(q(v.el, '.mem-plain')?.textContent).toBe('**not markdown**');
  });

  it('AC-1.7 opens a memory as a file with its path, fields and body', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    const first = options(v.el)[0] ?? null;
    await click(first);
    const row = MODEL.folders[0]?.rows[0] as MemoryFileRow;
    expect(src.actions('memory.file')).toEqual([
      { action: 'memory.file', mogId: MOG_A, itemId: row.itemId },
    ]);
    expect(q(v.el, '[data-memory-path]')?.textContent).toBe('/memories/sales/facts/fake-fact-1.md');
    expect(q(v.el, '[data-memory-file-loading]')).not.toBeNull();
    await src.file({
      mogId: MOG_A,
      itemId: row.itemId,
      path: row.path,
      text: fileText(row, 'The fake mart refreshes at **six**.'),
    });
    expect(q(v.el, '.mem-doc-body strong')?.textContent).toBe('six');
    const fields = [...v.el.querySelectorAll('.mem-field dt')].map((d) => d.textContent);
    expect(fields).toEqual(['id', 'kind', 'title', 'status']);
    expect(q(v.el, '.mem-field:nth-child(3) dd')?.textContent).toBe('Fake fact 1');
  });

  it('a file for another memory is not shown', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    await click(options(v.el)[0] ?? null);
    const other = MODEL.folders[0]?.rows[1] as MemoryFileRow;
    await src.file({ mogId: MOG_A, itemId: other.itemId, path: other.path, text: 'x' });
    expect(q(v.el, '[data-memory-file-loading]')).not.toBeNull();
  });

  it('AC-1.8 Delete needs a confirm click and runs after the Undo time', async () => {
    vi.useFakeTimers();
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    await click(options(v.el)[0] ?? null);
    const row = MODEL.folders[0]?.rows[0] as MemoryFileRow;
    await click(buttonByText(v.el, 'Delete…'));
    expect(src.actions('memory.delete')).toHaveLength(0);
    await click(buttonByText(v.el, 'Delete'));
    // The row hides at once, with Undo.
    expect(options(v.el).map((o) => o.getAttribute('data-item'))).not.toContain(row.itemId);
    expect(q(v.el, '[data-memory-undo]')?.textContent).toContain('Memory deleted');
    expect(src.actions('memory.delete')).toHaveLength(0);
    await act(async () => {
      vi.advanceTimersByTime(MEMORY_UNDO_MS);
    });
    expect(src.actions('memory.delete')).toEqual([
      { action: 'memory.delete', mogId: MOG_A, itemId: row.itemId },
    ]);
    expect(q(v.el, '[data-memory-undo]')).toBeNull();
  });

  it('Undo keeps the memory and sends no delete', async () => {
    vi.useFakeTimers();
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    await click(options(v.el)[0] ?? null);
    await click(buttonByText(v.el, 'Delete…'));
    await click(buttonByText(v.el, 'Delete'));
    await click(buttonByText(v.el, 'Undo'));
    await act(async () => {
      vi.advanceTimersByTime(MEMORY_UNDO_MS * 2);
    });
    expect(src.actions('memory.delete')).toHaveLength(0);
    expect(options(v.el)).toHaveLength(3);
  });

  it('Cancel on the confirm deletes nothing', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    await click(options(v.el)[0] ?? null);
    await click(buttonByText(v.el, 'Delete…'));
    await click(buttonByText(v.el, 'Cancel'));
    expect(buttonByText(v.el, 'Delete…')).not.toBeNull();
    expect(q(v.el, '[data-memory-undo]')).toBeNull();
  });

  it('a pending delete goes at once when the area goes away', async () => {
    vi.useFakeTimers();
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    await click(options(v.el)[0] ?? null);
    await click(buttonByText(v.el, 'Delete…'));
    await click(buttonByText(v.el, 'Delete'));
    await act(async () => v.root.unmount());
    expect(src.actions('memory.delete')).toHaveLength(1);
  });

  it('the arrow keys move in the tree and the list', async () => {
    const src = new LocalSource();
    const v = await render(<MemoryArea active source={src} />);
    await src.list(MODEL);
    const tree = q(v.el, '[role="tree"]') as HTMLElement;
    const key = async (t: HTMLElement, k: string) =>
      act(async () => {
        t.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
      });
    await key(tree, 'ArrowDown');
    expect(q(v.el, '[data-kind-folder="facts"]')?.getAttribute('aria-selected')).toBe('true');
    const list = q(v.el, '[role="listbox"]') as HTMLElement;
    await key(list, 'ArrowDown');
    expect(options(v.el)[0]?.getAttribute('aria-selected')).toBe('true');
    expect(src.actions('memory.file')).toHaveLength(1);
  });
});

describe('hub-memory shapes', () => {
  it('drops bad rows and keeps good ones', () => {
    const m = asHubMemory({
      folders: [
        {
          mogId: MOG_A,
          folder: 'sales',
          mogName: 'Sales',
          rows: [mem(1), { ...mem(2), kind: 'person' }, { ...mem(3), path: '/etc/x' }],
        },
        { mogId: 1 },
      ],
    });
    expect(m?.folders).toHaveLength(1);
    expect(m?.folders[0]?.rows.map((r) => r.title)).toEqual(['Fake fact 1']);
    expect(asHubMemory(null)).toBeUndefined();
  });

  it('splitFile reads the frontmatter and the body', () => {
    const { fields, body } = splitFile('---\nid: x\ntitle: "a: b"\n---\nBody\n');
    expect(fields).toEqual([
      ['id', 'x'],
      ['title', 'a: b'],
    ]);
    expect(body).toBe('Body\n');
    expect(splitFile('no front').body).toBe('no front');
  });
});
