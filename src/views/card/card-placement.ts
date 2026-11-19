/**
 * Where the card window goes beside its MOG (lamp-alive). The Rust host places the window at
 * show time (`card_beside_sized` in `src-tauri/src/windows.rs`); this is the same math, for the
 * view that refits the window to its content: the edge that faces the MOG never moves, so a pill
 * on the left of the MOG grows to the left and one on the right grows to the right.
 *
 * The visible edge of the pop-up is `CARD_GAP` from the lamp art. `CARD_EDGE_PAD` is the clear
 * window round the pop-up (shadow and tail, `card.css`). All units are logical px.
 */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type CardSide = 'left' | 'right';

/** Mirrors `MOG_ART_LEFT_INSET` and `MOG_ART_RIGHT_INSET` of `windows.rs`. */
export const MOG_ART_LEFT_INSET = 18;
export const MOG_ART_RIGHT_INSET = 13;
/** Mirrors `MOG_HEAD_Y`, `CARD_GAP`, `CARD_EDGE_PAD` and `CARD_TAIL_Y` of `windows.rs`. */
export const MOG_HEAD_Y = 72;
export const CARD_GAP = 10;
export const CARD_EDGE_PAD = 12;
export const CARD_TAIL_Y = 30;

/** The side of the MOG the card window is on: by where the centre of each window is. */
export function sideOf(card: Rect, mog: Rect): CardSide {
  return card.x + card.w / 2 < mog.x + mog.w / 2 ? 'left' : 'right';
}

/**
 * The card window rectangle of size `w` by `h` beside `mog`, on `screen`. Right of the MOG if it
 * fits, else left; clamped inside `screen` in both directions. `prefer` keeps the current side as
 * long as it still fits, so a card does not jump across the MOG when it grows.
 */
export function placeCard(
  mog: Rect,
  screen: Rect | undefined,
  w: number,
  h: number,
  prefer: CardSide = 'right',
): { rect: Rect; side: CardSide } {
  const rightX = mog.x + mog.w - MOG_ART_RIGHT_INSET + CARD_GAP - CARD_EDGE_PAD;
  const leftX = mog.x + MOG_ART_LEFT_INSET - CARD_GAP + CARD_EDGE_PAD - w;
  let x = prefer === 'left' ? leftX : rightX;
  let side: CardSide = prefer;
  let y = mog.y + MOG_HEAD_Y - CARD_TAIL_Y;
  if (screen) {
    const fitsRight = rightX >= screen.x && rightX + w <= screen.x + screen.w;
    const fitsLeft = leftX >= screen.x && leftX + w <= screen.x + screen.w;
    if (prefer === 'right' && !fitsRight && fitsLeft) [x, side] = [leftX, 'left'];
    else if (prefer === 'left' && !fitsLeft && fitsRight) [x, side] = [rightX, 'right'];
    else if (!fitsLeft && !fitsRight) {
      // Neither side fits: take the side with more room, then clamp.
      const roomRight = screen.x + screen.w - (mog.x + mog.w);
      const roomLeft = mog.x - screen.x;
      side = roomLeft > roomRight ? 'left' : 'right';
      x = side === 'left' ? leftX : rightX;
    }
    x = Math.min(Math.max(x, screen.x), Math.max(screen.x, screen.x + screen.w - w));
    y = Math.min(Math.max(y, screen.y), Math.max(screen.y, screen.y + screen.h - h));
  }
  return { rect: { x, y, w, h }, side };
}

/** The gap between the lamp art and the visible edge of a card at `rect` (logical px). */
export function visibleGap(rect: Rect, mog: Rect, side: CardSide): number {
  return side === 'right'
    ? rect.x + CARD_EDGE_PAD - (mog.x + mog.w - MOG_ART_RIGHT_INSET)
    : mog.x + MOG_ART_LEFT_INSET - (rect.x + rect.w - CARD_EDGE_PAD);
}
