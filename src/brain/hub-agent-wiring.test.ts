/** Slice 2a: the brain side of the hub Agent panel. IPC doubles; all values are fake. */
import { HUB_SCOPE, asHubAgentView, type HubAgentView } from '../shared/hub-agent';
import type { GenieCodeStatus } from '../shared/ipc';
import { BrainBus } from './bus';
import { OPEN_NOTICE, wireHubAgent, type HubAgentWiringDeps } from './hub-agent-wiring';

const T1 = 'a'.repeat(32);
const READY: GenieCodeStatus = {
  binary: 'ready',
  pinned: '0.5.0',
  agentProfile: null,
  agentProfileState: 'ready',
};
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

  it('"Open in Agent" sends the notice at once, and a late hub gets it with agent.ready once', async () => {
    const r = rig();
    await r.wiring.open();
    expect(r.sent.at(-1)?.notice).toEqual({ kind: 'info', text: OPEN_NOTICE });
    r.click({ action: 'agent.ready' });
    await flush();
    expect(r.sent.at(-1)?.notice?.text).toBe(OPEN_NOTICE);
    r.click({ action: 'agent.ready' });
    await flush();
    expect(r.sent.at(-1)?.notice).toBeUndefined();
  });

  it('sends nothing before the Genie Code status is known, and a failed send never rejects', async () => {
    const r = rig(null);
    await r.wiring.push();
    expect(r.sent).toEqual([]);
    const bus = new BrainBus();
    const w = wireHubAgent(bus, {
      client: {
        panel: () => ({
          session: { state: 'none', agentProfile: null, turns: 0, queued: 0, tokens: 0 },
          waiting: [],
          earlier: [],
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
