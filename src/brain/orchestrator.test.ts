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
  let settings = { endpoint: HAIKU, cleanDictation: true, cloudDictation: false };
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
        cloudDictation: args.cloudDictation ?? settings.cloudDictation,
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
        cloudDictation: false,
        state: 'ready',
        homeName: 'Sales',
      },
    ]);
    expect(JSON.stringify(r.sent)).not.toMatch(/tested/i);
  });

  it('a model that the gateway no longer lists still shows as the selected one', async () => {
    const r = rig();
    r.setSettings({ endpoint: 'system.ai.old-model', cleanDictation: true, cloudDictation: false });
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

  it('[ADR 0123] the dictation engine choice writes cloudDictation and the reader follows it', async () => {
    const r = rig();
    expect(r.o.cloudDictation()).toBe(false);
    r.click({ action: 'orchestrator.cloud_dictation', on: true });
    await flush();
    expect(r.writes).toEqual([{ cloudDictation: true }]);
    expect(r.sent.at(-1)?.cloudDictation).toBe(true);
    expect(r.o.cloudDictation()).toBe(true);
    r.click({ action: 'orchestrator.cloud_dictation', on: false });
    await flush();
    expect(r.o.cloudDictation()).toBe(false);
    // A bad value writes nothing.
    r.click({ action: 'orchestrator.cloud_dictation', on: 'yes' });
    await flush();
    expect(r.writes).toHaveLength(2);
    // Off until the list exists.
    expect(rig({ settings: () => undefined }).o.cloudDictation()).toBe(false);
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

  it('[M3] a turn that ends failed, signed_out or offline reads the status again; others do not', async () => {
    const r = rig();
    await r.o.refresh();
    const base = r.statusReads.length;
    for (const outcome of ['answered', 'stopped', 'timeout']) r.o.turnEnded(outcome);
    await flush();
    expect(r.statusReads.length).toBe(base);
    r.setStatus({ state: 'unreachable', model: HAIKU });
    for (const outcome of ['failed', 'signed_out', 'offline']) r.o.turnEnded(outcome);
    await flush();
    expect(r.statusReads.length).toBe(base + 3);
    expect(r.o.status()?.state).toBe('unreachable');
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
