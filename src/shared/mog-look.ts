/**
 * Where one MOG's eyes look (lamp-alive). The Rust host reads the global pointer (the MOG window
 * gets no mouse move while the pointer is outside it), turns it into a vector from the eye centre
 * (`crates/mog-core/src/mog_look.rs`) and sends `mog.look` `{ mogId, dx, dy }` to the brain. The
 * brain passes `{ dx, dy }` on to that one MOG view as the Tauri event `mog:look` (this file).
 * The view only listens (ARC-4). A pointer position never reaches the brain or a view.
 */
export const MOG_LOOK_EVENT = 'mog:look';

/**
 * The `action` of the `view_emit` `click` that a MOG view sends when the user hides or shows its
 * art (payload `{ action, hidden }`), and once when it loads. Hide only hides the art, the OS
 * window stays visible, so the host must be told, or it would keep polling the pointer for a
 * hidden MOG. The host (`mog_look_host::hide_notice`) takes it and does not relay it to the
 * brain: it is not a user click.
 */
export const MOG_VIEW_HIDDEN = 'view.hidden';

/** A look vector in the unit disc; x grows right, y grows down. `0, 0` is straight ahead. */
export interface MogLook {
  dx: number;
  dy: number;
}

const unit = (v: unknown): number | undefined =>
  typeof v === 'number' && Number.isFinite(v) ? Math.max(-1, Math.min(1, v)) : undefined;

/** The look vector of a payload, clamped to the unit disc; undefined if it is not one. */
export function asMogLook(p: unknown): MogLook | undefined {
  if (typeof p !== 'object' || p === null) return undefined;
  const dx = unit((p as Record<string, unknown>)['dx']);
  const dy = unit((p as Record<string, unknown>)['dy']);
  if (dx === undefined || dy === undefined) return undefined;
  const len = Math.hypot(dx, dy);
  return len > 1 ? { dx: dx / len, dy: dy / len } : { dx, dy };
}
