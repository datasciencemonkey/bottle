// @vitest-environment jsdom
/**
 * #559: the small post-dictation pop-up beside the owner MOG. It replaces the raw card list:
 * one newest text, one Ask Genie, a fade when ignored, and a Copy pop-up that stays.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { render, tauri, viewEmits } from '../test-tauri';
import { CardView } from './CardView';
import { FADE_AFTER_MS, FADE_OUT_MS } from './Popup';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-000000000001';

/** The card a run of three PTT asks gives today: three blurbs, three Ask Genie chips. */
const asks = {
  mogId: ID,
  blurbs: [
    { id: 'ask:u3', text: '"What was revenue last week"', completed: true },
    { id: 'ask:u2', text: '"Show churn by region"', completed: true },
    { id: 'ask:u1', text: '"Top ten customers"', completed: true },
  ],
  chips: [
    { id: 'ask_genie:u1', label: 'Ask Genie' },
    { id: 'ask_genie:u2', label: 'Ask Genie' },
    { id: 'ask_genie:u3', label: 'Ask Genie' },
  ],
  historyCount: 3,
};

/** The #498 Copy card of a refused paste. */
const refused = {
  mogId: ID,
  blurbs: [
    {
      id: 'u9',
      text: 'Meet at noon\nCould not insert: no text field has focus',
      completed: true,
    },
  ],
  chips: [{ id: 'copy-u9', label: 'Copy' }],
  historyCount: 0,
};

let roots: Root[] = [];
const mountCard = async () => {
  const r = await render(<CardView mogId={ID} />);
  roots.push(r.root);
  return r;
};
const popup = (el: HTMLElement) => el.querySelector('[data-popup]') as HTMLElement | null;
/** The pop-up that the test expects on screen. */
const pop = (el: HTMLElement) => el.querySelector('[data-popup]') as HTMLElement;
const advance = (ms: number) => act(async () => void vi.advanceTimersByTime(ms));
const hover = (el: HTMLElement, on: boolean) =>
  act(async () => {
    el.dispatchEvent(
      new MouseEvent(on ? 'mouseover' : 'mouseout', {
        bubbles: true,
        relatedTarget: document.body,
      }),
    );
  });

