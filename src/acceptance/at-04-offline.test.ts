/**
 * AT-04 "Dictate offline" (#231), section 15. Pass: local text returns marked "not cleaned" and
 * the insertion works; no cloud request occurs; the confirmed ask goes to the outbox and replays
 * when online; the embedding request waits in the outbox.
 *
 * "No cloud request" is checked with `src/test-support/cloud-calls.ts` on the IPC log of the
 * brain. The fallback chain (#52) runs on the IPC double. The command grammar and the offline classifier (#143), the
 * ask outbox (#41) and offline recall (#199) are not built, so these tests stay `it.skip` with the
 * feature issue. The embedding outbox (#102) runs on doubles of R10, the network and the
 * embedder. The network-off device run is #175 and a person's step (docs/tests/AT-DICTATION-STT-OFFLINE.md). The transcript identity contract (CAP-72) runs in
 * `crates/mog-core/tests/at_04_offline.rs`.
 */
import type { SegmentRef, Tags } from '../shared/interfaces';
import { createGenieBridge } from '../brain/components/genie-bridge';
import { EmbeddingQueue } from '../brain/components/embedding-queue';
import { LocalMemoryService, createMemoryService } from '../brain/components/memory-service';
import type { LocalStmtId } from '../brain/components/memory-store';
import { createRouterContextGate } from '../brain/components/router-context-gate';
import type { RouterInput } from '../brain/components/router-step';
import { createSttProvider, ipcSttConfig } from '../brain/components/stt-provider';
import { core } from '../brain/test-core';
import { argsOf, callsTo, cloudCalls } from '../test-support/cloud-calls';
import { createGenieOneMcpDouble } from '../test-support/genie-one-mcp';
import {
  ItemEmbedderDouble,
  NetworkStatusDouble,
  OutboxDataDouble,
  OutboxStoreDouble,
} from '../test-support/memory-double';
import type { JsonObject } from '../shared/ipc';

