/**
 * The owner rule for the answer of an agent turn. Pure, no React, no IPC.
 * - Exactly one answered call that the agent did not withhold and that is not from an earlier
 *   turn: that Genie answer is the answer, shown as it is. The text of the model is hidden.
 * - More than one such call, all to the same workspace (the agent asked again with other words):
 *   the last one is the answer (it answers the refined question), shown as it is. The earlier
 *   calls of that workspace fold under it.
 * - Any other count: the text of the model is the main answer when it has text, and the
 *   workspace answers fold under it.
 */
import { openInDatabricksUrl } from './link-trust';
import type { AgentPartView, AgentTurnView } from './ipc';

/** The text of a part whose answer the core withheld (a quarantine hit). */
export const WITHHELD_TEXT = 'This answer was held back. It looked like it held instructions.';

/** What the rule needs to know about one part of a turn. */
export interface AnswerPart {
  answered: boolean;
  withheld: boolean;
  fromEarlier: boolean;
  /** One key for each workspace (its name or its profile). */
  workspace: string;
}

const counts = (p: AnswerPart) => p.answered && !p.withheld && !p.fromEarlier;

/**
 * The index in `parts` of the part whose Genie answer is the answer, or -1 when the text of the
 * model is the answer. `parts` are in the order the calls ended.
 */
export function loneAnswerIndex(parts: readonly AnswerPart[]): number {
  const idx = parts.flatMap((p, i) => (counts(p) ? [i] : []));
  const last = idx.at(-1);
  if (last === undefined) return -1;
  const ws = parts[last]?.workspace;
  return idx.every((i) => parts[i]?.workspace === ws) ? last : -1;
}

/** True when a Genie answer of the turn is the answer (see {@link loneAnswerIndex}). */
export const isLoneAnswer = (parts: readonly AnswerPart[]): boolean => loneAnswerIndex(parts) >= 0;

/**
 * The parts that fold under the lone answer: the other answered calls of its workspace (an
 * earlier wording). A withheld part, a part of an earlier turn and a failed part stay in view.
 */
export function foldedUnderLone(parts: readonly AnswerPart[]): Set<number> {
  const lone = loneAnswerIndex(parts);
  if (lone < 0) return new Set();
  return new Set(parts.flatMap((p, i) => (i !== lone && counts(p) ? [i] : [])));
}

/** The text of the link to the Genie One conversation of a workspace. */
export const openGenieOneText = (workspace: string): string => `Open Genie One in ${workspace}`;

// ---------- the owner's "attach Genie's answer to the clipboard" request ----------

/** The `answer` section's markdown of a part's sections, if it has one. */
function answerMdOf(sections: readonly { id: string; md?: string }[]): string | undefined {
  return sections.find((s) => s.id === 'answer')?.md;
}

/**
 * The [`AnswerPart`] of a live [`AgentPartView`]. Mirrors `answerPartOf` in
 * `src/views/card/AgentTurnCard.tsx` (same rule, same `WITHHELD_TEXT` check): this copy takes no
 * React or view import, so the brain's clipboard decision never pulls in the view's dependencies.
 */
function toAnswerPart(p: AgentPartView): AnswerPart {
  return {
    answered: true,
    withheld: answerMdOf(p.sections) === WITHHELD_TEXT,
    fromEarlier: p.fromEarlier === true,
    workspace: p.workspace.name,
  };
}

/** A fenced (```…```) or inline (`…`) code span: code is verbatim, never touched. */
const FENCE = /(```[\s\S]*?```)/;
const INLINE_CODE = /(`[^`\n]*`)/;

/**
 * Strip a `[n]` citation marker from `text` only when `n` is one of `citationNumbers` (ADR 0122).
 * Pasted outside Bottle a real citation marker has no footnote or footer link to point to, so it
 * would read as a stray reference number, not real content. Everything else that merely looks
 * like one — an array index (`array[1]`), a year (`[2024]`), a real Markdown link (`[1](url)` or
 * `[1][ref]`) — is left alone: the number must be a known citation, the character right before
 * the marker must not be a word character or `]`, and the marker must not be followed by `(` or
 * `[`. With no citation numbers, nothing is stripped.
 */
export function stripCitationMarkers(text: string, citationNumbers: readonly number[]): string {
  if (citationNumbers.length === 0) return text;
  const nums = new Set(citationNumbers);
  const wordOrBracket = /[\w\]]/;
  const stripProse = (prose: string): string =>
    prose.replace(
      /([ \t]?)\[(\d+)\]/g,
      (whole: string, lead: string, numStr: string, offset: number) => {
        if (!nums.has(Number(numStr))) return whole;
        // The character right before the bracket itself (not before `lead`): with a leading
        // space that is always the space, so a space-separated marker is never blocked by this
        // check. With no leading space, it is whatever truly touches the bracket.
        const beforeBracket = prose[offset + lead.length - 1];
        if (beforeBracket !== undefined && wordOrBracket.test(beforeBracket)) return whole;
        const after = prose[offset + whole.length];
        if (after === '(' || after === '[') return whole;
        return '';
      },
    );
  // Fenced code blocks first, then inline code spans inside what is left: both stay verbatim.
  return text
    .split(FENCE)
    .map((part, i) =>
      i % 2 === 1
        ? part
        : part
            .split(INLINE_CODE)
            .map((s, j) => (j % 2 === 1 ? s : stripProse(s)))
            .join(''),
    )
    .join('');
}

/**
 * The plain text a finished turn's card or Conversations reply actually shows, by the same rule
 * the view draws by: the one Genie answer that {@link loneAnswerIndex} names, verbatim, when one
 * applies; only otherwise the model's combined note (its `[n]` citation markers stripped, since
 * they are junk once pasted outside Bottle). `undefined` when the turn has no answer to show
 * (every call withheld, or the model's note was hidden by quarantine): the caller must never copy
 * then.
 */
export function finishedAnswerText(
  turn: Pick<AgentTurnView, 'summary' | 'parts'>,
): string | undefined {
  const lone = loneAnswerIndex(turn.parts.map(toAnswerPart));
  if (lone >= 0) {
    const md = answerMdOf(turn.parts[lone]?.sections ?? []);
    return md !== undefined && md.trim() !== '' ? md : undefined;
  }
  if (!turn.summary) return undefined;
  const md = turn.summary.md;
  if (md.trim() === '') return undefined;
  const citationNumbers = (turn.summary.citations ?? []).map((c) => c.n);
  return stripCitationMarkers(md, citationNumbers);
}

/** A workspace answer that a turn holds: where its Genie One link comes from. */
export interface FooterSource {
  name: string;
  deepLink?: string | undefined;
  workspaceHost?: string | undefined;
}

/**
 * The "Open Genie One in <workspace>" links of a turn: one for each workspace, in the order of
 * its first source. The link is the trusted deep link of the last source of that workspace that
 * has one. A workspace with no trusted deep link has no link: Bottle never builds a Genie One
 * URL from ids (`openInDatabricksUrl`).
 */
export function genieOneLinks<S extends FooterSource>(
  sources: readonly S[],
): { url: string; name: string; source: S }[] {
  const by = new Map<string, { url: string; name: string; source: S } | undefined>();
  for (const s of sources) {
    const url = openInDatabricksUrl(s.deepLink, s.workspaceHost);
    if (!by.has(s.name)) by.set(s.name, undefined);
    if (url !== undefined) by.set(s.name, { url, name: s.name, source: s });
  }
  return [...by.values()].filter((l) => l !== undefined);
}
