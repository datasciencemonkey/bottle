// @vitest-environment jsdom
/**
 * The hub Conversations area (#103, #40): the saved Ask Genie exchanges, newest first, as a
 * listbox with day headers and a detail pane. The view renders what the brain sends on
 * `hub:conversations` and runs no store statement (ARC-2). All values are plainly fake.
 */
import { act, useEffect } from 'react';
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
    conversationId: '',
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

async function mount(
  active = true,
  onOpen = vi.fn(),
  composer?: (thread: { conversationId: string; exchangeId: string; profile: string }) => unknown,
) {
  const r = await render(
    <Conversations active={active} onOpen={onOpen} now={() => NOW} composer={composer as never} />,
  );
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

  describe('threads (#145)', () => {
    const CONV = 'conv-shared-1';

    it('[#145] groups exchanges that share a conversationId into one list entry and one thread', async () => {
      const { el } = await mount();
      const thread = [
        row(11, at(19, 9, 0), { question: 'What tables can you see?', conversationId: CONV }),
        row(12, at(19, 9, 5), {
          question: 'How many are there?',
          answerMd: '**152** catalogs.',
          conversationId: CONV,
        }),
      ];
      await tauri.emitTo('hub', 'hub:conversations', { rows: [thread[1], thread[0], ...rows] });
      // One list entry for the whole thread, keyed on its latest exchange.
      expect(options(el).map((o) => o.getAttribute('data-request'))).toEqual([
        thread[1]?.requestId,
        ...rows.map((r) => r.requestId),
      ]);
      // The detail shows every exchange of the thread, oldest first.
      const d = detail(el);
      const exchanges = [...d.querySelectorAll('[data-exchange]')];
      expect(exchanges.map((x) => x.getAttribute('data-exchange'))).toEqual([
        thread[0]?.requestId,
        thread[1]?.requestId,
      ]);
      expect(exchanges[0]?.textContent).toContain('What tables can you see?');
      expect(exchanges[1]?.textContent).toContain('How many are there?');
      expect(exchanges[1]?.textContent).toContain('152');
    });

    it('[#145] the composer is a footer row of the pane, outside the scrolling exchange list', async () => {
      const { el } = await mount(true, vi.fn(), () => <div data-testid="fake-composer" />);
      await tauri.emitTo('hub', 'hub:conversations', {
        rows: [row(40, at(19, 9, 0), { conversationId: 'conv-pin-1' })],
      });
      const pane = detail(el);
      const list = pane.querySelector('.conv-thread') as HTMLElement;
      const slot = pane.querySelector('[data-slot="thread-composer"]') as HTMLElement;
      expect(list.querySelector('[data-exchange]')).not.toBeNull();
      expect(list.contains(slot)).toBe(false);
      expect(slot.parentElement).toBe(pane);
    });

    it('[#145] a new follow-up answer extends the same pane and keeps the composer mounted', async () => {
      const mounts = vi.fn();
      const Probe = () => {
        useEffect(() => {
          mounts();
        }, []);
        return <div data-testid="fake-composer" />;
      };
      const { el } = await mount(true, vi.fn(), () => <Probe />);
      const first = row(11, at(19, 9, 0), { conversationId: CONV });
      await tauri.emitTo('hub', 'hub:conversations', { rows: [first] });
      const pane = detail(el);
      const second = row(12, at(19, 9, 5), { conversationId: CONV, question: 'And then?' });
      await tauri.emitTo('hub', 'hub:conversations', { rows: [second, first] });
      expect(detail(el)).toBe(pane);
      expect(pane.querySelectorAll('[data-exchange]')).toHaveLength(2);
      // The composer is not remounted, so it never announces the thread inactive in between.
      expect(mounts).toHaveBeenCalledTimes(1);
    });

    it('[#145] renders the composer below the thread, with its conversationId, exchangeId and profile', async () => {
      const spy = vi.fn(() => <div data-testid="fake-composer" />);
      const { el } = await mount(true, vi.fn(), spy);
      await tauri.emitTo('hub', 'hub:conversations', {
        rows: [row(20, at(19, 9, 0), { conversationId: CONV, profile: 'fake-west' })],
      });
      expect(spy).toHaveBeenCalledWith({
        conversationId: CONV,
        exchangeId: row(20, at(19, 9, 0)).requestId,
        profile: 'fake-west',
      });
      const slot = detail(el).querySelector('[data-slot="thread-composer"]');
      expect(slot?.querySelector('[data-testid="fake-composer"]')).not.toBeNull();
    });

    it('[review] never mounts the composer while the area is inactive (hub shows another view)', async () => {
      const spy = vi.fn(() => <div data-testid="fake-composer" />);
      const { el } = await mount(false, vi.fn(), spy);
      await tauri.emitTo('hub', 'hub:conversations', {
        rows: [row(22, at(19, 9, 0), { conversationId: CONV, profile: 'fake-west' })],
      });
      expect(spy).not.toHaveBeenCalled();
      expect(el.querySelector('[data-testid="fake-composer"]')).toBeNull();
      expect(el.querySelector('[data-slot="thread-composer"] textarea')).toBeNull();
    });

    it('[#145] no composer for a thread with no conversationId (saved before #584): the hint only', async () => {
      const spy = vi.fn(() => <div data-testid="fake-composer" />);
      const { el } = await mount(true, vi.fn(), spy);
      await tauri.emitTo('hub', 'hub:conversations', { rows });
      expect(spy).not.toHaveBeenCalled();
      const slot = detail(el).querySelector('[data-slot="thread-composer"]');
      expect(slot?.textContent).toBe('Hold Right Option and tap G to ask by voice.');
    });

    it('[#145] with no composer prop at all, the slot shows only the voice hint (#103)', async () => {
      const { el } = await mount();
      await tauri.emitTo('hub', 'hub:conversations', {
        rows: [row(21, at(19, 9, 0), { conversationId: CONV })],
      });
      expect(detail(el).querySelector('[data-slot="thread-composer"]')?.textContent).toBe(
        'Hold Right Option and tap G to ask by voice.',
      );
    });
  });
});

