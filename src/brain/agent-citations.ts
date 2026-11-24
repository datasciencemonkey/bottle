/**
 * ADR 0122: the `[n]` markers of a combined answer. The model writes `[n]`, where `n` is the
 * 1-based index into the `sources` of the turn result. The brain maps each valid marker to a
 * citation (the workspace chip, the call and the Open in Databricks link). The view draws the
 * list and never parses or resolves anything itself (ARC-2).
 *
 * One workspace is one source for the user: two calls to the same workspace (the agent asked
 * again with other words) get one number. The brain writes the text again with these numbers,
 * 1, 2, … in the order each workspace is first cited.
 */
import { openInDatabricksUrl } from '../shared/link-trust';
import type { AgentCitation, AgentPartView } from '../shared/ipc';

/** A `[n]` marker that is not a link label, with the spaces before it. */
const MARKER = /( *)\[(\d{1,3})\](?!\()/g;

/** The citations of a summary, and its text with one number for each workspace. */
export interface WorkspaceCitations {
  md: string;
  /** One for each number, sorted by `n`. */
  citations: AgentCitation[];
  /** The workspace (`sources[].workspaceId`, the alias) of each number, in the order of `n`. */
  workspaceIds: string[];
}

const linkOf = (part: AgentPartView) => {
  const footer = part.sections.find((s) => s.id === 'footer');
  if (footer?.id !== 'footer') return undefined;
  const deepLink = openInDatabricksUrl(footer.deepLink, footer.workspaceHost);
  return deepLink !== undefined && footer.workspaceHost !== undefined
    ? { deepLink, workspaceHost: footer.workspaceHost }
    : undefined;
};

/**
 * Number the markers of `md` by workspace. A marker is valid when `n` is in range, its source has
 * a part, and that part was not withheld from the agent. A valid marker becomes `[k]`, where `k`
 * is the number of its workspace. Any other marker goes from the text, so it never shows as a
 * number that means something else. Each citation reaches the last cited call of its workspace
 * (in the order of `parts`), with the checked link of the last cited call that has one.
 */
export function citationsByWorkspace(
  md: string,
  sources: readonly { callId: string; workspaceId: string }[],
  parts: ReadonlyMap<string, AgentPartView>,
): WorkspaceCitations {
  const byWs = new Map<string, { k: number; calls: Set<string> }>();
  const text = md.replace(MARKER, (_all, spaces: string, raw: string) => {
    const source = sources[Number(raw) - 1];
    const part = source && parts.get(source.callId);
    if (source === undefined || part === undefined || part.notShownToAgent === true) return '';
    const ws = byWs.get(source.workspaceId) ?? { k: byWs.size + 1, calls: new Set<string>() };
    ws.calls.add(source.callId);
    byWs.set(source.workspaceId, ws);
    return `${spaces}[${ws.k}]`;
  });
  const order = [...parts.values()];
  const citations: AgentCitation[] = [];
  for (const { k, calls } of byWs.values()) {
    const cited = order.filter((p) => calls.has(p.callId));
    const last = cited.at(-1);
    if (last === undefined) continue;
    const link = cited
      .map(linkOf)
      .filter((l) => l !== undefined)
      .at(-1);
    citations.push({ n: k, callId: last.callId, workspace: last.workspace, ...link });
  }
  return { md: text, citations, workspaceIds: [...byWs.keys()] };
}
