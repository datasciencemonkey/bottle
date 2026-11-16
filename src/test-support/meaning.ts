// The AT-05 corpus and meaning oracle (#231). AT-05: run a corpus with numbers, units, negation,
// names, table names and self-corrections; numbers and entities stay exact or the raw text shows.
//
// Every item is synthetic (no real transcript, name or customer data). `keep` lists the tokens
// whose meaning must survive cleanup exactly: numbers with their units, negations, names, table
// names, and for a self-correction the corrected value. The oracle compares tokens (lower case,
// punctuation removed, decimals, percents and dotted table names kept, thousands commas dropped),
// the same rules as the Rust STT scorer `mog_core::stt_bench::normalize` (ADR 0034).
//
// The oracle checks the test side of Table CAP-CHECK that a corpus can decide: a kept token
// changes, a number (a token with a digit) appears, changes or goes (only a self-correction may
// drop its retracted number), the length ratio is outside 0.5-1.3, a new URL or new code appears. "An answer to
// the question appears" needs a judge and is not checked here. Failures name the item id, never
// the cleaned text.

export const MEANING_CATEGORIES = [
  'numbers',
  'units',
  'negation',
  'names',
  'table_names',
  'self_corrections',
] as const;
export type MeaningCategory = (typeof MEANING_CATEGORIES)[number];

export interface MeaningItem {
  id: string;
  category: MeaningCategory;
  /** The raw STT text (lower case, as a local model gives it). */
  raw: string;
  /** Tokens that must stay exact in the cleaned text. */
  keep: string[];
}

export const MEANING_CORPUS: readonly MeaningItem[] = [
  {
    id: 'num-1',
    category: 'numbers',
    raw: 'um ship 1,250 units to the north region by friday',
    keep: ['1,250'],
  },
  {
    id: 'num-2',
    category: 'numbers',
    raw: 'the margin went from 3.5 to 4.25 percent last quarter',
    keep: ['3.5', '4.25'],
  },
  {
    id: 'num-3',
    category: 'numbers',
    raw: 'so we closed 17 deals and lost 4 of them in march',
    keep: ['17', '4'],
  },
  {
    id: 'num-4',
    category: 'numbers',
    raw: 'the forecast is 2.8 million for q3 and 3.1 million for q4',
    keep: ['2.8', 'q3', '3.1', 'q4'],
  },
  {
    id: 'num-5',
    category: 'numbers',
    raw: 'please call extension 4417 after 2 pm',
    keep: ['4417', '2'],
  },
  {
    id: 'unit-1',
    category: 'units',
    raw: 'the p95 latency is 850 ms on the m1 laptop',
    keep: ['850 ms'],
  },
  {
    id: 'unit-2',
    category: 'units',
    raw: 'we moved 12 tb of data into the new volume',
    keep: ['12 tb'],
  },
  {
    id: 'unit-3',
    category: 'units',
    raw: 'revenue grew 7% while cost grew 2%',
    keep: ['7%', '2%'],
  },
  {
    id: 'unit-4',
    category: 'units',
    raw: 'the job ran for 45 minutes on 8 nodes',
    keep: ['45 minutes', '8 nodes'],
  },
  {
    id: 'unit-5',
    category: 'units',
    raw: 'each box weighs 2.5 kg and costs 30 dollars',
    keep: ['2.5 kg', '30 dollars'],
  },
  {
    id: 'neg-1',
    category: 'negation',
    raw: 'do not send the draft to the customer yet',
    keep: ['not'],
  },
  {
    id: 'neg-2',
    category: 'negation',
    raw: "we don't have the numbers for emea so hold the report",
    keep: ["don't"],
  },
  {
    id: 'neg-3',
    category: 'negation',
    raw: 'the pipeline never finished last night',
    keep: ['never'],
  },
  {
    id: 'neg-4',
    category: 'negation',
    raw: 'no region missed the target except apac',
    keep: ['no', 'except'],
  },
  {
    id: 'neg-5',
    category: 'negation',
    raw: 'it is not unlikely that the launch slips a week',
    keep: ['not unlikely'],
  },
  {
    id: 'name-1',
    category: 'names',
    raw: 'ask Priya Raman to review the forecast with Tomasz',
    keep: ['Priya Raman', 'Tomasz'],
  },
  {
    id: 'name-2',
    category: 'names',
    raw: 'Northwind Traders signed the renewal on monday',
    keep: ['Northwind Traders'],
  },
  {
    id: 'name-3',
    category: 'names',
    raw: 'the Contoso pilot and the Fabrikam pilot both start in june',
    keep: ['Contoso', 'Fabrikam'],
  },
  {
    id: 'name-4',
    category: 'names',
    raw: 'send it to Oyelaran and copy Nguyen',
    keep: ['Oyelaran', 'Nguyen'],
  },
  {
    id: 'name-5',
    category: 'names',
    raw: 'the Lisbon office and the Osaka office share one budget',
    keep: ['Lisbon', 'Osaka'],
  },
  {
    id: 'table-1',
    category: 'table_names',
    raw: 'join main.finance.invoices with the customers table',
    keep: ['main.finance.invoices'],
  },
  {
    id: 'table-2',
    category: 'table_names',
    raw: 'the view sales_gold.daily_bookings is stale again',
    keep: ['sales_gold.daily_bookings'],
  },
  {
    id: 'table-3',
    category: 'table_names',
    raw: 'count rows in ops.raw_events_v2 for yesterday',
    keep: ['ops.raw_events_v2'],
  },
  {
    id: 'table-4',
    category: 'table_names',
    raw: 'drop nothing just read hr.headcount_snapshot',
    keep: ['hr.headcount_snapshot'],
  },
  {
    id: 'table-5',
    category: 'table_names',
    raw: 'compare dev.ml.features_2024 with prod.ml.features_2024',
    keep: ['dev.ml.features_2024', 'prod.ml.features_2024'],
  },
  {
    id: 'fix-1',
    category: 'self_corrections',
    raw: 'set the limit to 50 no sorry 500 rows',
    keep: ['500'],
  },
  {
    id: 'fix-2',
    category: 'self_corrections',
    raw: 'meet on tuesday actually make that thursday at 3',
    keep: ['thursday', '3'],
  },
  {
    id: 'fix-3',
    category: 'self_corrections',
    raw: 'send it to Ana I mean to Bruno',
    keep: ['Bruno'],
  },
  {
    id: 'fix-4',
    category: 'self_corrections',
    raw: 'use the emea table no the apac table for this',
    keep: ['apac'],
  },
  {
    id: 'fix-5',
    category: 'self_corrections',
    raw: 'the budget is 40 thousand wait 45 thousand',
    keep: ['45 thousand'],
  },
];

