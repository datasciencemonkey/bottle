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
 * - never fire for an answer the user already dismissed (closed its card or ready pill) before
 *   it finished: a silent clipboard overwrite of something already dismissed would surprise them
 *   (owner decision, reversible). Callers check this themselves before calling in, so a
 *   dismissed answer never reaches this module at all.
 * - respect "Copy Genie answers to the clipboard" (Settings → Workspace), read once per answer;
 * - wait for `whenClear` (the same arrival gate `createAgentCard` already uses) before writing,
 *   so a clipboard write of a finished answer never races the clipboard snapshot and restore of
 *   an in-flight dictation paste (ADR 0048): a simple "queue until the insertion is done";
 * - clear their own note `NOTE_MS` after setting it (the brain, not only the view's local fade
 *   timer), so "Copied" never resurfaces hours later on a remount or a reopened thread.
 */
import { answerLinkTrust, finishedAnswerText } from '../../shared/agent-answer-rule';
import { formatAnswerForClipboard } from '../../shared/answer-clipboard-format';
import { NO_LINKS } from '../../shared/link-trust';
import type { AgentTurnView } from '../../shared/ipc';

/** The outcome of one `clipboard_write_text` call, as the card or thread reply shows it. */
export type CopyOutcome = 'copied' | 'failed';

/** How long a note stays set before this module clears it again. Matches the view's own fade
 * timer (`src/views/card/copy-answer-note.tsx`'s `NOTE_MS`): kept as a literal here too, since a
 * brain module must not import from a view file. */
const NOTE_MS = 2_000;

export interface AnswerClipboardDeps {
  /** "Copy Genie answers to the clipboard", read once per answer. */
  enabled(): boolean;
  /** `clipboard_write_text`. `html` is the ADR 0124 amendment (paste formatted into Docs and
   * email, as cells into Excel): omitted for the manual "Copy Markdown" button, which writes
   * raw Markdown text only (`copyMarkdownNow`). */
  write(text: string, html?: string): Promise<'written' | 'too_large' | 'unsupported'>;
  /** Defer `fn` until no dictation insertion is in flight. */
  whenClear(fn: () => void): void;
  /** The outcome of a turn's copy (the ⌥G card and Conversations both show it), or `undefined`
   * to clear a note this module set earlier. */
  noteTurn(turnId: string, outcome: CopyOutcome | undefined): void;
  /** The outcome of a direct-path answer's copy (its own card), or `undefined` to clear it. */
  noteDirect(mogId: string, opId: string, outcome: CopyOutcome | undefined): void;
  /** A write (or a note callback) threw. A fixed code only: never the answer text. */
  onFault?(): void;
}

export interface AnswerClipboard {
  /** Call from `agent-run.ts`'s `deps.finished`. Never call for a dismissed turn. */
  onTurnFinished(mogId: string, turn: AgentTurnView): void;
  /** Call from `genie-direct.ts`'s `onAnswered`. Never call for a dismissed send. `host` is the
   * direct answer's workspace host (ADR 0124 amendment): the one host the HTML form's links
   * trust. */
  onDirectAnswered(mogId: string, opId: string, md: string, host?: string): void;
  /**
   * The owner's "put the Markdown as a little button on the Conversations copy" request: an
   * explicit click on the per-answer "Copy Markdown" control. Unlike {@link onTurnFinished}, it
   * ignores the "Copy Genie answers" toggle and the once-per-turn guard (the user may click it
   * again), and it always writes plain Markdown text only, with no `html`, so it pastes as raw
   * Markdown. A no-op when the turn has no answer to copy (withheld, or an empty model note).
   */
  copyMarkdownNow(turnId: string, turn: Pick<AgentTurnView, 'summary' | 'parts'>): void;
  /**
   * The owner's "Copy" button request: an explicit click on a finished turn's own Copy control,
   * formatted (html plus text, the same `formatAnswerForClipboard` output the auto-copy writes).
   * Unlike {@link onTurnFinished} it ignores the "Copy Genie answers" toggle and the
   * once-per-turn guard (the user may click it again, including with the toggle off); unlike
   * {@link copyMarkdownNow} it writes `html` too. A no-op when the turn has no answer to copy.
   */
  copyNow(turnId: string, turn: Pick<AgentTurnView, 'summary' | 'parts'>): void;
  /** As {@link copyNow}, for the direct path's own Copy control (`GenieCard`'s answer pill). */
  copyDirectNow(mogId: string, opId: string, md: string, host?: string): void;
}

