// @vitest-environment jsdom
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { inputEmits as inputs, render, tauri, viewEmits } from '../test-tauri';
import { CardView } from './CardView';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-000000000001';
const OTHER = '00000000-0000-4000-8000-000000000002';
const update = {
  mogId: ID,
  blurbs: [{ id: 'b1', text: 'Revenue was 4.2M' }],
  chips: [{ id: 'c1', label: 'Ask Genie' }],
  answerMd: '**4.2M** in Q3',
};

/** Window-level key listeners outlive a test unless the card unmounts, so each test unmounts. */
let roots: Root[] = [];
const mountCard = async (node: Parameters<typeof render>[0]) => {
  const r = await render(node);
  roots.push(r.root);
  return r;
};

describe('V3 card view', () => {
  beforeEach(() => tauri.reset());
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
  });

  it('[V3] subscribes to card:update and renders blurbs, chips and the answer as text', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    expect(tauri.listeners('card:update')).toBe(1);
    await tauri.emit('card:update', update);
    expect(el.textContent).toContain('Revenue was 4.2M');
    expect(el.querySelector('button[data-chip="c1"]')?.textContent).toBe('Ask Genie');
    expect(el.querySelector('[data-answer]')?.textContent).toBe('**4.2M** in Q3');
    expect(el.querySelector('strong')).toBeNull();
  });

  it('[MOG-05b] sends view_emit ready once, after it listens to card:update (#507)', async () => {
    tauri.label = `card-${ID}`;
    let listening = -1;
    tauri.invoke.mockImplementation(async (cmd: string, a?: unknown) => {
      const k = (a as { args?: { kind?: string } } | undefined)?.args?.kind;
      if (cmd === 'view_emit' && k === 'ready') listening = tauri.listeners('card:update');
      return {};
    });
    await mountCard(<CardView mogId={ID} />);
    expect(viewEmits().filter((e) => (e as { kind: string }).kind === 'ready')).toEqual([
      { mogId: ID, kind: 'ready' },
    ]);
    expect(listening).toBe(1);
    tauri.invoke.mockImplementation(async () => ({}));
    tauri.label = 'hub';
  });

  it('[V3] ignores updates for another MOG and malformed payloads', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...update, mogId: OTHER });
    await tauri.emit('card:update', { mogId: ID, blurbs: 'x' });
    expect(el.textContent).not.toContain('Revenue');
  });

  it('[V3] [ARC-2] sends chip, expand and cancel input only through view_emit', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', update);
    await act(async () => (el.querySelector('button[data-chip="c1"]') as HTMLElement).click());
    await act(async () =>
      (el.querySelector('button[data-action="expand"]') as HTMLElement).click(),
    );
    await act(async () =>
      el
        .querySelector('section')
        ?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })),
    );
    expect(inputs()).toEqual([
      { mogId: ID, kind: 'chip_click', payload: { chipId: 'c1' } },
      { mogId: ID, kind: 'expand' },
      { mogId: ID, kind: 'cancel' },
    ]);
  });

  const esc = (init: KeyboardEventInit = {}) =>
    act(async () => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', ...init }));
    });

  it('[UX-19b] Esc anywhere in the card window sends exactly one cancel', async () => {
    await mountCard(<CardView mogId={ID} />);
    await esc();
    expect(inputs()).toEqual([{ mogId: ID, kind: 'cancel' }]);
  });

  it('[UX-19b] a held Esc (key repeat) sends no second cancel', async () => {
    await mountCard(<CardView mogId={ID} />);
    await esc();
    await esc({ repeat: true });
    await esc({ repeat: true });
    expect(inputs()).toEqual([{ mogId: ID, kind: 'cancel' }]);
  });

  it('[UX-19b] [MOG-03a] Esc during IME composition only ends the composition', async () => {
    await mountCard(<CardView mogId={ID} />);
    await esc({ isComposing: true });
    await esc({ keyCode: 229 });
    expect(inputs()).toEqual([]);
  });

  it('[UX-19b] other keys send nothing', async () => {
    await mountCard(<CardView mogId={ID} />);
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    });
    expect(inputs()).toEqual([]);
  });

  it('[UX-19b] an unmounted card sends no cancel', async () => {
    const { root } = await render(<CardView mogId={ID} />);
    await act(async () => root.unmount());
    await esc();
    expect(inputs()).toEqual([]);
  });

  it('[MOG-03a] the dev demo card has a typing probe for the Mac check; other cards do not', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    expect(el.querySelector('textarea')).toBeNull();
    const demo = await mountCard(<CardView mogId="demo" />);
    expect(demo.el.querySelector('textarea[data-probe]')).not.toBeNull();
  });
  const stack = {
    mogId: ID,
    blurbs: [
      {
        id: 'g1',
        text: 'Revenue by region',
        pinned: false,
        completed: true,
        parts: [
          { id: 'p1', text: 'EMEA 1.1M' },
          { id: 'p2', text: 'AMER 2.9M' },
          { id: 'p3', text: 'APJ 0.2M' },
        ],
      },
      { id: 'b2', text: 'Churn was 3%', pinned: true, completed: true },
    ],
    chips: [],
    historyCount: 4,
  };
  const items = (el: HTMLElement) => [...el.querySelectorAll('li[data-blurb]')];

  it('[UX-11a] renders the blurbs in brain order and "N more in History"', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', stack);
    expect(items(el).map((li) => li.getAttribute('data-blurb'))).toEqual(['g1', 'b2']);
    const link = el.querySelector('button[data-action="history"]') as HTMLElement;
    expect(link.textContent).toBe('4 more in History');
    await act(async () => link.click());
    expect(inputs()).toEqual([{ mogId: ID, kind: 'click', payload: { target: 'history' } }]);
  });

  it('[UX-11a] shows no History link when nothing is in History', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...stack, historyCount: 0 });
    expect(el.querySelector('button[data-action="history"]')).toBeNull();
    await tauri.emit('card:update', update);
    expect(el.querySelector('button[data-action="history"]')).toBeNull();
  });

  it('[UX-11a] [UX-11b] drops a payload with bad split parts or a bad History count', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...stack, historyCount: -1 });
    await tauri.emit('card:update', { ...stack, historyCount: '4' });
    await tauri.emit('card:update', { ...stack, blurbs: [{ id: 'g', text: 't', parts: 'x' }] });
    expect(items(el)).toHaveLength(0);
  });

  it('[UX-11b] renders a 3-part split as one blurb with its parts', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', stack);
    expect(items(el)).toHaveLength(2);
    const parts = [...el.querySelectorAll('li[data-blurb="g1"] [data-part]')];
    expect(parts.map((p) => p.textContent)).toEqual(['EMEA 1.1M', 'AMER 2.9M', 'APJ 0.2M']);
  });

  it('[UX-11c] shows the pin state and sends a pin click through view_emit', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', stack);
    const pin = (id: string) =>
      el.querySelector(`li[data-blurb="${id}"] button[data-action="pin"]`) as HTMLElement;
    expect(pin('g1').getAttribute('aria-pressed')).toBe('false');
    expect(pin('b2').getAttribute('aria-pressed')).toBe('true');
    await act(async () => pin('g1').click());
    expect(inputs()).toEqual([
      { mogId: ID, kind: 'click', payload: { target: 'pin', blurbId: 'g1' } },
    ]);
  });

  it('[UX-12] sends pointer hover and keyboard focus of a blurb to the brain', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', stack);
    const li = el.querySelector('li[data-blurb="b2"]') as HTMLElement;
    const pin = li.querySelector('button[data-action="pin"]') as HTMLElement;
    await act(async () => {
      li.dispatchEvent(
        new MouseEvent('mouseover', { bubbles: true, relatedTarget: document.body }),
      );
    });
    await act(async () => {
      li.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body }));
    });
    await act(async () => pin.focus());
    await act(async () => pin.blur());
    expect(inputs()).toEqual([
      { mogId: ID, kind: 'hover', payload: { blurbId: 'b2', on: true, via: 'pointer' } },
      { mogId: ID, kind: 'hover', payload: { blurbId: 'b2', on: false, via: 'pointer' } },
      { mogId: ID, kind: 'hover', payload: { blurbId: 'b2', on: true, via: 'focus' } },
      { mogId: ID, kind: 'hover', payload: { blurbId: 'b2', on: false, via: 'focus' } },
    ]);
  });

  it('[UX-38] [AT-36] the card footer shows the STT badge the brain sends', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...update, sttBadge: 'STT: Databricks m1' });
    const footer = el.querySelector('footer [data-badge="stt"]') as HTMLElement;
    expect(footer.textContent).toBe('STT: Databricks m1');
    await tauri.emit('card:update', update);
    expect(el.querySelector('[data-badge="stt"]')).toBeNull();
  });

  it('[UX-38] drops a card update with a non-text STT badge', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...update, sttBadge: 42 });
    expect(el.textContent).not.toContain('Revenue');
  });
});

