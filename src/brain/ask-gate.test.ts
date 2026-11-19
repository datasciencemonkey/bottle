/**
 * #142 (ADR 0086): the T3 preview and chip step of a PTT or hands-free question.
 * Nothing goes to the main agent (and so to Genie) without the 1.5 s preview or a chip click (B1).
 */
import type { AgentTurn } from '../shared/interfaces';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { routerDecision } from '../test-support/model-invoke';
import {
  ASK_CHIP_LABEL,
  CUT_NOTICE,
  QUESTION_MAX_CHARS,
  NOT_SENT_LINE,
  PREVIEW_CONTROLS,
  createAskGate,
  previewText,
  type AskCard,
} from './ask-gate';
import type { ChipInput, ChipView } from './card-chips';
import { INJECTION_WARNING_CHIP } from './components/router-injection';
import type { BlurbInput } from './blurb-stack';
import type { OwnedUtterance } from './request-owner';
import { PREVIEW_MS, type UtteranceRoute } from './utterance-route';
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

const preview = (over: Partial<Extract<UtteranceRoute, { route: 'preview' }>> = {}): AskRoute => ({
  route: 'preview',
  question: Q,
  forced: false,
  previewMs: PREVIEW_MS,
  decision: decision(),
  ...over,
});

const chip = (over: Partial<Extract<UtteranceRoute, { route: 'chip' }>> = {}): AskRoute => ({
  route: 'chip',
  decision: decision({ confidence: 0.5 }),
  warning: undefined,
  ...over,
});

const labels = (vs: ChipView[]) => vs.map((v) => v.label);

describe('ask gate: the 1.5 s preview (RTR-04a, RTR-04b)', () => {
  it('[RTR-04a] [UX-17] shows the Appendix E preview "Sending in 1.5 s" with [Send now] [Edit] [Cancel]', () => {
    const t = setup();
    const u = t.owned();
    t.gate.onAsk(preview(), u);
    const card = t.cardOf(A);
    expect([...card.blurbs.values()].map((b) => b.text)).toEqual([`"${Q}"\nSending in 1.5 s`]);
    expect(previewText(Q)).toBe(`"${Q}"\nSending in 1.5 s`);
    expect(labels(card.shownControls())).toEqual(['Send now', 'Edit', 'Cancel']);
    expect(PREVIEW_CONTROLS.map((c) => c.label)).toEqual(['Send now', 'Edit', 'Cancel']);
    expect(t.shown).toEqual([A]);
    expect(t.gate.pending(A)).toBe('counting');
  });

  it('[RTR-04b] gives nothing to the main agent before 1.5 s, and the question once at 1.5 s', async () => {
    const t = setup();
    const u = t.owned();
    const r = preview();
    t.gate.onAsk(r, u);
    await t.run(PREVIEW_MS - 1);
    expect(t.sent).toEqual([]);
    expect(t.released).toEqual([]);
    await t.run(1);
    expect(t.sent).toEqual([
      { mogId: A, question: Q, decision: r.decision, recall: [], target: 'home' },
    ]);
    expect(t.released).toEqual([u.utteranceId]);
    expect(t.cardOf(A).shownControls()).toEqual([]);
    expect(t.gate.pending(A)).toBeUndefined();
    await t.run(10 * PREVIEW_MS);
    expect(t.sent).toHaveLength(1);
  });

  it('[RTR-17b] a forced ask (Shift) keeps the 1.5 s preview', async () => {
    const t = setup();
    t.gate.onAsk(preview({ forced: true, decision: decision({ confidence: 0.1 }) }), t.owned());
    expect(t.gate.pending(A)).toBe('counting');
    await t.run(PREVIEW_MS - 1);
    expect(t.sent).toEqual([]);
    await t.run(1);
    expect(t.sent).toHaveLength(1);
  });

  it('[RTR-04a] [Send now] sends at once, and only once', async () => {
    const t = setup();
    t.gate.onAsk(preview(), t.owned());
    t.cardOf(A).clickControl('preview_send_now');
    await t.run(0);
    expect(t.sent).toHaveLength(1);
    await t.run(2 * PREVIEW_MS);
    t.cardOf(A).clickControl('preview_send_now');
    await t.run(0);
    expect(t.sent).toHaveLength(1);
  });

  it('[RTR-04a] [Edit] stops the timer and holds the question with [Send now] [Cancel]; nothing sends by itself', async () => {
    const t = setup();
    t.gate.onAsk(preview(), t.owned());
    t.cardOf(A).clickControl('preview_edit');
    expect(t.gate.pending(A)).toBe('held');
    expect(labels(t.cardOf(A).shownControls())).toEqual(['Send now', 'Cancel']);
    expect([...t.cardOf(A).blurbs.values()][0]?.text).toBe(`"${Q}"`);
    await t.run(60 * PREVIEW_MS);
    expect(t.sent).toEqual([]);
    t.cardOf(A).clickControl('preview_send_now');
    await t.run(0);
    expect(t.sent).toHaveLength(1);
  });
});

