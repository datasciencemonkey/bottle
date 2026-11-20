/**
 * Number display for table cells only (#523). A long float from a query ("1220042.3509332137") or
 * a number in scientific notation ("3.62387909843917E8") reads badly in a narrow card. This shows
 * it with the user's thousands separators. Sentences are never touched.
 *
 * The rule is "never change what a value means", and it is decided for the whole column:
 * - A column is numeric only if every non-empty cell is one plain number. A column with any text,
 *   date, percent ("12.5%"), currency ("$1,234.50") or other cell is left exactly as written.
 * - A decimal with `|v| >= 1` and more than two decimals is rounded to two ("1220042.3509" ->
 *   "1,220,042.35"). A decimal with at most two decimals keeps the digits and trailing zeros the
 *   source wrote ("1.10" stays "1.10"), and only gets thousands separators.
 * - A value below 1 is never rounded: "0.0072", "0.0149" and "0.006" stay as written.
 * - Scientific notation is shown as a plain number only when its plain form is exact
 *   ("3.62387909843917E8" -> "362,387,909.84", "1.2E-7" -> "0.00000012").
 * - A value above Number.MAX_SAFE_INTEGER, or whose integer part has more than 15 digits, is left
 *   as written (a double cannot hold it). The decimals that the display rounds away do not count:
 *   "1220042.3509332137" shows as "1,220,042.35" and the raw value stays in the title. An exponent
 *   form with more than 15 significant digits is left as written, because it must be exact.
 * - Integers get thousands separators too, so one column reads the same all the way down, unless
 *   the column looks like ids or years (the header says id/key/code/zip/phone/year, a cell has a
 *   leading zero, or every cell is a four-digit year).
 * - The raw value always stays available: a changed cell sets it as its `title`.
 */

const DECIMAL = /^[-+]?(?:\d{1,40}(?:\.\d{0,40})?|\.\d{1,40})(?:[eE][-+]?\d{1,4})?$/;
const INTEGER = /^[-+]?\d{1,40}$/;
/** A header that names an identifier or a year, so its integers are labels, not quantities. */
const ID_HEADER =
  /(^|[^a-z0-9])(id|ids|uuid|key|code|zip|zipcode|postal|phone|fax|year|yr|ssn|ein|sku|ref|num|no|number|order|invoice)([^a-z0-9]|$)/i;
/** camelCase names such as `customerId` or `orderKey`. */
const ID_CAMEL = /[a-z](Id|Key|Code|Year|No|Num|Number)$/;

export interface CellNumber {
  /** What the cell shows. */
  text: string;
  /** True if `text` differs from the raw value, so the cell needs the raw value as a tooltip. */
  changed: boolean;
  /** True if the cell is one plain number (it aligns right when the whole column is). */
  numeric: boolean;
}

/** The user's locale in the app, or undefined (the runtime default) outside a browser. */
export function userLocale(): string | undefined {
  return typeof navigator !== 'undefined' && navigator.language ? navigator.language : undefined;
}

interface Seps {
  group: (int: number | bigint) => string;
  decimal: string;
}
const seps = new Map<string, Seps>();
function sepsOf(locale: string | undefined): Seps {
  const key = locale ?? '';
  let s = seps.get(key);
  if (!s) {
    let f: Intl.NumberFormat;
    try {
      f = new Intl.NumberFormat(locale, { maximumFractionDigits: 0 });
    } catch {
      f = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });
    }
    const decimal =
      new Intl.NumberFormat(f.resolvedOptions().locale)
        .formatToParts(1.5)
        .find((p) => p.type === 'decimal')?.value ?? '.';
    s = { group: (n) => f.format(n), decimal };
    seps.set(key, s);
  }
  return s;
}

