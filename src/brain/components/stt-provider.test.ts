import type { SegmentRef, Tags, Transcript } from '../../shared/interfaces';
import { argsOf, callsTo, cloudCalls } from '../../test-support/cloud-calls';
import { core } from '../test-core';
import { DEFAULT_ASR_MODEL } from './model-access';
import {
  COULD_NOT_TRANSCRIBE,
  HEDGE_MS,
  LANGUAGE_NOT_AVAILABLE,
  RETRY_WINDOW_MS,
  SttFailure,
  coversLanguage,
  createSttProvider,
  ipcSttConfig,
  type SttConfig,
} from './stt-provider';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);

const MOG = '00000000-0000-4000-8000-000000000052';
const tags: Tags = { app: 'bottle', mog: MOG, op: 'asr' };
const PROFILE = 'user-profile';
const seg: SegmentRef = {
  segmentId: 'ptt-lease-1',
  leaseId: 'lease-1',
  mogId: MOG,
  track: 'you',
  tStartMs: 0,
  tEndMs: 3_000,
  final: true,
};

type Provider = Transcript['provider'];

/** How the `stt_transcribe` double answers one provider. */
interface Plan {
  /** Reply after this many ms (fake timers). Default 0. */
  delayMs?: number;
  /** Reject with a content-free IPC error. */
  fail?: boolean;
  text?: string;
  lang?: string;
  /** Reply for another segment or provider (an invalid reply). */
  wrongSegment?: boolean;
}

interface Double {
  policy?: unknown;
  policyFails?: boolean;
  plans?: Partial<Record<Provider, Plan>>;
}

const transcript = (p: Provider, plan: Plan): Transcript => ({
  segmentId: plan.wrongSegment ? 'another-segment' : seg.segmentId,
  text: plan.text ?? `text from ${p}`,
  raw: plan.text ?? `text from ${p}`,
  ...(plan.lang === undefined ? {} : { lang: plan.lang }),
  provider: p,
  model: p === 'local_parakeet' ? 'parakeet-tdt-0.6b-v3-int8' : 'cloud-model',
  runtime: 'test-runtime',
  latencyMs: plan.delayMs ?? 0,
});

/** The IPC test double of the core: `policy_get` and `stt_transcribe`. */
function useCore(d: Double = {}): void {
  core.invoke.mockImplementation(async (cmd: string, payload?: unknown) => {
    if (cmd === 'policy_get') {
      if (d.policyFails) throw { error: 'not_implemented' };
      return d.policy ?? {};
    }
    if (cmd !== 'stt_transcribe') return {};
    const a = argsOf(payload) as { provider: Provider };
    const plan = d.plans?.[a.provider] ?? {};
    if (plan.delayMs) await new Promise((r) => setTimeout(r, plan.delayMs));
    if (plan.fail) throw { error: 'host_failed' };
    return transcript(a.provider, plan);
  });
}

/** The `stt_transcribe` calls in order, with their arguments. */
const sttCalls = () =>
  core.invoke.mock.calls
    .filter(([cmd]) => cmd === 'stt_transcribe')
    .map(([, a]) => argsOf(a) as Record<string, unknown>);
const providers = () => sttCalls().map((a) => a['provider']);

const ASR_ON = { asr: async () => ({ profile: PROFILE }) };
const WHISPER_ON = {
  whisper: async () => ({ profile: PROFILE, model: 'admin-whisper-endpoint' }),
};

async function failureOf(p: Promise<unknown>): Promise<SttFailure> {
  try {
    await p;
  } catch (e) {
    if (e instanceof SttFailure) return e;
    throw e;
  }
  throw new Error('no failure');
}

