/**
 * The T3 ask gate (#142, ADR 0086, ADR 0098): the ⌥G ask and the Ask Genie chip of a PTT or
 * hands-free utterance. Binding rule B1: nothing goes to Genie without a user action: ⌥G during
 * the talk-key hold, or a press on an Ask Genie chip.
 *
 * - RTR-17a, RTR-17b (ADR 0098): an `ask` route (⌥G) sends its exact text at once through the
 *   one hand-off seam `send(turn)`. It has no 1.5 s preview: ⌥G is the user action. ADR 0097
 *   (#573): in `startBrain` that seam sends it straight to Genie One MCP (T8 `submit`,
 *   `genie_ask`); the main agent takes no part. Before the send, the G-47 sanitizer and then the
 *   G-30a cap run on the text.
 * - RTR-05: a `chip` route shows the Appendix E chip 1 "Ask Genie" (2 min or 3 newer): the
 *   fallback of a dictated text (ADR 0098), or a session question (RTR-07). Only a press sends.
 * - RTR-14: an injection-suspected route shows the fixed warning chip of ADR 0056. It has no
 *   action: a click sends nothing.
 * - A missing target profile, a closed MOG or a failed hand-off sends nothing and shows
 *   "Not sent." No retry runs by itself.
 * - #557 (ADR 0096): with `pickProfile`, an ask or a press with no target profile opens the
 *   profile picker and sends nothing. The user action is the confirmation: when the user clicks
 *   a profile, the same text goes out once. Bottle never picks.
 *
 * The gate never calls `mcp_call` or `agent_*` itself: `send` is its only way out.
 * TODO(#200): the RecallGate hits go in `AgentTurn.recall`; until then it is empty.
 * TODO(#374): one owner routes all card input; until then `wireAskGate` routes the card's
 * `cancel` and `chip_click`.
 */
import type { AgentTurn, MogId, ProfileName, RouterDecision } from '../shared/interfaces';
import { onViewInput, type BrainBus } from './bus';
import { CHIP_CATALOGUE, type ChipAction } from './card-chips';
import type { CardStack, Scheduler } from './card-stack';
import type { MonotonicClock } from './clock';
import { INJECTION_WARNING_CHIP } from './components/router-injection';
import type { OwnedUtterance, RequestOwner } from './request-owner';
import { sanitizeQuestion } from './question-sanitizer';
import type { AskRoute } from './utterance-wiring';

/** Appendix E chip 1. */
export const ASK_CHIP_LABEL = 'Ask Genie';
/** The line of a question that was not sent (ADR 0086: Appendix E has no template for it). */
export const NOT_SENT_LINE = 'Not sent.';
/** G-30a, D29: the question cap. #557: a press sends the whole dictated text, so it is cut here. */
export const QUESTION_MAX_CHARS = 1000;
/** The notice of a cut question (G-30a). */
export const CUT_NOTICE = 'Cut to 1,000 characters.';

/** G-30a: cut a question over the cap at a word edge. */
export function capQuestion(q: string): { text: string; cut: boolean } {
  if (q.length <= QUESTION_MAX_CHARS) return { text: q, cut: false };
  const head = q.slice(0, QUESTION_MAX_CHARS + 1);
  const edge = head.search(/\s\S*$/);
  return {
    text: (edge > 0 ? head.slice(0, edge) : head.slice(0, QUESTION_MAX_CHARS)).trimEnd(),
    cut: true,
  };
}

/** Appendix E chip 1 expiry: 2 min or 3 newer chips. */
const ASK_TTL_MS = CHIP_CATALOGUE.ask_genie.ttlMs ?? 2 * 60_000;
const ASK_NEWER = CHIP_CATALOGUE.ask_genie.newer ?? 3;

/** The card of one MOG, as the gate uses it (the #498 card stack). */
export type AskCard = Pick<
  CardStack,
  'add' | 'setControls' | 'clearControls' | 'offerChip' | 'removeChip'
>;

