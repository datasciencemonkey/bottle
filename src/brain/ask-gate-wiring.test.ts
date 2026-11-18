/**
 * #142 (ADR 0086), end to end on the brain bus: the card input, the talk key and the T15 hand-off.
 * The check is the IPC itself: no `agent_send` (the main agent) and no `mcp_call` (Genie) goes out
 * before 1.5 s, after a cancel (Esc, a second key press, [Cancel], the card close), or without a
 * chip click (B1, RTR-04b, RTR-05).
 */
import { createSimScheduler } from '../test-support/sim-scheduler';
import { routerDecision } from '../test-support/model-invoke';
import { createAskGate, wireAskGate, type AskGate } from './ask-gate';
import { BrainBus } from './bus';
import { createCardStack, type CardStack, type CardStackUpdate } from './card-stack';
import { createAgentBridge } from './components/agent-bridge';
import { core } from './test-core';
import { PREVIEW_MS } from './utterance-route';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const A = '00000000-0000-4000-8000-0000000000a1';
const Q = 'What were EMEA bookings last week?';
const OUTBOUND = new Set(['agent_send', 'mcp_call']);

function setup() {
  core.reset();
  const sim = createSimScheduler();
  const bus = new BrainBus();
  const updates: CardStackUpdate[] = [];
  const stacks = new Map<string, CardStack>();
  const bridge = createAgentBridge();
  let open = true;
  // eslint-disable-next-line prefer-const
  let gate: AskGate;
  const stackFor = (mogId: string): CardStack => {
    let s = stacks.get(mogId);
    if (!s)
      stacks.set(
        mogId,
        (s = createCardStack(mogId, {
          clock: sim.clock,
          schedule: sim.at,
          send: (u) => void updates.push(u),
          openHistory: () => undefined,
          onChip: (a) => gate.chip(mogId, a),
        })),
      );
    return s;
  };
  gate = createAskGate({
    clock: sim.clock,
    schedule: sim.at,
    cardFor: stackFor,
    showCard: () => undefined,
    owner: { release: () => undefined },
    isOpen: () => open,
    target: () => 'home',
    send: (t) => bridge.send(t),
  });
  const off = wireAskGate(bus, gate, stackFor);
  const outbound = () =>
    core.invoke.mock.calls.filter(([cmd]) => OUTBOUND.has(cmd)).map(([cmd]) => cmd);
  const last = () => updates[updates.length - 1];
  const card = (kind: string, payload?: unknown, from = `card-${A}`) =>
    bus.dispatch({ event: 'view:input', payload: { from, input: { mogId: A, kind, payload } } });
  return {
    sim,
    bus,
    gate,
    off,
    outbound,
    last,
    card,
    close: () => void (open = false),
    run: (ms: number) => sim.runUntil(sim.clock.now() + ms, { settle: true }),
    preview: () =>
      gate.onAsk(
        {
          route: 'preview',
          question: Q,
          forced: false,
          previewMs: PREVIEW_MS,
          decision: routerDecision({ genie_question: Q, confidence: 0.9 }),
        },
        { utteranceId: 'u1', mogId: A },
      ),
    chip: () =>
      gate.onAsk(
        {
          route: 'chip',
          decision: routerDecision({ genie_question: Q, confidence: 0.4 }),
          warning: undefined,
        },
        { utteranceId: 'u2', mogId: A },
      ),
  };
}

describe('ask gate on the bus (B1)', () => {
  it('[RTR-04a] [RTR-04b] the card gets the preview; no agent or Genie call before 1.5 s; one agent_send at 1.5 s', async () => {
    const t = setup();
    t.preview();
    expect(t.last()?.blurbs.map((b) => b.text)).toEqual([`"${Q}"\nSending in 1.5 s`]);
    expect(t.last()?.chips.map((c) => c.label)).toEqual(['Send now', 'Edit', 'Cancel']);
    await t.run(PREVIEW_MS - 1);
    expect(t.outbound()).toEqual([]);
    await t.run(1);
    expect(t.outbound()).toEqual(['agent_send']);
    const args = core.invoke.mock.calls.find(([c]) => c === 'agent_send')?.[1];
    expect(args).toMatchObject({
      args: { mogId: A, turn: { mogId: A, question: Q, target: 'home' } },
    });
    expect(t.last()?.chips).toEqual([]);
  });

  const cancels: [string, (t: ReturnType<typeof setup>) => unknown][] = [
    ['Esc in the card (view:input cancel)', (t) => t.card('cancel')],
    [
      'a second talk-key press (hotkey:ptt down)',
      (t) => t.bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'down' } }),
    ],
    ['a click on [Cancel]', (t) => t.card('chip_click', { chipId: 'preview_cancel' })],
    ['the card close', (t) => t.gate.cardClosed(A)],
    ['the MOG close', (t) => t.close()],
  ];
  for (const [name, cancel] of cancels) {
    it(`[RTR-04b] ${name} during the preview: no agent or Genie call, ever`, async () => {
      const t = setup();
      t.preview();
      await t.run(PREVIEW_MS - 100);
      cancel(t);
      await t.run(10 * PREVIEW_MS);
      expect(t.outbound()).toEqual([]);
    });
  }

  it('[RTR-04b] Esc from another window does not cancel; a hover never sends', async () => {
    const t = setup();
    t.preview();
    t.card('cancel', undefined, 'card-someone-else');
    t.card('hover', { blurbId: 'x', on: true, via: 'pointer' });
    await t.run(PREVIEW_MS - 1);
    expect(t.outbound()).toEqual([]);
    await t.run(1);
    expect(t.outbound()).toEqual(['agent_send']);
  });

  it('[RTR-05] [UX-20c] the Ask Genie chip: no agent or Genie call without the click; one on the click', async () => {
    const t = setup();
    t.chip();
    const chip = t.last()?.chips.find((c) => c.label === 'Ask Genie');
    expect(chip).toBeDefined();
    t.card('hover', { blurbId: chip?.id, on: true, via: 'pointer' });
    await t.run(60_000);
    expect(t.outbound()).toEqual([]);
    t.card('chip_click', { chipId: chip?.id });
    await t.run(0);
    expect(t.outbound()).toEqual(['agent_send']);
  });

  it('[RTR-05] the Ask Genie chip expires after 2 min; a late click sends nothing', async () => {
    const t = setup();
    t.chip();
    const chip = t.last()?.chips.find((c) => c.label === 'Ask Genie');
    await t.run(2 * 60_000);
    expect(t.last()?.chips).toEqual([]);
    t.card('chip_click', { chipId: chip?.id });
    await t.run(0);
    expect(t.outbound()).toEqual([]);
  });

  it('[RTR-04b] after off() the bus input reaches the gate no more', async () => {
    const t = setup();
    t.off();
    t.preview();
    t.card('cancel');
    await t.run(PREVIEW_MS);
    // The cancel did not arrive, so the preview ended and sent (the gate itself still runs).
    expect(t.outbound()).toEqual(['agent_send']);
  });
});
