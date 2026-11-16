// @vitest-environment jsdom
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { render, tauri, viewEmits } from '../test-tauri';
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
    expect(viewEmits()).toEqual([
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
    expect(viewEmits()).toEqual([{ mogId: ID, kind: 'cancel' }]);
  });

  it('[UX-19b] a held Esc (key repeat) sends no second cancel', async () => {
    await mountCard(<CardView mogId={ID} />);
    await esc();
    await esc({ repeat: true });
    await esc({ repeat: true });
    expect(viewEmits()).toEqual([{ mogId: ID, kind: 'cancel' }]);
  });

  it('[UX-19b] [MOG-03a] Esc during IME composition only ends the composition', async () => {
    await mountCard(<CardView mogId={ID} />);
    await esc({ isComposing: true });
    await esc({ keyCode: 229 });
    expect(viewEmits()).toEqual([]);
  });

  it('[UX-19b] other keys send nothing', async () => {
    await mountCard(<CardView mogId={ID} />);
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    });
    expect(viewEmits()).toEqual([]);
  });

  it('[UX-19b] an unmounted card sends no cancel', async () => {
    const { root } = await render(<CardView mogId={ID} />);
    await act(async () => root.unmount());
    await esc();
    expect(viewEmits()).toEqual([]);
  });

  it('[MOG-03a] the dev demo card has a typing probe for the Mac check; other cards do not', async () => {
    const { el } = await mountCard(<CardView mogId={ID} />);
    expect(el.querySelector('textarea')).toBeNull();
    const demo = await mountCard(<CardView mogId="demo" />);
    expect(demo.el.querySelector('textarea[data-probe]')).not.toBeNull();
  });
});