describe('#559 post-dictation pop-up', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    tauri.reset();
  });
  afterEach(async () => {
    await act(async () => roots.forEach((r) => r.unmount()));
    roots = [];
    vi.useRealTimers();
  });

  it('[#559] [UX-11a] shows no list, no Pin, no History link and no Expand', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', asks);
    expect(popup(el)).not.toBeNull();
    expect(el.querySelector('ul, li')).toBeNull();
    expect(el.querySelector('[data-action="pin"]')).toBeNull();
    expect(el.querySelector('[data-action="history"]')).toBeNull();
    expect(el.querySelector('[data-action="expand"]')).toBeNull();
    expect(el.textContent).not.toContain('more in History');
  });

  it('[#559] shows one line: the newest text and exactly one Ask Genie for it', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', asks);
    expect(el.querySelector('[data-popup-text]')?.textContent).toBe('"What was revenue last week"');
    expect(el.textContent).not.toContain('churn');
    expect(el.textContent).not.toContain('Top ten');
    const buttons = [...el.querySelectorAll('button')];
    expect(buttons).toHaveLength(1);
    expect(buttons[0]?.textContent).toBe('Ask Genie');
    expect(buttons[0]?.getAttribute('data-chip')).toBe('ask_genie:u3');
  });

  it('[#559] [UX-20c] Ask Genie sends the chip click of the newest text, and nothing else', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', asks);
    await hover(pop(el), true);
    await act(async () => (el.querySelector('[data-chip="ask_genie:u3"]') as HTMLElement).click());
    expect(viewEmits()).toEqual([
      { mogId: ID, kind: 'hover', payload: { blurbId: 'ask:u3', on: true, via: 'pointer' } },
      { mogId: ID, kind: 'chip_click', payload: { chipId: 'ask_genie:u3' } },
    ]);
  });

  it('[#559] keyboard focus on Ask Genie keeps the pop-up open', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', asks);
    await act(async () => (el.querySelector('[data-chip="ask_genie:u3"]') as HTMLElement).focus());
    await advance(FADE_AFTER_MS * 3);
    expect(pop(el).getAttribute('data-state')).toBe('shown');
  });

  it('[#559] an ignored pop-up fades out, then asks the brain to hide the card', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', asks);
    await advance(FADE_AFTER_MS - 1);
    expect(pop(el).getAttribute('data-state')).toBe('shown');
    await advance(1);
    expect(pop(el).getAttribute('data-state')).toBe('fading');
    await advance(FADE_OUT_MS);
    expect(popup(el)).toBeNull();
    expect(viewEmits()).toEqual([{ mogId: ID, kind: 'cancel' }]);
  });

  it('[#559] hover keeps it open; it fades again only after the pointer leaves', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', asks);
    await advance(FADE_AFTER_MS - 1000);
    await hover(pop(el), true);
    await advance(FADE_AFTER_MS * 4);
    expect(pop(el).getAttribute('data-state')).toBe('shown');
    await hover(pop(el), false);
    await advance(FADE_AFTER_MS - 1);
    expect(pop(el).getAttribute('data-state')).toBe('shown');
    await advance(1);
    await advance(FADE_OUT_MS);
    expect(popup(el)).toBeNull();
  });

  it('[#559] a hover during the fade brings it back', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', asks);
    await advance(FADE_AFTER_MS);
    expect(pop(el).getAttribute('data-state')).toBe('fading');
    await hover(pop(el), true);
    await advance(FADE_OUT_MS * 10);
    expect(pop(el).getAttribute('data-state')).toBe('shown');
    expect(viewEmits()).not.toContainEqual({ mogId: ID, kind: 'cancel' });
  });

  it('[#559] a newer dictation shows the pop-up again after a fade', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', asks);
    await advance(FADE_AFTER_MS);
    await advance(FADE_OUT_MS);
    expect(popup(el)).toBeNull();
    await tauri.emit('card:update', {
      ...asks,
      blurbs: [{ id: 'ask:u4', text: '"Margin by product"', completed: true }, ...asks.blurbs],
      chips: [...asks.chips, { id: 'ask_genie:u4', label: 'Ask Genie' }],
    });
    expect(pop(el).getAttribute('data-state')).toBe('shown');
  });

  it('[#559] [MOG-06d] a refused paste shows the text and Copy, open, and never fades', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', refused);
    const p = pop(el);
    expect(p.getAttribute('data-variant')).toBe('copy');
    expect(el.querySelector('[data-popup-text]')?.textContent).toBe('Meet at noon');
    // One line of text: the MOG-06d reason is the accessible description of Copy, and its tooltip.
    const copy = el.querySelector('[data-chip="copy-u9"]') as HTMLElement;
    const why = document.getElementById(copy.getAttribute('aria-describedby') ?? '');
    expect(why?.textContent).toBe('Could not insert: no text field has focus');
    expect(p.getAttribute('title')).toBe('Meet at noon\nCould not insert: no text field has focus');
    expect(el.textContent).not.toContain('Ask Genie');
    await advance(FADE_AFTER_MS * 20);
    expect(pop(el).getAttribute('data-state')).toBe('shown');
    expect(viewEmits()).toEqual([]);
  });

  it('[#559] [MOG-05b] Copy sends the copy chip click, then the pop-up closes', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', refused);
    await act(async () => (el.querySelector('[data-chip="copy-u9"]') as HTMLElement).click());
    expect(popup(el)).toBeNull();
    expect(viewEmits()).toEqual([
      { mogId: ID, kind: 'chip_click', payload: { chipId: 'copy-u9' } },
      { mogId: ID, kind: 'cancel' },
    ]);
  });

  it('[#559] Dismiss closes the Copy pop-up and asks the brain to hide the card', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', refused);
    const dismiss = el.querySelector('button[data-action="dismiss"]') as HTMLElement;
    expect(dismiss.getAttribute('aria-label')).toBe('Dismiss');
    await act(async () => dismiss.click());
    expect(popup(el)).toBeNull();
    expect(viewEmits()).toEqual([{ mogId: ID, kind: 'cancel' }]);
  });

  it('[#559] [B1] a held preview keeps its controls and does not fade', async () => {
    const { el } = await mountCard();
    await tauri.emit('card:update', {
      mogId: ID,
      blurbs: [{ id: 'preview:u5', text: '"Revenue"\nSending to Genie in 1.5 s' }],
      chips: [
        { id: 'preview_send_now', label: 'Send now' },
        { id: 'preview_cancel', label: 'Cancel' },
      ],
    });
    expect(el.querySelector('[data-chip="preview_send_now"]')).not.toBeNull();
    expect(el.querySelector('[data-chip="preview_cancel"]')).not.toBeNull();
    await advance(FADE_AFTER_MS * 3);
    expect(pop(el).getAttribute('data-state')).toBe('shown');
  });
});

describe('#559 design tokens', () => {
  const read = (rel: string) => readFileSync(join(__dirname, rel), 'utf8');

  it('[#559] the card and the MOG views import one shared tokens file', () => {
    expect(read('main.tsx')).toMatch(/import '\.\.\/tokens\.css'/);
    expect(read('../mog/main.tsx')).toMatch(/import '\.\.\/tokens\.css'/);
    expect(read('../tokens.css')).toMatch(/--mog-surface:/);
  });

  it('[#559] the pop-up has focus states, a system font and respects reduced motion', () => {
    const css = read('card.css');
    expect(css).toMatch(/:focus-visible/);
    expect(css).toMatch(/prefers-reduced-motion:\s*reduce/);
    expect(read('../tokens.css')).toMatch(/-apple-system/);
    expect(css).toMatch(/appearance:\s*none/);
  });
});
