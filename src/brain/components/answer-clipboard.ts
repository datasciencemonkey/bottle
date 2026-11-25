/**
 * The owner's "attach Genie's answer to the clipboard" request.
 *
 * Two hooks feed this, because the default path for most users never creates an agent turn:
 * - `onTurnFinished`: from `agent-run.ts`'s `deps.finished`, which fires once, only for an
 *   `answered` turn, only after its view is fully settled (`view.parts` and `view.summary` final
 *   — the late pulls, `summarise` and `syncParts` have all run). This covers the ⌥G card and a
 *   Conversations thread reply, which draw the same `AgentTurnView` (H1).
 * - `onDirectAnswered`: from `genie-direct.ts`'s `onAnswered`, which fires once for a real Genie
 *   answer on the direct path (the router's direct route, the zero-call fallback and the
 *   refused-submit fallback all share this one `send`) (H2).
 *
 * Both:
 * - copy the exact text the card shows (the owner's answer rule,
 *   `src/shared/agent-answer-rule.ts`, for a turn; the verbatim answer for a direct send, which
 *   is always one Genie call with no citation markers);
 * - fire at most once per key (a turn ID or an op ID), guarded by a capped `Set`;
 * - never fire for a withheld (quarantined), errored, stopped or partial answer: callers only
 *   call in for a clean finish, and `finishedAnswerText` itself answers `undefined` for the rest;
 * - respect "Copy Genie answers to the clipboard" (Settings → Workspace), read once per answer;
 * - wait for `whenClear` (the same arrival gate `createAgentCard` already uses) before writing,
 *   so a clipboard write of a finished answer never races the clipboard snapshot and restore of
 *   an in-flight dictation paste (ADR 0048): a simple "queue until the insertion is done".
 */
import { finishedAnswerText } from '../../shared/agent-answer-rule';
import type { AgentTurnView } from '../../shared/ipc';

/** The outcome of one `clipboard_write_text` call, as the card or thread reply shows it. */
export type CopyOutcome = 'copied' | 'failed';

export interface AnswerClipboardDeps {
  /** "Copy Genie answers to the clipboard", read once per answer. */
  enabled(): boolean;
  /** `clipboard_write_text`. */
  write(text: string): Promise<'written' | 'too_large' | 'unsupported'>;
  /** Defer `fn` until no dictation insertion is in flight. */
  whenClear(fn: () => void): void;
  /** The outcome of a turn's copy (the ⌥G card and Conversations both show it). */
  noteTurn(turnId: string, outcome: CopyOutcome): void;
  /** The outcome of a direct-path answer's copy (its own card). */
  noteDirect(mogId: string, opId: string, outcome: CopyOutcome): void;
  /** A write (or a note callback) threw. A fixed code only: never the answer text. */
  onFault?(): void;
}

export interface AnswerClipboard {
  /** Call from `agent-run.ts`'s `deps.finished`. */
  onTurnFinished(mogId: string, turn: AgentTurnView): void;
  /** Call from `genie-direct.ts`'s `onAnswered`. */
  onDirectAnswered(mogId: string, opId: string, md: string): void;
}

/** Keys already handled are capped, so a very long session never grows this without bound. */
const MAX_HANDLED = 256;

export function createAnswerClipboard(deps: AnswerClipboardDeps): AnswerClipboard {
  const handled = new Set<string>();
  const remember = (key: string) => {
    handled.add(key);
    if (handled.size > MAX_HANDLED) handled.delete(handled.values().next().value as string);
  };
  /** Copy `text` once for `key`, then report the outcome through `note`. */
  const copyOnce = (key: string, text: string, note: (outcome: CopyOutcome) => void): void => {
    if (handled.has(key)) return;
    remember(key);
    if (!deps.enabled()) return;
    deps.whenClear(() => {
      deps.write(text).then(
        (outcome) => note(outcome === 'written' ? 'copied' : 'failed'),
        () => {
          deps.onFault?.();
          note('failed');
        },
      );
    });
  };
  return {
    onTurnFinished(_mogId, turn) {
      const text = finishedAnswerText(turn);
      if (text === undefined) return;
      copyOnce(`turn:${turn.turnId}`, text, (outcome) => deps.noteTurn(turn.turnId, outcome));
    },
    onDirectAnswered(mogId, opId, md) {
      if (md.trim() === '') return;
      copyOnce(`direct:${opId}`, md, (outcome) => deps.noteDirect(mogId, opId, outcome));
    },
  };
}
