// @vitest-environment jsdom
/**
 * AT-20 "Without pointer or animation" (#230), section 15: keyboard only, a screen reader,
 * reduced motion and greyscale for every critical flow. Pass: all critical flows complete,
 * every state has text, Stop is visible and works by keyboard.
 *
 * What ships runs here against the thin views (#226, #78, #9): the 7 moods, the one-action card
 * Esc, chip labels and hover, overlay text, DOM-driven hover and click. The rest is pending with
 * `it.skip` and the feature issue in the title; a skipped test never counts as a pass. The
 * screen-reader, greyscale and reduced-motion runs on a device are a person's steps:
 * docs/tests/AT-MOGS-TALKKEY-A11Y.md. The window part of UX-19a is `src-tauri/tests/at_01_mogs.rs`.
 */
import { readFileSync } from 'node:fs';
import { act } from 'react';
import type { ExchangeRef, Mood } from '../shared/interfaces';
import { decideCardEsc, handleCardEsc } from '../brain/card-esc';
import { createMoodMapper, MOODS } from '../brain/components/mood-mapper';
import { accessibleText, contrastRatio } from '../test-support/a11y';
import { render, tauri, viewEmits } from '../views/test-tauri';
import { CardView } from '../views/card/CardView';
import { MogView } from '../views/mog/MogView';
import { offlineRiveParams } from '../views/mog/rive-loader';
import { Hub } from '../views/hub/Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../views/test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../views/test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../views/test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-0000000000a2';
const noOverlays = { attention: false, recording: false, offline: false, localOnly: false };
const ref: ExchangeRef = {
  backend: 'genie_one_mcp',
  profile: 'p',
  host: 'h',
  conversationId: 'c1',
};

const unmounts: (() => void)[] = [];
async function mount(node: Parameters<typeof render>[0]) {
  const r = await render(node);
  unmounts.push(() => r.root.unmount());
  return r;
}

function key(type: 'keydown', init: KeyboardEventInit) {
  return act(async () => {
    window.dispatchEvent(new KeyboardEvent(type, { bubbles: true, ...init }));
  });
}

