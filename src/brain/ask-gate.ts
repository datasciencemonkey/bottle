/**
 * The T3 ask gate (#142, ADR 0086): the 1.5 s preview and the Ask Genie chip of a PTT or
 * hands-free question. Binding rule B1: nothing goes to Genie without
 * the cancelable preview or a click on an Ask Genie chip.
 *
 * - RTR-04a: a `preview` route (#59) shows the Appendix E Preview: `"<question>"`,
 *   "Sending in 1.5 s" and [Send now] [Edit] [Cancel]. RTR-17b: a forced ask previews too.
 * - RTR-04b: when the 1.5 s end with no cancel, the question goes out through the one hand-off
 *   seam `send(turn)`. ADR 0097 (#573): in `startBrain` that seam sends it straight to Genie One
 *   MCP (T8 `submit`, `genie_ask`); the main agent takes no part. Before the send, the G-47
 *   sanitizer and then the G-30a cap run on the text. [Cancel], Esc in the card, a second talk-key
 *   press, the card close and a closed MOG cancel: nothing is sent. [Edit] stops the timer and
 *   holds the question with [Send now] [Cancel]; it never sends by itself. TODO(#526): the
 *   card text box for the edit.
 * - RTR-05: a `chip` route (confidence below 0.7, or a session question, RTR-07) shows the
 *   Appendix E chip 1 "Ask Genie" (2 min or 3 newer). Only a click on it sends (UX-20c).
 * - RTR-14: an injection-suspected route shows the fixed warning chip of ADR 0056. It has no
 *   action: a click sends nothing.
 * - A missing target profile, a closed MOG or a failed hand-off sends nothing and shows
 *   "Not sent." No retry runs by itself.
 * - #557 (ADR 0096): with `pickProfile`, a press on Ask Genie with no target profile opens the
 *   profile picker and sends nothing. The press is the confirmation: when the user clicks a
 *   profile, the same text goes out once. Bottle never picks. With no target, a preview route is
 *   a chip, so only a press can start the picker.
 *
 * The gate never calls `mcp_call` or `agent_*` itself: `send` is its only way out.
 * ADR 0097 amends ADR 0086 here: the seam is the direct Genie One send of `genie-direct.ts`.
 * TODO(#200): the RecallGate hits go in `AgentTurn.recall`; until then it is empty.
 * TODO(#374): one owner routes all card input; until then `wireAskGate` routes the card's
 * `cancel` and `chip_click`.
 */
import type { AgentTurn, MogId, ProfileName, RouterDecision } from '../shared/interfaces';
import { EVENTS } from '../shared/ipc';
import { onViewInput, type BrainBus } from './bus';
import { CHIP_CATALOGUE, type ChipAction, type ChipView } from './card-chips';
import type { CardStack, Scheduler } from './card-stack';
import type { MonotonicClock } from './clock';
import { INJECTION_WARNING_CHIP } from './components/router-injection';
import { asPttEvent } from './ptt';
import type { OwnedUtterance, RequestOwner } from './request-owner';
import { sanitizeQuestion } from './question-sanitizer';
import type { AskRoute } from './utterance-wiring';

/** Appendix E chip 1. */
export const ASK_CHIP_LABEL = 'Ask Genie';
/** Appendix E Preview, line 2. */
export const PREVIEW_LINE = 'Sending in 1.5 s';
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

/** The Appendix E Preview controls, in order. */
export const PREVIEW_CONTROLS = [
  { id: 'preview_send_now', label: 'Send now' },
  { id: 'preview_edit', label: 'Edit' },
  { id: 'preview_cancel', label: 'Cancel' },
] as const satisfies readonly ChipView[];
const [SEND_NOW, , CANCEL] = PREVIEW_CONTROLS;
/** After [Edit]: the question waits for [Send now] or [Cancel]. */
const HELD_CONTROLS: ChipView[] = [{ ...SEND_NOW }, { ...CANCEL }];

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
  /** `window_card_show`, so the preview or the chip is seen. */
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
  /** The `onAsk` of the utterance hook (#59): a preview or a chip, with its live owner. */
  onAsk(r: AskRoute, owned: OwnedUtterance): void;
  /** The card stack's `onChip`: a click on a live chip of `mogId` (UX-20c). */
  chip(mogId: MogId, action: ChipAction): void;
  /** Esc in the card of `mogId`. True when it cancelled a preview (UX-19b: one thing per Esc). */
  escape(mogId: MogId): boolean;
  /** A talk-key press: every preview is cancelled. */
  keyDown(): void;
  /**
   * The card of `mogId` closed: its preview is cancelled. TODO(#374): no card-hide signal reaches
   * the brain yet, so nothing calls it in `startBrain`; a closed MOG is caught when the timer ends.
   */
  cardClosed(mogId: MogId): void;
  /** The preview state of `mogId`, if one is open. */
  pending(mogId: MogId): 'counting' | 'held' | undefined;
}

const quoted = (q: string) => `"${q}"`;
/** Appendix E Preview, lines 1 and 2. */
export const previewText = (q: string) => `${quoted(q)}\n${PREVIEW_LINE}`;
const notSentText = (q: string) => `${quoted(q)}\n${NOT_SENT_LINE}`;

interface Preview {
  owned: OwnedUtterance;
  question: string;
  decision: RouterDecision;
  state: 'counting' | 'held';
  blurbId: string;
  stopTimer?: (() => void) | undefined;
}

