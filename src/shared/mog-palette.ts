/**
 * MOG identity colours (#ui-4): a fixed, accessible palette so several MOGs never look alike.
 *
 * The brain and every view read this file. It holds names, hex values and pure functions only,
 * no state. It is not a contract file. The colour is stored as `mog.ui.variant` (already in the
 * Appendix F schema, `crates/mog-core/src/mog_config.rs`, doc'd as "`auto` or a colour variant
 * name"), so no contract change is needed: an unknown or `auto` variant falls back to a colour
 * that `paletteColorOf` derives from the MOG ID, so even a MOG saved before this feature shows a
 * stable, distinct tint.
 *
 * Every colour keeps white pill text at or above the WCAG AA threshold for small text (4.5:1),
 * checked by `mog-palette.test.ts` with the real formula, not an assumption.
 */
import type { MogId } from './interfaces';

/** The on-accent text colour of every palette entry (`tokens.css` `--mog-on-accent`). */
export const PALETTE_TEXT = '#ffffff';

/**
 * `tokens.css` `--mog-surface`/`--mog-face`: the Genie card's dark surface (#ui-4 review #603,
 * finding 5). Keep in sync with `tokens.css`.
 */
export const CARD_SURFACE_HEX = '#18202e';

/**
 * `tokens.css` `--mog-text-muted`: the Genie card's label colour for the target MOG's name
 * (`genie-card.css` `.genie-identity`, review #603, finding 5). The accent itself failed AA for
 * every palette colour as 11px bold text on `CARD_SURFACE_HEX` (2.1:1 to 3:1, all below the
 * 4.5:1 small-text threshold): the palette was tuned for white text on the accent
 * (`PALETTE_TEXT`), not accent text on the dark surface. The name is neutral text instead, and
 * the colour rides a small dot beside it (`.genie-identity-dot`, which carries `--mog-id-color`
 * and so needs no contrast of its own, the same as the card's existing accent stripe). Keep in
 * sync with `tokens.css`.
 */
export const CARD_LABEL_TEXT_HEX = '#b6bfd0';

/** One accent colour a MOG can carry. */
export interface MogPaletteColor {
  /** Stored in `mog.ui.variant`; stable across releases (never renumber or reuse an id). */
  id: string;
  /** The hub swatch label and the `aria-label` of the picker button. */
  label: string;
  /** The pill, ring and glow fill. White text on it is >= 4.5:1 (`mog-palette.test.ts`). */
  hex: string;
}

/**
 * The fixed palette (6-8 colours, D-identity). Each hex also lives as a `--mog-palette-<id>`
 * token in `src/views/tokens.css`; keep the two in sync.
 */
export const MOG_PALETTE: readonly MogPaletteColor[] = [
  { id: 'coral', label: 'Coral', hex: '#b8322a' },
  { id: 'amber', label: 'Amber', hex: '#8a4b08' },
  { id: 'emerald', label: 'Emerald', hex: '#047857' },
  { id: 'teal', label: 'Teal', hex: '#0f6b66' },
  { id: 'sky', label: 'Sky', hex: '#0b5fa5' },
  { id: 'indigo', label: 'Indigo', hex: '#4338ca' },
  { id: 'violet', label: 'Violet', hex: '#6d28d9' },
  { id: 'rose', label: 'Rose', hex: '#be123c' },
];

/** The variant ID of the first palette entry (`create()`'s fallback when the store is empty). */
export const DEFAULT_VARIANT: string = MOG_PALETTE[0]?.id ?? 'coral';

function hexToRgb(hex: string): [number, number, number] | undefined {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m?.[1]) return undefined;
  const n = m[1];
  return [parseInt(n.slice(0, 2), 16), parseInt(n.slice(2, 4), 16), parseInt(n.slice(4, 6), 16)];
}

/** The sRGB -> linear-light step of the WCAG relative luminance formula. */
function linearize(c: number): number {
  const v = c / 255;
  return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
}

function relativeLuminance(rgb: [number, number, number]): number {
  const [r, g, b] = rgb.map(linearize);
  return 0.2126 * (r ?? 0) + 0.7152 * (g ?? 0) + 0.0722 * (b ?? 0);
}

/**
 * The WCAG contrast ratio of two colours (1 to 21), order-independent. `undefined` for a hex
 * that is not `#rrggbb`.
 */
export function contrastRatio(hexA: string, hexB: string): number | undefined {
  const a = hexToRgb(hexA);
  const b = hexToRgb(hexB);
  if (!a || !b) return undefined;
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const lighter = Math.max(la, lb);
  const darker = Math.min(la, lb);
  return (lighter + 0.05) / (darker + 0.05);
}

/** A small, fast, non-cryptographic string hash (FNV-1a), for a stable fallback index. */
function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

const byId = new Map(MOG_PALETTE.map((c) => [c.id, c]));

/**
 * The palette colour for a MOG. `variant` is `mog.ui.variant`: a known palette id wins. Any
 * other value (`auto`, an empty string, or a legacy value this build does not know) falls back
 * to a colour hashed from `mogId`, so the same MOG always shows the same fallback tint and two
 * different MOGs usually differ.
 */
export function paletteColorOf(
  variant: string | undefined,
  mogId: MogId | string,
): MogPaletteColor {
  const known = variant !== undefined ? byId.get(variant) : undefined;
  if (known) return known;
  const i = hash(mogId) % MOG_PALETTE.length;
  return (
    MOG_PALETTE[i] ?? MOG_PALETTE[0] ?? { id: DEFAULT_VARIANT, label: 'Coral', hex: '#b8322a' }
  );
}

/**
 * The variant id for a new MOG (D-identity: "next unused, or by index"): the first palette
 * colour that no MOG in `usedVariants` has yet, else the palette entry at `count % length`, so
 * MOGs past the palette size still get a deterministic, evenly spread colour.
 */
export function nextVariant(usedVariants: readonly string[], count = usedVariants.length): string {
  const used = new Set(usedVariants);
  const free = MOG_PALETTE.find((c) => !used.has(c.id));
  if (free) return free.id;
  return (MOG_PALETTE[count % MOG_PALETTE.length] ?? MOG_PALETTE[0])?.id ?? DEFAULT_VARIANT;
}
