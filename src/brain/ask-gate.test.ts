/**
 * #142 (ADR 0086), ADR 0098: the T3 ask step of a PTT or hands-free utterance. Nothing goes to
 * Genie without a user action (B1): ⌥G during the talk-key hold, or a press on an Ask Genie chip.
 */
import type { AgentTurn } from '../shared/interfaces';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { routerDecision } from '../test-support/model-invoke';
import {
  ASK_CHIP_LABEL,
  CUT_NOTICE,
  QUESTION_MAX_CHARS,
  NOT_SENT_LINE,
  createAskGate,
  type AskCard,
} from './ask-gate';
import type { ChipInput, ChipView } from './card-chips';
import { INJECTION_WARNING_CHIP } from './components/router-injection';
import type { BlurbInput } from './blurb-stack';
import type { OwnedUtterance } from './request-owner';
import type { UtteranceRoute } from './utterance-route';
import type { AskRoute } from './utterance-wiring';

const A = '00000000-0000-4000-8000-0000000000a1';
const B = '00000000-0000-4000-8000-0000000000b2';
const Q = 'What were EMEA bookings last week?';

/** A card double that keeps the state the gate sets. */
function fakeCard() {
  const blurbs = new Map<string, BlurbInput>();
  const controls = new Map<string, { list: ChipView[]; onClick?: (id: string) => void }>();
  const chips = new Map<string, ChipInput>();
  const card: AskCard = {
    add: (b) => void blurbs.set(b.id, { ...(blurbs.get(b.id) ?? {}), ...b }),
    setControls: (owner, list, onClick) =>
      void controls.set(owner, onClick ? { list, onClick } : { list }),
    clearControls: (owner) => void controls.delete(owner),
    offerChip: (c) => void chips.set(c.id, c),
    removeChip: (id) => void chips.delete(id),
  };
  const shownControls = () => [...controls.values()].flatMap((c) => c.list);
  /** A click on a control, as the card stack routes it. */
  const clickControl = (id: string) => {
    for (const c of controls.values()) if (c.list.some((x) => x.id === id)) c.onClick?.(id);
  };
  return { card, blurbs, controls, chips, shownControls, clickControl };
}

function setup(opts: { open?: boolean; target?: string | undefined; pick?: boolean } = {}) {
  const sim = createSimScheduler();
  const cards = new Map<string, ReturnType<typeof fakeCard>>();
  const cardOf = (id: string) => {
    let c = cards.get(id);
    if (!c) cards.set(id, (c = fakeCard()));
    return c;
  };
  const sent: AgentTurn[] = [];
  const released: string[] = [];
  const shown: string[] = [];
  let open = opts.open ?? true;
  let target = 'target' in opts ? opts.target : 'home';
  /** #557: the profile picker calls. `pickDone` saves the user's pick, then runs the callback. */
  const picks: { mogId: string; then: () => void }[] = [];
  const pickDone = (profile: string) => {
    target = profile;
    for (const p of picks.splice(0)) p.then();
  };
  const gate = createAskGate({
    clock: sim.clock,
    schedule: sim.at,
    cardFor: (id) => cardOf(id).card,
    showCard: (id) => void shown.push(id),
    owner: { release: (u) => void released.push(u) },
    isOpen: () => open,
    target: () => target,
    send: async (t) => void sent.push(t),
    ...(opts.pick ? { pickProfile: (mogId, then) => void picks.push({ mogId, then }) } : {}),
  });
  let n = 0;
  const owned = (mogId = A): OwnedUtterance => ({ utteranceId: `u${++n}`, mogId });
  return {
    sim,
    gate,
    cardOf,
    sent,
    released,
    shown,
    owned,
    close: () => void (open = false),
    picks,
    pickDone,
    run: (ms: number) => sim.runUntil(sim.clock.now() + ms, { settle: true }),
  };
}

const decision = (over: Parameters<typeof routerDecision>[0] = {}) =>
  routerDecision({ genie_question: Q, cleaned_text: Q, confidence: 0.9, ...over });