vi.mock('@tauri-apps/api/core', async () => (await import('../brain/test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';
const tags: Tags = { app: 'bottle', mog: MOG, op: 'asr' };
const routerInput: RouterInput = {
  profile: 'caller-profile',
  speakerTrack: 'you',
  captureMode: 'ptt',
  pendingClarification: '',
  recentTopics: [],
  activeMogs: [{ id: MOG, name: 'Finance' }],
  profiles: [{ name: 'home', host: 'https://home.example.test', aliases: [] }],
  currentTarget: 'home',
};
const seg: SegmentRef = {
  segmentId: 's1',
  leaseId: 'lease-1',
  mogId: MOG,
  track: 'you',
  tStartMs: 0,
  tEndMs: 3_000,
  final: true,
};

describe('AT-04 dictate offline', () => {
  beforeEach(() => core.reset());

  it('[AT-04] [CAP-60a] each STT failure step goes to the next provider', async () => {
    // Databricks ASR selected (opt-in after a probe pass) and failing: the chain goes to local
    // Parakeet (8.3).
    core.invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'policy_get') return {};
      const p = (argsOf(args) as { provider?: string } | undefined)?.provider;
      if (cmd === 'stt_transcribe' && p === 'dbx_gemini') throw new Error('OFFLINE');
      return {
        segmentId: 's1',
        text: 'hi',
        raw: 'hi',
        provider: p,
        model: 'm',
        runtime: 'r',
        latencyMs: 300,
      };
    });
    const stt = createSttProvider({
      config: ipcSttConfig({ asr: async () => ({ profile: 'user-profile' }) }),
    });
    const t = await stt.transcribe(seg, tags);
    expect(t.provider).toBe('local_parakeet');
    expect(callsTo(core.invoke.mock.calls, ['stt_transcribe'])).toHaveLength(2);
  });

  it('[AT-04] [CAP-60b] with SttPolicy=local_only no ASR request leaves the device', async () => {
    core.invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'policy_get') return { SttPolicy: 'local_only' };
      const p = (argsOf(args) as { provider?: string } | undefined)?.provider;
      return {
        segmentId: 's1',
        text: 'hi',
        raw: 'hi',
        provider: p,
        model: 'm',
        runtime: 'r',
        latencyMs: 300,
      };
    });
    const stt = createSttProvider({
      config: ipcSttConfig({ asr: async () => ({ profile: 'user-profile' }) }),
    });
    expect((await stt.transcribe(seg, tags)).provider).toBe('local_parakeet');
    expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
  });

  it.skip('[AT-04] [RTR-15a] "stop recording" works offline through the command grammar TODO(#143)', async () => {
    const d = await createRouterContextGate().route('stop recording', MOG, routerInput);
    expect([d.intent, d.command]).toEqual(['command', 'stop_recording']);
    expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
  });

  it.skip('[AT-04] [RTR-15c] offline, a question mark or a wh-word gives a question that waits for a click TODO(#143)', async () => {
    core.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'model_invoke') throw new Error('OFFLINE');
      return {};
    });
    const gate = createRouterContextGate();
    expect((await gate.route('what were the sales in march', MOG, routerInput)).intent).toBe(
      'question',
    );
    expect((await gate.route('the sales in march?', MOG, routerInput)).intent).toBe('question');
    expect((await gate.route('the sales in march were fine', MOG, routerInput)).intent).toBe(
      'dictation',
    );
    // TODO(#143): the question shows an "Ask Genie" chip and goes to the outbox only on a click;
    // with no click, nothing is sent to Genie.
    expect(callsTo(core.invoke.mock.calls, ['mcp_call'])).toEqual([]);
  });

  it.skip('[AT-04] [G-46a] [G-46b] confirmed offline asks queue with a 24 h TTL and replay FIFO one at a time TODO(#41)', async () => {
    const genie = createGenieOneMcpDouble({
      host: 'dbc-fake.cloud.databricks.com',
      workspaceId: '1',
    });
    const bridge = createGenieBridge();
    // TODO(#41): offline, submit two confirmed asks and one unconfirmed chip; the outbox holds the
    // two asks only (G-46a). Online again, the bridge shows the workspace line and replays them in
    // order, one ask at a time (G-46b).
    expect(bridge.operations(MOG)).toEqual([]);
    expect(genie.asks()).toHaveLength(2);
    expect(genie.maxConcurrentPolls()).toBe(1);
  });

  it('[AT-04] [MEM-08f] [MEM-19d] an offline embedding request waits in the outbox and survives a restart', async () => {
    // The brain reaches R10 over mem_local_exec; the core answers from a stateful outbox double
    // (the database file). The network and the embedder are doubles: no endpoint is called.
    const data = new OutboxDataDouble();
    const r10 = new OutboxStoreDouble(data);
    core.invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd !== 'mem_local_exec') return {};
      const a = argsOf(args) as { stmtId: LocalStmtId; params: JsonObject };
      return r10.exec(a.stmtId, a.params);
    });
    const net = new NetworkStatusDouble(false);
    const offlineEmbedder = new ItemEmbedderDouble(data);
    const q1 = new EmbeddingQueue({ net, embedder: offlineEmbedder, mogIds: () => [MOG] });
    q1.start();
    const mem = new LocalMemoryService(undefined, { embeddingQueue: q1 });
    await mem.storeItem({
      mogId: MOG,
      type: 'fact',
      text: 'Q3 target is 4.2M',
      source: 'you',
      confidence: 0.9,
    });
    await q1.idle();
    expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
    expect(offlineEmbedder.seen).toEqual([]);
    expect(r10.jobs(MOG)).toHaveLength(1);
    q1.stop();

    // Restart: a new queue on the same store still has the request, and it runs online.
    const embedder = new ItemEmbedderDouble(data);
    const q2 = new EmbeddingQueue({ net, embedder, mogIds: () => [MOG] });
    q2.start();
    net.set(true);
    await q2.idle();
    expect(embedder.seen).toHaveLength(1);
    expect(r10.jobs(MOG)).toEqual([]);
    expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
    // TODO(#198): the real embedder sends model_invoke (role embed); TODO(#466): the real
    // net.status payload drives NetworkStatus. The network-off device run is #175.
  });

  it.skip('[AT-04] [MEM-22a] [MEM-22b] offline recall uses FTS5 and recency only and is flagged offline_fts_only TODO(#199)', async () => {
    const r = await createMemoryService().gate({
      mogId: MOG,
      text: 'what is the Q3 target',
      intent: 'memory_question',
      target: { workspaceId: '1' },
    });
    expect(r.degraded).toBe('offline_fts_only');
    expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
  });

  it.skip('[AT-04] [NFR-17a] [NFR-17b] offline, dictation inserts text marked "not cleaned" and makes no cloud request TODO(#175)', () => {
    // TODO(#175): the network-off run across components: PTT with local STT, insert_text with the
    // raw text and the "not cleaned" mark, an ask in the outbox, an embedding in the outbox, and
    // cloudCalls(log) is [] for the whole run.
    expect.unreachable('TODO(#175): the offline flow across components is not built');
  });
});
