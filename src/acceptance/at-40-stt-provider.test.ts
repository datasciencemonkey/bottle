/**
 * AT-40 "STT benchmark gate" (#231), section 15: the SttProvider rules that the benchmark gates.
 * The scorer and the device run are `crates/mog-core/tests/at_40_stt_bench.rs`
 * (`mog_core::stt_bench`, ADR 0034).
 *
 * SttProvider (#52) runs on the IPC double. The Databricks ASR client with its guards (#49) is
 * not built, so its tests stay `it.skip` with the feature issue. The release record (CAP-65a, CAP-65b) is #53.
 * A skipped test never counts as a pass.
 */
import type { SegmentRef, Tags } from '../shared/interfaces';
import { SttFailure, createSttProvider, ipcSttConfig } from '../brain/components/stt-provider';
import { core } from '../brain/test-core';
import { argsOf, callsTo } from '../test-support/cloud-calls';

vi.mock('@tauri-apps/api/core', async () => (await import('../brain/test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-0000000000a1';
const tags: Tags = { app: 'bottle', mog: MOG, op: 'asr' };
const seg = (ms: number): SegmentRef => ({
  segmentId: `s${ms}`,
  leaseId: 'lease-1',
  mogId: MOG,
  track: 'you',
  tStartMs: 0,
  tEndMs: ms,
  final: true,
});
const sttProviders = () =>
  core.invoke.mock.calls
    .filter(([cmd]) => cmd === 'stt_transcribe')
    .map(([, a]) => (argsOf(a) as { provider: string }).provider);

const ASR_OPT_IN = { asr: async () => ({ profile: 'user-profile' }) };

/** The core double: `policy_get` gives `policy`; `stt_transcribe` answers for its provider. */
function useStt(policy: unknown = {}): void {
  core.invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'policy_get') return policy;
    const a = argsOf(args) as { segmentId?: string; provider?: string };
    return {
      segmentId: a.segmentId,
      text: 'text',
      raw: 'text',
      provider: a.provider,
      model: 'm',
      runtime: 'r',
      latencyMs: 300,
    };
  });
}

describe('AT-40 SttProvider rules', () => {
  beforeEach(() => core.reset());

  it('[AT-40] [CAP-47a] [CAP-47b] SttProvider offers local Parakeet and Databricks ASR, and Whisper only with an admin config', async () => {
    useStt();
    const stt = createSttProvider();
    expect(stt.activeProvider()).toBe('local_parakeet');
    await stt.transcribe(seg(3_000), tags);
    // Databricks ASR after a passed probe and the user's opt-in (#51 fills the opt-in).
    const asr = createSttProvider({ config: ipcSttConfig(ASR_OPT_IN) });
    await asr.transcribe(seg(3_000), tags);
    // No config has a Whisper endpoint, so no call names dbx_whisper.
    expect(sttProviders()).toEqual(['local_parakeet', 'dbx_gemini']);
    expect(sttProviders()).not.toContain('dbx_whisper');
  });

  it.skip('[AT-40] [CAP-54a] [CAP-54b] no Databricks ASR call for less than 300 ms of speech, and silence is trimmed TODO(#49)', async () => {
    const stt = createSttProvider();
    await stt.transcribe(seg(250), tags);
    expect(sttProviders()).not.toContain('dbx_gemini');
    // TODO(#49): a 3 s segment with 1 s of silence at each end goes out starting and ending with
    // speech +- 100 ms (read the trimmed bounds from the ASR client double).
    expect(callsTo(core.invoke.mock.calls, ['dbx_http'])).toEqual([]);
  });

  it.skip('[AT-40] [CAP-56a] [CAP-56b] ASR text above 6 words per second or a known hallucination phrase is dropped and local STT runs TODO(#49)', async () => {
    core.invoke.mockImplementation(async (_cmd: string, args?: unknown) => {
      const p = (argsOf(args) as { provider?: string } | undefined)?.provider;
      const text = p === 'dbx_gemini' ? 'Thank you for watching.' : '';
      return {
        segmentId: 's',
        text,
        raw: text,
        provider: p,
        model: 'm',
        runtime: 'r',
        latencyMs: 300,
      };
    });
    const t = await createSttProvider().transcribe(seg(2_000), tags);
    expect(t.provider).toBe('local_parakeet');
    expect(t.text).toBe('');
  });

  it('[AT-40] [CAP-64a] [CAP-64b] a language Parakeet does not cover goes to Databricks ASR, or the MOG says it is not available', async () => {
    // A Japanese clip with Databricks ASR permitted goes to dbx_gemini (CAP-64a).
    useStt();
    const t = await createSttProvider({ config: ipcSttConfig(ASR_OPT_IN) }).transcribe(
      seg(3_000),
      tags,
      { language: 'ja' },
    );
    expect(t.lang).toBe('ja');
    expect(t.provider).toBe('dbx_gemini');
    // With local_only it gives the fixed text and no empty transcript (CAP-64b).
    core.reset();
    useStt({ SttPolicy: 'local_only' });
    const failed = createSttProvider({ config: ipcSttConfig(ASR_OPT_IN) }).transcribe(
      seg(3_000),
      tags,
      { language: 'ja' },
    );
    await expect(failed).rejects.toBeInstanceOf(SttFailure);
    await expect(failed).rejects.toMatchObject({
      kind: 'language_not_available',
      userText: 'Transcription of this language is not available on this device.',
    });
    expect(sttProviders()).toEqual([]);
  });

  it.skip('[AT-40] [CAP-65a] [CAP-65b] the release record has a current benchmark report, and only a passing provider can be the default TODO(#53)', () => {
    // TODO(#53): read the release record; its AT-40 report has the build and model of this
    // release, and SttProvider refuses a default provider whose report is not `pass`.
    expect.unreachable('TODO(#53): the benchmark runner and the release record are not built');
  });
});
