// Adapted from cjpais/Handy src-tauri/src/audio_toolkit/text.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
/**
 * The deterministic pre-pass of T5 CleanupProvider (#55; CAP-66a, glossary "pre-pass", F1
 * step 9, ADR 0070): fillers, the user dictionary and numerals, before the cleanup model.
 *
 * The filler lists, the capital hand-over and the stutter collapse follow Handy's
 * `remove_filler_words` and `collapse_stutters`. The pre-pass is pure: the same input gives the
 * same output, and a second run changes nothing. It never logs.
 */

/**
 * Filler sounds that are no word in any language the STT models give, so they go in every
 * language (Handy `UNIVERSAL_FILLER_WORDS`).
 */
const UNIVERSAL_FILLERS = [
  'uh',
  'uhm',
  'umm',
  'uhh',
  'uhhh',
  'ehh',
  'ehm',
  'ahm',
  'hmm',
  'hm',
  'mmm',
];

/**
 * Fillers that are words in another language ("um" is "a" in Portuguese and "at" in German), so
 * they go only for that language (Handy `gated_filler_words_for_language`).
 */
const GATED_FILLERS: Readonly<Record<string, readonly string[]>> = {
  en: ['um', 'ah', 'eh'],
  de: ['äh', 'ähm'],
  fr: ['euh'],
};

/** One user dictionary entry: the spoken form and the written form. */
export interface DictionaryEntry {
  from: string;
  to: string;
}

