/**
 * #573: the card window takes the height of its content (the pop-up pill, the status line or the
 * answer card), so the answer is never cut and no clear part of the window lies over the user's
 * app. The width stays the R11 width (`CARD_WIDTH` in `src-tauri/src/windows.rs`).
 *
 * R11 puts the top of the card beside the MOG; a taller card grows down. If it then goes past the
 * bottom of the screen, it moves up to stay on the screen. The card capability grants only the
 * window size, position and monitor reads and writes that this needs.
 */
import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi';
import { currentMonitor, getCurrentWindow } from '@tauri-apps/api/window';

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

/** The window seams. Tests give a fake; the app uses the Tauri window of this view. */
export interface CardWindow {
  setHeight(h: number): Promise<void>;
  /** The logical top and the screen bottom (logical), if the platform gives them. */
  place(): Promise<{ x: number; top: number; screenBottom: number } | undefined>;
  moveTo(x: number, top: number): Promise<void>;
}

export const tauriCardWindow = (): CardWindow => {
  const w = getCurrentWindow();
  return {
    setHeight: (h) => w.setSize(new LogicalSize(CARD_WINDOW_WIDTH, h)),
    async place() {
      const [scale, pos, mon] = await Promise.all([
        w.scaleFactor(),
        w.outerPosition(),
        currentMonitor(),
      ]);
      if (!mon) return undefined;
      const p = pos.toLogical(scale);
      const m = mon.position.toLogical(mon.scaleFactor);
      const z = mon.size.toLogical(mon.scaleFactor);
      return { x: p.x, top: p.y, screenBottom: m.y + z.height };
    },
    moveTo: (x, top) => w.setPosition(new LogicalPosition(x, top)),
  };
};

/**
 * Make a fitter: each call sets the window to the height of `contentPx`, at most once per
 * height, and keeps the bottom on the screen. `windowPx` is the window height now, if known: R11
 * sets a hidden card back to one line when it shows it again, so a different height fits again.
 * A failed step leaves the window as it is.
 */
export function createWindowFit(win: () => CardWindow = tauriCardWindow) {
  let last: number | undefined;
  return async (contentPx: number, windowPx?: number): Promise<void> => {
    const h = cardWindowHeight(contentPx);
    if (h === last && (windowPx === undefined || Math.round(windowPx) === h)) return;
    last = h;
    try {
      const w = win();
      // Move up first, then grow, so no frame goes past the bottom of the screen.
      const at = await w.place();
      if (at && at.top + h > at.screenBottom) await w.moveTo(at.x, at.screenBottom - h);
      await w.setHeight(h);
    } catch {
      // The window keeps its size; the card scrolls in it.
      last = undefined;
    }
  };
}
