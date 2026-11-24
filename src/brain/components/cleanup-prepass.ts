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
  en: ['um', 'ah', 'eh', 'er'],
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

const NUMBER_ALT = () =>
  [...Object.keys(UNITS), ...Object.keys(TENS), 'hundred', ...Object.keys(SCALES)].join('|');

/**
 * Puts `NUMBER_BREAK` in place of a filler between two number words, so that "twenty uh five"
 * does not become one run once the filler goes.
 */
function markNumberBreaks(text: string, language: string): string {
  let out = text;
  for (const w of fillersFor(language)) {
    const re = new RegExp(
      `(?<=(?<!${WORD_CHAR})(?:${NUMBER_ALT()})[,.]?\\s+)${escapeRegExp(w)}[,.]?(?=\\s+(?:${NUMBER_ALT()})(?!${WORD_CHAR}))`,
      'giu',
    );
    out = out.replace(re, NUMBER_BREAK);
  }
  return out;
}

/** The filler words for a language. */
function fillersFor(language: string): string[] {
  const base = language.split(/[-_]/)[0]?.toLowerCase() ?? '';
  return [...UNIVERSAL_FILLERS, ...(GATED_FILLERS[base] ?? [])];
}

/** Removes filler sounds. "um", "ah" and "eh" go only for English. */
export function removeFillers(text: string, language = 'en'): string {
  let out = text;
  for (const w of fillersFor(language)) out = removeMatches(out, wordPattern(w, '[,.]?'));
  return tidy(out);
}

/**
 * Collapses a word said 3 or more times in a row to one (Handy `collapse_stutters`). A number
 * word never collapses: "five five five" is a digit read-out, not a stutter.
 */
