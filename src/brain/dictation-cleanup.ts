/**
 * Dictation cleanup around the router step (ADR 0122; CAP-66a, CAP-67a, CAP-67b, RTR-17a).
 *
 * `withDictationCleanup` wraps the route deps so that every pasted text is the pre-pass text or a
 * model rewrite that passes one exact rule (`modelTextGrounded`):
 * - The router input is the pre-pass text (`cleanup-prepass.ts`).
 * - Only a dictation call has the deadline. When it passes, the pre-pass text is used at once,
 *   with no retry. A late reply changes nothing. A ⌥G ask and a session capture make no router
 *   model call: the local heuristic and the injection guard decide (M1, RTR-14).
 * - The model `cleaned_text` is pasted only if its word sequence equals the pre-pass word sequence
 *   (see `modelTextGrounded`). Any other word added, dropped, reordered or changed gives the
 *   pre-pass text. A model text with an injection pattern also gives the pre-pass text.
 * - A ⌥G ask sends the pre-pass text, never the model text (`askText`, RTR-17a, ADR 0122).
 * - With `cleanDictation()` false the raw `text.trim()` is used and NO model call is made (H2).
 *   The pre-pass and the model `cleaned_text` are both skipped.
 *
 * It never logs text.
 */
import type { MogId, RouterDecision } from '../shared/interfaces';
import { prepass } from './components/cleanup-prepass';
import { applyInjectionGuard, detectInjection } from './components/router-injection';
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
  /** The "Clean up dictation" setting (default true). Off: no model call at all (H2). */
  cleanDictation?: () => boolean;
  /** A lower deadline for tests. It is never above `DICTATION_CLEANUP_DEADLINE_MS`. */
  deadlineMs?: number;
  /** The timer of the deadline. Default: `setTimeout`. */
  timer?: DeadlineTimer;
}

// ---------- the check of the model text ----------

/** Words that a filler or a false start leaves out. The model may delete them and no others. */
const DELETABLE: ReadonlySet<string> = new Set(
  'um uh er ah hmm like so well okay right actually basically'.split(' '),
);

/** The words of `s`: lower case, no punctuation, no quote marks. A contraction is one word. */
function wordsOf(s: string): string[] {
  return (
    s
      .toLowerCase()
      .replace(/’/g, "'")
      .match(/[\p{L}\p{N}']+/gu) ?? []
  )
    .map((w) => w.replace(/^'+|'+$/g, ''))
    .filter((w) => w !== '');
}

/** The words after the allowed deletions: fillers, "you know", "I mean", adjacent exact repeats. */
function normalWords(s: string): string[] {
  const out: string[] = [];
  const ws = wordsOf(s);
  for (let i = 0; i < ws.length; i++) {
    const w = ws[i] as string;
    const pair = `${w} ${ws[i + 1] ?? ''}`;
    if (pair === 'you know' || pair === 'i mean') {
      i++;
      continue;
    }
    if (!DELETABLE.has(w)) out.push(w);
  }
  // Collapse an exact adjacent repeat of 1 to 3 words, until nothing changes.
  for (let again = true; again;) {
    again = false;
    for (let n = 1; n <= 3 && !again; n++) {
      for (let i = 0; i + 2 * n <= out.length; i++) {
        if (out.slice(i, i + n).join(' ') === out.slice(i + n, i + 2 * n).join(' ')) {
          out.splice(i + n, n);
          again = true;
          break;
        }
      }
    }
  }
  return out;
}

/** For each "?", the number of words before it. A moved or dropped "?" changes this list. */
function questionMarks(s: string): number[] {
  const at: number[] = [];
  for (let i = s.indexOf('?'); i >= 0; i = s.indexOf('?', i + 1))
    at.push(normalWords(s.slice(0, i)).length);
  return at;
}

/**
 * The one rule for the model text: its word sequence equals the pre-pass word sequence, after
 * lower case, no punctuation and quote marks, the deletion of `DELETABLE` words, "you know" and
 * "I mean", and the collapse of adjacent exact repeats. The model may change only punctuation,
 * capitals and spacing, and delete those fillers and repeats. A contraction ("do not" and
 * "don't"), a dropped qualifier, a reorder or a new word is a change. Each `?` must stand after the same word.
 */
export function modelTextGrounded(modelText: string, pre: string): boolean {
  if (modelText.trim() === '') return false;
  if (questionMarks(modelText).join() !== questionMarks(pre).join()) return false;
  const a = normalWords(modelText);
  const b = normalWords(pre);
  return a.length === b.length && a.every((w, i) => w === b[i]);
}

/** Defence in depth: an injection pattern in the model text. */
function modelTextHostile(modelText: string): boolean {
  return detectInjection(modelText).length > 0;
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
      opts: RouteOptions,
    ): Promise<RouterDecision> {
      // H2: with the switch off, no model call. The local heuristic and the injection guard decide.
      if (!cleanOn()) {
        return applyInjectionGuard(offlineDecision(text.trim()), text).decision;
      }
      const pre = safePrepass(text);
      // M1: a ⌥G ask and a session capture never wait for the router model. The ask sends the
      // pre-pass text anyway. The local injection guard still gives the RTR-14 warning.
      if (!opts.dictation) return applyInjectionGuard(offlineDecision(pre), text).decision;
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
      if (modelTextHostile(d.cleaned_text)) {
        // H2: the guard flags the decision and the pre-pass text goes on.
        return applyInjectionGuard(
          { ...d, cleaned_text: pre, injection_suspected: true },
          `${d.cleaned_text}\n${pre}`,
        ).decision;
      }
      if (modelTextGrounded(d.cleaned_text, pre)) return d;
      // The model text fails the rule: the pre-pass text goes on, also as the question.
      return { ...d, cleaned_text: pre, genie_question: d.genie_question === '' ? '' : pre };
    },
  };
}
