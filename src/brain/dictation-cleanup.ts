/**
 * Dictation cleanup around the router step (ADR 0122; CAP-66a, CAP-67a, CAP-67b, RTR-17a).
 *
 * `withDictationCleanup` wraps the route deps so that every pasted text is the pre-pass text or a
 * checked model rewrite of it:
 * - The router input is the pre-pass text (`cleanup-prepass.ts`).
 * - Only a dictation call has the deadline. When it passes, the pre-pass text is used at once,
 *   with no retry. A late reply changes nothing. A ⌥G ask, a session capture and a command keep
 *   the plain router step.
 * - The model `cleaned_text` is pasted only if it passes `acceptModelText`: it keeps the words of
 *   the pre-pass text in order, adds none, drops at most 30 percent, keeps every negator, yes/no
 *   word, question mark, number and entity, and holds no new URL, email or injection pattern.
 *   Else the pre-pass text is pasted.
 * - A ⌥G ask sends the pre-pass text, never the model text (`askText`, RTR-17a, ADR 0122).
 * - With `cleanDictation()` false the raw `text.trim()` is used, as before ADR 0122. The pre-pass
 *   and the model `cleaned_text` are both skipped.
 *
 * It never logs text.
 */
import type { MogId, RouterDecision } from '../shared/interfaces';
import { POLARITY_WORDS, prepass, protectedSpansOf } from './components/cleanup-prepass';
import { applyInjectionGuard, detectInjection } from './components/router-injection';
import { groundedIn, numbersOf } from './components/router-grounding';
import type { RouterInput } from './components/router-step';
import { offlineDecision } from './offline-intent';
import type { RouteOptions, UtteranceRouteDeps } from './utterance-route';

/**
 * The deadline of a dictation router call. A live probe (ADR 0122) shows 1.4 to 2.4 s for Haiku
 * 5.5 and 1.3 to 1.7 s for gpt-oss-20b, so 900 ms (CAP-67a) would fall back almost every time.
 */
export const DICTATION_CLEANUP_DEADLINE_MS = 1500;

/** A timer that tests can drive by hand. `start` returns the cancel function. */
export interface DeadlineTimer {
  start(ms: number, fn: () => void): () => void;
}

const systemTimer: DeadlineTimer = {
  start(ms, fn) {
    const t = setTimeout(fn, ms);
    return () => clearTimeout(t);
  },
};

export interface DictationCleanupConfig {
  /**
   * The "Clean up dictation" setting (default true). TODO(integration): the contract lane adds
   * `cleanDictation`; the integration step passes a reader of the real setting here.
   */
  cleanDictation?: () => boolean;
  /** A lower deadline for tests. It is never above `DICTATION_CLEANUP_DEADLINE_MS`. */
  deadlineMs?: number;
  /** The timer of the deadline. Default: `setTimeout`. */
  timer?: DeadlineTimer;
}

// ---------- the check of the model text (H1, H2) ----------

/** Contraction forms the model may write for words of the input ("do not" to "don't"). */
const CONTRACTIONS: ReadonlySet<string> = new Set(
  (
    'im ive ill id its thats theres whats lets dont doesnt didnt wont cant isnt arent wasnt werent ' +
    'wouldnt couldnt shouldnt youre were theyre weve youve theyve youll well theyll hes shes'
  ).split(' '),
);

/** Negators. A contraction such as "dont" counts as one. */
const NEGATORS: ReadonlySet<string> = new Set(
  [...POLARITY_WORDS]
    .filter((w) => !['yes', 'yeah', 'yep', 'yup'].includes(w))
    .map(stripApostrophes),
);
const YES_WORDS: ReadonlySet<string> = new Set(['yes', 'yeah', 'yep', 'yup']);

