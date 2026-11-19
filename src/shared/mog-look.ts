/**
 * Where one MOG's eyes look (lamp-alive). The Rust host reads the global pointer (the MOG window
 * gets no mouse move while the pointer is outside it), turns it into a vector from the eye centre
 * (`crates/mog-core/src/mog_look.rs`) and sends `mog.look` `{ mogId, dx, dy }` to the brain. The
 * brain passes `{ dx, dy }` on to that one MOG view as the Tauri event `mog:look` (this file).
 * The view only listens (ARC-4). A pointer position never reaches the brain or a view.
 */
export const MOG_LOOK_EVENT = 'mog:look';

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
