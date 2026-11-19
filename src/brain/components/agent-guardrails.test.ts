/**
 * T15 AgentBridge guardrails (#141, ADR 0085): RTR-03c, RTR-21b, RTR-29a..d, F5 step 4, AT-42.
 *
 * The brain gets the agent's UI tool calls (`agent.tool_call`) and answers with
 * `agent_tool_result`. Only the 4 UI tools reach a handler. No handler of the agent path can start
 * capture, approve context, publish or create a Page, or switch the workspace. A hop proposal
 * becomes the "Switch to <profile>?" chip, and only a click on it calls `profile_select`.
 * The adversarial set is the Rust fixture `at_42_adversarial.json`; its texts are data.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import type { RouterDecision } from '../../shared/interfaces';
import { ChipRow } from '../card-chips';
import { core } from '../test-core';
import { buildAgentTurn, createAgentBridge, type AgentToolHandlers } from './agent-bridge';
import { HopChips } from './agent-hop';
import type { DispatchOutcome, HopProposal } from './router-dispatch';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);

const MOG = '00000000-0000-4000-8000-000000000141';

interface Adversarial {
  id: string;
  source: 'transcript' | 'memory';
  asks: string;
  text: string;
}

const SET: Adversarial[] = (
  JSON.parse(
    readFileSync(
      resolve(__dirname, '../../../crates/mog-core/tests/fixtures/at_42_adversarial.json'),
      'utf8',
    ),
  ) as { items: Adversarial[] }
).items;

/** The IPC commands that the agent path must never reach (RTR-29a..d, B1). */
const FORBIDDEN = [
  'capture_start',
  'capture_resume',
  'profile_select',
  'profile_login',
  'profile_login_cancel',
  'mcp_call',
  'dbx_http',
  'config_write',
  'insert_text',
  // ADR 0103: only a user action starts a recording or changes its settings.
  'recording_start',
  'recording_settings_write',
];

const UI_ARGS = (text: string): [string, Record<string, unknown>][] => [
  ['show_blurb', { text }],
  ['set_mood', { mood: 'answering' }],
  ['propose_page', { title: 'Net revenue', body: text }],
  ['ask_user', { question: text, choices: ['Yes', 'No'] }],
];

function handlers(): AgentToolHandlers & { log: string[] } {
  const log: string[] = [];
  return {
    log,
    showBlurb: vi.fn(async () => {
      log.push('show_blurb');
      return { shown: true };
    }),
    setMood: vi.fn(async () => {
      log.push('set_mood');
      return { applied: true };
    }),
    proposePage: vi.fn(async () => {
      log.push('propose_page');
      return { reviewId: 'r-1' };
    }),
    askUser: vi.fn(async () => {
      log.push('ask_user');
      return { answer: 'No' };
    }),
  };
}

const commands = () => core.invoke.mock.calls.map((c) => c[0]);

beforeEach(() => core.reset());