/** ADR 0098: the route of an utterance with ⌥G (`hotkey:ptt` up, `ask: true`). */
const ask = (over: Partial<Extract<UtteranceRoute, { route: 'ask' }>> = {}): AskRoute => ({
  route: 'ask',
  question: Q,
  decision: decision({ genie_question: 'Rewritten?', confidence: 0.1 }),
  ...over,
});

const chip = (over: Partial<Extract<UtteranceRoute, { route: 'chip' }>> = {}): AskRoute => ({
  route: 'chip',
  decision: decision({ confidence: 0.5 }),
  warning: undefined,
  ...over,
});

/** Longer than any timer of the gate but the 2 min chip expiry. */
const LATER_MS = 15_000;

describe('ask gate: ⌥G sends at once (RTR-17a, RTR-17b, ADR 0098)', () => {
  it('[RTR-17a] [RTR-17b] [RTR-04b] an ⌥G ask sends the exact text once, at once, with no preview', async () => {
    const t = setup();
    const u = t.owned();
    const r = ask();
    t.gate.onAsk(r, u);
    await t.run(0);
    // The exact text, not the router's rewrite, goes through the send seam (ADR 0097).
    expect(t.sent).toEqual([
      { mogId: A, question: Q, decision: r.decision, recall: [], target: 'home' },
    ]);
    const card = t.cardOf(A);
    // No preview: no "Sending in 1.5 s" line, no controls and no chip.
    expect([...card.blurbs.values()].map((b) => b.text)).toEqual([`"${Q}"`]);
    expect(card.shownControls()).toEqual([]);
    expect(card.chips.size).toBe(0);
    expect(t.released).toEqual([u.utteranceId]);
    expect(t.shown).toEqual([A]);
    await t.run(LATER_MS);
    expect(t.sent).toHaveLength(1);
  });

  it('[RTR-17a] [W-07d] with no profile an ⌥G ask opens the picker; the pick sends the text once', async () => {
    const t = setup({ target: undefined, pick: true });
    t.gate.onAsk(ask(), t.owned());
    await t.run(0);
    expect(t.picks.map((p) => p.mogId)).toEqual([A]);
    expect(t.sent).toEqual([]);
    t.pickDone('picked');
    await t.run(0);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ mogId: A, question: Q, target: 'picked' });
  });

  it('[RTR-17a] with no chosen target profile and no picker nothing is sent (Bottle never picks one)', async () => {
    const t = setup({ target: undefined });
    t.gate.onAsk(ask(), t.owned());
    await t.run(LATER_MS);
    expect(t.sent).toEqual([]);
    expect([...t.cardOf(A).blurbs.values()][0]?.text).toBe(`"${Q}"\n${NOT_SENT_LINE}`);
  });

  it('[RTR-17a] an ⌥G ask of a closed MOG sends nothing', async () => {
    const t = setup({ open: false });
    t.gate.onAsk(ask(), t.owned());
    await t.run(LATER_MS);
    expect(t.sent).toEqual([]);
  });

  it('[RTR-14] an ⌥G ask with an injection-suspected decision shows the warning and never sends', async () => {
    const t = setup();
    t.gate.onAsk(ask({ decision: decision({ injection_suspected: true }) }), t.owned());
    await t.run(LATER_MS);
    expect(t.sent).toEqual([]);
    expect(t.cardOf(A).shownControls()).toEqual([{ ...INJECTION_WARNING_CHIP }]);
  });

  it('[G-47] [G-30a] an ⌥G ask goes through the sanitizer and the cap', async () => {
    const t = setup();
    t.gate.onAsk(
      ask({ question: 'What was\u200b [EMEA](https://evil.example.test) revenue?' }),
      t.owned(),
    );
    await t.run(0);
    expect(t.sent.map((x) => x.question)).toEqual(['What was EMEA revenue?']);
  });

  it('[RTR-04b] a failed hand-off is shown as not sent and is not retried', async () => {
    const sim = createSimScheduler();
    const card = fakeCard();
    let calls = 0;
    const gate = createAskGate({
      clock: sim.clock,
      schedule: sim.at,
      cardFor: () => card.card,
      showCard: () => undefined,
      owner: { release: () => undefined },
      isOpen: () => true,
      target: () => 'home',
      send: async () => {
        calls++;
        throw new Error('session unavailable');
      },
    });
    gate.onAsk(ask(), { utteranceId: 'u1', mogId: A });
    await sim.runUntil(LATER_MS, { settle: true });
    expect(calls).toBe(1);
    expect([...card.blurbs.values()][0]?.text).toBe(`"${Q}"\n${NOT_SENT_LINE}`);
  });
});

