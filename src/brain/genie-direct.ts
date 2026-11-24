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
 *   First the sections or Try again, then `showCard` (awaited), then the outcome blurb and
 *   `onPhase('answered' | 'failed')`: the card shows next to the MOG even if it was hidden while
 *   Genie worked, and it changes from the pill to the card in one update.
 * - UX slice 1: this card update is an arrival, and it goes through the arrival gate. While the
 *   talk key is held it waits (the pill reads "Answer ready" for an answer), and it applies
 *   1.2 s after the paste of the hold. The returned promise resolves after the update. The
 *   history write (#40) is no arrival: it runs at the terminal event.
 *
 * No failure is sent again by itself (G-15b). A failure shows [Try again]; only a click on it
 * sends the same question again, as a new request. TODO(#86): the footer and the deep link.
 * TODO(#150): the table preview.
 *
 * #40 (AT-09): when a sent request ends, `onEnd` gets its end once, after the card update.
 * `genie-history.ts` saves it as one local exchange row. A request that was never sent (a
 * `submit` that throws) has no end.
 */
import type { AgentTurn, AskRequest, GenieBridge, MogId, ProfileName } from '../shared/interfaces';
import { ANSWER_READY, ASKING_GENIE, genieBlurbId, genieStage } from '../shared/genie-status';
import type { ArrivalGate } from './arrival-gate';
import type { CardStack } from './card-stack';
import { isTerminal, type TerminalAskEvent } from './components/ask-events';
import type { ChipView } from './card-chips';
import type { GenieEnd } from './genie-history';
import { WORKSPACE_CODES } from './components/genie-errors';
import { outcomeBlurbInput } from './outcome-blurb';

/** The "Where your data goes" line of an answer card. TODO(#94): built from the live config (UX-24a). */
export const GENIE_DATA_FLOW_TEXT = 'Your question went to Genie One in your workspace.';

/** The control of a failed request (GL-5). A click sends the same question once more. */
export const TRY_AGAIN_LABEL = 'Try again';

/** The text link beside Try again when the workspace is the likely cause (VPN, proxy, network). */
export const CHANGE_WORKSPACE_LABEL = 'Change workspace…';

/** R-8 (slice 2a): the chip on a direct answer that sends the same question to the agent. */
export const ASK_OTHERS_LABEL = 'Ask other workspaces too';

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
  /** #523: the workspace host of a profile (`profiles_list`); the card trusts links on it only. */
  hostOf?: ((profile: ProfileName) => Promise<string | undefined>) | undefined;
  /** #573: `window_card_show`, after the card holds the outcome. */
  showCard?: ((mogId: MogId) => unknown) | undefined;
  /** "Change workspace…": show the hub on this MOG's Edit form (the home workspace field). */
  changeWorkspace?: ((mogId: MogId) => unknown) | undefined;
  /**
   * UX slice 1: the arrival gate. While the user holds the talk key, the terminal event changes
   * only the pill to "Answer ready" (an answer) and the card update waits for `whenClear`.
   */
  arrivals?: Pick<ArrivalGate, 'isHeld' | 'whenClear'> | undefined;
  /**
   * R-8 (slice 2a): the chip "Ask other workspaces too" for an answered direct request, or
   * undefined when the agent cannot run. The chip id is `agent-more:<opId>`. A click goes to the
   * agent wiring (R-9), which holds the question.
   */
  offerOthers?: ((turn: AgentTurn, opId: string) => ChipView | undefined) | undefined;
}

/** The options of one send. */
export interface DirectSendOptions {
  /** The fallback reason of slice 2a, one line: "Genie Code is not installed. Asked sales only." */
  notice?: string | undefined;
  /** The fallback of an agent turn: its row joins the turn (`split_group_id` = the turn UUID). */
  splitGroupId?: string | undefined;
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
export function createGenieDirectSend(
  deps: GenieDirectDeps,
): (turn: AgentTurn, opts?: DirectSendOptions) => Promise<void> {
  const safely = (fn: () => unknown) => {
    try {
      void Promise.resolve(fn()).catch(() => undefined);
    } catch {
      // A mood or window fault never stops the request or changes the card.
    }
  };
  /** The chip owner of each MOG's newest R-8 chip, so a new request takes the old one away. */
  const moreOwner = new Map<MogId, string>();
  const send = (turn: AgentTurn, opts: DirectSendOptions = {}): Promise<void> => {
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
    // The new status blurb goes on top first, then the old answer goes: the card never shows the
    // old answer without its sections (no flash).
    let shown = ASKING_GENIE;
    card.add({ id, text: shown });
    card.setExpanded(undefined);
    const oldMore = moreOwner.get(turn.mogId);
    if (oldMore !== undefined) card.clearControls?.(oldMore);
    moreOwner.delete(turn.mogId);
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
      // #523: the card draws links and "Open in Databricks" only for this workspace.
      const host =
        end.t === 'answer' ? await deps.hostOf?.(turn.target).catch(() => undefined) : undefined;
      const outcome = end;
      // #40: the history write is not an arrival: it runs at the terminal event, and a fault in it never reaches the card.
      try {
        deps.onEnd?.({
          opId,
          mogId: turn.mogId,
          profile: turn.target,
          question: turn.question,
          end: outcome,
          ...(opts.splitGroupId !== undefined ? { splitGroupId: opts.splitGroupId } : {}),
          ...accepted,
        });
      } catch {
        // The history logs its own fixed code; the answer on the card stays as it is.
      }
      const apply = async (): Promise<void> => {
        // The answer sections and the Try again control go on while the blurb still works, so the
        // card goes from the status pill to the answer or failure card in one step.
        if (outcome.t === 'answer') {
          // Slice 2a: a fallback tells the user, in one line before the answer, why only one
          // workspace was asked.
          card.setExpanded({
            answerMd: opts.notice ? `*${opts.notice}*\n\n${outcome.md}` : outcome.md,
            keyNumbers: outcome.keyNumbers.map((k) => k.text),
            ...(outcome.deepLink ? { deepLink: outcome.deepLink } : {}),
            ...(host ? { workspaceHost: host } : {}),
            dataFlowText: GENIE_DATA_FLOW_TEXT,
          });
        } else if (phaseOf(outcome) === 'failed' && card.setControls) {
          // GL-5: one user action sends the same question again, as a new request (G-15b).
          const owner = `retry:${id}`;
          // A workspace failure (VPN, proxy, network) also offers a text link to the MOG's Edit
          // form. Try again stays the only button (W-07h: one primary action).
          const wsId = `workspace:${id}`;
          const offerWorkspace =
            outcome.t === 'failed' &&
            WORKSPACE_CODES.has(outcome.code) &&
            deps.changeWorkspace !== undefined;
          const chips = [{ id: owner, label: TRY_AGAIN_LABEL }];
          if (offerWorkspace) chips.push({ id: wsId, label: CHANGE_WORKSPACE_LABEL });
          card.setControls(owner, chips, (clicked) => {
            if (clicked === wsId) {
              safely(() => deps.changeWorkspace?.(turn.mogId));
              return;
            }
            card.clearControls?.(owner);
            safely(() => send(turn, opts));
          });
        }
        if (outcome.t === 'answer' && card.setControls) {
          const more = deps.offerOthers?.(turn, opId);
          if (more) {
            const owner = `more:${id}`;
            moreOwner.set(turn.mogId, owner);
            card.setControls(owner, [more]);
          }
        }
        // R11 places the card first, then the outcome goes on it, so the view grows the card from
        // its final place (no race between the view fit and the R11 move).
        if (deps.showCard) {
          try {
            await deps.showCard(turn.mogId);
          } catch {
            // The card stays where it was; the outcome still goes on it.
          }
        }
        card.add(outcomeBlurbInput(id, outcome));
        safely(() => deps.onPhase?.(turn.mogId, phaseOf(outcome), opId));
      };
      const arrivals = deps.arrivals;
      // The pill tells the user an answer is ready, and the card waits for the key-up.
      if (arrivals?.isHeld() && phaseOf(outcome) === 'answered')
        card.add({ id, text: ANSWER_READY });
      await new Promise<void>((resolve, reject) => {
        const run = () => void apply().then(resolve, reject);
        if (arrivals) arrivals.whenClear(run);
        else run();
      });
    })();
  };
  return send;
}
