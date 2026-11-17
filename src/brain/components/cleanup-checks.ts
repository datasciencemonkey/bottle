/**
 * Table CAP-CHECK (#55; CAP-68, 8.1 step 6, ADR 0070): the meaning checks of a cleanup model
 * reply. The baseline is the model input, the pre-pass text: the pre-pass is deterministic, so
 * only the model can change the meaning. Any failed check keeps the raw text.
 *
 * Rows: a number changes · an entity changes · the length ratio is outside 0.5-1.3 · new URLs
 * appear · new code appears · an answer to the question appears. ADR 0070 adds one stricter
 * row: a negation goes or appears. A false alarm only keeps the raw text, so each check leans
 * to fail. The checks return check names only, never text.
 */
import { entitiesOf } from './router-grounding';

export type CleanupCheck =
  'empty' | 'number' | 'entity' | 'length_ratio' | 'new_url' | 'new_code' | 'answer' | 'negation';

/** The length ratio bounds of Table CAP-CHECK. */
export const LENGTH_RATIO_MIN = 0.5;
export const LENGTH_RATIO_MAX = 1.3;

const sortedJoin = (xs: string[]) => [...xs].sort().join(' ');

/** The numbers of `s`, thousands commas dropped: "1,250" and "1250" are the same number. */
function numbers(s: string): string[] {
  return (s.match(/\d+(?:[.,]\d+)*/g) ?? []).map((n) => n.replace(/,(?=\d{3}(?:\D|$))/g, ''));
}

/** Dotted or snake-case names, lowercase: table names such as `main.finance.invoices`. */
function identifiers(s: string): string[] {
  return (s.match(/[\p{L}\p{N}_]+(?:\.[\p{L}\p{N}_]+)+|[\p{L}\p{N}]*_[\p{L}\p{N}_]*/gu) ?? [])
    .map((x) => x.toLowerCase())
    .filter((x) => /[\p{L}]/u.test(x) && !/^\d+(?:\.\d+)*$/.test(x));
}

/** The lowercase words of `s`. */
function words(s: string): string[] {
  return (s.match(/[\p{L}\p{N}][\p{L}\p{N}'&-]*/gu) ?? []).map((w) => w.toLowerCase());
}

const URL_RE = /\bhttps?:\/\/\S+|\bwww\.[^\s]+/gi;
const CODE_RE = /`|<\s*\/?\s*script|\$\(|\$\{|<\/?[a-z][a-z0-9-]*(?:\s[^>]*)?>/i;

function urls(s: string): Set<string> {
  return new Set((s.match(URL_RE) ?? []).map((u) => u.replace(/[.,!?;:]+$/, '').toLowerCase()));
}

/** Whether `s` is a JSON object or array: a model that answers with data, not text. */
function isJsonValue(s: string): boolean {
  const t = s.trim();
  if (!/^[[{][\s\S]*[\]}]$/.test(t)) return false;
  try {
    JSON.parse(t);
    return true;
  } catch {
    return false;
  }
}

const NEGATIONS: ReadonlySet<string> = new Set([
  'not',
  'no',
  'never',
  'none',
  'nobody',
  'nothing',
  'neither',
  'nor',
  'nowhere',
  'cannot',
  'without',
]);

/** The negations of `s`: negation words and words that end in "n't". */
function negations(s: string): string[] {
  return words(s.replace(/’/g, "'")).filter((w) => NEGATIONS.has(w) || /n't$/.test(w));
}

const QUESTION_START =
  /^(?:what|whats|what's|who|whom|whose|which|when|where|why|how|is|are|was|were|do|does|did|can|could|would|should|will|shall|has|have|had|may)\b/i;

/** Whether `s` reads as a question. */
function isQuestion(s: string): boolean {
  const t = s.trim();
  return t.endsWith('?') || QUESTION_START.test(t);
}

/** The sentences of `s`; a dot inside a number or a name does not end a sentence. */
function sentenceCount(s: string): number {
  return s.split(/[.?!]+(?=\s|$)/).filter((x) => /[\p{L}\p{N}]/u.test(x)).length;
}

/**
 * "An answer to the question appears": for a question, the reply adds a sentence, or it is no
 * longer a question and it holds a word that the question did not have.
 */
function answers(input: string, output: string): boolean {
  if (!isQuestion(input)) return false;
  if (sentenceCount(output) > Math.max(1, sentenceCount(input))) return true;
  if (output.trim().endsWith('?')) return false;
  const known = new Set(words(input));
  return words(output).some((w) => !known.has(w));
}

/** The CAP-CHECK checks that `output` fails against the model input `input`. */
export function failedChecks(input: string, output: string): CleanupCheck[] {
  const out = output.trim();
  if (out === '') return ['empty'];
  const failed: CleanupCheck[] = [];

  if (sortedJoin(numbers(input)) !== sortedJoin(numbers(out))) failed.push('number');

  // An entity changes: a name or a table name goes, or a new one appears. Case does not count,
  // so "emea" may become "EMEA" and a sentence may start with a capital.
  const inWords = new Set(words(input));
  const outWords = new Set(words(out));
  const entityChanged =
    entitiesOf(input).some((e) => !outWords.has(e)) ||
    entitiesOf(out).some((e) => !inWords.has(e)) ||
    sortedJoin(identifiers(input)) !== sortedJoin(identifiers(out));
  if (entityChanged) failed.push('entity');

  const ratio = out.length / Math.max(1, input.trim().length);
  if (ratio < LENGTH_RATIO_MIN || ratio > LENGTH_RATIO_MAX) failed.push('length_ratio');

  const known = urls(input);
  if ([...urls(out)].some((u) => !known.has(u))) failed.push('new_url');

  if ((CODE_RE.test(out) && !CODE_RE.test(input)) || isJsonValue(out)) failed.push('new_code');

  if (answers(input, out)) failed.push('answer');

  if (sortedJoin(negations(input)) !== sortedJoin(negations(out))) failed.push('negation');

  return failed;
}
