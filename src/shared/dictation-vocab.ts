/**
 * Dictation words: a deterministic post-correction pass over the text of a local transcript.
 *
 * Why: Parakeet through sherpa-onnx spells some Databricks terms wrong ("Gateway API" came out
 * as "G Way N API"). sherpa-onnx v2.0.1 has hotword biasing in its config, but the vendored
 * NeMo transducer path was not measured, so this pass runs after decoding on every transcript.
 *
 * Rules:
 * - A rule is "heard" to "write". Matching is case-insensitive and needs word boundaries on
 *   both sides. Text outside a match is never touched.
 * - The user list (Settings, "Dictation words") wins over the built-in list.
 * - A built-in term that starts lowercase (such as "lakehouse") takes a capital letter when
 *   the heard words started with one.
 */

/** The localStorage key of the user list (hub Settings). */
export const DICTATION_WORDS_STORAGE_KEY = 'bottle.dictation.words';

export interface VocabRule {
  /** The words the engine wrote. */
  heard: string;
  /** The text to write instead. */
  write: string;
}

const MAX_RULES = 200;
const MAX_FIELD = 80;

/** Known mis-hearings of Databricks terms. Each "heard" is specific enough to be safe. */
export const BUILT_IN_RULES: readonly VocabRule[] = [
  { heard: 'g way n', write: 'Gateway' },
  { heard: 'gee way n', write: 'Gateway' },
  { heard: 'gee way', write: 'Gateway' },
  { heard: 'g way', write: 'Gateway' },
  { heard: 'uni tea catalog', write: 'Unity Catalog' },
  { heard: 'uni tea catalogue', write: 'Unity Catalog' },
  { heard: 'unity catalogue', write: 'Unity Catalog' },
  { heard: 'lake base', write: 'Lakebase' },
  { heard: 'lake flow', write: 'Lakeflow' },
  { heard: 'lake house', write: 'lakehouse' },
  { heard: 'data bricks', write: 'Databricks' },
  { heard: 'data brick', write: 'Databricks' },
  { heard: 'ml flow', write: 'MLflow' },
  { heard: 'm l flow', write: 'MLflow' },
  { heard: 'ml-flow', write: 'MLflow' },
  { heard: 'py spark', write: 'PySpark' },
  { heard: 'pie spark', write: 'PySpark' },
  { heard: 'jeannie space', write: 'Genie space' },
  { heard: 'jeannie spaces', write: 'Genie spaces' },
  { heard: 'jeanie space', write: 'Genie space' },
  { heard: 'jeanie spaces', write: 'Genie spaces' },
  { heard: 'jenny space', write: 'Genie space' },
  { heard: 'genie spaces', write: 'Genie spaces' },
];

const normalize = (s: string): string => s.trim().toLowerCase().replace(/\s+/g, ' ');

const hasWordChar = (s: string): boolean => /[\p{L}\p{N}]/u.test(s);

const escapeRegex = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * The user list text: one "heard → write" per line. "->" and "=>" work too. A blank line or a
 * line that starts with "#" is skipped.
 */
export function parseUserWords(text: string): { rules: VocabRule[]; invalid: number } {
  const rules: VocabRule[] = [];
  let invalid = 0;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line === '' || line.startsWith('#')) continue;
    const m = /^(.+?)\s*(?:→|->|=>)\s*(.+)$/.exec(line);
    const heard = m?.[1]?.trim() ?? '';
    const write = m?.[2]?.trim() ?? '';
    if (
      heard === '' ||
      write === '' ||
      heard.length > MAX_FIELD ||
      write.length > MAX_FIELD ||
      !hasWordChar(heard) ||
      heard.replace(/\s+/g, ' ') === write.replace(/\s+/g, ' ') ||
      rules.length >= MAX_RULES
    ) {
      invalid += 1;
      continue;
    }
    rules.push({ heard, write });
  }
  return { rules, invalid };
}

interface Entry {
  write: string;
  builtIn: boolean;
}

/** Fix `text` with the user rules first, then the built-in rules. It never throws. */
export function correctDictation(text: string, userRules: readonly VocabRule[] = []): string {
  if (text === '') return text;
  const entries = new Map<string, Entry>();
  const add = (rules: readonly VocabRule[], builtIn: boolean): void => {
    for (const r of rules) {
      const key = normalize(r.heard);
      if (key === '' || entries.has(key)) continue;
      entries.set(key, { write: r.write, builtIn });
    }
  };
  add(userRules, false);
  add(BUILT_IN_RULES, true);
  // Longest first, so "g way n" beats "g way" in the one alternation.
  const keys = [...entries.keys()].sort((a, b) => b.length - a.length);
  const alternation = keys.map((k) => k.split(' ').map(escapeRegex).join('\\s+')).join('|');
  const re = new RegExp(`(?<![\\p{L}\\p{N}])(?:${alternation})(?![\\p{L}\\p{N}])`, 'giu');
  return text.replace(re, (match) => {
    const e = entries.get(normalize(match));
    if (e === undefined) return match;
    if (e.builtIn && /^\p{Lu}/u.test(match) && /^\p{Ll}/u.test(e.write)) {
      return e.write.charAt(0).toUpperCase() + e.write.slice(1);
    }
    return e.write;
  });
}

type ReadStore = Pick<Storage, 'getItem'>;
type WriteStore = Pick<Storage, 'setItem'>;

const defaultStore = (): Storage | undefined => {
  try {
    return typeof window !== 'undefined' ? window.localStorage : undefined;
  } catch {
    return undefined;
  }
};

/** The saved user list text ("" with none or with a store that fails). */
export function readUserWordsText(storage: ReadStore | undefined = defaultStore()): string {
  try {
    return storage?.getItem(DICTATION_WORDS_STORAGE_KEY) ?? '';
  } catch {
    return '';
  }
}

/** Save the user list text. A failed store only means it does not last. */
export function writeUserWordsText(
  text: string,
  storage: WriteStore | undefined = defaultStore(),
): void {
  try {
    storage?.setItem(DICTATION_WORDS_STORAGE_KEY, text);
  } catch {
    // A disabled store: the list just does not survive a reload.
  }
}

/** The correction the brain applies to each transcript: it reads the saved list each time. */
export function createDictationCorrector(
  read: () => string = readUserWordsText,
): (text: string) => string {
  return (text) => {
    let rules: VocabRule[] = [];
    try {
      rules = parseUserWords(read()).rules;
    } catch {
      // An unreadable list: only the built-in rules apply.
    }
    try {
      return correctDictation(text, rules);
    } catch {
      return text;
    }
  };
}