describe('AgentBridge tool decisions (T15, #141)', () => {
  it('[RTR-21b] each UI tool call reaches its own handler and the result goes back by agent_tool_result', async () => {
    const h = handlers();
    const bridge = createAgentBridge({ handlers: h });
    for (const [tool, args] of UI_ARGS('Revenue grew 4 %')) {
      await bridge.handleToolCall({ mogId: MOG, callId: `tc-${tool}`, tool, args });
    }
    expect(h.log).toEqual(['show_blurb', 'set_mood', 'propose_page', 'ask_user']);
    const results = core.invoke.mock.calls.filter((c) => c[0] === 'agent_tool_result');
    expect(results.map((c) => (c[1] as { args: { callId: string } }).args.callId)).toEqual([
      'tc-show_blurb',
      'tc-set_mood',
      'tc-propose_page',
      'tc-ask_user',
    ]);
    expect((results[2]?.[1] as { args: unknown }).args).toEqual({
      mogId: MOG,
      callId: 'tc-propose_page',
      result: { ok: true, result: { reviewId: 'r-1' } },
    });
  });

  it('[RTR-21b] onTool answers a call of the MOG with the handler result', async () => {
    const h = handlers();
    const bridge = createAgentBridge({ handlers: h });
    await expect(
      bridge.onTool({ mogId: MOG, callId: 'c', tool: 'set_mood', args: { mood: 'idle' } }),
    ).resolves.toEqual({ ok: true, result: { applied: true } });
    expect(h.setMood).toHaveBeenCalledWith(MOG, { mood: 'idle' });
  });

  it('[RTR-21b] a call with no MOG runs nothing', async () => {
    const h = handlers();
    const r = await createAgentBridge({ handlers: h }).onTool({
      callId: 'c',
      tool: 'set_mood',
      args: { mood: 'idle' },
    });
    expect(r).toEqual({ ok: false, error: 'unknown_mog' });
    expect(h.log).toEqual([]);
  });

  it('[RTR-21b] a local tool name or bad arguments run no UI handler', async () => {
    const h = handlers();
    const bridge = createAgentBridge({ handlers: h });
    const bad: [string, unknown][] = [
      ['memory.write', { type: 'fact', text: 'x' }],
      ['transcript.search', { query: 'x' }],
      ['show_blurb', {}],
      ['show_blurb', { text: 42 }],
      ['set_mood', { mood: 'listening' }],
      ['propose_page', { title: 'x' }],
      ['ask_user', { question: '' }],
      ['ask_user', { question: 'q', choices: 'yes' }],
    ];
    for (const [tool, args] of bad) {
      const r = await bridge.onTool({ mogId: MOG, callId: 'c', tool: tool as never, args });
      expect(r, `${tool}`).toMatchObject({ ok: false });
    }
    expect(h.log).toEqual([]);
  });

  it('[RTR-21b] with no handler wired the tool answers not_available', async () => {
    const r = await createAgentBridge().onTool({
      mogId: MOG,
      callId: 'c',
      tool: 'show_blurb',
      args: { text: 'x' },
    });
    expect(r).toEqual({ ok: false, error: 'not_available' });
  });
});

describe('AgentBridge guardrails over the AT-42 adversarial set (#141)', () => {
  it('[RTR-29a][RTR-29b][RTR-29c][RTR-29d][AT-42] no agent tool call reaches capture, context, Page, Genie or profile commands', async () => {
    expect(SET.length).toBeGreaterThanOrEqual(12);
    const h = handlers();
    const bridge = createAgentBridge({ handlers: h });
    for (const a of SET) {
      for (const [tool, args] of UI_ARGS(a.text)) {
        await bridge.handleToolCall({ mogId: MOG, callId: `tc-${a.id}-${tool}`, tool, args });
      }
      // The tool names that the text asks for are not tools here.
      for (const tool of [
        'capture_start',
        'context_approve',
        'page_publish',
        'profile_select',
        a.asks,
      ]) {
        await bridge.handleToolCall({ mogId: MOG, callId: `tc-${a.id}-x`, tool, args: {} });
      }
    }
    const sent = new Set(commands());
    expect([...sent]).toEqual(['agent_tool_result']);
    for (const f of FORBIDDEN) expect(sent.has(f), f).toBe(false);
  });

  it('[RTR-03c][AT-42] adversarial text is passed on as data and asks for nothing more', async () => {
    const h = handlers();
    const bridge = createAgentBridge({ handlers: h });
    for (const a of SET) {
      h.log.length = 0;
      await bridge.handleToolCall({
        mogId: MOG,
        callId: `tc-${a.id}`,
        tool: 'show_blurb',
        args: { text: a.text },
      });
      expect(h.log, a.id).toEqual(['show_blurb']);
      expect(h.showBlurb).toHaveBeenLastCalledWith(MOG, { text: a.text });
    }
  });

  it('[RTR-29c] propose_page only proposes for review: the handler result never says published', async () => {
    const h = handlers();
    h.proposePage = vi.fn(async () => ({ reviewId: 'r-1', state: 'PUBLISHED', pageId: 'p-9' }));
    const r = await createAgentBridge({ handlers: h }).onTool({
      mogId: MOG,
      callId: 'c',
      tool: 'propose_page',
      args: { title: 'Net revenue', body: SET[6]?.text ?? '' },
    });
    expect(r).toEqual({ ok: true, result: { reviewId: 'r-1' } });
  });

  it('[RTR-29a] set_mood can never claim the mic is open', async () => {
    const h = handlers();
    const bridge = createAgentBridge({ handlers: h });
    for (const mood of ['listening', 'recording', 'sleeping']) {
      const r = await bridge.onTool({ mogId: MOG, callId: 'c', tool: 'set_mood', args: { mood } });
      expect(r, mood).toMatchObject({ ok: false });
    }
    expect(h.setMood).not.toHaveBeenCalled();
  });
});

