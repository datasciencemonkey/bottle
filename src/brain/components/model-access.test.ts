/**
 * The model service access check (#20, ADR 0074; ADM-04a, ADM-04b, ADM-09, AT-36).
 *
 * Every probe goes through the `ModelServiceProbe` fake below. No test calls a workspace.
 */
import {
  MODEL_SERVICES,
  checkModelServices,
  createModelServiceProbe,
  modelPolicyFrom,
  type AccessReport,
  type InferenceTables,
  type ModelPolicy,
  type ModelService,
  type ModelServiceProbe,
  type ProbeTarget,
  type ServiceAccess,
} from './model-access';

/** A fake probe: a fixed answer for each service, and a log of each probe target. */
function fakeProbe(
  access: Partial<Record<ModelService, ServiceAccess | Error>> = {},
  tables: InferenceTables | Error = 'off',
) {
  const calls: ProbeTarget[] = [];
  const tableCalls: ProbeTarget[] = [];
  const probe: ModelServiceProbe = {
    async access(t) {
      calls.push(t);
      const a = access[t.service] ?? 'ok';
      if (a instanceof Error) throw a;
      return a;
    },
    async asrInferenceTables(t) {
      tableCalls.push(t);
      if (tables instanceof Error) throw tables;
      return tables;
    },
  };
  return { probe, calls, tableCalls };
}

const DEFAULT_POLICY: ModelPolicy = { sttPolicy: 'cloud_allowed', models: {} };

const statusOf = (r: AccessReport, s: ModelService) => {
  const found = r.services.find((x) => x.service === s);
  if (!found) throw new Error(`no status for ${s}`);
  return found;
};