describe('ask gate: a cancel sends nothing (RTR-04b)', () => {
  const cancels: [string, (t: ReturnType<typeof setup>) => void][] = [
    ['[Cancel]', (t) => t.cardOf(A).clickControl('preview_cancel')],
    ['Esc in the card', (t) => expect(t.gate.escape(A)).toBe(true)],
    ['a second talk-key press', (t) => t.gate.keyDown()],
    ['the card close', (t) => t.gate.cardClosed(A)],
  ];
  for (const [name, cancel] of cancels) {
    it(`[RTR-04b] ${name} during the preview sends nothing`, async () => {
      const t = setup();
      const u = t.owned();
      t.gate.onAsk(preview(), u);
      await t.run(PREVIEW_MS / 2);
      cancel(t);
      expect(t.gate.pending(A)).toBeUndefined();
      expect(t.released).toEqual([u.utteranceId]);
      await t.run(10 * PREVIEW_MS);
      expect(t.sent).toEqual([]);
      const card = t.cardOf(A);
      expect(card.shownControls()).toEqual([]);
      expect([...card.blurbs.values()][0]).toMatchObject({
        text: `"${Q}"\n${NOT_SENT_LINE}`,
        completed: true,
      });
      // A late click on a stale control does nothing.
      card.clickControl('preview_send_now');
      await t.run(0);
      expect(t.sent).toEqual([]);
    });
  }

  it('[RTR-04b] [Edit] then Esc sends nothing', async () => {
    const t = setup();
    t.gate.onAsk(preview(), t.owned());
    t.cardOf(A).clickControl('preview_edit');
    expect(t.gate.escape(A)).toBe(true);
    await t.run(10 * PREVIEW_MS);
    expect(t.sent).toEqual([]);
  });

  it('[RTR-04b] a MOG that closed during the preview gets nothing sent', async () => {
    const t = setup();
    t.gate.onAsk(preview(), t.owned());
    t.close();
    await t.run(PREVIEW_MS);
    expect(t.sent).toEqual([]);
    expect(t.gate.pending(A)).toBeUndefined();
  });

  it('[RTR-04b] with no chosen target profile nothing is sent (Bottle never picks one)', async () => {
    const t = setup({ target: undefined });
    t.gate.onAsk(preview(), t.owned());
    await t.run(PREVIEW_MS);
    expect(t.sent).toEqual([]);
    expect([...t.cardOf(A).blurbs.values()][0]?.text).toBe(`"${Q}"\n${NOT_SENT_LINE}`);
  });

  it('[UX-19b] one Esc cancels only the preview of its own MOG; with no preview it does nothing', async () => {
    const t = setup();
    t.gate.onAsk(preview(), t.owned(A));
    t.gate.onAsk(preview(), t.owned(B));
    expect(t.gate.escape(B)).toBe(true);
    expect(t.gate.escape(B)).toBe(false);
    await t.run(PREVIEW_MS);
    expect(t.sent.map((s) => s.mogId)).toEqual([A]);
  });

  it('[RTR-04b] a new ask for the same MOG replaces the old preview, which never sends', async () => {
    const t = setup();
    const first = t.owned();
    t.gate.onAsk(preview({ question: 'old question?' }), first);
    await t.run(PREVIEW_MS / 2);
    t.gate.onAsk(preview(), t.owned());
    expect(t.released).toEqual([first.utteranceId]);
    await t.run(PREVIEW_MS);
    expect(t.sent.map((s) => s.question)).toEqual([Q]);
  });
});

describe('ask gate: the Ask Genie chip (RTR-05)', () => {
  it('[RTR-05] a low-confidence question shows the Ask Genie chip and no preview; nothing goes out before the click', async () => {
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
    expect(t.gate.pending(A)).toBeUndefined();
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
    await t.run(10 * PREVIEW_MS);
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
    await t.run(10 * PREVIEW_MS);
    expect(t.sent).toEqual([]);
    expect(t.gate.pending(A)).toBeUndefined();
  });

  it('[RTR-14] a preview route with an injection-suspected decision shows the warning, never the preview', async () => {
    const t = setup();
    t.gate.onAsk(preview({ decision: decision({ injection_suspected: true }) }), t.owned());
    expect(t.gate.pending(A)).toBeUndefined();
    expect(t.cardOf(A).shownControls()).toEqual([{ ...INJECTION_WARNING_CHIP }]);
    await t.run(10 * PREVIEW_MS);
    expect(t.sent).toEqual([]);
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
    gate.onAsk(preview(), { utteranceId: 'u1', mogId: A });
    await sim.runUntil(10 * PREVIEW_MS, { settle: true });
    expect(calls).toBe(1);
    expect([...card.blurbs.values()][0]?.text).toBe(`"${Q}"\n${NOT_SENT_LINE}`);
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

describe('ask gate: no profile, Shift, size (#557, ADR 0096)', () => {
  it('[#557] [RTR-17b] with no profile and a picker, a Shift preview is a chip of the previewed text; nothing sends by itself', async () => {
    const t = setup({ target: undefined, pick: true });
    t.gate.onAsk(
      preview({ question: 'shift text', decision: decision({ genie_question: '' }) }),
      t.owned(),
    );
    expect(t.gate.pending(A)).toBeUndefined();
    expect(t.cardOf(A).shownControls()).toEqual([]);
    await t.run(PREVIEW_MS * 4);
    expect(t.sent).toEqual([]);
    expect(t.picks).toEqual([]);
    const id = [...t.cardOf(A).chips.keys()][0] ?? '';
    t.gate.chip(A, { id, kind: 'ask_genie' });
    t.pickDone('picked');
    await t.run(0);
    expect(t.sent.map((x) => x.question)).toEqual(['shift text']);
  });

  it('[#557] [RTR-14] with no profile, an injection-suspected Shift ask is still only the warning chip', async () => {
    const t = setup({ target: undefined, pick: true });
    t.gate.onAsk(preview({ decision: decision({ injection_suspected: true }) }), t.owned());
    expect([...t.cardOf(A).chips.values()]).toEqual([]);
    expect(t.cardOf(A).shownControls()).toEqual([{ ...INJECTION_WARNING_CHIP }]);
  });

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
    expect([...t.cardOf(A).blurbs.values()].at(-1)?.text).toContain(NOT_SENT_LINE);
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