interface Offer {
  mogId: MogId;
  question: string;
  decision: RouterDecision;
  offeredAt: number;
  blurbId: string;
}

const firstNonEmpty = (...xs: string[]) => xs.find((x) => x.trim() !== '') ?? '';

export function createAskGate(deps: AskGateDeps): AskGate {
  const previews = new Map<MogId, Preview>();
  /** The live Ask Genie chips, by chip ID, oldest first. */
  const offers = new Map<string, Offer>();

  const show = (mogId: MogId) => {
    try {
      void Promise.resolve(deps.showCard(mogId)).catch(() => undefined);
    } catch {
      // The card stays hidden; the preview still times out and the user can still cancel.
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

  /** Close a preview: no timer, no controls, no owner. */
  const end = (p: Preview) => {
    p.stopTimer?.();
    p.stopTimer = undefined;
    previews.delete(p.owned.mogId);
    deps.cardFor(p.owned.mogId).clearControls(p.blurbId);
    deps.owner.release(p.owned.utteranceId);
  };
  const live = (p: Preview) => previews.get(p.owned.mogId) === p;

  const cancel = (p: Preview) => {
    if (!live(p)) return;
    end(p);
    deps
      .cardFor(p.owned.mogId)
      .add({ id: p.blurbId, text: notSentText(p.question), completed: true });
  };
  const sendNow = (p: Preview) => {
    if (!live(p)) return;
    end(p);
    handOff(p.owned.mogId, p.question, p.decision, p.blurbId);
  };
  const hold = (p: Preview) => {
    if (!live(p) || p.state === 'held') return;
    p.stopTimer?.();
    p.stopTimer = undefined;
    p.state = 'held';
    const card = deps.cardFor(p.owned.mogId);
    card.add({ id: p.blurbId, text: quoted(p.question) });
    card.setControls(p.blurbId, HELD_CONTROLS, (id) => control(p, id));
  };
  const control = (p: Preview, id: string) => {
    if (id === 'preview_send_now') sendNow(p);
    else if (id === 'preview_edit') hold(p);
    else if (id === 'preview_cancel') cancel(p);
  };

  const startPreview = (r: Extract<AskRoute, { route: 'preview' }>, owned: OwnedUtterance) => {
    const p: Preview = {
      owned,
      question: r.question,
      decision: r.decision,
      state: 'counting',
      blurbId: `preview:${owned.utteranceId}`,
    };
    previews.set(owned.mogId, p);
    const card = deps.cardFor(owned.mogId);
    card.add({ id: p.blurbId, text: previewText(r.question) });
    card.setControls(
      p.blurbId,
      PREVIEW_CONTROLS.map((c) => ({ ...c })),
      (id) => control(p, id),
    );
    p.stopTimer = deps.schedule(deps.clock.now() + r.previewMs, () => {
      p.stopTimer = undefined;
      // A MOG that closed during the preview gets nothing sent (the card close).
      if (!deps.isOpen(owned.mogId)) return cancel(p);
      sendNow(p);
    });
    show(owned.mogId);
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
      // One preview per MOG: a new ask replaces the old preview, which never sends.
      const old = previews.get(owned.mogId);
      if (old) cancel(old);
      // RTR-14, twice: a flagged decision never previews, also if a route says `preview`.
      // #557: with no target and a picker, a preview is a chip; only a press starts the picker.
      const canPreview = !deps.pickProfile || deps.target(owned.mogId) !== undefined;
      if (r.route === 'preview' && !r.decision.injection_suspected && canPreview)
        return startPreview(r, owned);
      // A chip holds no owner: the click names its MOG.
      deps.owner.release(owned.utteranceId);
      offerChip(
        r.route === 'chip'
          ? r
          : {
              route: 'chip',
              decision: { ...r.decision, genie_question: r.question },
              warning: r.decision.injection_suspected ? 'injection_suspected' : undefined,
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
    escape(mogId) {
      const p = previews.get(mogId);
      if (!p) return false;
      cancel(p);
      return true;
    },
    keyDown() {
      for (const p of [...previews.values()]) cancel(p);
    },
    cardClosed(mogId) {
      const p = previews.get(mogId);
      if (p) cancel(p);
    },
    pending: (mogId) => previews.get(mogId)?.state,
  };
}

/**
 * Feed the gate from the brain Channel: Esc (`view:input` `cancel`) and chip clicks from the
 * card of the MOG, and every `hotkey:ptt` down (a second key press cancels). Returns `off`.
 */
export function wireAskGate(
  bus: BrainBus,
  gate: AskGate,
  stackFor: (mogId: MogId) => Pick<CardStack, 'input'>,
  card: { hideCard?: (mogId: MogId) => void } = {},
): () => void {
  const offs = [
    onViewInput(bus, (input) => {
      // Only the card of the MOG speaks for its preview and its chips.
      if (input.from !== `card-${input.mogId}`) return;
      // UX-19b: one cancel does one thing. It ends a preview, else it hides the card (#559: the
      // pop-up's fade, Copy and Dismiss). TODO(#374): an in-flight request is cancelled first.
      if (input.kind === 'cancel') {
        if (!gate.escape(input.mogId)) card.hideCard?.(input.mogId);
      } else if (input.kind === 'chip_click')
        stackFor(input.mogId).input('chip_click', input.payload);
    }),
    bus.on(EVENTS['hotkey.ptt'].name, (p) => {
      if (asPttEvent(p, 0)?.phase === 'down') gate.keyDown();
    }),
  ];
  return () => offs.forEach((off) => off());
}
