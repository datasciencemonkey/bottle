import { createDictationCorrector } from '../shared/dictation-vocab';
import type { SegmentRef, Tags, Transcript } from '../shared/interfaces';
import type { CardUpdate } from '../shared/ipc';
import {
  COULD_NOT_TRANSCRIBE,
  LANGUAGE_NOT_AVAILABLE,
  SttFailure,
  type SttProvider,
} from './components/stt-provider';
import {
  CLOUD_FALLBACK_TEXT,
  cloudFallbackCard,
  createLifecycleTranscriber,
  sttFailureCard,
} from './stt-wiring';

const MOG = '00000000-0000-4000-8000-000000000052';
const seg: SegmentRef = {
  segmentId: 'ptt-lease-1',
  leaseId: 'lease-1',
  mogId: MOG,
  track: 'you',
  tStartMs: 0,
  tEndMs: 2_000,
  final: true,
};

function t4(result: Transcript | SttFailure | Error): SttProvider & { seen: Tags[] } {
  const seen: Tags[] = [];
  return {
    seen,
    async transcribe(_s, tags) {
      seen.push(tags);
      if (result instanceof Error) throw result;
      return result;
    },
    async retry() {
      throw new SttFailure('could_not_transcribe', seg.segmentId, false);
    },
    activeProvider: () => 'local_parakeet',
    sttUse: (_m) => ({ provider: 'local_parakeet', model: '' }),
  };
}

