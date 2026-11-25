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

/**
 * Strip bare `[n]` citation markers (ADR 0122) from `md`. Pasted outside Bottle they have no
 * footnote or footer link to point to, so they would read as stray reference numbers, not real
 * content. A real Markdown link (`[1](url)` or `[1][ref]`) is untouched: only a marker with no
 * following `(` or `[` is removed, with the one space before it, if any.
 */
export function stripCitationMarkers(md: string): string {
  return md.replace(/[ \t]?\[\d+\](?![([])/g, '');
}

/**
 * The plain text a finished turn's card or Conversations reply actually shows: the model's
 * combined note (its `[n]` markers stripped, since they are junk once pasted outside Bottle) when
 * the turn has one, else the one Genie answer that {@link loneAnswerIndex} names, verbatim.
 * `undefined` when the turn has no answer to show (every call withheld, or the model's note was
 * hidden by quarantine): the caller must never copy then.
 */
export function finishedAnswerText(
  turn: Pick<AgentTurnView, 'summary' | 'parts'>,
): string | undefined {
  if (turn.summary) {
    const md = turn.summary.md;
    return md.trim() === '' ? undefined : stripCitationMarkers(md);
  }
  const lone = loneAnswerIndex(turn.parts.map(toAnswerPart));
  if (lone < 0) return undefined;
  const md = answerMdOf(turn.parts[lone]?.sections ?? []);
  return md !== undefined && md.trim() !== '' ? md : undefined;
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