export interface PrepassOptions {
  /** The user dictionary. Empty by default. */
  dictionary?: readonly DictionaryEntry[];
  /** The language of the transcript (BCP 47). Default `en`. */
  language?: string;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Letters or digits on both sides of a match make it part of a word. */
const WORD_CHAR = '[\\p{L}\\p{N}_]';

function wordPattern(word: string, suffix = ''): RegExp {
  return new RegExp(`(?<!${WORD_CHAR})${escapeRegExp(word)}(?!${WORD_CHAR})${suffix}`, 'giu');
}

/** Whether a word put after `kept` opens a sentence. */
function opensSentence(kept: string): boolean {
  const last = kept.trimEnd().slice(-1);
  return last === '' || '.!?…'.includes(last);
}

/** Collapses runs of spaces and trims. */
function tidy(s: string): string {
  return s
    .replace(/\s{2,}/g, ' ')
    .replace(/\s+([,.!?;:])/g, '$1')
    .trim();
}

/** Deletes each match. A capital filler that opened a sentence gives its capital to the next word. */
function removeMatches(text: string, pattern: RegExp): string {
  let kept = '';
  let resume = 0;
  let capitalOwed = false;
  const push = (segment: string) => {
    if (capitalOwed) {
      const i = segment.search(/[\p{L}\p{N}]/u);
      if (i >= 0) {
        capitalOwed = false;
        kept += segment.slice(0, i) + segment.charAt(i).toUpperCase() + segment.slice(i + 1);
        return;
      }
    }
    kept += segment;
  };
  for (const m of text.matchAll(pattern)) {
    push(text.slice(resume, m.index));
    if (/^\p{Lu}/u.test(m[0]) && opensSentence(kept)) capitalOwed = true;
    resume = m.index + m[0].length;
  }
  push(text.slice(resume));
  return kept;
}

/** Removes filler sounds. "um", "ah" and "eh" go only for English. */
export function removeFillers(text: string, language = 'en'): string {
  const base = language.split(/[-_]/)[0]?.toLowerCase() ?? '';
  const words = [...UNIVERSAL_FILLERS, ...(GATED_FILLERS[base] ?? [])];
  let out = text;
  for (const w of words) out = removeMatches(out, wordPattern(w, '[,.]?'));
  return tidy(out);
}

/** Collapses a word said 3 or more times in a row to one (Handy `collapse_stutters`). */
export function collapseStutters(text: string): string {
  const words = text.split(/\s+/).filter((w) => w !== '');
  const out: string[] = [];
  for (let i = 0; i < words.length;) {
    const w = words[i] as string;
    let n = 1;
    if (/^\p{L}+$/u.test(w)) {
      while (i + n < words.length && (words[i + n] as string).toLowerCase() === w.toLowerCase())
        n++;
    }
    out.push(w);
    i += n >= 3 ? n : 1;
  }
  return out.join(' ');
}

/** Replaces each dictionary entry on whole words, without regard to case. */
export function applyDictionary(text: string, dictionary: readonly DictionaryEntry[]): string {
  let out = text;
  // Longer entries first, so "unity catalog" wins over "unity".
  const sorted = [...dictionary]
    .filter((e) => e.from.trim() !== '')
    .sort((a, b) => b.from.length - a.from.length);
  for (const e of sorted) {
    const spaced = escapeRegExp(e.from.trim()).replace(/\s+/g, '\\s+');
    const re = new RegExp(`(?<!${WORD_CHAR})${spaced}(?!${WORD_CHAR})`, 'giu');
    out = out.replace(re, () => e.to);
  }
  return out;
}

// ---------- numerals ----------

const UNITS: Readonly<Record<string, number>> = {
  zero: 0,
  one: 1,
  two: 2,
  three: 3,
  four: 4,
  five: 5,
  six: 6,
  seven: 7,
  eight: 8,
  nine: 9,
  ten: 10,
  eleven: 11,
  twelve: 12,
  thirteen: 13,
  fourteen: 14,
  fifteen: 15,
  sixteen: 16,
  seventeen: 17,
  eighteen: 18,
  nineteen: 19,
};
const TENS: Readonly<Record<string, number>> = {
  twenty: 20,
  thirty: 30,
  forty: 40,
  fifty: 50,
  sixty: 60,
  seventy: 70,
  eighty: 80,
  ninety: 90,
};
const SCALES: Readonly<Record<string, number>> = {
  thousand: 1_000,
  million: 1_000_000,
  billion: 1_000_000_000,
};

const isNumberWord = (w: string) =>
  w in UNITS || w in TENS || w === 'hundred' || w in SCALES || w === 'and';

/** Parses 1-99 at `i`: a unit or teen, or a tens word with an optional unit 1-9. */
function parseBelow100(ws: string[], i: number): [number, number] | undefined {
  const w = ws[i];
  if (w === undefined) return undefined;
  if (w in UNITS) return [UNITS[w] as number, i + 1];
  if (w in TENS) {
    const next = ws[i + 1];
    const u = next === undefined ? undefined : UNITS[next];
    if (u !== undefined && u >= 1 && u <= 9) return [(TENS[w] as number) + u, i + 2];
    return [TENS[w] as number, i + 1];
  }
  return undefined;
}

/** Parses 1-999 at `i`: "[unit] hundred [and] [below 100]" or "below 100". */
function parseBelow1000(ws: string[], i: number): [number, number] | undefined {
  const first = ws[i];
  const u = first === undefined ? undefined : UNITS[first];
  if (u !== undefined && u >= 1 && u <= 9 && ws[i + 1] === 'hundred') {
    let j = i + 2;
    let rest = 0;
    const afterAnd = ws[j] === 'and' ? j + 1 : j;
    const tail = parseBelow100(ws, afterAnd);
    if (tail) {
      rest = tail[0];
      j = tail[1];
    }
    return [u * 100 + rest, j];
  }
  return parseBelow100(ws, i);
}

/** Parses a whole cardinal that uses every word of `ws`, or gives undefined. */
function parseCardinal(ws: string[]): number | undefined {
  let total = 0;
  let i = 0;
  let lastScale = Infinity;
  while (i < ws.length) {
    const part = parseBelow1000(ws, i);
    if (!part) return undefined;
    const [n, j] = part;
    const scaleWord = ws[j];
    const scale = scaleWord === undefined ? undefined : SCALES[scaleWord];
    if (scale !== undefined) {
      if (scale >= lastScale) return undefined;
      lastScale = scale;
      total += n * scale;
      i = j + 1;
    } else {
      if (j !== ws.length) return undefined;
      total += n;
      i = j;
    }
  }
  return i === ws.length && ws.length > 0 ? total : undefined;
}

/** A word token split into its number words and its trailing punctuation, or undefined. */
function numberToken(tok: string): { words: string[]; punct: string } | undefined {
  const m = /^([\p{L}]+(?:-[\p{L}]+)*)([,.!?;:]*)$/u.exec(tok);
  if (!m) return undefined;
  const words = (m[1] as string).toLowerCase().split('-');
  return words.every(isNumberWord) ? { words, punct: m[2] as string } : undefined;
}

/**
 * Writes spoken cardinals as digits: "twenty five" → "25", "one hundred and twenty" → "120".
 * A run converts only when it is one whole cardinal of 2 or more words, or one word of 10 or
 * more. A lone "one" to "nine", a digit read-out ("one two three"), a run next to a digit
 * ("40 thousand") and a run before "point" stay as they are, so a meaning never changes
 * (ADR 0070).
 */
export function spokenNumeralsToDigits(text: string): string {
  // Even indexes are words, odd indexes the whitespace between them.
  const tokens = text.split(/(\s+)/);
  const out: string[] = [];
  let i = 0;
  while (i < tokens.length) {
    const first = numberToken(tokens[i] as string);
    if (!first || first.words[0] === 'and') {
      out.push(tokens[i] as string);
      i++;
      continue;
    }
    // The run: number words up to a token with punctuation or a token that is not a number.
    const words: string[] = [];
    let end = i;
    let punct = '';
    for (let j = i; j < tokens.length; j += 2) {
      const t = numberToken(tokens[j] as string);
      if (!t) break;
      words.push(...t.words);
      end = j;
      punct = t.punct;
      if (t.punct !== '') break;
    }
    // A trailing "and" is not part of the number.
    while (words[words.length - 1] === 'and' && punct === '' && end > i) {
      words.pop();
      end -= 2;
    }
    const prev = out.length >= 2 ? (out[out.length - 2] as string) : '';
    const next = punct === '' ? ((tokens[end + 2] as string | undefined) ?? '') : '';
    const nearDigit = /\d[,.!?;:]*$/.test(prev) || /^\d/.test(next) || /^point\b/i.test(next);
    const value = parseCardinal(words);
    if (value !== undefined && !nearDigit && (words.length >= 2 || value >= 10)) {
      out.push(`${value}${punct}`);
    } else {
      // Not one whole cardinal: keep the whole run as it is.
      out.push(...tokens.slice(i, end + 1));
    }
    i = end + 1;
  }
  return out.join('');
}

/** The number of words of `text`, for the 4-word rule (CAP-66b). */
export function wordCount(text: string): number {
  return text.split(/\s+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;
}

/** The pre-pass: fillers, stutters, the user dictionary, then numerals (CAP-66a). */
export function prepass(raw: string, opts: PrepassOptions = {}): string {
  const noFillers = removeFillers(raw, opts.language ?? 'en');
  const noStutters = collapseStutters(noFillers);
  const dictionary = applyDictionary(noStutters, opts.dictionary ?? []);
  return tidy(spokenNumeralsToDigits(dictionary));
}