describe('AgentTurn payload (#141)', () => {
  it('[RTR-21b][RTR-03c] buildAgentTurn takes question, decision, recall and target from the preview outcome', () => {
    const decision = { intent: 'question', cleaned_text: 'q' } as RouterDecision;
    const recall = [
      { itemId: 'i1', type: 'fact' as const, text: SET[4]?.text ?? '', score: 0.9, ageH: 1 },
    ];
    const outcome: Extract<DispatchOutcome, { kind: 'preview' }> = {
      kind: 'preview',
      decision,
      question: 'What was revenue in Q3?',
      target: 'bottle-home',
      scope: 'current',
      recall,
    };
    expect(buildAgentTurn(MOG, outcome)).toEqual({
      mogId: MOG,
      question: 'What was revenue in Q3?',
      decision,
      recall,
      target: 'bottle-home',
    });
  });
});

describe('Hop proposals wait for the "Switch to <profile>?" chip (RTR-29d, F5 step 4)', () => {
  const proposal: HopProposal = {
    command: 'switch_workspace_session',
    profile: 'prod',
    host: 'prod.cloud.databricks.com',
    scope: 'session',
    requiresUserAction: true,
  };

  it('[RTR-29d] an offer shows the chip and switches nothing', () => {
    const row = new ChipRow();
    const hops = new HopChips({ chips: row, now: () => 0 });
    const id = hops.offer(MOG, proposal);
    expect(id).toBeDefined();
    expect(row.view().chips).toEqual([{ id, label: 'Switch to prod?' }]);
    expect(core.invoke).not.toHaveBeenCalled();
  });

  it('[RTR-29d] only a click calls profile_select with the proposed profile and scope', async () => {
    const row = new ChipRow();
    const hops = new HopChips({ chips: row, now: () => 0 });
    const id = hops.offer(MOG, proposal) ?? '';
    await expect(hops.click(id)).resolves.toBe(true);
    expect(core.invoke).toHaveBeenCalledTimes(1);
    expect(core.invoke).toHaveBeenCalledWith('profile_select', {
      args: { mogId: MOG, profile: 'prod', scope: 'session' },
    });
    expect(row.view().chips).toEqual([]);
    await expect(hops.click(id)).resolves.toBe(false);
    expect(core.invoke).toHaveBeenCalledTimes(1);
  });

  it('[RTR-29d] an expired, dismissed or unknown chip never switches', async () => {
    let now = 0;
    const row = new ChipRow();
    const hops = new HopChips({ chips: row, now: () => now });
    const id = hops.offer(MOG, proposal) ?? '';
    now = 30_000;
    row.tick(now);
    await expect(hops.click(id)).resolves.toBe(false);
    const id2 = hops.offer(MOG, { ...proposal, profile: 'dev', scope: 'question' }) ?? '';
    hops.dismiss(id2);
    await expect(hops.click(id2)).resolves.toBe(false);
    await expect(hops.click('nope')).resolves.toBe(false);
    expect(core.invoke).not.toHaveBeenCalled();
  });

  it('[RTR-29d] offerOutcome takes only a hop outcome that needs the user', () => {
    const row = new ChipRow();
    const hops = new HopChips({ chips: row, now: () => 0 });
    expect(hops.offerOutcome(MOG, { kind: 'hop', proposal })).toBeDefined();
    expect(hops.offerOutcome(MOG, { kind: 'hop_refused', reason: 'host_not_allowed' })).toBe(
      undefined,
    );
    expect(
      hops.offerOutcome(MOG, {
        kind: 'hop',
        proposal: { ...proposal, requiresUserAction: false as unknown as true },
      }),
    ).toBe(undefined);
    expect(core.invoke).not.toHaveBeenCalled();
  });

  it('[RTR-29d][AT-42] adversarial agent text that asks for a switch only becomes data', async () => {
    const h = handlers();
    const bridge = createAgentBridge({ handlers: h });
    for (const a of SET.filter((x) => x.asks === 'switch_workspace')) {
      await bridge.handleToolCall({
        mogId: MOG,
        callId: `tc-${a.id}`,
        tool: 'ask_user',
        args: { question: a.text, choices: ['Switch to prod', 'Stay'] },
      });
    }
    expect(commands().filter((c) => c === 'profile_select')).toEqual([]);
  });
});
