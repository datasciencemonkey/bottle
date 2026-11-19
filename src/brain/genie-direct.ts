/**
 * UC2 Ask Genie, direct (#573, ADR 0097, user decision 2026-11-19). The ask gate's one hand-off
 * seam sends the approved question straight to T8 GenieBridge `submit` with the `genie_one_mcp`
 * binding on the MOG's target profile. T8 sends one `genie_ask` through `mcp_call` (the McpTransport
 * allowlist, #470; the host adds the R6 token, #464) and polls with the PollActor (#483).
 * The main agent and the Agent SDK take no part, and Bottle adds no typed Genie IPC route.
 *
 * The AskEvents show on the card of the owner MOG (MOG-13a), in one blurb per request:
 * - at the send (G-44, before `genie_ask` returns): the Appendix E Acknowledged line;
 * - on `progress`: "Working... m:ss" (G-17a). TODO(#87): the newest step with banned words
 *   filtered (G-17b, UX-16a). Until then no upstream step text shows.
 * - on the terminal event: the `outcome-blurb.ts` text (the answer, or the fixed failure text of
 *   GL-5), and for an answer the expanded sections of `card-sections.ts` (answer, key numbers).
 *
 * No failure is sent again (G-15b): the outcome controls ([Retry], [Open in Databricks]) do not
 * show yet. TODO(#86): the outcome controls, the footer and the deep link. TODO(#150): the table
 * preview. TODO(#40): the workspace host in the Acknowledged line ("this workspace" until then).
 *
 * #40 (AT-09): when a sent request ends, `onEnd` gets its end once, after the card update.
 * `genie-history.ts` saves it as one local exchange row. A request that was never sent (a
 * `submit` that throws) has no end.
 */
import type { AgentTurn, AskRequest, GenieBridge, MogId, ProfileName } from '../shared/interfaces';
import type { CardStack } from './card-stack';
import { isTerminal, type TerminalAskEvent } from './components/ask-events';
import type { GenieEnd } from './genie-history';
import { outcomeBlurbInput } from './outcome-blurb';

/** The "Where your data goes" line of an answer card. TODO(#94): built from the live config (UX-24a). */
export const GENIE_DATA_FLOW_TEXT = 'Your question went to Genie One in your workspace.';

/** Appendix E Acknowledged: "Asking Genie in <host> (profile <p>)". TODO(#40): the host. */
export const workingLine = (profile: ProfileName) =>
  `Asking Genie in this workspace (profile ${profile})`;

/** G-17a: "Working... m:ss". */
export function elapsedLine(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `Working\u2026 ${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export interface GenieDirectDeps {
  /** T8. Only `submit` is used. */
  genie: Pick<GenieBridge, 'submit'>;
  /** The card of the owner MOG. */
  cardFor(mogId: MogId): Pick<CardStack, 'add' | 'setExpanded'>;
  /** A new operation ID per press. */
  newOpId(): string;
  /** #40: called once when a sent request ends, after the card shows the outcome. */
  onEnd?: ((x: GenieEnd) => void) | undefined;
}

/**
 * The `send` of the ask gate. It resolves when the request has ended; it rejects only when
 * `submit` throws before anything is sent (the gate then shows "Not sent.").
 */
export function createGenieDirectSend(deps: GenieDirectDeps): (turn: AgentTurn) => Promise<void> {
  return (turn) => {
    const req: AskRequest = {
      opId: deps.newOpId(),
      mogId: turn.mogId,
      binding: { kind: 'genie_one_mcp', profile: turn.target },
      question: turn.question,
      tags: { app: 'bottle', mog: turn.mogId, op: 'genie' },
    };
    const { opId, events } = deps.genie.submit(req);
    const card = deps.cardFor(turn.mogId);
    const id = `genie:${opId}`;
    card.add({ id, text: workingLine(turn.target) });
    return (async () => {
      let end: TerminalAskEvent | undefined;
      let accepted: { conversationId: string; responseId: string } | undefined;
      try {
        for await (const e of events) {
          if (e.t === 'accepted')
            accepted = { conversationId: e.conversationId, responseId: e.responseId };
          if (isTerminal(e)) {
            card.add(outcomeBlurbInput(id, e));
            if (e.t === 'answer')
              card.setExpanded({
                answerMd: e.md,
                keyNumbers: e.keyNumbers.map((k) => k.text),
                dataFlowText: GENIE_DATA_FLOW_TEXT,
              });
            end = e;
            break;
          }
          if (e.t === 'progress') card.add({ id, text: elapsedLine(e.elapsedMs) });
        }
      } catch {
        // T8 ends every request in one terminal event (G-21b), so this is a client fault. The
        // card must not stay on the working line, and nothing is sent again (G-15b).
      }
      if (!end) {
        end = { t: 'uncertain' };
        card.add(outcomeBlurbInput(id, end));
      }
      // #40: the history write comes after the card, and a fault in it never reaches the card.
      try {
        deps.onEnd?.({
          opId,
          mogId: turn.mogId,
          profile: turn.target,
          question: turn.question,
          end,
          ...accepted,
        });
      } catch {
        // The history logs its own fixed code; the answer on the card stays as it is.
      }
    })();
  };
}
