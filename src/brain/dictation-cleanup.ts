/**
 * Dictation cleanup around the router step (ADR 0122; CAP-66a, CAP-67a, CAP-67b).
 *
 * `withDictationCleanup` wraps the route deps so that every pasted text is the pre-pass text or a
 * grounded model rewrite of it:
 * - The router input is the pre-pass text (`cleanup-prepass.ts`).
 * - The router call has a 1500 ms deadline. When it passes, the pre-pass text is used at once, with
 *   no retry. A late reply changes nothing.
 * - The model `cleaned_text` must be grounded in the pre-pass text: no number, name or entity that
 *   the input does not hold, and no number lost. Else the pre-pass text is used.
 * - With `cleanDictation()` false the raw `text.trim()` is used, as before ADR 0122. The pre-pass
 *   and the model `cleaned_text` are both skipped.
 *
 * The decision keeps the model intent, so a ⌥G ask still sends `cleaned_text` (RTR-17a, D69). It
 * never logs text.
 */
import type { MogId, RouterDecision } from '../shared/interfaces';
import { prepass } from './components/cleanup-prepass';
import { applyInjectionGuard } from './components/router-injection';
import { groundedIn, numbersOf } from './components/router-grounding';
import type { RouterInput } from './components/router-step';
import { offlineDecision } from './offline-intent';
import type { UtteranceRouteDeps } from './utterance-route';

/** The deadline of a dictation router call (ADR 0122: the live probe shows 1.4 to 2.4 s per call; CAP-67a said 900 ms). */
export const DICTATION_CLEANUP_DEADLINE_MS = 1500;

export interface DictationCleanupConfig {
  /**
   * The "Clean up dictation" setting (default true). TODO(integration): the contract lane adds
   * `cleanDictation`; the integration step passes a reader of the real setting here.
   */
  cleanDictation?: () => boolean;
  /** A lower deadline for tests. It is never above `DICTATION_CLEANUP_DEADLINE_MS`. */
  deadlineMs?: number;
}

/** Whether the model text only holds what the pre-pass text holds (no new fact, no lost number). */
export function modelTextGrounded(modelText: string, pre: string): boolean {
  if (modelText.trim() === '') return false;
  if (!groundedIn(modelText, pre)) return false;
  const kept = new Set(numbersOf(modelText));
  return numbersOf(pre).every((n) => kept.has(n));
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
  return {
    ...deps,
    async route(text: string, mogId: MogId, input: RouterInput): Promise<RouterDecision> {
      if (!cleanOn()) {
        const decision = await deps.route(text, mogId, input);
        return { ...decision, cleaned_text: text.trim() };
      }
      const pre = safePrepass(text);
      let timer: ReturnType<typeof setTimeout> | undefined;
      const deadline = new Promise<Raced>((resolve) => {
        timer = setTimeout(() => resolve({ kind: 'deadline' }), deadlineMs);
      });
      const call = deps
        .route(pre, mogId, input)
        .then((decision): Raced => ({ kind: 'decision', decision }));
      let raced: Raced;
      try {
        raced = await Promise.race([call, deadline]);
      } finally {
        clearTimeout(timer);
      }
      // No retry: a late reply is dropped.
      if (raced.kind === 'deadline') {
        call.catch(() => undefined);
        return applyInjectionGuard(offlineDecision(pre), pre).decision;
      }
      const d = raced.decision;
      if (modelTextGrounded(d.cleaned_text, pre)) return d;
      // The model text is not grounded: the pre-pass text goes on. The question is checked again.
      const genie =
        d.genie_question !== '' && !groundedIn(d.genie_question, pre) ? pre : d.genie_question;
      return { ...d, cleaned_text: pre, genie_question: genie };
    },
  };
}
