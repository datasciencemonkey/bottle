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
import { Window, currentMonitor, getCurrentWindow } from '@tauri-apps/api/window';
import { CARD_EDGE_PAD, placeCard, sideOf, type CardSide, type Rect } from './card-placement';

/** `CARD_WIDTH` of R11 (logical px). */
export const CARD_WINDOW_WIDTH = 340;
/** The compact pill is never narrower than this window (logical px). */
export const CARD_WINDOW_MIN_WIDTH = 200;
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
  /** The MOG window this card belongs to, if it is there. */
  mog?: Rect | undefined;
  /** The monitor the card is on. */
  screen?: Rect | undefined;
}

/** The window seams. Tests give a fake; the app uses the Tauri window of this view. */
export interface CardWindow {
  /** Move and size the window in one step. */
  setBounds(r: Rect): Promise<void>;
  place(): Promise<CardPlace | undefined>;
}

export const tauriCardWindow = (mogId?: string): CardWindow => {
  const w = getCurrentWindow();
  return {
    setBounds: async (r) => {
      // Move first when it grows toward the MOG side, so no frame jumps; the order is the same
      // for every call, one step each (Tauri applies them in order on the window thread).
      await w.setPosition(new LogicalPosition(r.x, r.y));
      await w.setSize(new LogicalSize(r.w, r.h));
    },
    async place() {
      const [scale, pos, size, mon] = await Promise.all([
        w.scaleFactor(),
        w.outerPosition(),
        w.outerSize(),
        currentMonitor(),
      ]);
      const p = pos.toLogical(scale);
      const sz = size.toLogical(scale);
      const out: CardPlace = { card: { x: p.x, y: p.y, w: sz.width, h: sz.height } };
      if (mon) {
        const m = mon.position.toLogical(mon.scaleFactor);
        const z = mon.size.toLogical(mon.scaleFactor);
        out.screen = { x: m.x, y: m.y, w: z.width, h: z.height };
      }
      if (mogId !== undefined) {
        try {
          const mogWin = await Window.getByLabel(`mog-${mogId}`);
          if (mogWin) {
            const [ms, mp, mz] = await Promise.all([
              mogWin.scaleFactor(),
              mogWin.outerPosition(),
              mogWin.outerSize(),
            ]);
            const a = mp.toLogical(ms);
            const b = mz.toLogical(ms);
            out.mog = { x: a.x, y: a.y, w: b.width, h: b.height };
          }
        } catch {
          // No MOG geometry: the card keeps its left edge, as before.
        }
      }
      return out;
    },
  };
};

/** Mark the page with the side of the MOG the card is on (the tail mirrors in `card.css`). */
export function applyCardSide(side: CardSide, doc: Document = document): void {
  doc.documentElement.dataset['side'] = side;
}

/** Room for the `0:13` timer of the status pill, which comes in after 3 s (logical px). */
export const TIMER_RESERVE_PX = 44;

/** The width of the card window for the content on screen: a compact pill, or the full card. */
export function cardWindowWidth(measured: number | undefined): number {
  if (measured === undefined || !Number.isFinite(measured) || measured <= 0)
    return CARD_WINDOW_WIDTH;
  const w = Math.ceil(measured) + 2 * CARD_EDGE_PAD;
  return Math.min(CARD_WINDOW_WIDTH, Math.max(CARD_WINDOW_MIN_WIDTH, w));
}

/**
 * The natural width (px) of the compact pill on screen (the pop-up or the Genie status pill), or
 * `undefined` for the answer card, which keeps the full width. The pill is measured without the
 * `max-width` of the window, so a longer text widens the window again.
 */
export function measureCompactWidth(doc: Document = document): number | undefined {
  const pill = doc.querySelector<HTMLElement>('.popup, .genie-working');
  if (!pill) return undefined;
  const prev = pill.style.maxWidth;
  pill.style.maxWidth = 'none';
  // Layout width (`offsetWidth`), not the box under a transform: the pill fades and scales in.
  const w = pill.offsetWidth;
  pill.style.maxWidth = prev;
  // The status pill shows its timer only after 3 s: leave room for it, so it never clips.
  const timerLater =
    pill.classList.contains('genie-working') && !pill.querySelector('[data-genie-timer]');
  return timerLater ? w + TIMER_RESERVE_PX : w;
}

/**
 * Make a fitter: each call sets the window to the height of `contentPx` and the width of the
 * content, at most once per size, and keeps the edge that faces the MOG fixed. `windowPx` is the
 * window height now, if known: R11 sets a hidden card back to one line when it shows it again, so
 * a different height fits again. `onSide` gets the side of the MOG (the view aligns its tail
 * with it). A failed step leaves the window as it is.
 */
export function createWindowFit(
  win: () => CardWindow = tauriCardWindow,
  opts: { measureWidth?: () => number | undefined; onSide?: (side: CardSide) => void } = {},
) {
  const measure = opts.measureWidth ?? (() => measureCompactWidth());
  let last: string | undefined;
  let side: CardSide | undefined;
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
      const mog = at.mog;
      if (mog) {
        // The edge that faces the MOG stays: place the new size beside the MOG again.
        const prefer = side ?? sideOf(at.card, mog);
        const placed = placeCard(mog, at.screen, width, h, prefer);
        side = placed.side;
        opts.onSide?.(placed.side);
        await w.setBounds(placed.rect);
        return;
      }
      // No MOG geometry: keep the left edge, and stay on the screen (as before).
      let { x, y } = at.card;
      if (at.screen) {
        y = Math.min(y, at.screen.y + at.screen.h - h);
        x = Math.min(x, at.screen.x + at.screen.w - width);
      }
      await w.setBounds({ x, y, w: width, h });
    } catch {
      // The window keeps its size; the card scrolls in it.
      last = undefined;
    }
  };
}
