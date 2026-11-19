import {
  CARD_EDGE_PAD,
  CARD_GAP,
  MOG_HEAD_Y,
  placeCard,
  sideOf,
  visibleGap,
  type Rect,
} from './card-placement';

const SCREEN: Rect = { x: 0, y: 0, w: 1440, h: 900 };
const mog = (x: number, y = 300): Rect => ({ x, y, w: 160, h: 192 });

describe('card placement beside the MOG (lamp-alive)', () => {
  it('puts the card on the right, 10 px from the lamp art, with its tail at head height', () => {
    const m = mog(600);
    const { rect, side } = placeCard(m, SCREEN, 340, 64);
    expect(side).toBe('right');
    expect(visibleGap(rect, m, side)).toBe(CARD_GAP);
    expect(rect.y).toBe(m.y + MOG_HEAD_Y - 30);
  });

  it('puts a narrow pill on the left as close as a wide card (no clear window between)', () => {
    const m = mog(1250);
    for (const w of [224, 340]) {
      const { rect, side } = placeCard(m, SCREEN, w, 64);
      expect(side, `${w}`).toBe('left');
      expect(visibleGap(rect, m, side), `${w}`).toBe(CARD_GAP);
      // The window itself ends 12 px (the clear pad) past the visible edge, inside the MOG window.
      expect(rect.x + rect.w - (m.x + 18)).toBe(CARD_EDGE_PAD - CARD_GAP);
    }
  });

  it('flips to the left at the right screen edge and back to the right at the left edge', () => {
    expect(placeCard(mog(1100), SCREEN, 340, 64).side).toBe('left');
    expect(placeCard(mog(40), SCREEN, 340, 64, 'left').side).toBe('right');
    expect(placeCard(mog(900), SCREEN, 340, 64).side).toBe('right');
  });

  it('keeps the current side while it still fits, so a growing card does not jump the MOG', () => {
    expect(placeCard(mog(600), SCREEN, 340, 64, 'left').side).toBe('left');
    expect(placeCard(mog(600), SCREEN, 340, 64, 'right').side).toBe('right');
  });

  it('clamps inside the screen when neither side fits', () => {
    const small: Rect = { x: 0, y: 0, w: 400, h: 300 };
    const { rect } = placeCard(mog(120, 40), small, 340, 200);
    expect(rect.x).toBeGreaterThanOrEqual(0);
    expect(rect.x + rect.w).toBeLessThanOrEqual(400);
    expect(rect.y + rect.h).toBeLessThanOrEqual(300);
  });

  it('handles a second monitor with a negative origin', () => {
    const left: Rect = { x: -1440, y: 0, w: 1440, h: 900 };
    const m = mog(-250);
    const { rect, side } = placeCard(m, left, 340, 64);
    expect(side).toBe('left');
    expect(rect.x).toBeGreaterThanOrEqual(left.x);
    expect(rect.x + rect.w).toBeLessThanOrEqual(left.x + left.w);
    expect(visibleGap(rect, m, side)).toBe(CARD_GAP);
  });

  it('moves a tall card up only as far as it must', () => {
    const { rect } = placeCard(mog(600, 760), SCREEN, 340, 300);
    expect(rect.y + rect.h).toBe(900);
  });

  it('places with no screen as the right side beside the MOG', () => {
    const { rect, side } = placeCard(mog(600), undefined, 340, 64);
    expect(side).toBe('right');
    expect(rect.x).toBe(600 + 160 - 13 + 10 - 12);
  });

  it('finds the side of a window by its centre', () => {
    const m = mog(600);
    expect(sideOf({ x: 745, y: 0, w: 340, h: 64 }, m)).toBe('right');
    expect(sideOf({ x: 600 + 18 - 10 + 12 - 340, y: 0, w: 340, h: 64 }, m)).toBe('left');
  });
});