describe('checkModelServices (ADM-04a, ADM-04b, ADM-09)', () => {
  it('[ADM-04a] [AT-36] with access to every service, Databricks ASR and the main agent stay on', async () => {
    const { probe, calls } = fakeProbe();
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: true }, probe);
    expect(calls.map((c) => c.service).sort()).toEqual([...MODEL_SERVICES].sort());
    expect(r.sttPolicy).toBe('cloud_allowed');
    expect(r.sttReason).toBeNull();
    expect(r.agentRoute).toBe('main_agent');
    expect(r.localOnly).toBe(false);
    expect(r.stt.provider).toBe('dbx_gemini');
    for (const s of r.services) {
      expect(s.access).toBe('ok');
      expect(s.fallback).toBe('none');
    }
  });

  it('[ADM-04a] [AT-36] no access to the ASR service applies SttPolicy=local_only with a clear status', async () => {
    const { probe } = fakeProbe({ asr: 'no_access' });
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: true }, probe);
    expect(r.sttPolicy).toBe('local_only');
    expect(r.sttReason).toBe('no_access');
    expect(r.localOnly).toBe(true);
    expect(r.stt.provider).toBe('local_parakeet');
    const asr = statusOf(r, 'asr');
    expect(asr.fallback).toBe('local_stt');
    expect(asr.text).toMatch(/on this device/i);
    expect(asr.text).toMatch(/admin/i);
    // The agent route does not depend on the ASR service.
    expect(r.agentRoute).toBe('main_agent');
  });

  it('[ADM-04a] [AT-36] no access to the main agent model service routes with the fallback classifier', async () => {
    const { probe } = fakeProbe({ agent: 'no_access' });
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: true }, probe);
    expect(r.agentRoute).toBe('fallback_classifier');
    const agent = statusOf(r, 'agent');
    expect(agent.fallback).toBe('fallback_classifier');
    expect(agent.text).toMatch(/admin/i);
    expect(r.sttPolicy).toBe('cloud_allowed');
  });

  it('[ADM-04a] each small-model service without access gets its own fallback', async () => {
    const { probe } = fakeProbe({
      cleanup: 'no_access',
      router: 'no_access',
      writer: 'no_access',
      judge: 'no_access',
      embed: 'no_access',
    });
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: false }, probe);
    expect(statusOf(r, 'cleanup').fallback).toBe('prepass_only');
    expect(statusOf(r, 'router').fallback).toBe('heuristic_router');
    expect(statusOf(r, 'writer').fallback).toBe('writer_off');
    expect(statusOf(r, 'judge').fallback).toBe('judge_off');
    expect(statusOf(r, 'embed').fallback).toBe('embedding_queue');
    for (const s of ['cleanup', 'router', 'writer', 'judge', 'embed'] as const) {
      expect(statusOf(r, s).text).toMatch(/admin/i);
    }
    expect(r.agentRoute).toBe('main_agent');
  });

  it('[ADM-04a] MDM SttPolicy=cloud_only without ASR access still falls back to local_only', async () => {
    const { probe, calls } = fakeProbe({ asr: 'no_access' });
    const r = await checkModelServices(
      { policy: { sttPolicy: 'cloud_only', models: {} }, asrSelected: false },
      probe,
    );
    // cloud_only selects Databricks ASR, so the check probes it.
    expect(calls.some((c) => c.service === 'asr')).toBe(true);
    expect(r.sttPolicy).toBe('local_only');
    expect(r.sttReason).toBe('no_access');
  });

  it('[ADM-04a] an unreachable service or a probe that throws never throws and applies the fallback', async () => {
    const secret = new Error('test-token-not-real https://example.invalid/x');
    const { probe } = fakeProbe({ asr: secret, agent: 'unreachable', cleanup: secret });
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: true }, probe);
    expect(statusOf(r, 'asr').access).toBe('unreachable');
    expect(r.sttPolicy).toBe('local_only');
    expect(r.sttReason).toBe('unreachable');
    expect(r.agentRoute).toBe('fallback_classifier');
    expect(statusOf(r, 'cleanup').fallback).toBe('prepass_only');
    // The status is content-free: no probe error text reaches it.
    const all = JSON.stringify(r);
    expect(all).not.toContain('test-token-not-real');
    expect(all).not.toContain('example.invalid');
  });

  it('[ADM-04a] [ADM-09] a probe answer outside the known values counts as a failure', async () => {
    const probe: ModelServiceProbe = {
      access: async (t) => (t.service === 'asr' ? 'ok' : ('maybe' as ServiceAccess)),
      asrInferenceTables: async () => 'perhaps' as InferenceTables,
    };
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: true }, probe);
    expect(r.asrInferenceTables).toBe('unknown');
    expect(statusOf(r, 'agent').access).toBe('unreachable');
    expect(r.agentRoute).toBe('fallback_classifier');
    for (const s of r.services) expect(typeof s.text).toBe('string');
  });

  it('[ADM-04b] [AT-36] when cross-geography routing blocks Databricks ASR, SttPolicy=local_only applies', async () => {
    const { probe } = fakeProbe({ asr: 'geo_blocked' });
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: true }, probe);
    expect(r.sttPolicy).toBe('local_only');
    expect(r.sttReason).toBe('geo_blocked');
    expect(r.stt.provider).toBe('local_parakeet');
    const asr = statusOf(r, 'asr');
    expect(asr.access).toBe('geo_blocked');
    expect(asr.fallback).toBe('local_stt');
    expect(asr.text).toMatch(/cross-geography routing/i);
    expect(asr.text).toMatch(/on this device/i);
  });

  it('[ADM-04b] [CAP-57c] local STT needs no action: the ASR service is not probed', async () => {
    const { probe, calls, tableCalls } = fakeProbe({ asr: 'geo_blocked' });
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: false }, probe);
    expect(calls.some((c) => c.service === 'asr')).toBe(false);
    expect(tableCalls).toEqual([]);
    expect(r.sttPolicy).toBe('cloud_allowed');
    expect(r.sttReason).toBeNull();
    expect(r.stt.provider).toBe('local_parakeet');
    const asr = statusOf(r, 'asr');
    expect(asr.access).toBe('not_checked');
    expect(asr.fallback).toBe('none');
    expect(asr.text).not.toMatch(/admin/i);
  });

  it('[CAP-60b] [ADM-04b] MDM SttPolicy=local_only sends no ASR probe at all', async () => {
    const { probe, calls, tableCalls } = fakeProbe();
    const r = await checkModelServices(
      { policy: { sttPolicy: 'local_only', models: {} }, asrSelected: true },
      probe,
    );
    expect(calls.some((c) => c.service === 'asr')).toBe(false);
    expect(tableCalls).toEqual([]);
    expect(r.sttPolicy).toBe('local_only');
    expect(r.sttReason).toBe('policy');
    expect(r.localOnly).toBe(true);
    expect(r.stt.provider).toBe('local_parakeet');
  });

  it('[ADM-09] inference tables on the ASR service apply SttPolicy=local_only', async () => {
    const { probe, tableCalls } = fakeProbe({}, 'on');
    const r = await checkModelServices({ policy: DEFAULT_POLICY, asrSelected: true }, probe);
    expect(tableCalls.map((t) => t.service)).toEqual(['asr']);
    expect(r.asrInferenceTables).toBe('on');
    expect(r.sttPolicy).toBe('local_only');
    expect(r.sttReason).toBe('inference_tables_on');
    expect(statusOf(r, 'asr').text).toMatch(/inference tables/i);
  });

  it('[ADM-09] inference tables off keeps Databricks ASR; an unknown state says it is not verified', async () => {
    const off = await checkModelServices(
      { policy: DEFAULT_POLICY, asrSelected: true },
      fakeProbe({}, 'off').probe,
    );
    expect(off.asrInferenceTables).toBe('off');
    expect(off.sttPolicy).toBe('cloud_allowed');

    const unknown = await checkModelServices(
      { policy: DEFAULT_POLICY, asrSelected: true },
      fakeProbe({}, new Error('denied')).probe,
    );
    expect(unknown.asrInferenceTables).toBe('unknown');
    expect(unknown.sttPolicy).toBe('cloud_allowed');
    expect(statusOf(unknown, 'asr').text).toMatch(/inference tables/i);
    expect(statusOf(unknown, 'asr').text).toMatch(/not verified/i);
  });

  it('[ADM-04a] [AT-36] the probe gets the MDM model service name for each service', async () => {
    const { probe, calls } = fakeProbe();
    await checkModelServices(
      {
        policy: {
          sttPolicy: 'cloud_allowed',
          models: { asr: 'system.ai.asr-test', embed: 'system.ai.embed-test' },
        },
        asrSelected: true,
      },
      probe,
    );
    expect(calls.find((c) => c.service === 'asr')?.model).toBe('system.ai.asr-test');
    expect(calls.find((c) => c.service === 'embed')?.model).toBe('system.ai.embed-test');
    expect(calls.find((c) => c.service === 'router')?.model).toBeUndefined();
  });

  it('[ADM-04a] [CAP-57e] the unwired default probe never enables Databricks ASR TODO(#19)', async () => {
    const r = await checkModelServices(
      { policy: DEFAULT_POLICY, asrSelected: true },
      createModelServiceProbe(),
    );
    expect(statusOf(r, 'asr').access).toBe('not_checked');
    expect(r.sttPolicy).toBe('local_only');
    expect(r.sttReason).toBe('not_verified');
    // An unchecked agent service is not a failure: the AgentHost reports its own failures.
    expect(r.agentRoute).toBe('main_agent');
  });
});