/** Keys already handled are capped, so a very long session never grows this without bound. */
const MAX_HANDLED = 256;

export function createAnswerClipboard(deps: AnswerClipboardDeps): AnswerClipboard {
  const handled = new Set<string>();
  const remember = (key: string) => {
    handled.add(key);
    if (handled.size > MAX_HANDLED) handled.delete(handled.values().next().value as string);
  };
  /** Write `text`/`html` once, then report the outcome through `note`, and clear it again. */
  const writeOnce = (
    text: string,
    html: string | undefined,
    note: (outcome: CopyOutcome | undefined) => void,
  ): void => {
    deps
      .write(text, html)
      // HTML about doubles the bytes: an answer past the cap still copies as plain text.
      .then((outcome) =>
        outcome === 'too_large' && html !== undefined ? deps.write(text) : outcome,
      )
      .then(
        (outcome) => {
          note(outcome === 'written' ? 'copied' : 'failed');
          setTimeout(() => note(undefined), NOTE_MS);
        },
        () => {
          deps.onFault?.();
          note('failed');
          setTimeout(() => note(undefined), NOTE_MS);
        },
      );
  };
  /** Copy `text`/`html` once for `key` (the once-per-answer auto-copy), gated on the toggle. */
  const copyOnce = (
    key: string,
    text: string,
    html: string | undefined,
    note: (outcome: CopyOutcome | undefined) => void,
  ): void => {
    if (handled.has(key)) return;
    remember(key);
    if (!deps.enabled()) return;
    deps.whenClear(() => writeOnce(text, html, note));
  };
  return {
    onTurnFinished(_mogId, turn) {
      const md = finishedAnswerText(turn);
      if (md === undefined) return;
      const { text, html } = formatAnswerForClipboard(md, answerLinkTrust(turn));
      copyOnce(`turn:${turn.turnId}`, text, html, (outcome) => deps.noteTurn(turn.turnId, outcome));
    },
    onDirectAnswered(mogId, opId, md, host) {
      if (md.trim() === '') return;
      const { text, html } = formatAnswerForClipboard(
        md,
        host !== undefined ? { profileHost: host, allowedAppUrls: [] } : NO_LINKS,
      );
      copyOnce(`direct:${opId}`, text, html, (outcome) => deps.noteDirect(mogId, opId, outcome));
    },
    copyMarkdownNow(turnId, turn) {
      const text = finishedAnswerText(turn);
      if (text === undefined) return;
      deps.whenClear(() => writeOnce(text, undefined, (outcome) => deps.noteTurn(turnId, outcome)));
    },
    copyNow(turnId, turn) {
      const md = finishedAnswerText(turn);
      if (md === undefined) return;
      const { text, html } = formatAnswerForClipboard(md, answerLinkTrust(turn));
      deps.whenClear(() => writeOnce(text, html, (outcome) => deps.noteTurn(turnId, outcome)));
    },
    copyDirectNow(mogId, opId, md, host) {
      if (md.trim() === '') return;
      const { text, html } = formatAnswerForClipboard(
        md,
        host !== undefined ? { profileHost: host, allowedAppUrls: [] } : NO_LINKS,
      );
      deps.whenClear(() =>
        writeOnce(text, html, (outcome) => deps.noteDirect(mogId, opId, outcome)),
      );
    },
  };
}