describe('the capture lifecycle STT step (#52)', () => {
  it('[CAP-47a] the text of the T4 transcript goes on, with the asr tags of the MOG', async () => {
    const stt = t4({
      segmentId: seg.segmentId,
      text: 'hello',
      raw: 'hello',
      provider: 'local_parakeet',
      model: 'm',
      runtime: 'r',
      latencyMs: 1,
    });
    const show = vi.fn();
    expect(await createLifecycleTranscriber(stt, show)(seg)).toBe('hello');
    expect(stt.seen).toEqual([{ app: 'bottle', mog: MOG, op: 'asr' }]);
    expect(show).not.toHaveBeenCalled();
  });

  describe('[ADR 0123] Databricks dictation', () => {
    const tr = (provider: Transcript['provider'], text = 'hello'): Transcript => ({
      segmentId: seg.segmentId,
      text,
      raw: text,
      provider,
      model: 'm',
      runtime: 'r',
      latencyMs: 1,
    });

    it('a cloud transcript goes on with no notice', async () => {
      const show = vi.fn();
      const text = await createLifecycleTranscriber(
        t4(tr('dbx_gemini', 'cloud words')),
        show,
        (t) => t,
        () => true,
      )(seg);
      expect(text).toBe('cloud words');
      expect(show).not.toHaveBeenCalled();
    });

    it('with the choice on, a local transcript shows the fallback notice and goes on', async () => {
      const show = vi.fn(async (_m: string, _u: CardUpdate) => undefined);
      const text = await createLifecycleTranscriber(
        t4(tr('local_parakeet', 'local words')),
        show,
        (t) => t,
        () => true,
      )(seg);
      expect(text).toBe('local words');
      expect(show).toHaveBeenCalledWith(MOG, cloudFallbackCard(MOG, seg.segmentId));
      expect(cloudFallbackCard(MOG, seg.segmentId).blurbs[0]?.text).toBe(CLOUD_FALLBACK_TEXT);
      // The notice holds no transcript text.
      expect(JSON.stringify(cloudFallbackCard(MOG, seg.segmentId))).not.toContain('local words');
    });

    it('with the choice off (the default), a local transcript shows no notice', async () => {
      const show = vi.fn();
      expect(await createLifecycleTranscriber(t4(tr('local_parakeet')), show)(seg)).toBe('hello');
      expect(show).not.toHaveBeenCalled();
    });

    it('an empty local transcript (no speech) shows no notice', async () => {
      const show = vi.fn();
      await createLifecycleTranscriber(
        t4(tr('local_parakeet', '  ')),
        show,
        (t) => t,
        () => true,
      )(seg);
      expect(show).not.toHaveBeenCalled();
    });

    it('a closed card does not stop the text', async () => {
      const text = await createLifecycleTranscriber(
        t4(tr('local_parakeet', 'local words')),
        async () => {
          throw new Error('closed');
        },
        (t) => t,
        () => true,
      )(seg);
      expect(text).toBe('local words');
    });
  });

  it('dictation words: the corrected text goes on, so a paste and an ask get it', async () => {
    const stt = t4({
      segmentId: seg.segmentId,
      text: 'Can I create a G Way N API?',
      raw: 'Can I create a G Way N API?',
      provider: 'local_parakeet',
      model: 'm',
      runtime: 'r',
      latencyMs: 1,
    });
    const text = await createLifecycleTranscriber(
      stt,
      vi.fn(),
      createDictationCorrector(() => ''),
    )(seg);
    expect(text).toBe('Can I create a Gateway API?');
  });

  it('[CAP-64b] [AT-40] "not available" shows on the MOG, and no text is routed', async () => {
    const show = vi.fn(async (_m: string, _u: CardUpdate) => undefined);
    const text = await createLifecycleTranscriber(
      t4(new SttFailure('language_not_available', seg.segmentId, false)),
      show,
    )(seg);
    expect(text).toBe('');
    expect(show).toHaveBeenCalledWith(MOG, {
      mogId: MOG,
      blurbs: [{ id: `stt-${seg.segmentId}`, text: LANGUAGE_NOT_AVAILABLE }],
      chips: [],
    });
  });

  it('[CAP-60a] "Could not transcribe. Retry" shows on the MOG at the end of the chain', async () => {
    const show = vi.fn(async (_m: string, _u: CardUpdate) => undefined);
    await createLifecycleTranscriber(
      t4(new SttFailure('could_not_transcribe', seg.segmentId, true)),
      show,
    )(seg);
    expect(show.mock.calls[0]?.[1].blurbs[0]?.text).toBe(COULD_NOT_TRANSCRIBE);
  });

  it('[CAP-60a] another error routes no text and shows the failure text, with no error detail', async () => {
    const show = vi.fn(async (_m: string, _u: CardUpdate) => undefined);
    const text = await createLifecycleTranscriber(t4(new Error('detail')), show)(seg);
    expect(text).toBe('');
    expect(JSON.stringify(show.mock.calls)).not.toContain('detail');
  });

  it('[CAP-64b] a failed card send still routes no text', async () => {
    const show = vi.fn(async () => {
      throw new Error('card closed');
    });
    const stt = t4(new SttFailure('language_not_available', seg.segmentId, false));
    expect(await createLifecycleTranscriber(stt, show)(seg)).toBe('');
  });

  it('[CAP-64b] the failure card holds the fixed text only', () => {
    expect(
      sttFailureCard(MOG, new SttFailure('language_not_available', 'ptt-lease-9', false)),
    ).toEqual({
      mogId: MOG,
      blurbs: [{ id: 'stt-ptt-lease-9', text: LANGUAGE_NOT_AVAILABLE }],
      chips: [],
    });
  });

  it('[ADR 0113] the remember order of Rust goes on, its text untouched', async () => {
    const stt = t4({
      segmentId: seg.segmentId,
      text: 'remember this: Sam',
      raw: 'remember this: Sam',
      provider: 'local_parakeet',
      model: 'm',
      runtime: 'r',
      latencyMs: 1,
      remember: { ticket: 'tkt', text: 'Sam' },
    });
    expect(await createLifecycleTranscriber(stt, vi.fn(), (t) => t.toUpperCase())(seg)).toEqual({
      text: 'REMEMBER THIS: SAM',
      remember: { ticket: 'tkt', text: 'Sam' },
    });
  });
});
