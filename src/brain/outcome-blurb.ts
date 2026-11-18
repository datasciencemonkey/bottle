/**
 * The card render of each terminal AskEvent (#33, ADR 0084) with the Appendix E templates:
 * Completed, Partial answer (G-23a), Failed with its code and the failure action map (G-23b),
 * Clarify (G-22), Gave up (10 min), Ask uncertain and Cancelled.
 *
 * It reads only the normalised event, never the backend kind (G-21a). No line holds a banned
 * word (UX-16a): every fixed text is a Bottle text, and a failure shows only the GL-5 user text.
 * TODO(#86): the 120-char line cuts, the bold key numbers, the scope line and the
 * "<profile> · <backend label> · Open in Databricks" footer (UX-13..UX-15).
 * TODO(#210): the safe markdown render of the answer.
 */
import type { BlurbInput } from './blurb-stack';
import type { TerminalAskEvent } from './components/ask-events';
import { failureActions, userTextFor, type FailureAction } from './components/genie-errors';

/** The card controls of an outcome: the failure actions plus the clarify controls. */
export type OutcomeControl = FailureAction | 'answer_by_voice' | 'type';

export interface OutcomeBlurb {
  state: 'completed' | 'partial' | 'clarify' | 'failed' | 'gave_up' | 'uncertain' | 'cancelled';
  /** Line 1, then line 2 if the template has one. */
  lines: string[];
  controls: readonly OutcomeControl[];
  /** Appendix E chip 16 labels ("Answer: <suggestion>"); they act only on a click (UX-20c). */
  suggestions: string[];
}

/** What the caller knows beyond the event. */
export interface OutcomeOpts {
  /** The GL-5 row of a failure, if the caller has it (`Gl5Result.row`). */
  row?: string;
}

function failureControls(
  code: Extract<TerminalAskEvent, { t: 'failed' }>['code'],
  opts: OutcomeOpts,
): readonly OutcomeControl[] {
  // R8 already ran the countdown and its tries (ADR 0075): this failure is final, so the user
  // retries. The brain starts no second countdown.
  if (code === 'RATE_LIMITED') return ['retry'];
  // ARC-F1: a `poll_inflight` busy means the question is already in Genie and a poll is open.
  // [New conversation] would ask it again, so only [Wait] shows.
  if (code === 'CONVERSATION_BUSY' && opts.row === 'transport:poll_inflight') return ['wait'];
  return failureActions(code);
}

/** The Appendix E blurb of one terminal event. */
export function outcomeBlurb(e: TerminalAskEvent, opts: OutcomeOpts = {}): OutcomeBlurb {
  const blurb = (
    state: OutcomeBlurb['state'],
    lines: string[],
    controls: readonly OutcomeControl[] = [],
    suggestions: string[] = [],
  ): OutcomeBlurb => ({ state, lines, controls, suggestions });
  switch (e.t) {
    case 'answer':
      return e.incomplete
        ? // G-23a: "Partial answer: <answer>" | "Genie did not finish all steps." | [Ask again]
          blurb(
            'partial',
            [`Partial answer: ${e.md}`, userTextFor('GENIE_INCOMPLETE')],
            ['ask_again'],
          )
        : blurb('completed', [e.md], ['open_in_databricks']);
    case 'needs_input':
      // G-22: "Genie asks: <question>" | suggested answers as chips | [Answer by voice] [Type]
      return blurb(
        'clarify',
        [`Genie asks: ${e.prompt}`],
        ['answer_by_voice', 'type'],
        e.options.map((o) => `Answer: ${o}`),
      );
    case 'failed':
      // G-23b and every failure: "<user_text>" | "Code <code>" | <action>
      return blurb('failed', [e.userText, `Code ${e.code}`], failureControls(e.code, opts));
    case 'abandoned':
      return blurb('gave_up', [userTextFor('TIMEOUT_GAVE_UP')], failureActions('TIMEOUT_GAVE_UP'));
    case 'uncertain':
      return blurb('uncertain', [userTextFor('ASK_UNCERTAIN')], failureActions('ASK_UNCERTAIN'));
    case 'cancel_requested':
      return blurb('cancelled', ['Cancellation requested']);
  }
}

/** The card blurb of a terminal event: its lines as the text, marked completed. */
export function outcomeBlurbInput(
  id: string,
  e: TerminalAskEvent,
  opts: OutcomeOpts = {},
): BlurbInput {
  return { id, text: outcomeBlurb(e, opts).lines.join('\n'), completed: true };
}
