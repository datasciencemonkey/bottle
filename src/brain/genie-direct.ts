/**
 * UC2 Ask Genie, direct (#573, ADR 0097, user decision 2026-11-19). The ask gate's one hand-off
 * seam sends the approved question straight to T8 GenieBridge `submit` with the `genie_one_mcp`
 * binding on the MOG's target profile. T8 sends one `genie_ask` through `mcp_call` (the McpTransport
 * allowlist, #470; the host adds the R6 token, #464) and polls with the PollActor (#483).
 * The main agent and the Agent SDK take no part, and Bottle adds no typed Genie IPC route.
 *
 * The AskEvents show on the card of the owner MOG (MOG-13a), in one blurb per request. Genie One
 * does not stream tokens, so the user sees live status, then the answer (#573):
 * - at the send (G-44, before `genie_ask` returns): "Asking Genie", and `onPhase('working')`, so
 *   the MOG shows its working mood in the same task;
 * - on `progress`: the fixed stage line of the newest Genie step (`genieStage`, G-17b, UX-16a).
 *   No upstream step text shows. The card view runs the "m:ss" timer (G-17a).
 * - on the terminal event: the `outcome-blurb.ts` text (the answer, or the fixed failure text of
 *   GL-5), and for an answer the expanded sections of `card-sections.ts` (answer, key numbers).
 *   Then `onPhase('answered' | 'failed')` and `showCard`: the card shows next to the MOG even if
 *   it was hidden while Genie worked.
 *
 * No failure is sent again by itself (G-15b). A failure shows [Try again]; only a click on it
 * sends the same question again, as a new request. TODO(#86): the footer and the deep link.
 * TODO(#150): the table preview.
 *
 * #40 (AT-09): when a sent request ends, `onEnd` gets its end once, after the card update.
 * `genie-history.ts` saves it as one local exchange row. A request that was never sent (a
 * `submit` that throws) has no end.
 */
import type { AgentTurn, AskRequest, GenieBridge, MogId } from '../shared/interfaces';
import { ASKING_GENIE, genieBlurbId, genieStage } from '../shared/genie-status';
import type { CardStack } from './card-stack';
import { isTerminal, type TerminalAskEvent } from './components/ask-events';
import type { GenieEnd } from './genie-history';
import { outcomeBlurbInput } from './outcome-blurb';

/** The "Where your data goes" line of an answer card. TODO(#94): built from the live config (UX-24a). */
export const GENIE_DATA_FLOW_TEXT = 'Your question went to Genie One in your workspace.';

/** The control of a failed request (GL-5). A click sends the same question once more. */
export const TRY_AGAIN_LABEL = 'Try again';

/** The request phases that the MOG mood shows (`capture-mood.ts`). */
export type GeniePhase = 'working' | 'answered' | 'failed' | 'cancelled';

export interface GenieDirectDeps {
  /** T8. Only `submit` is used. */
  genie: Pick<GenieBridge, 'submit'>;
  /** The card of the owner MOG. */
  cardFor(
    mogId: MogId,
  ): Pick<CardStack, 'add' | 'setExpanded'> &
    Partial<Pick<CardStack, 'setControls' | 'clearControls'>>;
  /** A new operation ID per press. */
  newOpId(): string;
  /** #40: called once when a sent request ends, after the card shows the outcome. */
  onEnd?: ((x: GenieEnd) => void) | undefined;
  /** #573: the MOG mood follows the request (working, then answered or failed). */
  onPhase?: ((mogId: MogId, phase: GeniePhase, opId: string) => void) | undefined;
  /** #573: `window_card_show`, after the card holds the outcome. */
  showCard?: ((mogId: MogId) => unknown) | undefined;
}

/** The mood phase of a terminal event. A clarify question is an answer: Genie answered. */
function phaseOf(e: TerminalAskEvent): GeniePhase {
  if (e.t === 'cancel_requested') return 'cancelled';
  return e.t === 'answer' || e.t === 'needs_input' ? 'answered' : 'failed';
}

/**
 * The `send` of the ask gate. It resolves when the request has ended; it rejects only when
 * `submit` throws before anything is sent (the gate then shows "Not sent.").
 */
export function createGenieDirectSend(deps: GenieDirectDeps): (turn: AgentTurn) => Promise<void> {
  const safely = (fn: () => unknown) => {
    try {
      void Promise.resolve(fn()).catch(() => undefined);
    } catch {
      // A mood or window fault never stops the request or changes the card.
    }
  };
  const send = (turn: AgentTurn): Promise<void> => {
    const req: AskRequest = {
      opId: deps.newOpId(),
      mogId: turn.mogId,
      binding: { kind: 'genie_one_mcp', profile: turn.target },
      question: turn.question,
      tags: { app: 'bottle', mog: turn.mogId, op: 'genie' },
    };
    const { opId, events } = deps.genie.submit(req);
    const card = deps.cardFor(turn.mogId);
    const id = genieBlurbId(opId);
    // A new request never shows the answer of an older one.
    card.setExpanded(undefined);
    let shown = ASKING_GENIE;
    card.add({ id, text: shown });
    if (deps.onPhase) safely(() => deps.onPhase?.(turn.mogId, 'working', opId));
    return (async () => {
      let end: TerminalAskEvent | undefined;
      let accepted: { conversationId: string; responseId: string } | undefined;
      try {
        for await (const e of events) {
          if (e.t === 'accepted')
            accepted = { conversationId: e.conversationId, responseId: e.responseId };
          if (isTerminal(e)) {
            end = e;
            break;
          }
          if (e.t === 'progress') {
            const stage = genieStage(e.steps);
            if (stage !== undefined && stage !== shown) card.add({ id, text: (shown = stage) });
          }
        }
      } catch {
        // T8 ends every request in one terminal event (G-21b), so this is a client fault. The
        // card must not stay on the working line, and nothing is sent again (G-15b).
      }
      end ??= { t: 'uncertain' };
      card.add(outcomeBlurbInput(id, end));
      if (end.t === 'answer')
        card.setExpanded({
          answerMd: end.md,
          keyNumbers: end.keyNumbers.map((k) => k.text),
          dataFlowText: GENIE_DATA_FLOW_TEXT,
        });
      else if (phaseOf(end) === 'failed' && card.setControls) {
        // GL-5: one user action sends the same question again, as a new request (G-15b).
        const owner = `retry:${id}`;
        card.setControls(owner, [{ id: owner, label: TRY_AGAIN_LABEL }], () => {
          card.clearControls?.(owner);
          safely(() => send(turn));
        });
      }
      safely(() => deps.onPhase?.(turn.mogId, phaseOf(end), opId));
      if (deps.showCard) safely(() => deps.showCard?.(turn.mogId));
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
  return send;
}