export interface AskGateDeps {
  clock: MonotonicClock;
  schedule: Scheduler;
  cardFor(mogId: MogId): AskCard;
  /** `window_card_show`, so the ask or the chip is seen. */
  showCard(mogId: MogId): unknown;
  owner: Pick<RequestOwner, 'release'>;
  /** True while the MOG window is open. A closed MOG gets nothing sent. */
  isOpen(mogId: MogId): boolean;
  /** The MOG's current target profile. Undefined while none is chosen: Bottle never picks one. */
  target(mogId: MogId): ProfileName | undefined;
  /** The hand-off seam: the approved question goes out. ADR 0097: straight to Genie One MCP. */
  send(turn: AgentTurn): Promise<void>;
  /** #557: show the profile picker of a MOG; `then` runs once after the user saves a pick. */
  pickProfile?(mogId: MogId, then: () => void): void;
}

export interface AskGate {
  /** The `onAsk` of the utterance hook (#59): an ⌥G ask or a chip, with its live owner. */
  onAsk(r: AskRoute, owned: OwnedUtterance): void;
  /** The card stack's `onChip`: a click on a live chip of `mogId` (UX-20c). */
  chip(mogId: MogId, action: ChipAction): void;
}

const quoted = (q: string) => `"${q}"`;
const notSentText = (q: string) => `${quoted(q)}\n${NOT_SENT_LINE}`;

interface Offer {
  mogId: MogId;
  question: string;
  decision: RouterDecision;
  offeredAt: number;
  blurbId: string;
}

const firstNonEmpty = (...xs: string[]) => xs.find((x) => x.trim() !== '') ?? '';