describe('ask gate: the Ask Genie fallback chip (RTR-05, ADR 0098)', () => {
  it('[RTR-05] a dictation shows the Ask Genie fallback chip; nothing goes out before the press', async () => {
    const t = setup();
    const u = t.owned();
    t.gate.onAsk(chip(), u);
    const card = t.cardOf(A);
    const chips = [...card.chips.values()];
    expect(chips).toHaveLength(1);
    const chipId = chips[0]?.id ?? '';
    expect(chips[0]).toMatchObject({ kind: 'ask_genie', label: ASK_CHIP_LABEL });
    expect(ASK_CHIP_LABEL).toBe('Ask Genie');
    expect(card.shownControls()).toEqual([]);
    // The chip holds no owner: the request owner is released now.
    expect(t.released).toEqual([u.utteranceId]);
    await t.run(60_000);
    expect(t.sent).toEqual([]);
    t.gate.chip(A, { id: chipId, kind: 'ask_genie' });
    await t.run(0);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ mogId: A, question: Q, target: 'home', recall: [] });
    expect(card.chips.size).toBe(0);
    // A second click does nothing.
    t.gate.chip(A, { id: chipId, kind: 'ask_genie' });
    await t.run(0);
    expect(t.sent).toHaveLength(1);
  });

  it('[RTR-05] a chip click after the 2 min expiry sends nothing', async () => {
    const t = setup();
    t.gate.onAsk(chip(), t.owned());
    const id = [...t.cardOf(A).chips.keys()][0] ?? '';
    await t.run(2 * 60_000);
    t.gate.chip(A, { id, kind: 'ask_genie' });
    await t.run(0);
    expect(t.sent).toEqual([]);
  });

  it('[RTR-05] a click on a chip of another MOG, or an unknown chip, sends nothing', async () => {
    const t = setup();
    t.gate.onAsk(chip(), t.owned(A));
    const id = [...t.cardOf(A).chips.keys()][0] ?? '';
    t.gate.chip(B, { id, kind: 'ask_genie' });
    t.gate.chip(A, { id: 'nope', kind: 'ask_genie' });
    await t.run(0);
    expect(t.sent).toEqual([]);
  });

  it('[RTR-07] a session question is a chip: no preview and no send before the click', async () => {
    const t = setup();
    t.gate.onAsk(chip({ decision: decision({ confidence: 0.95 }) }), t.owned());
    await t.run(LATER_MS);
    expect(t.sent).toEqual([]);
    expect(t.cardOf(A).chips.size).toBe(1);
  });

  it('[RTR-14] an injection-suspected utterance shows the warning chip and never sends, also on a click', async () => {
    const t = setup();
    t.gate.onAsk(chip({ warning: 'injection_suspected' }), t.owned());
    const card = t.cardOf(A);
    expect(card.chips.size).toBe(0);
    expect(card.shownControls()).toEqual([{ ...INJECTION_WARNING_CHIP }]);
    card.clickControl(INJECTION_WARNING_CHIP.id);
    await t.run(LATER_MS);
    expect(t.sent).toEqual([]);
  });
});