/** The comparison tokens of a text. */
export function tokens(text: string): string[] {
  const s = text
    .toLowerCase()
    .replace(/(?<=\d),(?=\d)/g, '')
    // A minus sign stays only at the start of a number: "-3" keeps its sign, "a-b" splits.
    .replace(/-(?!\d)|(?<=[\p{L}\p{N}])-/gu, ' ');
  return s
    .replace(/[^\p{L}\p{N}.%'_-]+/gu, ' ')
    .split(' ')
    .map((t) => t.replace(/^[.']+|[.']+$/g, ''))
    .filter((t) => t.length > 0);
}

function containsRun(hay: string[], needle: string[]): boolean {
  if (needle.length === 0) return false;
  for (let i = 0; i + needle.length <= hay.length; i++) {
    if (needle.every((t, j) => hay[i + j] === t)) return true;
  }
  return false;
}

/** The tokens with a digit, sorted: the numbers of a text. */
function numberTokens(text: string): string[] {
  return tokens(text)
    .filter((t) => /\d/.test(t))
    .sort();
}

/** True when every token of `sub` is in `all`, as often or less often. */
function subMultiset(sub: string[], all: string[]): boolean {
  const left = [...all];
  return sub.every((t) => {
    const i = left.indexOf(t);
    if (i < 0) return false;
    left.splice(i, 1);
    return true;
  });
}

const URL_RE = /\bhttps?:\/\/\S+|\bwww\.\S+/gi;
const CODE_RE = /`|```|<script|\$\(/i;

/** The ways `out` loses the meaning of `item`. Empty when the meaning is kept. */
export function meaningFailures(
  item: MeaningItem,
  out: { text: string; usedRaw: boolean },
): string[] {
  if (out.usedRaw) {
    return out.text === item.raw
      ? []
      : [`${item.id}: usedRaw is set but the text is not the raw text`];
  }
  const failures: string[] = [];
  const got = tokens(out.text);
  item.keep.forEach((k, i) => {
    if (!containsRun(got, tokens(k))) failures.push(`${item.id}: kept token ${i + 1} changed`);
  });
  // "A number changes" (Table CAP-CHECK): no number may appear or change. Only a
  // self-correction may drop a number (the retracted value).
  const rawNums = numberTokens(item.raw);
  const outNums = numberTokens(out.text);
  const numbersOk =
    item.category === 'self_corrections'
      ? subMultiset(outNums, rawNums)
      : outNums.join(' ') === rawNums.join(' ');
  if (!numbersOk) failures.push(`${item.id}: a number changed, appeared or was dropped`);
  const ratio = out.text.length / item.raw.length;
  if (ratio < 0.5 || ratio > 1.3) {
    failures.push(`${item.id}: length ratio ${ratio.toFixed(2)} is outside 0.5-1.3`);
  }
  const rawUrls = new Set(item.raw.match(URL_RE) ?? []);
  if ((out.text.match(URL_RE) ?? []).some((u) => !rawUrls.has(u))) {
    failures.push(`${item.id}: a new URL appears`);
  }
  if (CODE_RE.test(out.text) && !CODE_RE.test(item.raw)) {
    failures.push(`${item.id}: new code appears`);
  }
  return failures;
}