export function createAskGate(deps: AskGateDeps): AskGate {
  /** The live Ask Genie chips, by chip ID, oldest first. */
  const offers = new Map<string, Offer>();

  const show = (mogId: MogId) => {
    try {
      void Promise.resolve(deps.showCard(mogId)).catch(() => undefined);
    } catch {
      // The card stays hidden. The send does not wait for it.
    }
  };

  /** Send the question through the seam (ADR 0097: Genie One MCP). The only way out of the gate. */
  const handOff = (mogId: MogId, question: string, decision: RouterDecision, blurbId: string) => {
    const card = deps.cardFor(mogId);
    const target = deps.target(mogId);
    if (target === undefined && deps.pickProfile && deps.isOpen(mogId)) {
      // #557: the press waits for the user's pick. A pick that still gives no target sends nothing.
      deps.pickProfile(mogId, () => {
        if (deps.target(mogId) !== undefined) handOff(mogId, question, decision, blurbId);
      });
      return;
    }
    if (target === undefined || !deps.isOpen(mogId)) {
      card.add({ id: blurbId, text: notSentText(question), completed: true });
      return;
    }
    // G-47 (#573): the sanitizer runs first, then the G-30a (#557) size check, then the send.
    // RTR-14 ran before the offer. A text with nothing left after the sanitizer is not sent.
    const clean = sanitizeQuestion(question);
    if (clean === '') {
      // The removed markup is not shown again: the line says only that nothing went out.
      card.add({ id: blurbId, text: NOT_SENT_LINE, completed: true });
      return;
    }
    const capped = capQuestion(clean);
    card.add({
      id: blurbId,
      text: capped.cut ? `${quoted(capped.text)}\n${CUT_NOTICE}` : quoted(capped.text),
      completed: true,
    });
    const turn: AgentTurn = { mogId, question: capped.text, decision, recall: [], target };
    let sent: Promise<void>;
    try {
      sent = deps.send(turn);
    } catch (e) {
      sent = Promise.reject(e);
    }
    // ARC-F14: a rejected send means nothing took the question. No retry by itself (G-15b).
    // TODO(#143): the fallback classifier route of RTR-15b.
    void sent.catch(() => card.add({ id: blurbId, text: notSentText(question), completed: true }));
  };

  const offerChip = (r: Extract<AskRoute, { route: 'chip' }>, owned: OwnedUtterance) => {
    const { mogId, utteranceId } = owned;
    const card = deps.cardFor(mogId);
    if (r.warning === 'injection_suspected') {
      // RTR-14: a warning with no action. It goes with the Ask Genie expiry.
      const owner = `warning:${utteranceId}`;
      // #557: the warning shows with its own text, not with an older blurb.
      const text = firstNonEmpty(r.decision.genie_question, r.decision.cleaned_text);
      if (text !== '') card.add({ id: owner, text: quoted(text), completed: true });
      card.setControls(owner, [{ ...INJECTION_WARNING_CHIP }]);
      deps.schedule(deps.clock.now() + ASK_TTL_MS, () => card.clearControls(owner));
      show(mogId);
      return;
    }
    const question = firstNonEmpty(r.decision.genie_question, r.decision.cleaned_text);
    if (question === '') return;
    const id = `ask_genie:${utteranceId}`;
    const blurbId = `ask:${utteranceId}`;
    offers.set(id, {
      mogId,
      question,
      decision: r.decision,
      offeredAt: deps.clock.now(),
      blurbId,
    });
    // The "3 newer chips" rule: the oldest offers of this MOG go.
    const mine = [...offers].filter(([, o]) => o.mogId === mogId);
    for (const [old] of mine.slice(0, Math.max(0, mine.length - ASK_NEWER))) offers.delete(old);
    card.add({ id: blurbId, text: quoted(question), completed: true });
    card.offerChip({ id, kind: 'ask_genie', label: ASK_CHIP_LABEL, owner: blurbId });
    show(mogId);
  };

  return {
    onAsk(r, owned) {
      // Neither route holds the owner: the send and the chip click name their MOG.
      deps.owner.release(owned.utteranceId);
      // RTR-17a, RTR-17b: ⌥G sends at once. RTR-14, twice: a flagged decision never sends.
      if (r.route === 'ask' && !r.decision.injection_suspected) {
        show(owned.mogId);
        return handOff(owned.mogId, r.question, r.decision, `ask:${owned.utteranceId}`);
      }
      offerChip(
        r.route === 'chip'
          ? r
          : {
              route: 'chip',
              decision: { ...r.decision, genie_question: r.question },
              warning: 'injection_suspected',
            },
        owned,
      );
    },
    chip(mogId, action) {
      if (action.kind !== 'ask_genie') return;
      const o = offers.get(action.id);
      if (!o || o.mogId !== mogId) return;
      offers.delete(action.id);
      deps.cardFor(mogId).removeChip(action.id);
      if (deps.clock.now() - o.offeredAt >= ASK_TTL_MS) return;
      handOff(mogId, o.question, o.decision, o.blurbId);
    },
  };
}

/**
 * Feed the gate from the brain Channel: chip clicks and hover from the card of the MOG, and Esc
 * (`view:input` `cancel`), which hides the card. Returns `off`.
 *
 * `hover` (review round 1, #103): the Genie card reports its own pointer hold the same way the
 * legacy panel always has, so the stack's own, longer idle timer (UX-12) never ages a completed
 * Genie blurb out while the user still reads it. TODO(#374): the rest of the card's input
 * (`toggle_pin`, `open_history`) is not yet routed here.
 */
export function wireAskGate(
  bus: BrainBus,
  stackFor: (mogId: MogId) => Pick<CardStack, 'input'>,
  card: { hideCard?: (mogId: MogId) => void } = {},
): () => void {
  return onViewInput(bus, (input) => {
    // Only the card of the MOG speaks for its chips and hover.
    if (input.from !== `card-${input.mogId}`) return;
    // UX-19b, #559: a cancel hides the card (the pop-up's fade, Copy and Dismiss).
    // TODO(#374): an in-flight request is cancelled first.
    if (input.kind === 'cancel') card.hideCard?.(input.mogId);
    else if (input.kind === 'chip_click') stackFor(input.mogId).input('chip_click', input.payload);
    else if (input.kind === 'hover') stackFor(input.mogId).input('hover', input.payload);
  });
}
