// @vitest-environment jsdom
import {
  CARD_WINDOW_MAX_HEIGHT,
  CARD_WINDOW_MIN_HEIGHT,
  CARD_WINDOW_MIN_WIDTH,
  CARD_WINDOW_WIDTH,
  cardWindowHeight,
  cardWindowWidth,
  createWindowFit,
  type CardPlace,
  type CardWindow,
} from './fit-window';
import type { Rect } from './card-placement';

const SCREEN: Rect = { x: 0, y: 0, w: 1440, h: 900 };
const MOG: Rect = { x: 600, y: 300, w: 160, h: 192 };

function fakeWindow(place: CardPlace | undefined, fail = false) {
  const log: Rect[] = [];
  const w: CardWindow = {
    setBounds: (r) => {
      log.push(r);
      return fail ? Promise.reject(new Error('fake')) : Promise.resolve();
    },
    place: () => Promise.resolve(place),
  };
  return { w, log };
}

describe('card window fit (#573, lamp-alive)', () => {
  it('keeps the height between one pop-up line and the cap', () => {
    expect(cardWindowHeight(20)).toBe(CARD_WINDOW_MIN_HEIGHT);
    expect(cardWindowHeight(180.2)).toBe(181);
    expect(cardWindowHeight(5000)).toBe(CARD_WINDOW_MAX_HEIGHT);
    expect(cardWindowHeight(Number.NaN)).toBe(CARD_WINDOW_MIN_HEIGHT);
  });

  it('sizes the window to a compact pill, and keeps the full width for the answer card', () => {
    expect(cardWindowWidth(undefined)).toBe(CARD_WINDOW_WIDTH);
    expect(cardWindowWidth(Number.NaN)).toBe(CARD_WINDOW_WIDTH);
    // The pill width plus the clear window round it (12 px on each side).
    expect(cardWindowWidth(210.2)).toBe(211 + 24);
    expect(cardWindowWidth(10)).toBe(CARD_WINDOW_MIN_WIDTH);
    expect(cardWindowWidth(900)).toBe(CARD_WINDOW_WIDTH);
  });

  it('sets each new size once', async () => {
    const f = fakeWindow({
      card: { x: 745, y: 342, w: 340, h: 64 },
      mog: MOG,
      screen: SCREEN,
    });
    let measured: number | undefined = 200;
    const fit = createWindowFit(() => f.w, { measureWidth: () => measured });
    await fit(64);
    await fit(63.6);
    measured = undefined;
    await fit(200);
    expect(f.log.map((r) => [r.w, r.h])).toEqual([
      [224, 64],
      [340, 200],
    ]);
  });

  it('fits again when the window lost its height (R11 shows a hidden card at one line)', async () => {
    const f = fakeWindow({ card: { x: 745, y: 342, w: 340, h: 64 }, mog: MOG, screen: SCREEN });
    const fit = createWindowFit(() => f.w, { measureWidth: () => undefined });
    await fit(200);
    await fit(200, 200);
    await fit(200, 64);
    expect(f.log).toHaveLength(2);
  });

  it('keeps the edge facing the MOG fixed: a pill on the right grows right, one on the left grows left', async () => {
    const right = fakeWindow({ card: { x: 745, y: 342, w: 340, h: 64 }, mog: MOG, screen: SCREEN });
    await createWindowFit(() => right.w, { measureWidth: () => 200 })(64);
    expect(right.log[0]?.x).toBe(745); // the left edge of the window, beside the MOG, stays

    // The same MOG near the right screen edge: the card is on its left, and its right edge stays.
    const edge: Rect = { x: 1250, y: 300, w: 160, h: 192 };
    const startRight = edge.x + 18 - 10 + 12; // the right edge of a card on the left
    const left = fakeWindow({
      card: { x: startRight - 340, y: 342, w: 340, h: 64 },
      mog: edge,
      screen: SCREEN,
    });
    const sides: string[] = [];
    await createWindowFit(() => left.w, {
      measureWidth: () => 200,
      onSide: (s) => sides.push(s),
    })(64);
    const r = left.log[0] as Rect;
    expect(r.x + r.w).toBe(startRight);
    expect(r.w).toBe(224);
    expect(sides).toEqual(['left']);
  });

  it('leaves no clear window between the MOG and the card: the gap is 10 px at every width', async () => {
    const edge: Rect = { x: 1250, y: 300, w: 160, h: 192 };
    for (const measured of [150, 200, 328, undefined]) {
      const f = fakeWindow({ card: { x: 900, y: 342, w: 340, h: 64 }, mog: edge, screen: SCREEN });
      await createWindowFit(() => f.w, { measureWidth: () => measured })(64);
      const r = f.log[0] as Rect;
      expect(edge.x + 18 - (r.x + r.w - 12), `${measured}`).toBe(10);
    }
  });

  it('moves a card up that would go past the bottom of the screen', async () => {
    const f = fakeWindow({
      card: { x: 745, y: 800, w: 340, h: 64 },
      mog: { x: 600, y: 760, w: 160, h: 192 },
      screen: SCREEN,
    });
    await createWindowFit(() => f.w, { measureWidth: () => undefined })(300);
    const r = f.log[0] as Rect;
    expect(r.y + r.h).toBe(900);
  });

  it('keeps the left edge and stays on screen when the MOG window cannot be read', async () => {
    const f = fakeWindow({ card: { x: 10, y: 800, w: 340, h: 64 }, screen: SCREEN });
    await createWindowFit(() => f.w, { measureWidth: () => undefined })(300);
    expect(f.log).toEqual([{ x: 10, y: 600, w: 340, h: 300 }]);
  });

  it('tries again after a failed resize', async () => {
    const f = fakeWindow(
      { card: { x: 745, y: 342, w: 340, h: 64 }, mog: MOG, screen: SCREEN },
      true,
    );
    const fit = createWindowFit(() => f.w, { measureWidth: () => undefined });
    await fit(200);
    await fit(200);
    expect(f.log).toHaveLength(2);
  });
});

describe('lamp-alive: the side mark', () => {
  it('marks the page with the side, for the tail in card.css', async () => {
    const { applyCardSide } = await import('./fit-window');
    applyCardSide('left');
    expect(document.documentElement.dataset['side']).toBe('left');
    applyCardSide('right');
    expect(document.documentElement.dataset['side']).toBe('right');
    delete document.documentElement.dataset['side'];
  });
});