describe('modelPolicyFrom (Appendix C STT and model keys)', () => {
  it('[ADM-04a] reads SttPolicy and the model service keys of the MDM policy', () => {
    expect(
      modelPolicyFrom({
        SttPolicy: 'cloud_only',
        AsrModelService: 'system.ai.asr-test',
        CleanupModelService: 'system.ai.cleanup-test',
        RouterModelService: 'system.ai.router-test',
        WriterModelService: 'system.ai.writer-test',
        EmbeddingModelService: 'system.ai.embed-test',
      }),
    ).toEqual({
      sttPolicy: 'cloud_only',
      models: {
        asr: 'system.ai.asr-test',
        cleanup: 'system.ai.cleanup-test',
        router: 'system.ai.router-test',
        writer: 'system.ai.writer-test',
        embed: 'system.ai.embed-test',
      },
    });
  });

  it('[ADM-04a] defaults to cloud_allowed with no MDM policy', () => {
    for (const p of [undefined, null, {}, 'x', 3]) {
      expect(modelPolicyFrom(p)).toEqual({ sttPolicy: 'cloud_allowed', models: {} });
    }
  });

  it('[CAP-60b] an SttPolicy value Bottle does not know means local_only', () => {
    expect(modelPolicyFrom({ SttPolicy: 'cloud' }).sttPolicy).toBe('local_only');
    expect(modelPolicyFrom({ SttPolicy: 1 }).sttPolicy).toBe('local_only');
  });

  it('[ADM-04a] ignores a model key that is not a non-empty string', () => {
    expect(modelPolicyFrom({ AsrModelService: '', RouterModelService: 7 }).models).toEqual({});
  });
});