/** Parse a plain number into sign, digits before and after the point, and a flag for an exponent. */
function parts(t: string) {
  const m = /^([-+]?)(\d*)(?:\.(\d*))?(?:[eE]([-+]?\d+))?$/.exec(t);
  if (!m) return undefined;
  const sign = m[1] ?? '';
  let int = m[2] ?? '';
  let frac = m[3] ?? '';
  const exp = m[4];
  const sci = exp !== undefined;
  if (sci) {
    const e = Number(exp);
    if (!Number.isInteger(e) || Math.abs(e) > 40) return undefined;
    // Move the point as text, so the plain form is exact.
    const all = int + frac;
    const point = int.length + e;
    if (point <= 0) {
      int = '0';
      frac = '0'.repeat(-point) + all;
    } else if (point >= all.length) {
      int = all + '0'.repeat(point - all.length);
      frac = '';
    } else {
      int = all.slice(0, point);
      frac = all.slice(point);
    }
  }
  int = int.replace(/^0+(?=\d)/, '');
  if (int === '') int = '0';
  frac = frac.replace(/0+$/, (z) => (sci ? '' : z));
  return { sign, int, frac, sci };
}

/** Significant digits of the written number: from the first non-zero digit to the last digit. */
function sigDigits(int: string, frac: string): number {
  const all = (int + frac).replace(/^0+/, '');
  return all === '' ? 0 : all.length;
}

function formatOne(raw: string, id: boolean, locale: string | undefined): CellNumber {
  const same = (): CellNumber => ({ text: raw, changed: false, numeric: true });
  const t = raw.trim();
  const p = parts(t);
  if (!p) return same();
  const { sign, int, frac, sci } = p;
  const s = sepsOf(locale);
  const isInt = frac === '' && !/[.]/.test(t.replace(/[eE].*$/, ''));
  if (int.length > 16) return same();
  const value = Number(`${int}.${frac || '0'}`);
  if (!Number.isFinite(value) || value > Number.MAX_SAFE_INTEGER) return same();
  // The digits the source wrote (an exponent form is judged on its plain digits).
  // A plain decimal is judged on its integer part only: the decimals that the display rounds away
  // on purpose stay in the title. An exponent form must be exact, so its digits are counted.
  if (int.length > 15 || (sci && sigDigits(int, frac) > 15)) return same();
  const sgn = sign === '-' ? '-' : sign === '+' && !sci ? '+' : '';
  const done = (body: string): CellNumber => {
    const text = sgn + body;
    return { text, changed: text !== t, numeric: true };
  };
  if (isInt && !sci) {
    if (id || /^[-+]?0\d/.test(t)) return same();
    return done(s.group(Number(int)));
  }
  if (value < 1) {
    // Never rounded. A plain decimal is shown as written; an exponent form as its exact plain form.
    if (!sci) return same();
    return done(`0${s.decimal}${frac}`.replace(/[.,٫]$/, ''));
  }
  if (/^[-+]?0\d/.test(t)) return same();
  const decimals = frac.length;
  if (decimals <= 2) {
    return done(s.group(Number(int)) + (decimals > 0 ? s.decimal + frac : ''));
  }
  // Round half up on the written digits (no binary floating point), so 15 integer digits stay exact.
  let scaled = BigInt(int + frac.slice(0, 2).padEnd(2, '0'));
  if ((frac[2] ?? '0') >= '5') scaled += 1n;
  const f = (scaled % 100n).toString().padStart(2, '0');
  return done(`${s.group(scaled / 100n)}${s.decimal}${f}`);
}

/** True if a header names an identifier or a year. */
export const isIdHeader = (h: string): boolean =>
  ID_HEADER.test(h.trim()) || ID_CAMEL.test(h.trim());

/**
 * The display of one column. `cells` are the raw cells of the body rows. A column that is not
 * wholly numeric comes back unchanged, so nothing is guessed from a single cell.
 */
export function formatColumn(header: string, cells: string[], locale?: string): CellNumber[] {
  const plain = (c: string) => ({ text: c, changed: false, numeric: false });
  const filled = cells.map((c) => c.trim()).filter((c) => c !== '');
  const numeric =
    filled.length > 0 &&
    filled.every((c) => c.length <= 80 && (INTEGER.test(c) || DECIMAL.test(c)));
  if (!numeric) return cells.map(plain);
  const years =
    filled.every((c) => /^\d{4}$/.test(c)) && filled.every((c) => +c >= 1800 && +c <= 2200);
  const id = isIdHeader(header) || years;
  const loc = locale ?? userLocale();
  return cells.map((c) =>
    c.trim() === '' ? { text: c, changed: false, numeric: false } : formatOne(c, id, loc),
  );
}
