/** Slice 2a: the brain side of the hub Agent panel. IPC doubles; all values are fake. */
import { HUB_SCOPE, asHubAgentView, type HubAgentView } from '../shared/hub-agent';
import type { GenieCodeStatus } from '../shared/ipc';
import { BrainBus } from './bus';
import { clearingAgentPanel, wireHubAgent, type HubAgentWiringDeps } from './hub-agent-wiring';

const T1 = 'a'.repeat(32);
const READY: GenieCodeStatus = {
  binary: 'ready',
  pinned: '0.5.0',
  agentProfile: null,
  agentProfileState: 'ready',
};
const THREAD = '41111111-2222-4333-8444-000000000001';
const flush = () => new Promise((r) => setTimeout(r, 0));

function rig(genie: GenieCodeStatus | null = READY) {
  const bus = new BrainBus();
  const log: string[] = [];
  const sent: HubAgentView[] = [];
  const deps: HubAgentWiringDeps = {
    client: {
      panel: () => ({
        session: { state: 'none', agentProfile: null, turns: 0, queued: 0, tokens: 0 },
        waiting: [],
        earlier: [],
        threads: [{ turnId: T1, threadId: THREAD }],
      }),
      stop: async (id) => void log.push(`stop:${id}`),
      newSession: async () => void log.push('new'),
      refreshSession: async () => void log.push('refresh'),
    },
    genieCode: () => genie ?? undefined,
    send: async (v) => void sent.push(v),
    openSettings: () => void log.push('settings'),
  };
  const wiring = wireHubAgent(bus, deps);
  const click = (payload: unknown, from = 'hub', mogId = HUB_SCOPE, kind = 'click') =>
    bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
  return { wiring, click, log, sent };
}

describe('hub Agent panel wiring', () => {
  it('[AC-2a.25] sends a valid panel on agent.ready, to the hub only', async () => {
    const r = rig();
    r.click({ action: 'agent.ready' });
    await flush();
    expect(r.log).toEqual(['refresh']);
    expect(r.sent).toHaveLength(1);
    expect(asHubAgentView(r.sent[0])).toBeDefined();
    expect(r.sent[0]?.notice).toBeUndefined();
  });

  it('runs Stop, a queued remove, New session and Open Settings, each once', async () => {
    const r = rig();
    r.click({ action: 'agent.stop', turnId: T1 });
    r.click({ action: 'agent.remove_queued', turnId: T1 });
    r.click({ action: 'agent.new_session' });
    r.click({ action: 'agent.open_settings' });
    await flush();
    expect(r.log).toEqual([`stop:${T1}`, `stop:${T1}`, 'new', 'settings']);
    // The panel is sent again after each change.
    expect(r.sent).toHaveLength(3);
  });

  it('drops a bad action, another window, another scope and a non-click', async () => {
    const r = rig();
    r.click({ action: 'agent.stop', turnId: 'short' });
    r.click({ action: 'agent.nope' });
    r.click({ action: 'agent.new_session' }, `card-${HUB_SCOPE}`);
    r.click({ action: 'agent.new_session' }, 'hub', 'some-mog');
    r.click({ action: 'agent.new_session' }, 'hub', HUB_SCOPE, 'hover');
    await flush();
    expect(r.log).toEqual([]);
    expect(r.sent).toEqual([]);
  });

  it('sends the thread of each turn, and no notice (the Agent area is gone)', async () => {
    const r = rig();
    await r.wiring.push();
    expect(r.sent[0]?.threads).toEqual([{ turnId: T1, threadId: THREAD }]);
    expect(r.sent[0]?.notice).toBeUndefined();
  });

  it('[L5] with no Genie Code status the panel still draws, with a placeholder', async () => {
    const r = rig(null);
    await r.wiring.push();
    expect(r.sent).toHaveLength(1);
    expect(asHubAgentView(r.sent[0])?.genieCode.binary).toBe('not_installed');
  });

  it('[M1] a Wipe clears the panel content, and a failed Wipe does not', async () => {
    const calls: string[] = [];
    const ok = clearingAgentPanel(
      { dialog: () => ({}) as never, confirm: async () => ({}) as never },
      () => void calls.push('clear'),
    );
    await ok.confirm('everything', false);
    const bad = clearingAgentPanel(
      { dialog: () => ({}) as never, confirm: () => Promise.reject(new Error('no')) },
      () => void calls.push('bad'),
    );
    await bad.confirm('everything', false).catch(() => undefined);
    expect(calls).toEqual(['clear']);
  });

  it('[L-1] the fence is up before the Wipe runs, stays after it, and lifts when the Wipe fails', async () => {
    const calls: string[] = [];
    const flow = (fail: boolean) => ({
      dialog: () => ({}) as never,
      confirm: async () => {
        calls.push('confirm');
        if (fail) throw new Error('no');
        return {} as never;
      },
    });
    const fence = () => {
      calls.push('fence');
      return () => void calls.push('lift');
    };
    await clearingAgentPanel(flow(false), () => void calls.push('clear'), fence).confirm(
      'everything',
      false,
    );
    await clearingAgentPanel(flow(true), () => void calls.push('clear2'), fence)
      .confirm('everything', false)
      .catch(() => undefined);
    expect(calls).toEqual(['fence', 'confirm', 'clear', 'fence', 'confirm', 'lift']);
  });

  it('a failed send never rejects', async () => {
    const bus = new BrainBus();
    const w = wireHubAgent(bus, {
      client: {
        panel: () => ({
          session: { state: 'none', agentProfile: null, turns: 0, queued: 0, tokens: 0 },
          waiting: [],
          earlier: [],
          threads: [],
        }),
        stop: async () => undefined,
        newSession: async () => undefined,
        refreshSession: async () => undefined,
      },
      genieCode: () => READY,
      send: () => Promise.reject(new Error('no hub')),
      openSettings: () => undefined,
    });
    await expect(w.push()).resolves.toBeUndefined();
  });
});
