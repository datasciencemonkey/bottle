/** ADR 0122: the brain side of the orchestrator model controls. IPC doubles; all values are fake. */
import { HUB_SCOPE, type HubOrchestratorView } from '../shared/hub-orchestrator';
import type { OrchestratorModels, OrchestratorStatus } from '../shared/ipc';
import { BrainBus } from './bus';
import { wireOrchestrator, type OrchestratorDeps } from './orchestrator';

const HAIKU = 'system.ai.claude-haiku-5-5';
const OSS = 'system.ai.gpt-oss-20b';
const MODELS: OrchestratorModels = {
  models: [
    { id: HAIKU, label: 'Claude Haiku 5.5', tested: true, tools: true },
    { id: OSS, label: 'GPT OSS 20B', tested: false, tools: true },
  ],
  selected: HAIKU,
  defaultModel: HAIKU,
};
const flush = () => new Promise((r) => setTimeout(r, 0));

function rig(over: Partial<OrchestratorDeps> = {}) {
  const bus = new BrainBus();
  const sent: HubOrchestratorView[] = [];
  const statusReads: (boolean | undefined)[] = [];
  const writes: unknown[] = [];
  let status: OrchestratorStatus = { state: 'ready', model: HAIKU };
  let settings = { endpoint: HAIKU, cleanDictation: true };
  const deps: OrchestratorDeps = {
    models: async () => MODELS,
    status: async (recheck) => {
      statusReads.push(recheck);
      return status;
    },
    settings: () => settings,
    homeName: () => 'Sales',
    write: async (args) => {
      writes.push(args);
      settings = {
        endpoint: args.agentEndpoint ?? settings.endpoint,
        cleanDictation: args.cleanDictation ?? settings.cleanDictation,
      };
    },
    send: async (v) => void sent.push(v),
    ...over,
  };
  const o = wireOrchestrator(bus, deps);
  const click = (payload: unknown, from = 'hub', mogId = HUB_SCOPE, kind = 'click') =>
    bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
  return {
    o,
    click,
    sent,
    writes,
    statusReads,
    setStatus: (s: OrchestratorStatus) => (status = s),
    setSettings: (s: typeof settings) => (settings = s),
  };
}

describe('orchestrator wiring', () => {
  it('sends the models by plain label, with no test flag, when the hub is ready', async () => {
    const r = rig();
    r.click({ action: 'orchestrator.ready' });
    await flush();
    expect(r.sent).toEqual([
      {
        models: [
          { id: HAIKU, label: 'Claude Haiku 5.5' },
          { id: OSS, label: 'GPT OSS 20B' },
        ],
        selected: HAIKU,
        cleanDictation: true,
        state: 'ready',
        homeName: 'Sales',
      },
    ]);
    expect(JSON.stringify(r.sent)).not.toMatch(/tested/i);
  });

  it('a model that the gateway no longer lists still shows as the selected one', async () => {
    const r = rig();
    r.setSettings({ endpoint: 'system.ai.old-model', cleanDictation: true });
    r.click({ action: 'orchestrator.ready' });
    await flush();
    expect(r.sent[0]?.models[0]).toEqual({ id: 'system.ai.old-model', label: 'old-model' });
    expect(r.sent[0]?.selected).toBe('system.ai.old-model');
  });

  it('a model choice writes agentEndpoint, reads the status and sends the view', async () => {
    const r = rig();
    r.click({ action: 'orchestrator.model', model: OSS });
    await flush();
    expect(r.writes).toEqual([{ agentEndpoint: OSS }]);
    expect(r.statusReads).toEqual([undefined]);
    expect(r.sent.at(-1)?.selected).toBe(OSS);
  });

  it('the switch writes cleanDictation', async () => {
    const r = rig();
    r.click({ action: 'orchestrator.clean_dictation', on: false });
    await flush();
    expect(r.writes).toEqual([{ cleanDictation: false }]);
    expect(r.sent.at(-1)?.cleanDictation).toBe(false);
  });

  it('[cleanDictation wiring] the reader follows the setting, and is on until the list exists', async () => {
    const r = rig();
    expect(r.o.cleanDictation()).toBe(true);
    r.click({ action: 'orchestrator.clean_dictation', on: false });
    await flush();
    expect(r.o.cleanDictation()).toBe(false);
    const none = rig({ settings: () => undefined });
    expect(none.o.cleanDictation()).toBe(true);
  });

  it('Check again reads the status with recheck, and a failed read keeps the last status', async () => {
    const r = rig();
    await r.o.refresh();
    expect(r.o.status()?.state).toBe('ready');
    r.setStatus({ state: 'unreachable', model: HAIKU });
    r.click({ action: 'orchestrator.recheck' });
    await flush();
    expect(r.statusReads.at(-1)).toBe(true);
    expect(r.o.status()?.state).toBe('unreachable');
    const bad = rig({ status: () => Promise.reject({ error: 'host_failed' }) });
    await bad.o.refresh();
    expect(bad.o.status()).toBeUndefined();
  });

  it('drops a bad action, another window, another scope and a non-click', async () => {
    const r = rig();
    r.click({ action: 'orchestrator.model', model: 'not-a-system-model' });
    r.click({ action: 'orchestrator.clean_dictation', on: 'yes' });
    r.click({ action: 'orchestrator.ready' }, `card-${HUB_SCOPE}`);
    r.click({ action: 'orchestrator.ready' }, 'hub', 'some-mog');
    r.click({ action: 'orchestrator.ready' }, 'hub', HUB_SCOPE, 'hover');
    await flush();
    expect(r.writes).toEqual([]);
    expect(r.sent).toEqual([]);
  });

  it('a send to a closed hub never rejects', async () => {
    const r = rig({ send: () => Promise.reject(new Error('no hub')) });
    await expect(r.o.refresh()).resolves.toBeUndefined();
  });
});