describe('the sticky composer slot (ui-1, #103)', () => {
  beforeEach(() => tauri.reset());

  const composerSlot = (el: HTMLElement) =>
    el.querySelector('[data-slot="thread-composer"]') as HTMLElement;

  it('shows only a static hint when no composer is passed; never a dead input', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows });
    const slot = composerSlot(el);
    expect(slot.textContent).toBe('Hold Right Option and tap G to ask by voice.');
    expect(slot.querySelector('input, textarea, button')).toBeNull();
  });

  it('fills the slot with the composer prop, given the selected thread (conversationId, exchangeId, profile)', async () => {
    const seen: { conversationId: string; exchangeId: string; profile: string }[] = [];
    const { el } = await render(
      <Conversations
        active
        onOpen={vi.fn()}
        now={() => NOW}
        composer={(thread) => {
          seen.push(thread);
          return <button type="button">Ask about this</button>;
        }}
      />,
    );
    const r = row(30, at(19, 9, 0), { conversationId: 'conv-slot-1', profile: 'fake-west' });
    await tauri.emitTo('hub', 'hub:conversations', { rows: [r] });
    const slot = composerSlot(el);
    expect(slot.querySelector('button')?.textContent).toBe('Ask about this');
    expect(seen).toEqual([
      { conversationId: 'conv-slot-1', exchangeId: r.requestId, profile: 'fake-west' },
    ]);
  });
});

describe('hub Conversations links (#523)', () => {
  beforeEach(() => tauri.reset());
  const WS = 'https://acme-prod.cloud.databricks.test';
  const DEEP = `${WS}/one/chat/threads/c1?o=1234&utm_source=genie_chat_mcp`;
  const one = (over: Partial<ConversationRow>) => [
    row(1, at(19, 14, 2), { workspaceHost: WS, ...over }),
  ];
  const linkClicks = () =>
    viewEmits().filter(
      (e) => (e as { payload?: { target?: string } }).payload?.target === 'link_open',
    ) as { mogId: string; payload: { url: string; requestId: string } }[];

  it('[#523] shows Open in Databricks in the thread header and sends link_open with the exchange', async () => {
    const { el } = await mount();
    const rows1 = one({ deepLink: DEEP });
    await tauri.emitTo('hub', 'hub:conversations', { rows: rows1 });
    const a = el.querySelector('a[data-action="open-in-databricks"]') as HTMLAnchorElement;
    expect(a.textContent).toBe('Open in Databricks');
    expect(a.getAttribute('title')).toBe(DEEP);
    const ev = new MouseEvent('click', { bubbles: true, cancelable: true, detail: 1 });
    await act(async () => void a.dispatchEvent(ev));
    expect(ev.defaultPrevented).toBe(true);
    expect(linkClicks()).toEqual([
      {
        mogId: 'conversations',
        kind: 'click',
        payload: { target: 'link_open', url: DEEP, requestId: rows1[0]?.requestId },
      },
    ]);
  });

  it.each([
    ['no deep_link', { deepLink: '' }],
    ['a deep_link of another host', { deepLink: 'https://evil.test/one/chat/threads/c1' }],
    ['a spoofed host', { deepLink: `${WS}.evil.test/x` }],
    ['an old row with no deepLink field', {}],
  ])('[#523] shows no Open in Databricks for %s', async (_n, over) => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', { rows: one(over) });
    expect(el.querySelector('[data-action="open-in-databricks"]')).toBeNull();
  });

  it('[#523] a trusted link in the answer sends link_open, never navigates; another host is text', async () => {
    const { el } = await mount();
    const r = one({ answerMd: `[chat](${WS}/a/b) and [docs](https://evil.test/d) and ${WS}/c` });
    await tauri.emitTo('hub', 'hub:conversations', { rows: r });
    const links = [...el.querySelectorAll('.conv-answer a')] as HTMLAnchorElement[];
    expect(links.map((a) => a.getAttribute('href'))).toEqual([`${WS}/a/b`, `${WS}/c`]);
    const evs = links.map(
      () => new MouseEvent('click', { bubbles: true, cancelable: true, detail: 1 }),
    );
    await act(async () => links.forEach((a, i) => a.dispatchEvent(evs[i] as Event)));
    expect(evs.every((e) => e.defaultPrevented)).toBe(true);
    expect(linkClicks().map((c) => c.payload.url)).toEqual([`${WS}/a/b`, `${WS}/c`]);
    expect(linkClicks().every((c) => c.payload.requestId === r[0]?.requestId)).toBe(true);
    expect(el.querySelector('.conv-answer .md-link--blocked')?.textContent).toContain('evil.test');
  });

  it('[#523] a table in a thread renders with aligned, readable numbers', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', 'hub:conversations', {
      rows: one({ answerMd: '| name | total |\n| --- | ---: |\n| Apple | 1220042.3509332137 |' }),
    });
    expect(el.querySelector('.conv-answer table.md-table')).not.toBeNull();
    const td = el.querySelector('.conv-answer tbody td:last-child') as HTMLElement;
    expect(td.getAttribute('data-align')).toBe('right');
    expect(td.getAttribute('title')).toBe('1220042.3509332137');
  });
});