describe('V3 card view: expanded sections and chips', () => {
  beforeEach(() => tauri.reset());
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
  });

  const sections = [
    { id: 'answer', md: '**4.2M** in Q3' },
    { id: 'keyNumbers', items: ['4.2M', '1.1M'] },
    { id: 'sql', sql: 'SELECT 1', collapsed: true },
    {
      id: 'tablePreview',
      columns: ['region', 'revenue'],
      rows: [['EMEA', '4.2M']],
      moreRows: 0,
    },
    { id: 'sentToGenie', text: 'Context sent: 12 characters' },
    { id: 'resend' },
    { id: 'copy', formats: ['markdown', 'csv'] },
    { id: 'footer', sttBadge: 'STT: local', deepLink: 'https://example.com/g' },
    { id: 'dataFlow', text: 'Audio stays on this Mac.' },
  ];

  it('[UX-18] [AT-09] renders the sections in the order the brain sends', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...update, sections });
    const ids = [...el.querySelectorAll('[data-section]')].map((n) =>
      n.getAttribute('data-section'),
    );
    expect(ids).toEqual(sections.map((s) => s.id));
    expect(el.querySelector('[data-section="keyNumbers"]')?.textContent).toContain('4.2M');
    expect(el.querySelector('[data-section="tablePreview"] td')?.textContent).toBe('EMEA');
    expect(el.querySelector('[data-section="footer"] [data-badge="stt"]')?.textContent).toBe(
      'STT: local',
    );
  });

  it('[UX-18] the SQL section is collapsed until the user opens it', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...update, sections });
    const d = el.querySelector('[data-section="sql"] details') as HTMLDetailsElement;
    expect(d.open).toBe(false);
  });

  it('[UX-18] Resend without context and Copy send a click to the brain', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...update, sections });
    await act(async () =>
      (el.querySelector('button[data-action="resend"]') as HTMLElement).click(),
    );
    await act(async () => (el.querySelector('button[data-copy="csv"]') as HTMLElement).click());
    expect(inputs()).toEqual([
      { mogId: ID, kind: 'click', payload: { target: 'resend' } },
      { mogId: ID, kind: 'click', payload: { target: 'copy', format: 'csv' } },
    ]);
  });

  it('[UX-20a] [AT-20] chips beyond the 4th show in the expanded card', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', {
      ...update,
      sections,
      overflowChips: [{ id: 'c5', label: 'Check again' }],
    });
    expect(el.querySelector('[data-section="footer"] button[data-chip="c5"]')?.textContent).toBe(
      'Check again',
    );
  });

  it('[UX-20b] [AT-20] every chip is a button with a text label', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', update);
    const chips = [...el.querySelectorAll('[data-chip]')];
    expect(chips.length).toBeGreaterThan(0);
    for (const c of chips) {
      expect(c.tagName).toBe('BUTTON');
      expect((c.textContent ?? '').trim().length).toBeGreaterThan(0);
    }
  });

  it('[UX-20c] [AT-20] hover and focus on a chip send nothing; a click sends one chip_click', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', update);
    const chip = el.querySelector('button[data-chip="c1"]') as HTMLElement;
    await act(async () => {
      chip.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }));
      chip.dispatchEvent(new MouseEvent('mouseenter'));
      chip.dispatchEvent(new PointerEvent('pointermove', { bubbles: true }));
    });
    await act(async () => chip.focus());
    await act(async () => chip.blur());
    expect((inputs() as { kind: string }[]).filter((e) => e.kind === 'chip_click')).toEqual([]);
    await act(async () => chip.click());
    expect(inputs()).toEqual([{ mogId: ID, kind: 'chip_click', payload: { chipId: 'c1' } }]);
  });

  it('[UX-18] drops a card update with a bad section list', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...update, sections: [{ id: 'nope' }] });
    expect(el.textContent).not.toContain('Revenue');
  });
});
