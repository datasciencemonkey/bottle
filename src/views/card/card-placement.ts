/**
 * How the card window keeps its place beside its MOG when it fits its content (lamp-alive).
 *
 * The Rust host places the window at show time (`card_beside_sized` in `src-tauri/src/windows.rs`):
 * the visible edge of the pill is `CARD_GAP` from the lamp art, on the right or the left of the
 * MOG, and it tells the view the side in the window title (`card_title`). The view then changes
 * the width and the height and keeps the edge that faces the MOG where it is, so a pill on the
 * left of the MOG grows to the left and one on the right grows to the right. The view never
 * reads the MOG window.
 *
 * `CARD_FACING_PAD` is the clear window between the window edge and the pill on the side that
 * faces the MOG. It equals `CARD_GAP`, so the window ends at the lamp art and never covers it.
 * `CARD_FAR_PAD` is the clear window on the other side (the shadow). All units are logical px.
 */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type CardSide = 'left' | 'right';

/** Mirrors `CARD_GAP`, `CARD_EDGE_PAD` and `CARD_FAR_PAD` of `windows.rs`, and `card.css`. */
export const CARD_GAP = 10;
export const CARD_FACING_PAD = 10;
export const CARD_FAR_PAD = 12;

/** The side in a card window title (`card_title` in `windows.rs`): `right` when it says nothing. */
export function sideFromTitle(title: string | undefined): CardSide {
  return title !== undefined && /\(left\)\s*$/.test(title) ? 'left' : 'right';
}

/**
 * The window rectangle of size `w` by `h` for the card now at `card`, on `side` of its MOG: the
 * edge that faces the MOG stays, and the rectangle is kept inside `screen` (if known).
 */
export function anchoredBounds(
  card: Rect,
  side: CardSide,
  screen: Rect | undefined,
  w: number,
  h: number,
): Rect {
  let x = side === 'right' ? card.x : card.x + card.w - w;
  let y = card.y;
  if (screen) {
    x = Math.min(Math.max(x, screen.x), Math.max(screen.x, screen.x + screen.w - w));
    y = Math.min(Math.max(y, screen.y), Math.max(screen.y, screen.y + screen.h - h));
  }
  return { x, y, w, h };
}