describe('ask gate: the Ask Genie press of a dictated question (#557, ADR 0096)', () => {
  it('[#557] [B1] nothing sends without a press, also after the chip expires', async () => {
    const t = setup();
    t.gate.onAsk(chip(), t.owned());
    await t.run(10 * 60_000);
    expect(t.sent).toEqual([]);
  });

  it('[#557] a press sends the exact text of the chip', async () => {
    const t = setup();
    const exact = 'what were emea bookings last week';
    t.gate.onAsk(
      chip({ decision: decision({ genie_question: exact, cleaned_text: exact }) }),
      t.owned(),
    );
    const id = [...t.cardOf(A).chips.keys()][0] ?? '';
    t.gate.chip(A, { id, kind: 'ask_genie' });
    await t.run(0);
    expect(t.sent.map((x) => x.question)).toEqual([exact]);
  });

  it('[#557] [W-07d] with no profile a press opens the profile picker and sends nothing; the pick then sends the text', async () => {
    const t = setup({ target: undefined, pick: true });
    t.gate.onAsk(chip(), t.owned());
    const id = [...t.cardOf(A).chips.keys()][0] ?? '';
    t.gate.chip(A, { id, kind: 'ask_genie' });
    await t.run(0);
    expect(t.picks.map((p) => p.mogId)).toEqual([A]);
    expect(t.sent).toEqual([]);
    expect([...t.cardOf(A).blurbs.values()].some((b) => b.text?.includes(NOT_SENT_LINE))).toBe(
      false,
    );
    // The user clicks a profile. The press was the confirmation, so the same text goes out once.
    t.pickDone('picked');
    await t.run(0);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ mogId: A, question: Q, target: 'picked' });
  });
});

describe('ask gate: size and sanitizer of a press (#557, ADR 0096)', () => {
  it('[#557] [G-30a] a pressed text over 1,000 chars is cut at a word edge before the send, with a notice', async () => {
    const t = setup();
    const long = `${'word '.repeat(300)}end`;
    t.gate.onAsk(
      chip({ decision: decision({ genie_question: long, cleaned_text: long }) }),
      t.owned(),
    );
    const id = [...t.cardOf(A).chips.keys()][0] ?? '';
    t.gate.chip(A, { id, kind: 'ask_genie' });
    await t.run(0);
    const q = t.sent[0]?.question ?? '';
    expect(q.length).toBeLessThanOrEqual(QUESTION_MAX_CHARS);
    expect(q.endsWith('word')).toBe(true);
    expect(long.startsWith(q)).toBe(true);
    expect([...t.cardOf(A).blurbs.values()].some((b) => b.text?.includes(CUT_NOTICE))).toBe(true);
  });

  it('[G-47] [G-30a] [#573] the press sends the sanitized text, and the cap runs after the sanitizer', async () => {
    const t = setup();
    const dirty = `${'<br>'.repeat(300)}What was\u200b [EMEA](https://evil.example.test) revenue?`;
    t.gate.onAsk(
      chip({ decision: decision({ genie_question: dirty, cleaned_text: dirty }) }),
      t.owned(),
    );
    const id = [...t.cardOf(A).chips.keys()][0] ?? '';
    t.gate.chip(A, { id, kind: 'ask_genie' });
    await t.run(0);
    // Over 1,000 chars before the sanitizer, short after it: so it is not cut.
    expect(dirty.length).toBeGreaterThan(QUESTION_MAX_CHARS);
    expect(t.sent.map((s) => s.question)).toEqual(['What was EMEA revenue?']);
    const texts = [...t.cardOf(A).blurbs.values()].map((b) => b.text);
    expect(texts).toContain('"What was EMEA revenue?"');
    expect(texts.some((x) => x.includes(CUT_NOTICE))).toBe(false);
  });

  it('[G-47] [#573] a press whose text is empty after the sanitizer sends nothing', async () => {
    const t = setup();
    const junk = '<br>\u200b```';
    t.gate.onAsk(
      chip({ decision: decision({ genie_question: junk, cleaned_text: junk }) }),
      t.owned(),
    );
    const id = [...t.cardOf(A).chips.keys()][0] ?? '';
    t.gate.chip(A, { id, kind: 'ask_genie' });
    await t.run(0);
    expect(t.sent).toEqual([]);
    // #575 review: the card does not show the removed markup again.
    expect([...t.cardOf(A).blurbs.values()].at(-1)?.text).toBe(NOT_SENT_LINE);
  });

  it('[#557] [RTR-14] an injection-suspected question shows its own text with the warning', () => {
    const t = setup();
    t.gate.onAsk(
      chip({ decision: decision({ injection_suspected: true }), warning: 'injection_suspected' }),
      t.owned(),
    );
    expect([...t.cardOf(A).blurbs.values()].map((b) => b.text)).toEqual([`"${Q}"`]);
  });
});
