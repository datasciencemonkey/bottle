/**
 * #142 (ADR 0086), ADR 0098, end to end on the brain bus: the card input and the T15 hand-off.
 * The check is the IPC itself: one send for an ⌥G ask (the user action), and no `agent_send` and
 * no `mcp_call` without a chip press (B1, RTR-04b, RTR-05, RTR-17a).
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createSimScheduler } from '../test-support/sim-scheduler';
import { routerDecision } from '../test-support/model-invoke';
import { createAskGate, wireAskGate, type AskGate } from './ask-gate';
import { BrainBus } from './bus';
import { createCardStack, type CardStack, type CardStackUpdate } from './card-stack';
import { createAgentBridge } from './components/agent-bridge';
import { core } from './test-core';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const A = '00000000-0000-4000-8000-0000000000a1';
const Q = 'What were EMEA bookings last week?';
const OUTBOUND = new Set(['agent_send', 'mcp_call']);
/**
 * The `agent_send` arguments of an ⌥G ask. The Rust test `preview_gate_handoff.rs` reads the
 * same file and sends it through `AgentHost::send`, which approves the question for `genie_ask`.
 */
const AGENT_SEND_FIXTURE: unknown = JSON.parse(
  readFileSync(
    join(import.meta.dirname, '../../crates/mog-core/tests/fixtures/preview_gate_agent_send.json'),
    'utf8',
  ),
);

function setup() {
  core.reset();
  const sim = createSimScheduler();
  const bus = new BrainBus();
  const updates: CardStackUpdate[] = [];
  const stacks = new Map<string, CardStack>();
  const bridge = createAgentBridge();
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
    isOpen: () => true,
    target: () => 'home',
    send: (t) => bridge.send(t),
  });
  const hides: string[] = [];
  const off = wireAskGate(bus, gate, stackFor, { hideCard: (id) => void hides.push(id) });
  const outbound = () =>
    core.invoke.mock.calls.filter(([cmd]) => OUTBOUND.has(cmd)).map(([cmd]) => cmd);
  const last = () => updates[updates.length - 1];
  const card = (kind: string, payload?: unknown, from = `card-${A}`) =>
    bus.dispatch({ event: 'view:input', payload: { from, input: { mogId: A, kind, payload } } });
  return {
    hides,
    sim,
    bus,
    gate,
    off,
    outbound,
    last,
    card,
    run: (ms: number) => sim.runUntil(sim.clock.now() + ms, { settle: true }),
    ask: () =>
      gate.onAsk(
        {
          route: 'ask',
          question: Q,
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
  it('[RTR-17a] [RTR-17b] [RTR-04b] an ⌥G ask sends one agent_send at once, with no preview on the card', async () => {
    const t = setup();
    t.ask();
    await t.run(0);
    expect(t.outbound()).toEqual(['agent_send']);
    const args = core.invoke.mock.calls.find(([c]) => c === 'agent_send')?.[1];
    expect(args).toMatchObject({
      args: { mogId: A, turn: { mogId: A, question: Q, target: 'home' } },
    });
    expect(t.last()?.blurbs.map((b) => b.text)).toEqual([`"${Q}"`]);
    expect(t.last()?.chips).toEqual([]);
    await t.run(15_000);
    expect(t.outbound()).toEqual(['agent_send']);
  });

  it('[RTR-04b] [RTR-29b] the agent_send of an ⌥G ask is the shared fixture the core approves', async () => {
    const t = setup();
    t.ask();
    await t.run(0);
    const sends = core.invoke.mock.calls.filter(([c]) => c === 'agent_send');
    expect(sends).toHaveLength(1);
    expect((sends[0]?.[1] as { args: unknown }).args).toEqual(AGENT_SEND_FIXTURE);
  });

  it('[RTR-17a] a talk-key press or a card cancel after an ⌥G ask sends nothing more', async () => {
    const t = setup();
    t.ask();
    await t.run(0);
    await t.bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: A, phase: 'down' } });
    await t.card('cancel');
    await t.run(15_000);
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

  it('[RTR-05] after off() a chip press on the bus reaches the gate no more', async () => {
    const t = setup();
    t.chip();
    const chip = t.last()?.chips.find((c) => c.label === 'Ask Genie');
    t.off();
    await t.card('chip_click', { chipId: chip?.id });
    await t.run(0);
    expect(t.outbound()).toEqual([]);
  });

  it('[UX-19b] [#559] a card cancel hides the card; a cancel from the MOG window does not', async () => {
    const t = setup();
    await t.card('cancel');
    expect(t.hides).toEqual([A]);
    await t.card('cancel', undefined, `mog-${A}`);
    expect(t.hides).toEqual([A]);
    t.off();
  });
});
