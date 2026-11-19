// @vitest-environment jsdom
/**
 * The hub Conversations area (#103, #40): the saved Ask Genie exchanges, newest first, as a
 * listbox with day headers and a detail pane. The view renders what the brain sends on
 * `hub:conversations` and runs no store statement (ARC-2). All values are plainly fake.
 */
import { act } from 'react';
import { render, tauri, viewEmits } from '../test-tauri';
import { Conversations } from './Conversations';
import type { ConversationRow } from '../../shared/hub-conversations';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const NOW = new Date(2026, 10, 19, 16, 30).getTime();
const at = (d: number, h: number, m: number) => new Date(2026, 10, d, h, m).getTime();
const HOST = 'example.cloud.databricks.test';

function row(n: number, askedAt: number, over: Partial<ConversationRow> = {}): ConversationRow {
  return {
    requestId: `41111111-2222-4333-8444-${String(n).padStart(12, '0')}`,
    mogId: '00000000-0000-4000-8000-00000000000a',
    mogName: 'Sales',
    question: `What were fake bookings ${n}?`,
    answerMd: `**Bookings** rose to $${n}M.`,
    keyNumbers: [{ text: `$${n}M`, label: 'bookings' }],
    askedAt,
    profile: 'fake-west',
    workspaceHost: HOST,
    status: 'answered',
    ...over,
  };
}

const rows = [
  row(3, at(19, 14, 2)),
  row(2, at(18, 9, 15), {
    status: 'failed',
    answerMd: '',
    keyNumbers: [],
    question: 'Fake failed question?',
  }),
  row(1, at(3, 11, 0)),
];

const ready = () =>
  viewEmits().filter(
    (e) => (e as { payload?: { action?: string } }).payload?.action === 'conversations.ready',
  );
const options = (el: HTMLElement) => [...el.querySelectorAll('[role="option"]')] as HTMLElement[];
const listbox = (el: HTMLElement) => el.querySelector('[role="listbox"]') as HTMLElement;
const selected = (el: HTMLElement) =>
  el.querySelector('[role="option"][aria-selected="true"]')?.getAttribute('data-request');
const detail = (el: HTMLElement) => el.querySelector('[data-conversation-detail]') as HTMLElement;
const key = async (target: HTMLElement, k: string) =>
  act(async () => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
  });

async function mount(active = true, onOpen = vi.fn()) {
  const r = await render(<Conversations active={active} onOpen={onOpen} now={() => NOW} />);
  return { ...r, onOpen };
}

