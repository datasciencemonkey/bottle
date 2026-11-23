/**
 * #573: the card window takes the height of its content (the pop-up pill, the status line or the
 * answer card), so the answer is never cut and no clear part of the window lies over the user's
 * app. The width stays the R11 width (`CARD_WIDTH` in `src-tauri/src/windows.rs`).
 *
 * R11 puts the top of the card beside the MOG; a taller card grows down. If it then goes past the
 * bottom of the screen, it moves up to stay on the screen. The edge that faces the MOG never moves
 * (`card-placement.ts`). The card capability grants only the window size, position and monitor
 * reads and writes that this needs, and the read of the MOG window.
 */
import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi';
import { currentMonitor, getCurrentWindow } from '@tauri-apps/api/window';
import {
  CARD_FACING_PAD,
  CARD_FAR_PAD,
  anchoredBounds,
  sideFromTitle,
  type CardSide,
  type Rect,
} from './card-placement';

/** `CARD_WIDTH` of R11 (logical px). */
export const CARD_WINDOW_WIDTH = 340;
/** `CARD_HEIGHT` of R11: the height of one pop-up line. The window is never lower. */
export const CARD_WINDOW_MIN_HEIGHT = 64;
/** The card window is never higher (the answer card scrolls inside it). */
export const CARD_WINDOW_MAX_HEIGHT = 340;

/** The window height for content of `contentPx` (logical px). */
export function cardWindowHeight(contentPx: number): number {
  const h = Math.ceil(Number.isFinite(contentPx) ? contentPx : 0);
  return Math.min(CARD_WINDOW_MAX_HEIGHT, Math.max(CARD_WINDOW_MIN_HEIGHT, h));
}

/** Everything the fit needs to know about the windows, in logical px. */
export interface CardPlace {
  /** This card window. */
  card: Rect;
  /** The side of its MOG the host put the card on. */
  side: CardSide;
  /** The monitor the card is on. */
  screen?: Rect | undefined;
}

/** The window seams. Tests give a fake; the app uses the Tauri window of this view. */
export interface CardWindow {
  /**
   * Move and size the window. Tauri has no single call for both, so the order keeps the transient
   * frame away from the MOG: a window that grows toward the left moves first, then grows; one that
   * shrinks shrinks first, then moves. The window never covers the lamp in between.
   */
  setBounds(r: Rect, from: Rect): Promise<void>;
  place(): Promise<CardPlace | undefined>;
}

export const tauriCardWindow = (): CardWindow => {
  const w = getCurrentWindow();
  const move = (r: Rect) => w.setPosition(new LogicalPosition(r.x, r.y));
  const size = (r: Rect) => w.setSize(new LogicalSize(r.w, r.h));
  return {
    async setBounds(r, from) {
      if (r.w > from.w) {
        await move(r);
        await size(r);
      } else {
        await size(r);
        await move(r);
      }
    },
    async place() {
      const [scale, pos, sz, title, mon] = await Promise.all([
        w.scaleFactor(),
        w.outerPosition(),
        w.outerSize(),
        w.title(),
        currentMonitor(),
      ]);
      const p = pos.toLogical(scale);
      const z = sz.toLogical(scale);
      const out: CardPlace = {
        card: { x: p.x, y: p.y, w: z.width, h: z.height },
        side: sideFromTitle(title),
      };
      if (mon) {
        const m = mon.position.toLogical(mon.scaleFactor);
        const d = mon.size.toLogical(mon.scaleFactor);
        out.screen = { x: m.x, y: m.y, w: d.width, h: d.height };
      }
      return out;
    },
  };
};

/** The width of the card window for the content on screen: a compact pill, or the full card. */
export function cardWindowWidth(measured: number | undefined): number {
  if (measured === undefined || !Number.isFinite(measured) || measured <= 0)
    return CARD_WINDOW_WIDTH;
  // Round up and leave 1 px: a fractional text width must never cut the last letter.
  const w = Math.ceil(measured) + 1 + CARD_FACING_PAD + CARD_FAR_PAD;
  return Math.min(CARD_WINDOW_WIDTH, w);
}

/** Mark the page with the side of the MOG the card is on (the tail mirrors in `card.css`). */
export function applyCardSide(side: CardSide, doc: Document = document): void {
  doc.documentElement.dataset['side'] = side;
}

/**
 * The natural width (px) of the compact pill on screen (the pop-up or the Genie status pill), or
 * `undefined` for the answer card, which keeps the full width. The pill is measured without the
 * `max-width` of the window, so a longer text, or the timer that comes in after 3 s, widens the
 * window again (the fit runs again when the card changes, `CardView.tsx`).
 */
export function measureCompactWidth(doc: Document = document): number | undefined {
  const pills = Array.from(doc.querySelectorAll<HTMLElement>('.popup, .genie-working'));
  if (pills.length === 0) return undefined;
  // UX slice 1: a pill and a pop-up can show together. The window takes the wider one.
  let widest = 0;
  for (const pill of pills) {
    const prev = pill.style.maxWidth;
    pill.style.maxWidth = 'none';
    // The layout width with its fraction: not `offsetWidth`, which rounds down and cut the last
    // letter, and not the box under a transform, because the pill fades and scales in. The used
    // `width` of a border-box is the whole pill. `getBoundingClientRect` is the fallback.
    const used = parseFloat(getComputedStyle(pill).width);
    const w = Number.isFinite(used) && used > 0 ? used : pill.getBoundingClientRect().width;
    pill.style.maxWidth = prev;
    widest = Math.max(widest, w);
  }
  return widest;
}

/**
 * Whether these DOM changes can change the width of the pill. A timer tick that keeps the number
 * of digits cannot, so it skips the layout read of the fit (one per second). A new node (the
 * timer appears) or a new text length (the stage line, a timer digit) can.
 */
export function widthMayChangeIn(
  records: readonly Pick<MutationRecord, 'type' | 'target' | 'oldValue'>[],
): boolean {
  return records.some((r) => {
    if (r.type !== 'characterData') return true;
    const timer = r.target.parentElement?.closest('[data-genie-timer]') != null;
    return !timer || (r.oldValue?.length ?? -1) !== (r.target.textContent?.length ?? -2);
  });
}

/**
 * Make a fitter: each call sets the window to the height of `contentPx` and the width of the
 * content, at most once per size, and keeps the edge that faces the MOG fixed. `windowPx` is the
 * window height now, if known: R11 sets a hidden card back to one line when it shows it again, so
 * a different height fits again. `onSide` gets the side of the MOG (the view mirrors its tail
 * with it). A failed step leaves the window as it is.
 */
export function createWindowFit(
  win: () => CardWindow = tauriCardWindow,
  opts: { measureWidth?: () => number | undefined; onSide?: (side: CardSide) => void } = {},
) {
  const measure = opts.measureWidth ?? (() => measureCompactWidth());
  let last: string | undefined;
  return async (contentPx: number, windowPx?: number): Promise<void> => {
    const h = cardWindowHeight(contentPx);
    const width = cardWindowWidth(measure());
    const key = `${width}x${h}`;
    if (key === last && (windowPx === undefined || Math.round(windowPx) === h)) return;
    last = key;
    try {
      const w = win();
      const at = await w.place();
      if (!at) return;
      opts.onSide?.(at.side);
      await w.setBounds(anchoredBounds(at.card, at.side, at.screen, width, h), at.card);
    } catch {
      // The window keeps its size; the card scrolls in it.
      last = undefined;
    }
  };
}
