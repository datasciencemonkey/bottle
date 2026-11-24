/**
 * ADR 0122: the `[n]` markers of a combined answer. The model writes `[n]`, where `n` is the
 * 1-based index into the `sources` of the turn result. The brain maps each valid marker to a
 * citation (the workspace chip, the call and the Open in Databricks link). The view draws the
 * list and never parses or resolves anything itself (ARC-2).
 */
import { CITATION_MARKER } from '../shared/citation-marker';
import { openInDatabricksUrl } from '../shared/link-trust';
import type { AgentCitation, AgentPartView } from '../shared/ipc';

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
  for (const m of md.matchAll(CITATION_MARKER)) seen.add(Number(m[1]));
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
      // L1: the view checks the link again against this host.
      ...(deepLink !== undefined && footer?.id === 'footer' && footer.workspaceHost !== undefined
        ? { workspaceHost: footer.workspaceHost }
        : {}),
    });
  }
  return out;
}
