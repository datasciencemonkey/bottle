// @vitest-environment jsdom
/** UX slice 1 (AC-1.13): the window fits again when a stacked pop-up leaves the pill alone. */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { render, tauri } from '../test-tauri';
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
});
