/**
 * The owner's "attach Genie's answer to the clipboard" request.
 *
 * Once an agent turn finishes (the ⌥G card and a Conversations thread reply draw the same
 * `AgentTurnView`, so one hook covers both), copy the exact text the card shows to the OS
 * clipboard with `clipboard_write_text`, then report a quiet "Copied to clipboard" / "Couldn't
 * copy" note back onto the turn.
 *
 * - Fires once per turn (`turn.state === 'done'`): a `Set` of handled turn IDs guards it, so a
 *   later `showTurn` of the same finished turn (for example a fresh card render) never copies
 *   twice.
 * - Never fires for a withheld (quarantined), errored, stopped or partial turn:
 *   `finishedAnswerText` (the owner's answer rule, `src/shared/agent-answer-rule.ts`) answers
 *   `undefined` for all of those, and for any turn that is not yet `done`.
 * - Respects "Copy Genie answers to the clipboard" (Settings → Workspace), read once at the
 *   moment the turn finishes: no debounce beyond the one wait below.
 * - Waits for `whenClear` (the same arrival gate `createAgentCard` already uses) before writing,
 *   so a clipboard write of a finished answer never races the clipboard snapshot and restore of
 *   an in-flight dictation paste (ADR 0048): a simple "queue until the insertion is done".
 */
import { finishedAnswerText } from '../../shared/agent-answer-rule';
import type { AgentTurnView } from '../../shared/ipc';

/** The outcome of one `clipboard_write_text` call, as the card or thread reply shows it. */
export type CopyOutcome = 'copied' | 'failed';

export interface AnswerClipboardDeps {
  /** "Copy Genie answers to the clipboard", read at the moment a turn finishes. */
  enabled(): boolean;
  /** `clipboard_write_text`. */
  write(text: string): Promise<'written' | 'too_large' | 'unsupported'>;
  /** Defer `fn` until no dictation insertion is in flight. */
  whenClear(fn: () => void): void;
  /** Show the note on the turn of `mogId`: the same `turn`, with the outcome attached. */
  note(mogId: string, turn: AgentTurnView, outcome: CopyOutcome): void;
  /** The write (or the note callback) threw. A fixed code only: never the answer text. */
  onFault?(): void;
}

export interface AnswerClipboard {
  /** Call on every `showTurn`. A no-op unless this is the turn's one clean finish. */
  onTurn(mogId: string, turn: AgentTurnView): void;
}

/** Turn IDs already handled are capped, so a very long session never grows this without bound. */
const MAX_HANDLED = 256;

export function createAnswerClipboard(deps: AnswerClipboardDeps): AnswerClipboard {
  const handled = new Set<string>();
  return {
    onTurn(mogId, turn) {
      if (turn.state !== 'done' || handled.has(turn.turnId)) return;
      const text = finishedAnswerText(turn);
      if (text === undefined) return;
      handled.add(turn.turnId);
      if (handled.size > MAX_HANDLED) handled.delete(handled.values().next().value as string);
      if (!deps.enabled()) return;
      deps.whenClear(() => {
        deps.write(text).then(
          (outcome) => deps.note(mogId, turn, outcome === 'written' ? 'copied' : 'failed'),
          () => {
            deps.onFault?.();
            deps.note(mogId, turn, 'failed');
          },
        );
      });
    },
  };
}
