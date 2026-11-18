/**
 * Cleanup and the router step in parallel on the pre-pass text (#135; RTR-09a, RTR-09b, D69,
 * F1 step 10, ADR 0065).
 *
 * `runCleanupAndRouter` takes the raw STT text, runs the pre-pass once (CAP-66a), then starts
 * `CleanupProvider.clean` on the raw text and `runRouterStep` on the pre-pass text in one tick,
 * so both calls start at the same time. Cleanup gets the raw text so that its CAP-68 fallback is
 * the real raw text (#55, ADR 0070). It waits for both, then merges them: the cleaned text is the
 * Genie question. The router `genie_question` only resolves a follow-up, and only when its
 * numbers and entities are in the cleaned text.
 *
 * Binding rules: this step only proposes. It never calls Genie, the agent, capture or a
 * workspace switch. A `dictation` decision gets no Genie question, so UC1 ends at insertion
 * (RTR-01c). The #117 guard runs again on the merged decision with the cleaned text. The trace
 * holds steps, phases and times only, never text.
 */
import type { MogId, RouterDecision } from '../../shared/interfaces';
import type { MonotonicClock } from '../clock';
import type { CleanupProvider, CleanupResult } from './cleanup-provider';
import { groundedIn } from './router-grounding';
import { applyInjectionGuard } from './router-injection';
import { runRouterStep, type RouterInput, type RouterStepDeps } from './router-step';

/**
 * The deadline of step 5 of the 8.1 budget (cleanup ∥ router step ∥ recall start).
 * TODO(#54): SP-07 measures the desktop p95 against this deadline.
 */
export const CLEANUP_ROUTER_DEADLINE_MS = 900;

/** One trace event. It holds no utterance, cleaned text or decision text. */
export interface ParallelTraceEvent {
  step: 'cleanup' | 'router';
  phase: 'start' | 'end';
  /** The time on the caller's monotonic clock (ms). */
  at: number;
}

export interface ParallelDeps {
  cleanup: CleanupProvider;
  /** The regional cleanup model (`cleanupModelFor`, D58). */
  cleanupModel?: string;
  /** The router step deps. `deadlineMs` defaults to `CLEANUP_ROUTER_DEADLINE_MS`. */
  router: RouterStepDeps;
  clock: MonotonicClock;
  trace?: (e: ParallelTraceEvent) => void;
}

export interface ParallelResult {
  /** The cleanup output, the raw text (CAP-68) or the pre-pass text (CAP-67b). */
  cleanedText: string;
  /** True when the raw text is kept: a cleanup reply failed a CAP-CHECK check (CAP-68). */
  usedRaw: boolean;
  /** True when the pre-pass text is kept with the mark "not cleaned" (CAP-67b, D69). */
  notCleaned: boolean;
  /** The router decision with `cleaned_text` and `genie_question` from the merge. */
  decision: RouterDecision;
  /** The Genie question. Empty for an intent that does not ask (dictation, context, ...). */
  genieQuestion: string;
}

export { entitiesOf, groundedIn, numbersOf } from './router-grounding';

const ASKS: ReadonlySet<string> = new Set(['question', 'memory_question']);

/**
 * The Genie question for a decision (RTR-09b): the cleaned text. The router `genie_question`
 * replaces it only for a follow-up whose numbers and entities are in the cleaned text.
 */
export function resolveGenieQuestion(cleanedText: string, d: RouterDecision): string {
  if (!ASKS.has(d.intent)) return '';
  const rewrite = d.genie_question.trim();
  if (d.follow_up && rewrite !== '' && groundedIn(rewrite, cleanedText)) return rewrite;
  return cleanedText;
}

/**
 * Runs the pre-pass on `raw`, then cleanup and the router step in parallel (RTR-09a), and merges
 * the results (RTR-09b). It never throws for a cleanup or router failure: cleanup falls back to
 * the pre-pass text, and the router step falls back to intent `context`.
 */
export async function runCleanupAndRouter(
  raw: string,
  mogId: MogId,
  input: RouterInput,
  deps: ParallelDeps,
): Promise<ParallelResult> {
  const trace = deps.trace ?? (() => undefined);
  const mark = (step: ParallelTraceEvent['step'], phase: ParallelTraceEvent['phase']) =>
    trace({ step, phase, at: deps.clock.now() });

  // F1 step 9: the pre-pass. It is deterministic, so cleanup's own pre-pass gives the same text.
  let prepassText: string;
  try {
    prepassText = deps.cleanup.prepass(raw);
  } catch {
    prepassText = raw;
  }
  const notCleaned = (): CleanupResult => ({
    text: prepassText,
    usedRaw: false,
    notCleaned: true,
    outcome: 'model_failed',
  });

  // One fan-out: neither call waits for the other (F1 step 10).
  mark('cleanup', 'start');
  let cleanCall: Promise<CleanupResult>;
  try {
    // Called now, not in a later microtask, so the router call starts in the same tick.
    cleanCall = deps.cleanup.clean(
      raw,
      { app: 'bottle', mog: mogId, op: 'cleanup' },
      {
        profile: input.profile,
        ...(deps.cleanupModel === undefined ? {} : { model: deps.cleanupModel }),
      },
    );
  } catch (e) {
    cleanCall = Promise.reject(e);
  }
  const cleaning = Promise.resolve(cleanCall)
    .catch(notCleaned)
    .finally(() => mark('cleanup', 'end'));
  mark('router', 'start');
  const routing = runRouterStep(prepassText, mogId, input, {
    ...deps.router,
    deadlineMs: deps.router.deadlineMs ?? CLEANUP_ROUTER_DEADLINE_MS,
  }).finally(() => mark('router', 'end'));

  const [answer, routed] = await Promise.all([cleaning, routing]);
  const cleaned = answer.text.trim() === '' ? notCleaned() : answer;
  const cleanedText = cleaned.text;
  const question = resolveGenieQuestion(cleanedText, routed);
  const merged: RouterDecision = { ...routed, cleaned_text: cleanedText, genie_question: question };
  // The #117 guard on the text that goes on: the cleaned text and the question (RTR-14).
  const guarded = applyInjectionGuard(merged, `${cleanedText}\n${question}`).decision;
  return {
    cleanedText,
    usedRaw: cleaned.usedRaw,
    notCleaned: cleaned.notCleaned,
    decision: guarded,
    genieQuestion: guarded.genie_question,
  };
}
