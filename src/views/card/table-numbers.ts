/**
 * Number display for table cells only (#523). A long float from a query ("1220042.3509332137") or
 * a number in scientific notation ("3.62387909843917E8") reads badly in a narrow card. This shows
 * it with the user's thousands separators and at most two decimals. Sentences are never touched.
 *
 * Rules, so a value never changes its meaning:
 * - Only a whole cell that is one plain number with a decimal part or an exponent is rewritten.
 * - An integer, a date, an id, a version, text, `NaN`, `Infinity`, a percent or a currency string
 *   stays as it is. A number with a leading zero ("007.5") stays.
 * - A non-zero value that two decimals would show as 0 ("1.2E-7") stays as it is.
 * - A value of 1e21 or more stays as it is (the digits would not be exact).
 * - The raw value always stays available: the cell sets it as its `title`.
 */

/** A whole cell that is one plain decimal number (bounded: a cell is at most a few hundred chars). */
const DECIMAL = /^[-+]?(?:\d{1,40}(?:\.\d{0,40})?|\.\d{1,40})(?:[eE][-+]?\d{1,4})?$/;
const INTEGER = /^[-+]?\d{1,40}$/;

export interface CellNumber {
  /** What the cell shows. */
  text: string;
  /** True if `text` differs from the raw value, so the cell needs the raw value as a tooltip. */
  changed: boolean;
  /** True if the cell is one plain number (integer or decimal): its column aligns right. */
  numeric: boolean;
}

const formats = new Map<string, Intl.NumberFormat>();
function formatter(locale: string | undefined): Intl.NumberFormat {
  const key = locale ?? '';
  let f = formats.get(key);
  if (!f) {
    try {
      f = new Intl.NumberFormat(locale, { maximumFractionDigits: 2, minimumFractionDigits: 0 });
    } catch {
      f = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
    }
    formats.set(key, f);
  }
  return f;
}

/** The user's locale in the app, or undefined (the runtime default) outside a browser. */
export function userLocale(): string | undefined {
  return typeof navigator !== 'undefined' && navigator.language ? navigator.language : undefined;
}

export function formatCellNumber(raw: string, locale?: string): CellNumber {
  const t = raw.trim();
  const same = (numeric: boolean): CellNumber => ({ text: raw, changed: false, numeric });
  if (t === '' || t.length > 80) return same(false);
  if (INTEGER.test(t)) return same(true);
  if (!DECIMAL.test(t)) return same(false);
  // A leading zero before another digit ("007.5", "01E3") is more likely a code than a quantity.
  if (/^[-+]?0\d/.test(t)) return same(true);
  const n = Number(t);
  if (!Number.isFinite(n)) return same(true);
  const abs = Math.abs(n);
  if (abs >= 1e21 || (abs !== 0 && abs < 0.005)) return same(true);
  const text = formatter(locale ?? userLocale()).format(n);
  // `-0.001` rounds to "-0": never show a sign on a zero.
  const shown = /^[-−]0(?:[.,]0+)?$/.test(text) ? text.replace(/^[-−]/, '') : text;
  return { text: shown, changed: shown !== t, numeric: true };
}
