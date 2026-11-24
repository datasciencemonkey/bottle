/**
 * ADR 0122: the `[n]` markers of a combined answer. The model writes `[n]`, where `n` is the
 * 1-based index into the `sources` of the turn result. The brain maps each valid marker to a
 * citation (the workspace chip, the call and the Open in Databricks link). The view draws the
 * list and never parses or resolves anything itself (ARC-2).
 *
 * `AgentCitation` is local to the brain until the contract change in
 * `bottle-tools/orch/cite-contract.md` moves it to `src/shared/ipc.ts`.
 */
import { openInDatabricksUrl } from '../shared/link-trust';
import type { AgentPartView, AgentTurnView, AgentWsChip } from '../shared/ipc';

export interface AgentCitation {
  /** The number in the text, 1-based. */
  n: number;
  callId: string;
  workspace: AgentWsChip;
  /** The checked Open in Databricks link of the part, when it has one. */
  deepLink?: string;
}

/** The summary of a turn with its citations (the shape of the contract change). */
export type AgentSummaryWithCitations = NonNullable<AgentTurnView['summary']> & {
  citations?: AgentCitation[];
};

/** A marker is `[n]` that is not the label of a link `[n](url)`. */
const MARKER = /\[(\d{1,3})\](?!\()/g;

/**
 * The citations of `md`, one for each distinct valid `n`, sorted by `n`. A marker is dropped (and
 * the text stays as it is) when `n` is out of range, when its source has no part, or when its
 * part was withheld from the agent.
 */
export function citationsOf(
  md: string,
  sources: readonly { callId: string }[],
  parts: ReadonlyMap<string, AgentPartView>,
): AgentCitation[] {
  const seen = new Set<number>();
  for (const m of md.matchAll(MARKER)) seen.add(Number(m[1]));
  const out: AgentCitation[] = [];
  for (const n of [...seen].sort((a, b) => a - b)) {
    const source = sources[n - 1];
    if (n < 1 || source === undefined) continue;
    const part = parts.get(source.callId);
    if (part === undefined || part.notShownToAgent === true) continue;
    const footer = part.sections.find((s) => s.id === 'footer');
    const deepLink =
      footer?.id === 'footer'
        ? openInDatabricksUrl(footer.deepLink, footer.workspaceHost)
        : undefined;
    out.push({
      n,
      callId: source.callId,
      workspace: part.workspace,
      ...(deepLink !== undefined ? { deepLink } : {}),
    });
  }
  return out;
}
