// @vitest-environment jsdom
/** UX slice 1 (AC-1.13): the window fits again when a stacked pop-up leaves the pill alone. */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { inputEmits, render, tauri } from '../test-tauri';
import { CardView } from './CardView';
import { FADE_AFTER_MS, FADE_OUT_MS } from './Popup';

const fits = vi.hoisted(() => ({ calls: [] as number[] }));
vi.mock('./fit-window', async (orig) => ({
  ...(await orig<typeof import('./fit-window')>()),
  createWindowFit: () => async (px: number) => void fits.calls.push(px),
}));
vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-000000000001';
const G = 'genie:00000000-0000-4000-8000-00000000c573';
const PILL_PX = 62;
const POPUP_PX = 48;

/** jsdom has no layout: the card section is as tall as the rows that it holds. */
const realRect = HTMLElement.prototype.getBoundingClientRect;
function fakeLayout(this: HTMLElement) {
  const rows = this.querySelector('[data-genie]') ? PILL_PX : 0;
  const popup = this.querySelector('[data-popup]') ? POPUP_PX : 0;
  return { ...realRect.call(this), height: rows + popup } as DOMRect;
}

let roots: Root[] = [];
describe('stacked pill and pop-up: the window fit (UX slice 1)', () => {
  beforeEach(() => {
    tauri.reset();
    fits.calls = [];
    vi.useFakeTimers();
    HTMLElement.prototype.getBoundingClientRect = fakeLayout;
  });
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
    HTMLElement.prototype.getBoundingClientRect = realRect;
    vi.useRealTimers();
    tauri.label = 'hub';
  });

  it('[AC-1.13] fits to both rows, then to the pill alone after the pop-up fades', async () => {
    tauri.label = `card-${ID}`;
    const r = await render(<CardView mogId={ID} />);
    roots.push(r.root);
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      mogId: ID,
      blurbs: [
        { id: 'd1', text: 'Quick update', completed: true },
        { id: G, text: 'Running the query', pinned: false, completed: false },
      ],
      chips: [],
    });
    expect(fits.calls.at(-1)).toBe(PILL_PX + POPUP_PX);
    await act(async () => void vi.advanceTimersByTime(FADE_AFTER_MS));
    await act(async () => void vi.advanceTimersByTime(FADE_OUT_MS));
    expect(r.el.querySelector('[data-popup]')).toBeNull();
    // The window follows the real height of the section: the pill alone.
    expect(fits.calls.at(-1)).toBe(PILL_PX);
    expect(fits.calls.filter((p) => p === PILL_PX).length).toBeGreaterThan(0);
  });
  it('a ready pill that has the sections, under a newer pop-up, shows no answer in the hold', async () => {
    tauri.label = `card-${ID}`;
    const r = await render(<CardView mogId={ID} />);
    roots.push(r.root);
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      mogId: ID,
      // The newest blurb is a dictation pop-up; the pending Genie pill says "Answer ready".
      blurbs: [
        { id: 'd1', text: 'Quick update', completed: true },
        { id: G, text: 'Answer ready', pinned: false, completed: false },
        { id: 'ask:u1', text: '"What was churn?"', pinned: false, completed: true },
      ],
      chips: [],
      // The brain puts the sections on with the ready pill (for the deep link).
      sections: [
        { id: 'answer', md: 'Churn was 4.2%.' },
        { id: 'keyNumbers', items: ['4.2%'] },
        {
          id: 'footer',
          deepLink: 'https://acme.cloud.databricks.test/one/chat/threads/c1?o=1',
          workspaceHost: 'https://acme.cloud.databricks.test',
        },
      ],
    });
    expect(r.el.querySelector('[data-genie="working"][data-ready]')).not.toBeNull();
    expect(r.el.querySelector('[data-popup]')).not.toBeNull();
    expect(r.el.querySelector('.card-panel')).toBeNull();
    expect(r.el.querySelector('[data-section="answer"]')).toBeNull();
    expect(r.el.querySelector('[data-answer]')).toBeNull();
    // The pill still has its link, read from the sections.
    expect(r.el.querySelector('a[data-action="open-in-databricks"]')).not.toBeNull();
  });

  it('a close of the stacked pill leaves the pop-up and sends no card_done', async () => {
    tauri.label = `card-${ID}`;
    const r = await render(<CardView mogId={ID} />);
    roots.push(r.root);
    const upd = (extra: object[] = []) => ({
      mogId: ID,
      blurbs: [
        { id: 'd1', text: 'Quick update', completed: true },
        { id: G, text: 'Running the query', pinned: false, completed: false },
        ...extra,
      ],
      chips: [],
    });
    await tauri.emitTo(`card-${ID}`, 'card:update', upd());
    const close = r.el.querySelector(
      '[data-genie="working"] button[data-action="dismiss"]',
    ) as HTMLElement;
    await act(async () => void close.click());
    expect(r.el.querySelector('[data-genie]')).toBeNull();
    expect(r.el.querySelector('[data-popup]')).not.toBeNull();
    expect(inputEmits().filter((e) => (e as { kind: string }).kind === 'click')).toEqual([]);
    // The answer lands: the card shows it again.
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      mogId: ID,
      // The real stack keeps the older pop-up blurb, under the newest one.
      blurbs: [
        { id: G, text: 'Churn was 4.2%.', pinned: false, completed: true },
        { id: 'd1', text: 'Quick update', completed: true },
      ],
      chips: [],
      sections: [{ id: 'answer', md: 'Churn was 4.2%.' }],
    });
    // The answer card is the newest blurb: it shows alone, the old pop-up does not.
    expect(r.el.querySelector('[data-genie="answer"]')).not.toBeNull();
    expect(r.el.querySelector('[data-popup]')).toBeNull();
  });
});

describe('the pop-up that wraps its action row: the window fit measures again', () => {
  let height = 48;
  beforeEach(() => {
    tauri.reset();
    fits.calls = [];
    height = 48;
    vi.useFakeTimers();
    HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
      return { ...realRect.call(this), height } as DOMRect;
    };
  });
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
    HTMLElement.prototype.getBoundingClientRect = realRect;
    vi.useRealTimers();
    tauri.label = 'hub';
  });

  it('[#559] a window resize after the width fit reads the new height of a wrapped row', async () => {
    tauri.label = `card-${ID}`;
    const r = await render(<CardView mogId={ID} />);
    roots.push(r.root);
    await tauri.emitTo(`card-${ID}`, 'card:update', {
      mogId: ID,
      blurbs: [{ id: 'ask:u1', text: '"Top ten customers"', completed: true }],
      chips: [
        { id: 'ask_genie:u1', label: 'Ask Genie' },
        { id: 'ws:other', label: 'Ask other workspaces too' },
      ],
    });
    expect(fits.calls.at(-1)).toBe(48);
    // The narrower window wraps the chips onto a second row: the section is taller now.
    height = 92;
    await act(async () => void window.dispatchEvent(new Event('resize')));
    expect(fits.calls.at(-1)).toBe(92);
  });
});