function stripApostrophes(w: string): string {
  return w.replace(/['’]/g, '');
}

interface Token {
  word: string;
  /** A contraction with an apostrophe: the model may write it for 2 words of the input. */
  free: boolean;
}

function tokens(s: string): Token[] {
  return (s.toLowerCase().match(/[\p{L}\p{N}'’]+/gu) ?? []).map((raw) => {
    const word = stripApostrophes(raw);
    return { word, free: word !== raw && CONTRACTIONS.has(word) };
  });
}

const count = (ts: Token[], set: ReadonlySet<string>) => ts.filter((t) => set.has(t.word)).length;

/**
 * Whether the model text may be pasted for the pre-pass text `pre`. The model words must be an
 * in-order subset of the input words, with free contraction forms. This rules out an added word, a
 * reorder, an answer for a question and a summary.
 */
export function modelTextGrounded(modelText: string, pre: string): boolean {
  if (modelText.trim() === '') return false;
  const m = tokens(modelText);
  const p = tokens(pre);
  // Order and no new word.
  let j = 0;
  let matched = 0;
  let free = 0;
  for (const t of m) {
    if (t.free) {
      free++;
      continue;
    }
    while (j < p.length && (p[j] as Token).word !== t.word) j++;
    if (j >= p.length) return false;
    j++;
    matched++;
  }
  // At most 30 percent of the input words may go (a contraction covers 2 input words).
  if (p.length - matched - 2 * free > Math.floor(p.length * 0.3)) return false;
  // Length: at most 10 percent more, with 2 characters for a final mark.
  if (modelText.trim().length > Math.ceil(pre.trim().length * 1.1) + 2) return false;
  // Meaning words: negators, yes words, question marks.
  if (count(m, NEGATORS) !== count(p, NEGATORS)) return false;
  if (count(m, YES_WORDS) !== count(p, YES_WORDS)) return false;
  if (pre.includes('?') && !modelText.includes('?')) return false;
  // Numbers and entities.
  if (!groundedIn(modelText, pre)) return false;
  const kept = new Set(numbersOf(modelText));
  return numbersOf(pre).every((n) => kept.has(n));
}

/** Whether the model text holds an injection pattern or a URL, email or code span that `pre` lacks. */
function modelTextHostile(modelText: string, pre: string): boolean {
  if (detectInjection(modelText).length > 0) return true;
  const known = new Set(protectedSpansOf(pre));
  return protectedSpansOf(modelText).some((s) => !known.has(s));
}

function safePrepass(text: string): string {
  try {
    return prepass(text);
  } catch {
    return text.trim();
  }
}

type Raced = { kind: 'decision'; decision: RouterDecision } | { kind: 'deadline' };

export function withDictationCleanup(
  deps: UtteranceRouteDeps,
  cfg: DictationCleanupConfig = {},
): UtteranceRouteDeps {
  const deadlineMs = Math.min(
    cfg.deadlineMs ?? DICTATION_CLEANUP_DEADLINE_MS,
    DICTATION_CLEANUP_DEADLINE_MS,
  );
  const cleanOn = cfg.cleanDictation ?? (() => true);
  const timer = cfg.timer ?? systemTimer;
  return {
    ...deps,
    askText: (text: string) => (cleanOn() ? safePrepass(text) : text.trim()),
    async route(
      text: string,
      mogId: MogId,
      input: RouterInput,
      opts?: RouteOptions,
    ): Promise<RouterDecision> {
      // An ask, a session capture and a command keep the plain router step.
      if (opts?.dictation === false) return deps.route(text, mogId, input, opts);
      if (!cleanOn()) {
        const decision = await deps.route(text, mogId, input, opts);
        return { ...decision, cleaned_text: text.trim() };
      }
      const pre = safePrepass(text);
      let cancel: () => void = () => undefined;
      const deadline = new Promise<Raced>((resolve) => {
        cancel = timer.start(deadlineMs, () => resolve({ kind: 'deadline' }));
      });
      const call = deps
        .route(pre, mogId, input, opts)
        .then((decision): Raced => ({ kind: 'decision', decision }));
      let raced: Raced;
      try {
        raced = await Promise.race([call, deadline]);
      } finally {
        cancel();
      }
      // No retry: a late reply is dropped.
      if (raced.kind === 'deadline') {
        call.catch(() => undefined);
        return applyInjectionGuard(offlineDecision(pre), pre).decision;
      }
      const d = raced.decision;
      if (modelTextHostile(d.cleaned_text, pre)) {
        // H2: the guard flags the decision and the pre-pass text goes on.
        return applyInjectionGuard(
          { ...d, cleaned_text: pre, injection_suspected: true },
          `${d.cleaned_text}\n${pre}`,
        ).decision;
      }
      if (modelTextGrounded(d.cleaned_text, pre)) return d;
      // The model text fails the check: the pre-pass text goes on. The question is checked again.
      const genie =
        d.genie_question !== '' && !groundedIn(d.genie_question, pre) ? pre : d.genie_question;
      return { ...d, cleaned_text: pre, genie_question: genie };
    },
  };
}
