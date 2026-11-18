// @vitest-environment jsdom
/**
 * The domain picker (PG-20) in the hub MOG row (home domain, PG-18) and in the Page review card
 * (one-draft change, PG-19), and the admin checklist text in the card (PG-11, PG-12).
 */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { CardView } from './card/CardView';
import { Hub } from './hub/Hub';
import { HUB_SCOPE } from '../shared/hub-notice';
import { inputEmits as allViewEmits, render, tauri } from './test-tauri';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('./test-tauri')).windowModule);

/** The emits of the pickers: drop `ready` (#507) and the hub-scope `notice.ready` (#27) and `mogs.ready` (#26). */
const viewEmits = () =>
  allViewEmits().filter((e) => (e as { mogId?: unknown }).mogId !== HUB_SCOPE);

const ID = '00000000-0000-4000-8000-000000000001';
const D1 = '7d3c1f2a-1b2c-4d5e-8f90-0a1b2c3d4e5f';
const D2 = '0f9e8d7c-6b5a-4c3d-9e2f-1a0b9c8d7e6f';
const overlays = { attention: false, recording: false, offline: false, localOnly: false };
const card = { mogId: ID, blurbs: [], chips: [] };
const options = [
  { id: D1, name: 'Finance' },
  { id: D2, name: 'Sales' },
];

/** The MOG emits of the hub (hub-scope emits are already dropped). */
const mogEmits = viewEmits;

let roots: Root[] = [];
const mount = async (node: Parameters<typeof render>[0]) => {
  const r = await render(node);
  roots.push(r.root);
  return r;
};

async function choose(select: HTMLSelectElement, value: string) {
  await act(async () => {
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

async function type(input: HTMLInputElement, value: string) {
  await act(async () => {
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    set?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('domain picker views', () => {
  beforeEach(() => {
    tauri.reset();
    tauri.label = 'hub';
  });
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
  });

  it('[PG-18] [PG-20] the hub MOG row shows the home domain and sends a pick via view_emit', async () => {
    const { el } = await mount(<Hub />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays,
      homeDomain: { scope: 'home', current: { id: D1, name: 'Finance' }, options },
    });
    const picker = el.querySelector(`li[data-mog="${ID}"] [data-domain-picker="home"]`);
    expect(picker?.textContent).toContain('Home domain');
    const select = picker?.querySelector('select') as HTMLSelectElement;
    expect(select.getAttribute('aria-label')).toBe('Home domain');
    expect(select.value).toBe(D1);
    expect([...select.options].map((o) => o.textContent)).toEqual(['Finance', 'Sales']);
    await choose(select, D2);
    expect(mogEmits()).toEqual([
      { mogId: ID, kind: 'click', payload: { target: 'domain', scope: 'home', domainId: D2 } },
    ]);
  });

  it('[PG-18] [PG-20] a current domain that is not in the list shows a placeholder, so every option can be picked', async () => {
    const { el } = await mount(<Hub />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays,
      homeDomain: { scope: 'home', current: { id: 'gone-domain' }, options },
    });
    const select = el.querySelector('[data-domain-picker="home"] select') as HTMLSelectElement;
    expect(select.value).toBe('');
    expect([...select.options].map((o) => o.value)).toEqual(['', D1, D2]);
    await choose(select, D1);
    expect(mogEmits()).toEqual([
      { mogId: ID, kind: 'click', payload: { target: 'domain', scope: 'home', domainId: D1 } },
    ]);
  });

  it('[PG-20] without the domain list the hub offers a paste field and sends the raw text', async () => {
    const { el } = await mount(<Hub />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays,
      homeDomain: {
        scope: 'home',
        paste: 'You cannot list domains. Paste the domain ID or its Catalog Explorer URL.',
      },
    });
    const picker = el.querySelector('[data-domain-picker="home"]') as HTMLElement;
    expect(picker.querySelector('select')).toBeNull();
    expect(picker.textContent).toContain('Paste the domain ID or its Catalog Explorer URL.');
    const input = picker.querySelector('input') as HTMLInputElement;
    expect(input.getAttribute('aria-label')).toBe('Domain ID or Catalog Explorer URL');
    await type(input, ` domains/${D1} `);
    await act(async () => (picker.querySelector('button') as HTMLButtonElement).click());
    expect(mogEmits()).toEqual([
      {
        mogId: ID,
        kind: 'click',
        payload: { target: 'domain_paste', scope: 'home', text: ` domains/${D1} ` },
      },
    ]);
  });

  it('[PG-20] the picker shows the error text that the brain sends', async () => {
    const { el } = await mount(<Hub />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays,
      homeDomain: { scope: 'home', paste: 'Paste.', error: 'This is not a domain ID.' },
    });
    expect(el.querySelector('[data-domain-picker] [role="alert"]')?.textContent).toBe(
      'This is not a domain ID.',
    );
  });

  it('[PG-19] the review card changes the domain for this draft only, with its review ID', async () => {
    tauri.label = `card-${ID}`;
    const { el } = await mount(<CardView mogId={ID} />);
    await tauri.emit('card:update', {
      ...card,
      pageDomain: { scope: 'draft', reviewId: 'r1', current: { id: D1, name: 'Finance' }, options },
    });
    const picker = el.querySelector('[data-domain-picker="draft"]') as HTMLElement;
    expect(picker.textContent).toContain('Domain for this draft');
    await choose(picker.querySelector('select') as HTMLSelectElement, D2);
    expect(viewEmits()).toEqual([
      {
        mogId: ID,
        kind: 'click',
        payload: { target: 'domain', scope: 'draft', reviewId: 'r1', domainId: D2 },
      },
    ]);
  });

  it('[PG-11] [PG-12] [AT-41] the card shows the admin checklist that the brain sends', async () => {
    tauri.label = `card-${ID}`;
    const { el } = await mount(<CardView mogId={ID} />);
    await tauri.emit('card:update', {
      ...card,
      pagesChecklist: {
        text: 'No home domain is set.',
        items: ['10. Unity Catalog Pages (Beta) is on in the home workspace.'],
      },
    });
    const box = el.querySelector('[data-pages-checklist]') as HTMLElement;
    expect(box.getAttribute('role')).toBe('alert');
    expect(box.textContent).toContain('No home domain is set.');
    expect([...box.querySelectorAll('li')].map((li) => li.textContent)).toEqual([
      '10. Unity Catalog Pages (Beta) is on in the home workspace.',
    ]);
  });

  it('[PG-20] drops a picker or checklist payload with the wrong shape', async () => {
    tauri.label = `card-${ID}`;
    const { el } = await mount(<CardView mogId={ID} />);
    await tauri.emit('card:update', { ...card, pageDomain: { scope: 'draft' } });
    await tauri.emit('card:update', { ...card, pageDomain: { scope: 'x', reviewId: 'r' } });
    await tauri.emit('card:update', { ...card, pagesChecklist: { text: 1, items: [] } });
    await tauri.emit('card:update', {
      ...card,
      pageDomain: { scope: 'draft', reviewId: 'r', options: [{ id: 1 }] },
    });
    expect(el.querySelector('[data-domain-picker]')).toBeNull();
    expect(el.querySelector('[data-pages-checklist]')).toBeNull();
  });
});