describe('T4 SttProvider (#52)', () => {
  beforeEach(() => core.reset());
  afterEach(() => vi.useRealTimers());

  describe('providers', () => {
    it('[CAP-47a] [CAP-57c] local Parakeet is the default and a new install sends no audio off the device', async () => {
      useCore();
      const stt = createSttProvider();
      expect(stt.activeProvider()).toBe('local_parakeet');
      const t = await stt.transcribe(seg, tags);
      expect(t.provider).toBe('local_parakeet');
      expect(t.text).toBe('text from local_parakeet');
      expect(sttCalls()).toEqual([{ segmentId: seg.segmentId, provider: 'local_parakeet', tags }]);
      expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
    });

    it('[CAP-47a] Databricks ASR runs after the opt-in, with the user profile and the default model', async () => {
      useCore();
      const stt = createSttProvider({ config: ipcSttConfig(ASR_ON) });
      const t = await stt.transcribe(seg, tags);
      expect(t.provider).toBe('dbx_gemini');
      expect(stt.activeProvider()).toBe('dbx_gemini');
      expect(sttCalls()).toEqual([
        {
          segmentId: seg.segmentId,
          provider: 'dbx_gemini',
          profile: PROFILE,
          model: DEFAULT_ASR_MODEL,
          tags,
        },
      ]);
    });

    it('[CAP-47a] MDM AsrModelService replaces the default ASR model', async () => {
      useCore({ policy: { AsrModelService: 'admin-asr-model' } });
      await createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      expect(sttCalls()[0]?.['model']).toBe('admin-asr-model');
    });

    it('[CAP-47b] [AT-40] no Whisper call occurs without the admin config, also when every other step fails', async () => {
      useCore({ plans: { local_parakeet: { fail: true }, dbx_gemini: { fail: true } } });
      const f = await failureOf(
        createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags),
      );
      expect(f.kind).toBe('could_not_transcribe');
      expect(providers()).not.toContain('dbx_whisper');
    });

    it('[CAP-47b] the admin Whisper endpoint runs when it is configured and local STT fails', async () => {
      useCore({ plans: { local_parakeet: { fail: true } } });
      const t = await createSttProvider({ config: ipcSttConfig(WHISPER_ON) }).transcribe(seg, tags);
      expect(t.provider).toBe('dbx_whisper');
      expect(sttCalls()[1]).toEqual({
        segmentId: seg.segmentId,
        provider: 'dbx_whisper',
        profile: PROFILE,
        model: 'admin-whisper-endpoint',
        tags,
      });
    });
  });

  describe('fallback chain (8.3)', () => {
    it('[CAP-60a] [AT-04] default chain: local Parakeet, then Whisper, then "Could not transcribe. Retry"', async () => {
      useCore({ plans: { local_parakeet: { fail: true }, dbx_whisper: { fail: true } } });
      const f = await failureOf(
        createSttProvider({ config: ipcSttConfig(WHISPER_ON) }).transcribe(seg, tags),
      );
      expect(providers()).toEqual(['local_parakeet', 'dbx_whisper']);
      expect(f.kind).toBe('could_not_transcribe');
      expect(f.userText).toBe(COULD_NOT_TRANSCRIBE);
      expect(COULD_NOT_TRANSCRIBE).toBe('Could not transcribe. Retry');
    });

    it('[CAP-60a] [AT-04] ASR chain: Databricks ASR, then local Parakeet, then Whisper, then the failure', async () => {
      useCore({
        plans: {
          dbx_gemini: { fail: true },
          local_parakeet: { fail: true },
          dbx_whisper: { fail: true },
        },
      });
      const f = await failureOf(
        createSttProvider({ config: ipcSttConfig({ ...ASR_ON, ...WHISPER_ON }) }).transcribe(
          seg,
          tags,
        ),
      );
      expect(providers()).toEqual(['dbx_gemini', 'local_parakeet', 'dbx_whisper']);
      expect(f.kind).toBe('could_not_transcribe');
    });

    it('[CAP-60a] [AT-04] a failed ASR step goes to local Parakeet', async () => {
      useCore({ plans: { dbx_gemini: { fail: true } } });
      const t = await createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      expect(t.provider).toBe('local_parakeet');
      expect(providers()).toEqual(['dbx_gemini', 'local_parakeet']);
    });

    it('[CAP-60a] [CAP-56b] an invalid ASR reply (empty text, another segment) goes to the next step', async () => {
      useCore({ plans: { dbx_gemini: { text: '  ' } } });
      expect(
        (await createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags)).provider,
      ).toBe('local_parakeet');
      core.reset();
      useCore({ plans: { dbx_gemini: { wrongSegment: true } } });
      expect(
        (await createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags)).provider,
      ).toBe('local_parakeet');
    });

    it('[CAP-60a] an empty local transcript is a valid "no speech" result, not a failure', async () => {
      useCore({ plans: { local_parakeet: { text: '' } } });
      const t = await createSttProvider({ config: ipcSttConfig(WHISPER_ON) }).transcribe(seg, tags);
      expect(t.text).toBe('');
      expect(providers()).toEqual(['local_parakeet']);
    });

    it('[CAP-60a] Retry runs the chain again for 60 s after the failure, then it is refused', async () => {
      vi.useFakeTimers({ now: 1_000_000 });
      useCore({ plans: { local_parakeet: { fail: true } } });
      const stt = createSttProvider();
      const f = await failureOf(stt.transcribe(seg, tags));
      expect(f.retryable).toBe(true);
      useCore();
      vi.setSystemTime(1_000_000 + RETRY_WINDOW_MS);
      expect((await stt.retry(seg, tags)).provider).toBe('local_parakeet');
      // A success ends the retry window of the segment.
      const again = await failureOf(stt.retry(seg, tags));
      expect(again.retryable).toBe(false);
    });

    it('[CAP-60a] Retry after 60 s is refused with no STT call', async () => {
      vi.useFakeTimers({ now: 1_000_000 });
      useCore({ plans: { local_parakeet: { fail: true } } });
      const stt = createSttProvider();
      await failureOf(stt.transcribe(seg, tags));
      vi.setSystemTime(1_000_000 + RETRY_WINDOW_MS + 1);
      const n = sttCalls().length;
      const f = await failureOf(stt.retry(seg, tags));
      expect(f.retryable).toBe(false);
      expect(sttCalls()).toHaveLength(n);
    });
  });

  describe('SttPolicy (Appendix C)', () => {
    it('[CAP-60b] [AT-04] MDM local_only removes every cloud step: no ASR and no Whisper request', async () => {
      useCore({ policy: { SttPolicy: 'local_only' }, plans: { local_parakeet: { fail: true } } });
      const stt = createSttProvider({ config: ipcSttConfig({ ...ASR_ON, ...WHISPER_ON }) });
      const f = await failureOf(stt.transcribe(seg, tags));
      expect(f.kind).toBe('could_not_transcribe');
      expect(providers()).toEqual(['local_parakeet']);
      expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
      expect(stt.activeProvider()).toBe('local_parakeet');
    });

    it('[CAP-60b] an unknown SttPolicy value or a failed policy read means local_only', async () => {
      useCore({ policy: { SttPolicy: 'cloud' } });
      await createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
      core.reset();
      useCore({ policyFails: true });
      await createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
    });

    it('[CAP-60b] a config source that throws means local_only', async () => {
      useCore();
      const config = {
        read: async (): Promise<SttConfig> => {
          throw new Error('config failed');
        },
      };
      expect((await createSttProvider({ config }).transcribe(seg, tags)).provider).toBe(
        'local_parakeet',
      );
      expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
    });

    it('[CAP-60b] a config with local_only and an ASR entry still sends no cloud request', async () => {
      useCore();
      const config = {
        read: async (): Promise<SttConfig> => ({
          sttPolicy: 'local_only',
          asr: { profile: PROFILE, model: DEFAULT_ASR_MODEL },
          whisper: { profile: PROFILE, model: 'w' },
        }),
      };
      await createSttProvider({ config }).transcribe(seg, tags);
      expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
    });

    it('[CAP-60b] [CAP-57e] MDM cloud_only without the probe pass (no opt-in) keeps local STT', async () => {
      useCore({ policy: { SttPolicy: 'cloud_only' } });
      await createSttProvider({ config: ipcSttConfig() }).transcribe(seg, tags);
      expect(providers()).toEqual(['local_parakeet']);
    });

    it('[AT-04] [CAP-60b] offline (ARC-F7), STT goes to local and no cloud request occurs', async () => {
      useCore();
      const stt = createSttProvider({
        config: ipcSttConfig({ ...ASR_ON, ...WHISPER_ON }),
        net: { online: () => false },
      });
      expect((await stt.transcribe(seg, tags)).provider).toBe('local_parakeet');
      expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
    });
  });

  describe('hedge (CAP-59a, CAP-59b)', () => {
    it('[CAP-59a] [AT-21] with no cloud reply, the local decode starts at 1.2 s and not before', async () => {
      vi.useFakeTimers();
      useCore({ plans: { dbx_gemini: { delayMs: 5_000 } } });
      const p = createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      await vi.advanceTimersByTimeAsync(HEDGE_MS - 1);
      expect(providers()).toEqual(['dbx_gemini']);
      await vi.advanceTimersByTimeAsync(1);
      expect(providers()).toEqual(['dbx_gemini', 'local_parakeet']);
      expect(HEDGE_MS).toBe(1_200);
      // CAP-59b: the local result is first, so it wins; the late cloud reply changes nothing.
      const t = await p;
      expect(t.provider).toBe('local_parakeet');
      await vi.advanceTimersByTimeAsync(5_000);
      expect(providers()).toEqual(['dbx_gemini', 'local_parakeet']);
    });

    it('[CAP-59a] a cloud reply before 1.2 s starts no local decode', async () => {
      vi.useFakeTimers();
      useCore({ plans: { dbx_gemini: { delayMs: 800 } } });
      const p = createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      await vi.advanceTimersByTimeAsync(800);
      expect((await p).provider).toBe('dbx_gemini');
      await vi.advanceTimersByTimeAsync(5_000);
      expect(providers()).toEqual(['dbx_gemini']);
    });

    it('[CAP-59b] [AT-21] a cloud reply after 1.2 s that comes before the local result wins', async () => {
      vi.useFakeTimers();
      useCore({ plans: { dbx_gemini: { delayMs: 1_500 }, local_parakeet: { delayMs: 1_000 } } });
      const p = createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      await vi.advanceTimersByTimeAsync(1_500);
      expect((await p).provider).toBe('dbx_gemini');
      expect(providers()).toEqual(['dbx_gemini', 'local_parakeet']);
    });

    it('[CAP-59b] a failed local decode in the hedge waits for the cloud result', async () => {
      vi.useFakeTimers();
      useCore({
        plans: { dbx_gemini: { delayMs: 2_000 }, local_parakeet: { delayMs: 100, fail: true } },
      });
      const p = createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      await vi.advanceTimersByTimeAsync(2_000);
      expect((await p).provider).toBe('dbx_gemini');
    });

    it('[CAP-59b] [CAP-60a] both hedged decodes fail: local is not run again and the chain goes to Whisper', async () => {
      vi.useFakeTimers();
      useCore({
        plans: {
          dbx_gemini: { delayMs: 2_000, fail: true },
          local_parakeet: { delayMs: 100, fail: true },
        },
      });
      const p = createSttProvider({
        config: ipcSttConfig({ ...ASR_ON, ...WHISPER_ON }),
      }).transcribe(seg, tags);
      await vi.advanceTimersByTimeAsync(2_000);
      expect((await p).provider).toBe('dbx_whisper');
      expect(providers()).toEqual(['dbx_gemini', 'local_parakeet', 'dbx_whisper']);
    });

    it('[CAP-59a] [CAP-60a] a cloud failure before 1.2 s starts the local decode at once', async () => {
      vi.useFakeTimers();
      useCore({ plans: { dbx_gemini: { delayMs: 300, fail: true } } });
      const p = createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      await vi.advanceTimersByTimeAsync(300);
      expect(providers()).toEqual(['dbx_gemini', 'local_parakeet']);
      expect((await p).provider).toBe('local_parakeet');
    });
  });

  describe('language routing (CAP-64a, CAP-64b, D59)', () => {
    it('[CAP-64a] Parakeet v3 covers the 25 European languages and not Japanese', () => {
      for (const l of ['en', 'de', 'fr', 'es', 'it', 'pt', 'pl', 'ru', 'uk', 'en-GB', 'PT-br']) {
        expect(coversLanguage(l)).toBe(true);
      }
      for (const l of ['ja', 'ja-JP', 'zh', 'ko', 'hi', 'ar', 'ga']) {
        expect(coversLanguage(l)).toBe(false);
      }
    });

    it('[CAP-64a] [AT-40] a Japanese clip goes to Databricks ASR when the policy permits it, and the hedge skips local', async () => {
      vi.useFakeTimers();
      useCore({ plans: { dbx_gemini: { delayMs: 2_000, text: 'こんにちは' } } });
      const p = createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags, {
        language: 'ja',
      });
      await vi.advanceTimersByTimeAsync(2_000);
      const t = await p;
      expect(t.provider).toBe('dbx_gemini');
      expect(t.lang).toBe('ja');
      expect(providers()).toEqual(['dbx_gemini']);
      expect(sttCalls()[0]?.['language']).toBe('ja');
    });

    it('[CAP-64a] [AT-40] local STT that reports Japanese hands the clip to Databricks ASR in the hedge', async () => {
      vi.useFakeTimers();
      useCore({
        plans: {
          dbx_gemini: { delayMs: 3_000, text: 'こんにちは' },
          local_parakeet: { delayMs: 100, text: 'konnichiwa', lang: 'ja' },
        },
      });
      const p = createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags);
      await vi.advanceTimersByTimeAsync(3_000);
      const t = await p;
      expect(t.provider).toBe('dbx_gemini');
      expect(t.lang).toBe('ja');
    });

    it('[CAP-64b] [AT-40] with local_only, a Japanese clip gives the "not available" message and no empty transcript', async () => {
      useCore({ policy: { SttPolicy: 'local_only' } });
      const f = await failureOf(
        createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags, {
          language: 'ja',
        }),
      );
      expect(f.kind).toBe('language_not_available');
      expect(f.userText).toBe(LANGUAGE_NOT_AVAILABLE);
      expect(LANGUAGE_NOT_AVAILABLE).toBe(
        'Transcription of this language is not available on this device.',
      );
      expect(providers()).toEqual([]);
      expect(cloudCalls(core.invoke.mock.calls)).toEqual([]);
    });

    it('[CAP-64b] [AT-40] local STT that reports an uncovered language, with no permitted provider, gives the message', async () => {
      useCore({ plans: { local_parakeet: { text: 'garbled', lang: 'ja-JP' } } });
      const f = await failureOf(createSttProvider().transcribe(seg, tags));
      expect(f.kind).toBe('language_not_available');
      expect(providers()).toEqual(['local_parakeet']);
    });

    it('[CAP-64b] a configured Whisper endpoint covers an uncovered language', async () => {
      useCore({ plans: { local_parakeet: { text: 'garbled', lang: 'ja' } } });
      const t = await createSttProvider({ config: ipcSttConfig(WHISPER_ON) }).transcribe(seg, tags);
      expect(t.provider).toBe('dbx_whisper');
      expect(sttCalls()[1]?.['language']).toBe('ja');
    });

    it('[CAP-64b] a permitted provider that fails on an uncovered language gives "Could not transcribe. Retry"', async () => {
      useCore({ plans: { dbx_gemini: { fail: true } } });
      const f = await failureOf(
        createSttProvider({ config: ipcSttConfig(ASR_ON) }).transcribe(seg, tags, {
          language: 'ja',
        }),
      );
      expect(f.kind).toBe('could_not_transcribe');
      expect(providers()).toEqual(['dbx_gemini']);
    });
  });

  describe('content-free errors and the STT badge source', () => {
    it('[CAP-72] [SEC] a failure holds no transcript text, no profile and no audio', async () => {
      useCore({
        policy: { SttPolicy: 'local_only' },
        plans: { local_parakeet: { text: 'secret dictation words', lang: 'ja' } },
      });
      const f = await failureOf(createSttProvider().transcribe(seg, tags));
      const shown = `${String(f)} ${f.message} ${JSON.stringify(f)}`;
      expect(shown).not.toContain('secret dictation words');
      expect(shown).not.toContain(PROFILE);
    });

    it('[UX-38] sttUse is the selected provider before a transcript, then the provider of the last one', async () => {
      useCore({ plans: { dbx_gemini: { fail: true } } });
      const stt = createSttProvider({ config: ipcSttConfig(ASR_ON) });
      expect(stt.sttUse()).toEqual({ provider: 'local_parakeet', model: '' });
      await stt.transcribe(seg, tags);
      expect(stt.sttUse()).toEqual({
        provider: 'local_parakeet',
        model: 'parakeet-tdt-0.6b-v3-int8',
      });
      expect(callsTo(core.invoke.mock.calls, ['dbx_http', 'model_invoke'])).toEqual([]);
    });
  });
});