describe('hub Conversations (#103)', () => {
  beforeEach(() => tauri.reset());

  it('[#103] [ARC-5] listens to this window only, then asks the brain for the list', async () => {
    await mount();
    expect(tauri.listenOptions.get('hub:conversations')).toEqual([{ target: 'hub' }]);
    expect(tauri.listenOptions.get('hub:conversations.open')).toEqual([{ target: 'hub' }]);
    expect(ready()).toEqual([
      { mogId: 'conversations', kind: 'click', payload: { action: 'conversations.ready' } },
    ]);
    // The view runs no store statement.
    expect(tauri.invoke.mock.calls.some(([c]) => c === 'mem_local_exec')).toBe(false);
  });

  it('[#103] asks again each time the area shows', async () => {
    const r = await mount(false);
    expect(ready()).toHaveLength(1);
    await act(async () =>
      r.root.render(<Conversations active={true} onOpen={vi.fn()} now={() => NOW} />),
    );
    expect(ready()).toHaveLength(2);
  });

  it('[#103] shows the empty state when there is no exchange', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows: [] });
    expect(el.textContent).toContain('No conversations yet.');
    expect(el.textContent).toContain('Hold Right Option and tap G to ask Genie.');
    expect(listbox(el)).toBeNull();
  });

  it('[#103] says so, with no error text, when the brain could not read the store', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows: [], unavailable: true });
    expect(el.textContent).toContain('Bottle cannot read your saved conversations right now.');
    expect(el.textContent).not.toContain('No conversations yet.');
  });

  it('[#103] lists newest first under day headers, with time and profile, and selects the newest', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows });
    expect(listbox(el).getAttribute('aria-label')).toBe('Conversations');
    const groups = [...el.querySelectorAll('[role="group"]')].map((g) =>
      g.getAttribute('aria-label'),
    );
    expect(groups[0]).toBe('Today');
    expect(groups[1]).toBe('Yesterday');
    expect(groups[2]).toMatch(/3/);
    expect(options(el).map((o) => o.getAttribute('data-request'))).toEqual(
      rows.map((r) => r.requestId),
    );
    const first = options(el)[0];
    expect(first?.textContent).toContain('What were fake bookings 3?');
    expect(first?.textContent).toContain('fake-west');
    expect(first?.textContent).toMatch(/2:02|14:02/);
    // A failed exchange says so in its row.
    expect(options(el)[1]?.textContent).toContain('Failed');
    expect(selected(el)).toBe(rows[0]?.requestId);
  });

  it('[#103] [SEC-25a] the detail shows the question, meta, safe markdown and key numbers', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', {
      rows: [
        row(5, at(19, 14, 2), {
          answerMd:
            '**Revenue** rose.\n\n<img src=x onerror=alert(1)>\n\n[link](https://evil.example/x) and [ws](https://example.cloud.databricks.test/genie)',
          keyNumbers: [{ text: '4.2 %', label: 'growth' }, { text: '$1.3M' }],
        }),
      ],
    });
    const d = detail(el);
    expect(d.querySelector('h2')?.textContent).toBe('What were fake bookings 5?');
    expect(d.textContent).toContain('fake-west');
    // The host is in the tooltip and, for screen readers, in hidden text.
    const profile = d.querySelector(`[title="${HOST}"]`);
    expect(profile?.textContent).toBe(`fake-west, ${HOST}`);
    expect(profile?.querySelector('.conv-sr')?.textContent).toBe(`, ${HOST}`);
    expect(d.textContent).toContain('Sales');
    expect(d.textContent).toContain('Answered');
    // Markdown through the card's allowlist renderer: no HTML element, only trusted links.
    expect(d.querySelector('strong')?.textContent).toBe('Revenue');
    expect(d.querySelector('img')).toBeNull();
    const links = [...d.querySelectorAll('a')].map((a) => a.getAttribute('href'));
    expect(links).toEqual(['https://example.cloud.databricks.test/genie']);
    const kn = [...d.querySelectorAll('[data-key-number]')].map((k) => k.textContent);
    expect(kn).toEqual(['4.2 %growth', '$1.3M']);
  });

  it('[ui-4] the list row and the detail show the same MOG colour dot', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', {
      rows: [row(6, at(19, 14, 2), { mogColorVariant: 'teal' })],
    });
    const listDot = el.querySelector('.conv-row .conv-mog-dot') as HTMLElement;
    expect(listDot.style.backgroundColor).toBe('rgb(15, 107, 102)');
    const detailDot = detail(el).querySelector('.conv-mog-dot') as HTMLElement;
    expect(detailDot.style.backgroundColor).toBe(listDot.style.backgroundColor);
  });

  it('[#103] a failed exchange shows its status and a calm line instead of an answer', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows });
    await act(async () => options(el)[1]?.click());
    expect(selected(el)).toBe(rows[1]?.requestId);
    const d = detail(el);
    expect(d.querySelector('h2')?.textContent).toBe('Fake failed question?');
    expect(d.textContent).toContain('Failed');
    expect(d.textContent).toContain('Genie could not answer this question.');
    expect(d.querySelector('[data-key-number]')).toBeNull();
  });

  it('[#103] keyboard: a listbox; Up, Down, Home and End move the selection, Enter opens the detail', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows });
    const lb = listbox(el);
    expect(lb.tabIndex).toBe(0);
    expect(lb.getAttribute('aria-activedescendant')).toBe(options(el)[0]?.id);
    await key(lb, 'ArrowDown');
    expect(selected(el)).toBe(rows[1]?.requestId);
    expect(lb.getAttribute('aria-activedescendant')).toBe(options(el)[1]?.id);
    await key(lb, 'ArrowDown');
    await key(lb, 'ArrowDown');
    expect(selected(el)).toBe(rows[2]?.requestId);
    await key(lb, 'ArrowUp');
    expect(selected(el)).toBe(rows[1]?.requestId);
    await key(lb, 'Home');
    expect(selected(el)).toBe(rows[0]?.requestId);
    await key(lb, 'End');
    expect(selected(el)).toBe(rows[2]?.requestId);
    lb.focus();
    await key(lb, 'Enter');
    expect(document.activeElement).toBe(detail(el));
  });

  it('[#103] an open from a double-click shows the area and selects that exchange, also before the list', async () => {
    const { el, onOpen } = await mount(false);
    await tauri.emitTo('hub', 'hub:conversations.open', { requestId: rows[1]?.requestId });
    expect(onOpen).toHaveBeenCalledTimes(1);
    await tauri.emitTo('hub', 'hub:conversations', { rows });
    expect(selected(el)).toBe(rows[1]?.requestId);
    // A newer list keeps the user's selection.
    await tauri.emitTo('hub', 'hub:conversations', { rows: [row(9, at(19, 16, 0)), ...rows] });
    expect(selected(el)).toBe(rows[1]?.requestId);
    // An open with no id shows the area and keeps the selection.
    await tauri.emitTo('hub', 'hub:conversations.open', {});
    expect(onOpen).toHaveBeenCalledTimes(2);
    expect(selected(el)).toBe(rows[1]?.requestId);
  });

  it('[#103] the hub acks each open, so the brain stops holding it', async () => {
    await mount(false);
    await tauri.emitTo('hub', 'hub:conversations.open', { requestId: rows[1]?.requestId });
    expect(
      viewEmits().filter(
        (e) => (e as { payload?: { action?: string } }).payload?.action === 'conversations.opened',
      ),
    ).toEqual([
      { mogId: 'conversations', kind: 'click', payload: { action: 'conversations.opened' } },
    ]);
  });

  it('[#103] a click selects a row and puts keyboard focus on the list', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows });
    await act(async () => options(el)[1]?.click());
    expect(document.activeElement).toBe(listbox(el));
    await key(listbox(el), 'ArrowDown');
    expect(selected(el)).toBe(rows[2]?.requestId);
  });

  it('[#103] a click on an answer link never navigates the hub window', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', {
      rows: [
        row(6, at(19, 14, 2), {
          answerMd: 'See [ws](https://example.cloud.databricks.test/genie)',
        }),
      ],
    });
    const a = detail(el).querySelector('a') as HTMLAnchorElement;
    const ev = new MouseEvent('click', { bubbles: true, cancelable: true });
    await act(async () => void a.dispatchEvent(ev));
    expect(ev.defaultPrevented).toBe(true);
  });

  it('[#103] a bad payload changes nothing', async () => {
    const { el, onOpen } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows });
    await tauri.emitTo('hub', 'hub:conversations', { rows: [{ requestId: 7 }] });
    await tauri.emitTo('hub', 'hub:conversations.open', { requestId: 7 });
    expect(options(el)).toHaveLength(3);
    expect(onOpen).not.toHaveBeenCalled();
  });
});
