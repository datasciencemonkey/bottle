import {
  CARD_FACING_PAD,
  CARD_FAR_PAD,
  CARD_GAP,
  anchoredBounds,
  sideFromTitle,
  type Rect,
} from './card-placement';

const SCREEN: Rect = { x: 0, y: 0, w: 1440, h: 900 };

describe('the card keeps the edge that faces its MOG (lamp-alive)', () => {
  it('reads the side from the window title the host sets', () => {
    expect(sideFromTitle('MOG card (left)')).toBe('left');
    expect(sideFromTitle('MOG card (right)')).toBe('right');
    expect(sideFromTitle('MOG card')).toBe('right');
    expect(sideFromTitle(undefined)).toBe('right');
  });

  it('on the right the left edge stays and the card grows to the right', () => {
    const card: Rect = { x: 745, y: 342, w: 340, h: 64 };
    expect(anchoredBounds(card, 'right', SCREEN, 234, 64)).toEqual({
      x: 745,
      y: 342,
      w: 234,
      h: 64,
    });
  });

  it('on the left the right edge stays and the card grows to the left', () => {
    const card: Rect = { x: 930, y: 342, w: 340, h: 64 };
    const r = anchoredBounds(card, 'left', SCREEN, 234, 64);
    expect(r.x + r.w).toBe(930 + 340);
    expect(r.w).toBe(234);
    const grown = anchoredBounds(r, 'left', SCREEN, 340, 226);
    expect(grown.x + grown.w).toBe(930 + 340);
  });

  it('the gap to the lamp art does not change at any width, because the facing edge is fixed', () => {
    // The host put the window edge at the art edge (pad = gap), on either side.
    expect(CARD_FACING_PAD).toBe(CARD_GAP);
    for (const w of [120, 234, 340]) {
      const left = anchoredBounds({ x: 930, y: 0, w: 340, h: 64 }, 'left', SCREEN, w, 64);
      expect(left.x + left.w).toBe(1270);
      const right = anchoredBounds({ x: 745, y: 0, w: 340, h: 64 }, 'right', SCREEN, w, 64);
      expect(right.x).toBe(745);
    }
    expect(CARD_FAR_PAD).toBeGreaterThanOrEqual(CARD_FACING_PAD);
  });

  it('moves a tall card up that would go past the bottom of the screen', () => {
    const r = anchoredBounds({ x: 745, y: 800, w: 340, h: 64 }, 'right', SCREEN, 340, 300);
    expect(r.y + r.h).toBe(900);
  });

  it('stays inside a monitor with a negative origin and does not leave its left edge', () => {
    const left: Rect = { x: -1440, y: 0, w: 1440, h: 900 };
    const r = anchoredBounds({ x: -1430, y: 100, w: 340, h: 64 }, 'left', left, 340, 64);
    expect(r.x).toBeGreaterThanOrEqual(-1440);
    expect(r.x + r.w).toBeLessThanOrEqual(0);
  });

  it('keeps the place with no screen read', () => {
    expect(anchoredBounds({ x: 10, y: 20, w: 340, h: 64 }, 'right', undefined, 200, 100)).toEqual({
      x: 10,
      y: 20,
      w: 200,
      h: 100,
    });
  });
});