export function collapseStutters(text: string): string {
  const words = text.split(/\s+/).filter((w) => w !== '');
  const out: string[] = [];
  for (let i = 0; i < words.length;) {
    const w = words[i] as string;
    let n = 1;
    if (/^\p{L}+$/u.test(w) && !isNumberWord(w.toLowerCase())) {
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

/** Ordinal words: "first" to "nineteenth", "twentieth" to "ninetieth", "hundredth", ... */
export const ORDINAL_WORDS: ReadonlySet<string> = new Set([
  ...(
    'first second third fourth fifth sixth seventh eighth ninth tenth eleventh twelfth ' +
    'thirteenth fourteenth fifteenth sixteenth seventeenth eighteenth nineteenth'
  ).split(' '),
  ...Object.keys(TENS).map((t) => `${t.slice(0, -1)}ieth`),
  'hundredth',
  ...Object.keys(SCALES).map((t) => `${t}th`),
]);

/** Currency words. A number after one stays in words: "a dollar fifty". */
const CURRENCY: ReadonlySet<string> = new Set(
  'dollar dollars buck bucks euro euros pound pounds quid yen rupee rupees franc francs'.split(' '),
);

/** A private-use mark for a filler between two number words, so the run does not join. */
const NUMBER_BREAK = '\uE000';

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

/** Words that join two numbers into a range or a list ("three to four hundred"). */
const CONNECTORS: ReadonlySet<string> = new Set(['to', 'or', 'and', 'through', 'thru', 'by', 'x']);

/** A digit token or a number word. */
const isNumberish = (w: string) =>
  /\d/.test(w) ||
  ORDINAL_WORDS.has(w) ||
  (w !== '' && w !== 'and' && w.split('-').every((p) => isNumberWord(p) || ORDINAL_WORDS.has(p)));

/**
 * Whether a run stands next to another number: a digit ("40 thousand"), a number in a comma
 * list ("twenty, thirty"), an ordinal ("twenty first"), the word "point" on either side ("two
 * point five million"), a currency word before it ("a dollar fifty"), a removed filler between
 * two number words, or a connector and a number ("three to four hundred"). Such a run stays in
 * words, because its value depends on its neighbours.
 */
function inNumberContext(prev: string, prev2: string, next: string, next2: string): boolean {
  if (isNumberish(prev) || isNumberish(next)) return true;
  if (prev === 'point' || next === 'point') return true;
  if (prev === NUMBER_BREAK || next === NUMBER_BREAK) return true;
  if (CURRENCY.has(prev)) return true;
  if (CONNECTORS.has(prev) && isNumberish(prev2)) return true;
  if (CONNECTORS.has(next) && isNumberish(next2)) return true;
  return false;
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
 * more. A lone "one" to "nine", a digit read-out ("one two three"), and a run next to another
 * number stay in words (`inNumberContext`: a digit, "point" on either side, or a range or list
 * such as "three to four hundred"). The rule leans to keep words (ADR 0070).
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
    const word = (t: string | undefined) => (t ?? '').toLowerCase().replace(/[,.!?;:]+$/, '');
    // The two words before the run (`out` holds words and spaces), and the two after it.
    const prev = word(out[out.length - 2]);
    const prev2 = word(out[out.length - 4]);
    // A comma keeps the list context ("twenty, thirty"); other punctuation ends it.
    const goesOn = punct === '' || punct === ',';
    const next = goesOn ? word(tokens[end + 2]) : '';
    const next2 = goesOn ? word(tokens[end + 4]) : '';
    const value = parseCardinal(words);
    if (
      value !== undefined &&
      !inNumberContext(prev, prev2, next, next2) &&
      (words.length >= 2 || value >= 10)
    ) {
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

const UNIT_NAMES = Object.keys(UNITS);
const TENS_NAMES = Object.keys(TENS);

/** The words of 1-999 in the form that `parseCardinal` reads. */
function below1000Words(n: number): string[] {
  const words: string[] = [];
  const h = Math.floor(n / 100);
  const r = n % 100;
  if (h > 0) words.push(UNIT_NAMES[h] as string, 'hundred');
  if (r >= 20) {
    words.push(TENS_NAMES[Math.floor(r / 10) - 2] as string);
    if (r % 10 > 0) words.push(UNIT_NAMES[r % 10] as string);
  } else if (r > 0 || words.length === 0) {
    words.push(UNIT_NAMES[r] as string);
  }
  return words;
}

/** The spoken form of `n`, for the pre-pass self-check: 3400 → "three thousand four hundred". */
function spokenFormOf(n: number): string {
  const words: string[] = [];
  let rest = n;
  for (const [name, scale] of Object.entries(SCALES).sort((a, b) => b[1] - a[1])) {
    const k = Math.floor(rest / scale);
    if (k > 0) {
      words.push(...below1000Words(k), name);
      rest %= scale;
    }
  }
  if (rest > 0 || words.length === 0) words.push(...below1000Words(rest));
  return words.join(' ');
}

/** The numbers of `text` as a sorted list: digit tokens (no thousands commas) and number words. */
function numberSignature(text: string): string {
  const tokens = text.toLowerCase().match(/[\p{L}\p{N}]+(?:[.,][\p{N}]+)*/gu) ?? [];
  return tokens
    .flatMap((t) => (/\d/.test(t) ? [t.replace(/,(?=\d{3}(?:\D|$))/g, '')] : t.split('-')))
    .filter((t) => /\d/.test(t) || ORDINAL_WORDS.has(t) || (t !== 'and' && isNumberWord(t)))
    .sort()
    .join(' ');
}

// ---------- protected spans (ADR 0122) ----------

const SPAN_OPEN = '\uE001';
const SPAN_CLOSE = '\uE002';
const SPAN_DIGIT_BASE = 0xe010;

/** Code, quoted text, URLs and email addresses. The pre-pass never edits inside them. */
const PROTECTED =
  /`[^`\n]*`|"[^"\n]*"|“[^”\n]*”|\b(?:https?:\/\/|www\.)[^\s]*[^\s.,;:!?)"'”]|[\p{L}\p{N}._%+-]{1,64}@[\p{L}\p{N}-]{1,63}(?:\.[\p{L}\p{N}-]{1,63})+/giu;

/** The longest text the pre-pass edits. A longer text is no dictation: it stays as it is. */
const MAX_PREPASS_CHARS = 8_000;

/** Swaps each protected span for a mark with no letters and no digits, so no rule can match it. */
function protectSpans(text: string): { masked: string; restore: (s: string) => string } {
  const spans: string[] = [];
  const masked = text.replace(PROTECTED, (m) => {
    const code = [...String(spans.length)]
      .map((d) => String.fromCharCode(SPAN_DIGIT_BASE + Number(d)))
      .join('');
    spans.push(m);
    return `${SPAN_OPEN}${code}${SPAN_CLOSE}`;
  });
  const restore = (s: string) =>
    s.replace(/\uE001([\uE010-\uE019]+)\uE002/g, (_m, code: string) => {
      const i = Number([...code].map((c) => c.charCodeAt(0) - SPAN_DIGIT_BASE).join(''));
      return spans[i] ?? '';
    });
  return { masked, restore };
}

// ---------- discourse fillers, repeats, false starts (ADR 0122) ----------

const DISCOURSE = '(?:you know|I mean|like)';

/** Negators and yes/no words. A filler next to one of them stays: it may carry the meaning. */
const POLARITY_WORDS: ReadonlySet<string> = new Set(
  (
    'no not never none nobody nothing neither nor nope cannot yes yeah yep yup ' +
    "dont doesnt didnt wont cant isnt arent wasnt werent wouldnt couldnt shouldnt don't doesn't didn't won't can't isn't aren't wasn't weren't wouldn't couldn't shouldn't"
  ).split(' '),
);

const bareWord = (w: string | undefined) =>
  (w ?? '').toLowerCase().replace(/^[^\p{L}\p{N}']+|[^\p{L}\p{N}']+$/gu, '');

/**
 * Whether a removal between `start` and `end` of `whole` would touch a number (a digit or a
 * spoken number word) or a yes/no or negator word next to it. Then the text stays as it is.
 */
function touchesMeaning(whole: string, start: number, end: number): boolean {
  const prev = bareWord(/(\S+)\s*$/u.exec(whole.slice(0, start).replace(/[,.]\s*$/, ''))?.[1]);
  const next = bareWord(/^\s*(\S+)/u.exec(whole.slice(end))?.[1]);
  return [prev, next].some(
    (w) => w !== '' && (/\p{N}/u.test(w) || isNumberish(w) || POLARITY_WORDS.has(w)),
  );
}

/**
 * Removes "you know", "I mean" and "like" only when commas or the sentence edge set them off, as in
 * "It was, like, huge." or "You know, we ship." A bare "I like pizza" or "you know the answer" stays.
 * A mark next to a number, a yes/no word or a negator stays too: "give me five, like, six apples",
 * "I said no, I mean, yes". It loops until nothing changes.
 */
export function removeDiscourseFillers(text: string): string {
  const step = (t: string): string => {
    let out = t.replace(
      new RegExp(`,\\s*${DISCOURSE}\\s*,\\s*`, 'giu'),
      (m: string, offset: number, whole: string) =>
        touchesMeaning(whole, offset, offset + m.length) ? m : ' ',
    );
    out = out.replace(
      new RegExp(`(^|[.!?]\\s+)${DISCOURSE}\\s*,\\s*`, 'giu'),
      (m: string, lead: string, offset: number, whole: string) =>
        touchesMeaning(whole, offset + lead.length, offset + m.length) ? m : lead,
    );
    out = out.replace(
      new RegExp(`\\s*,\\s*${DISCOURSE}(?=\\s*\\.?\\s*$)`, 'giu'),
      (m: string, offset: number, whole: string) =>
        touchesMeaning(whole, offset, offset + m.length) ? m : '',
    );
    return tidy(out);
  };
  let cur = text;
  for (let i = 0; i < 5; i++) {
    const next = step(cur);
    if (next === cur) break;
    cur = next;
  }
  return cur;
}

/** Words that a speaker says twice by slip and that are never a real double word. */
const STUTTER_WORDS: ReadonlySet<string> = new Set(
  (
    'i the a an to of in on at and but or so we you he she they it my your our their this for with ' +
    'are was be if as'
  ).split(' '),
);

const PHRASE_WORD = /^(?:\p{Ll}[\p{Ll}']*|I)$/u;

/**
 * Removes an unambiguous false start: a short function word said twice ("the the", "I I"), a 2 or
 * 3 word phrase said twice in a row ("I want to I want to go"), and a cut word that the next word
 * completes ("wh- what"). A double content word ("had had", "very very") stays.
 */
export function collapseRepeats(text: string): string {
  let words = text.split(/\s+/).filter((w) => w !== '');
  const key = (w: string) => w.toLowerCase();
  // Cut word: "th- the", "wh- what".
  words = words.filter((w, i) => {
    const frag = /^(\p{L}{1,3})-$/u.exec(w)?.[1];
    const next = words[i + 1];
    return !(frag && next && key(next).startsWith(key(frag)));
  });
  // Phrase repeat, longest first.
  for (const n of [3, 2]) {
    for (let i = 0; i + 2 * n <= words.length;) {
      const a = words.slice(i, i + n);
      const b = words.slice(i + n, i + 2 * n);
      const same = a.every((w, k) => key(w) === key(b[k] as string));
      if (same && b.every((w) => PHRASE_WORD.test(w)) && a.every((w) => /^[\p{L}']+$/u.test(w))) {
        words.splice(i + n, n);
      } else i++;
    }
  }
  // Function word twice.
  const out: string[] = [];
  for (const w of words) {
    const prev = out[out.length - 1];
    if (
      prev !== undefined &&
      STUTTER_WORDS.has(key(w)) &&
      key(prev) === key(w) &&
      PHRASE_WORD.test(w)
    )
      continue;
    out.push(w);
  }
  return out.join(' ');
}

const ABBREVIATIONS: ReadonlySet<string> = new Set(
  'e.g. i.e. etc. vs. dr. mr. mrs. ms. st. no. approx. inc. ltd. jr. sr. fig.'.split(' '),
);

/**
 * Capitalises the first letter of the text and of each sentence, only for a plain lowercase word.
 * A name, a code word, an abbreviation and a word with a digit or a symbol stay as they are.
 */
export function capitalizeSentences(text: string): string {
  return text.replace(
    /(^|[.!?]\s+)(\p{Ll}[\p{Ll}']*)(?![\p{L}\p{N}_.@/\\:-])/gu,
    (m: string, lead: string, word: string, offset: number, whole: string) => {
      if (lead !== '') {
        const before =
          whole
            .slice(0, offset + 1)
            .split(/\s+/)
            .pop() ?? '';
        if (ABBREVIATIONS.has(before.toLowerCase())) return m;
      }
      return lead + word.charAt(0).toUpperCase() + word.slice(1);
    },
  );
}

/** A comma pair around a filler: "is, um, ready" becomes "is ready". A number or yes/no word keeps it. */
function collapseFillerCommas(text: string, language: string): string {
  const alt = fillersFor(language).map(escapeRegExp).join('|');
  const re = new RegExp(`,\\s*(?:${alt})\\s*,\\s*`, 'giu');
  return text.replace(re, (m: string, offset: number, whole: string) =>
    touchesMeaning(whole, offset, offset + m.length) ? m : ' ',
  );
}

/** The pronoun "i" becomes "I", also in "i'm", "i'll", "i've" and "i'd". English only. */
function capitalizePronounI(text: string): string {
  return text.replace(
    /(?<![\p{L}\p{N}_.'’/-])i(?=(?:['’](?:m|ll|ve|d))?(?![\p{L}\p{N}_.)'’/-]))/gu,
    'I',
  );
}

/** A comma left before the end of a sentence or next to another comma goes. */
function dropDanglingCommas(text: string): string {
  return text
    .replace(/,(?:\s*,)+/g, ',')
    .replace(/,\s*([.!?])/g, '$1')
    .replace(/,\s*$/, '');
}

/**
 * The pre-pass: fillers, stutters, false starts, the user dictionary, numerals, then punctuation
 * spacing and sentence capitals (CAP-66a, ADR 0122). Code, quoted text, URLs and email addresses
 * pass through unchanged. It checks its own work (CAP-68): fillers, stutters and the dictionary
 * must not change a number, and the numerals step must only swap each run for its value. When a
 * check fails, the pre-pass gives the raw text, so a pre-pass error never reaches the card, the
 * router step or the model.
 */
export function prepass(raw: string, opts: PrepassOptions = {}): string {
  const language = opts.language ?? 'en';
  // A huge text is no dictation. A text that holds a mark character of ours cannot be masked.
  if (raw.length > MAX_PREPASS_CHARS || /[-]/.test(raw)) return raw.trim();
  const { masked, restore } = protectSpans(raw);
  const english = language.toLowerCase().startsWith('en');
  // The two filler passes feed each other ("like, um,"), so loop until nothing changes.
  let noDiscourse = markNumberBreaks(masked, language);
  for (let i = 0; i < 3; i++) {
    const marked = english ? removeDiscourseFillers(noDiscourse) : noDiscourse;
    const next = removeFillers(collapseFillerCommas(marked, language), language);
    if (next === noDiscourse) break;
    noDiscourse = next;
  }
  const noStutters = collapseRepeats(collapseStutters(noDiscourse));
  const dictionary = applyDictionary(noStutters, opts.dictionary ?? []);
  if (numberSignature(dictionary) !== numberSignature(masked)) return raw.trim();
  const out = tidy(spokenNumeralsToDigits(dictionary));
  // Write each new digit token back in words: the words must be the ones it replaced.
  const before = new Set(dictionary.match(/\d+/g) ?? []);
  const restored = out.replace(/\b\d+\b/g, (d) => (before.has(d) ? d : spokenFormOf(Number(d))));
  if (numberSignature(restored) !== numberSignature(dictionary)) return raw.trim();
  const spaced = dropDanglingCommas(tidy(out.split(NUMBER_BREAK).join(' ')));
  return restore(capitalizeSentences(english ? capitalizePronounI(spaced) : spaced));
}