describe('AT-20 without pointer or animation', () => {
  beforeEach(() => tauri.reset());
  afterEach(async () => {
    await act(async () => unmounts.splice(0).forEach((u) => u()));
  });

  // ---------- Runs now ----------

  it('[AT-20] [UX-02a] the app uses only the 7 moods of the Rive file', () => {
    const rive: readonly Mood[] = [
      'idle',
      'listening',
      'processing',
      'answering',
      'success',
      'error',
      'sleeping',
    ];
    expect([...MOODS].sort()).toEqual([...rive].sort());
    // Every input the mapper can get gives one of the 7 (a poke is not a mood).
    const map = createMoodMapper(() => 1_000);
    const seen = new Set<Mood>();
    for (let bits = 0; bits < 1 << 7; bits++) {
      const on = (i: number) => (bits & (1 << i)) !== 0;
      const out = map({
        paused: on(0),
        ownsCapture: on(1),
        answering: on(2),
        processing: on(3),
        dozing: on(4),
        ...(on(5) ? { errorUntil: 2_000 } : {}),
        ...(on(6) ? { successUntil: 2_000 } : {}),
      });
      expect(rive).toContain(out.mood);
      seen.add(out.mood);
    }
    expect(seen.size).toBeGreaterThan(1);
  });

  it('[AT-20] [UX-19b] one Esc in the card does exactly one thing, by keyboard only', async () => {
    await mount(<CardView mogId={ID} />);
    await key('keydown', { key: 'Escape' });
    await key('keydown', { key: 'Escape', repeat: true });
    expect(viewEmits()).toEqual([{ mogId: ID, kind: 'cancel' }]);

    // With a request in flight the Esc cancels it and the card stays; with none it hides.
    expect(decideCardEsc({ inFlight: ref })).toEqual({ t: 'cancel_request', ref });
    expect(decideCardEsc({})).toEqual({ t: 'hide_card' });
    const cancel = vi.fn(async () => {});
    const hideCard = vi.fn(async () => {});
    expect(await handleCardEsc(ID, { inFlight: ref }, { cancel, hideCard })).toEqual({
      t: 'cancel_request',
      ok: true,
    });
    expect([cancel.mock.calls.length, hideCard.mock.calls.length]).toEqual([1, 0]);

    // A failed cancel does not hide the card: no half state.
    const failing = vi.fn(async () => {
      throw new Error('offline');
    });
    const hide2 = vi.fn(async () => {});
    expect(await handleCardEsc(ID, { inFlight: ref }, { cancel: failing, hideCard: hide2 })).toEqual(
      { t: 'cancel_request', ok: false },
    );
    expect(hide2).not.toHaveBeenCalled();
  });

  it('[AT-20] [UX-20b] [UX-20c] every chip has a text label and hover never sends data', async () => {
    const { el } = await mount(<CardView mogId={ID} />);
    await tauri.emit('card:update', {
      mogId: ID,
      blurbs: [{ id: 'b1', text: 'Revenue was 4.2M' }],
      chips: [
        { id: 'c1', label: 'Ask Genie' },
        { id: 'c2', label: 'Open in Genie' },
      ],
    });
    const chips = [...el.querySelectorAll<HTMLButtonElement>('button[data-chip]')];
    expect(chips).toHaveLength(2);
    for (const c of chips) {
      expect(c.type).toBe('button');
      expect(accessibleText(c)).not.toBe('');
      await act(async () => {
        for (const t of ['mouseover', 'mouseenter', 'mousemove', 'focus'])
          c.dispatchEvent(new MouseEvent(t, { bubbles: true }));
      });
    }
    expect(viewEmits()).toEqual([]);
    await act(async () => chips[0]?.click());
    expect(viewEmits()).toEqual([{ mogId: ID, kind: 'chip_click', payload: { chipId: 'c1' } }]);
  });

  it('[AT-20] the REC, offline and attention overlays show text, not only colour', async () => {
    const { el } = await mount(<MogView mogId={ID} />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'listening',
      overlays: { attention: true, recording: true, offline: true, localOnly: false },
    });
    const overlays = [...el.querySelectorAll('[data-overlay]')];
    expect(overlays.map((o) => o.getAttribute('data-overlay')).sort()).toEqual([
      'attention',
      'offline',
      'recording',
    ]);
    const texts = overlays.map((o) => accessibleText(o));
    expect(texts.every((t) => t !== '')).toBe(true);
    expect(new Set(texts).size).toBe(3);
  });

  it('[AT-20] DOM pointer events drive hover and click of the MOG', async () => {
    const { el } = await mount(<MogView mogId={ID} />);
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await act(async () => {
      target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }));
      target.click();
    });
    expect(viewEmits()).toEqual([
      { mogId: ID, kind: 'hover' },
      { mogId: ID, kind: 'click' },
    ]);
  });

  // ---------- Pending: each title names the feature issue ----------

  it.skip('[AT-20] [UX-02b] the first frame is idle with demo=false, level=0, hover=false TODO(#12)', async () => {
    const { el } = await mount(<MogView mogId={ID} />);
    // TODO(#12): the Rive renderer replaces the placeholder and sets the inputs on load.
    const canvas = el.querySelector('canvas');
    expect(canvas).not.toBeNull();
    expect(canvas?.dataset['inputs']).toBe('demo=false,level=0,hover=false,mood=idle');
  });

  it.skip('[AT-20] [UX-08a] [UX-08b] the MOG view keeps the Rive listeners off TODO(#83)', () => {
    // TODO(#83): the MOG view builds its Rive instance with the listeners off.
    const params = offlineRiveParams({ src: '/mog.riv' });
    expect(params.shouldDisableRiveListeners).toBe(true);
  });

  it.skip('[AT-20] [UX-31a] [UX-31b] the canvas is aria-hidden and a hidden button reads "<name>, <state>" TODO(#96)', async () => {
    const { el } = await mount(<MogView mogId={ID} />);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'listening', overlays: noOverlays });
    expect(el.querySelector('canvas')?.getAttribute('aria-hidden')).toBe('true');
    const button = el.querySelector('button');
    expect(button).not.toBeNull();
    expect(accessibleText(button as Element)).toMatch(/^.+, .+$/);
  });

  it.skip('[AT-20] [UX-32a] [UX-32b] the card has a polite and an assertive live region TODO(#96)', async () => {
    const { el } = await mount(<CardView mogId={ID} />);
    expect(el.querySelectorAll('[aria-live="polite"]')).toHaveLength(1);
    expect(el.querySelectorAll('[aria-live="assertive"]')).toHaveLength(1);
  });

  it.skip('[AT-20] [UX-30] every action of Table UX-KEYS works by keyboard and Stop is visible TODO(#96)', async () => {
    const { el } = await mount(<Hub />);
    const stop = [...el.querySelectorAll('button')].find((b) => /stop/i.test(accessibleText(b)));
    expect(stop).toBeDefined();
    stop?.focus();
    expect(document.activeElement).toBe(stop);
  });

  it.skip('[AT-20] [UX-20a] a blurb shows 4 chips or less and the 5th goes to the expanded card TODO(#89)', async () => {
    const { el } = await mount(<CardView mogId={ID} />);
    const chips = [1, 2, 3, 4, 5].map((n) => ({ id: `c${n}`, label: `Chip ${n}` }));
    await tauri.emit('card:update', { mogId: ID, blurbs: [{ id: 'b1', text: 'x' }], chips });
    expect(el.querySelectorAll('button[data-chip]')).toHaveLength(4);
  });

  it.skip('[AT-20] [UX-23a] [UX-23b] meeting start and stop are told to the screen reader TODO(#101)', async () => {
    const { el } = await mount(<MogView mogId={ID} />);
    const rec = { ...noOverlays, recording: true };
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'listening', overlays: rec });
    expect(el.querySelector('[aria-live="assertive"]')?.textContent).toBe('Recording started');
  });

  it.skip('[AT-20] [UX-28] the last-interacted MOG shows a target marker in its name TODO(#73)', async () => {
    const { el } = await mount(<MogView mogId={ID} />);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'idle', overlays: noOverlays, target: true });
    expect(el.querySelector('[data-overlay="target"]')).not.toBeNull();
    expect(accessibleText(el.querySelector('button') as Element)).toMatch(/, target$/);
  });

  it.skip('[AT-20] [UX-33] each mood has a face glyph and pill text, distinct in greyscale TODO(#97)', async () => {
    const { el } = await mount(<MogView mogId={ID} />);
    const texts = new Set<string>();
    for (const mood of MOODS) {
      await tauri.emit('mog:view_state', { mogId: ID, mood, overlays: noOverlays });
      texts.add(accessibleText(el.querySelector('[data-pill]') as Element));
    }
    expect(texts.size).toBe(MOODS.length);
  });

  it.skip('[AT-20] [UX-34a] [UX-34b] [UX-34c] card text contrast is 4.5:1 or more in light and dark TODO(#97)', () => {
    // TODO(#97): the card style sheet with --card-fg / --card-bg for light and dark.
    const css = readFileSync('src/views/card/card.css', 'utf8');
    const pairs = [...css.matchAll(/--card-fg:\s*(#[0-9a-f]{6});\s*--card-bg:\s*(#[0-9a-f]{6})/gi)];
    expect(pairs.length).toBeGreaterThanOrEqual(2);
    for (const [, fg, bg] of pairs) expect(contrastRatio(fg ?? '', bg ?? '')).toBeGreaterThanOrEqual(4.5);
  });

  it.skip('[AT-20] [UX-35] with reduced motion the MOG uses static frames and a 5-step meter TODO(#97)', async () => {
    window.matchMedia = vi.fn(() => ({ matches: true }) as MediaQueryList);
    const { el } = await mount(<MogView mogId={ID} />);
    expect((el.querySelector('[data-mood]') as HTMLElement).dataset['motion']).toBe('reduced');
  });
});
