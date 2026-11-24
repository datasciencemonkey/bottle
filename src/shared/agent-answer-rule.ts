/**
 * The owner rule for the answer of an agent turn. Pure, no React, no IPC.
 * - Exactly one answered call that the agent did not withhold and that is not from an earlier
 *   turn: that Genie answer is the answer, shown as it is. The text of the model is hidden.
 * - Any other count: the text of the model is the main answer when it has text, and the
 *   workspace answers fold under it.
 */
import { openInDatabricksUrl } from './link-trust';

/** The text of a part whose answer the core withheld (a quarantine hit). */
export const WITHHELD_TEXT = 'This answer was held back. It looked like it held instructions.';

/** What the rule needs to know about one part of a turn. */
export interface AnswerPart {
  answered: boolean;
  withheld: boolean;
  fromEarlier: boolean;
}

/** True when the Genie answer of the turn is the answer: one part counts, and only one. */
export const isLoneAnswer = (parts: readonly AnswerPart[]): boolean =>
  parts.filter((p) => p.answered && !p.withheld && !p.fromEarlier).length === 1;

/** A workspace that the summary cites: where its "Open in Databricks" link comes from. */
export interface FooterSource {
  name: string;
  deepLink?: string | undefined;
  workspaceHost?: string | undefined;
}

/**
 * The "Open in Databricks" links under a summary: one for each workspace with a trusted deep
 * link, once for each URL, and none that the summary text already carries.
 */
export function summaryFooterLinks(
  md: string,
  sources: readonly FooterSource[],
): { url: string; name: string }[] {
  const seen = new Set<string>();
  const out: { url: string; name: string }[] = [];
  for (const s of sources) {
    const url = openInDatabricksUrl(s.deepLink, s.workspaceHost);
    if (url === undefined || seen.has(url) || md.includes(url)) continue;
    seen.add(url);
    out.push({ url, name: s.name });
  }
  return out;
}
