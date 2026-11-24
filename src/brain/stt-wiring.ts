/**
 * The STT step of the capture lifecycle (#14, #52, ADR 0088): T4 SttProvider for one final
 * segment.
 *
 * - The transcript text goes on to the utterance route, after the dictation-words correction
 *   (`shared/dictation-vocab.ts`): a paste and an ⌥G ask both get the fixed text.
 * - An `SttFailure` ("Could not transcribe. Retry", or CAP-64b "Transcription of this language
 *   is not available on this device.") shows its fixed text on the card of the MOG that owns
 *   the segment, and no text is routed. So no empty transcript ends without a message.
 * - The card holds the fixed text only: never transcript text, a profile or an error detail.
 *
 * TODO(#60): the dictation view gives the Retry control that calls `SttProvider.retry`.
 */
import type { MogId, SegmentRef } from '../shared/interfaces';
import type { CardUpdate } from '../shared/ipc';
import type { Transcribed } from './capture-lifecycle';
import { SttFailure, type SttProvider } from './components/stt-provider';

/** ADR 0123: the fixed text of the notice when Databricks dictation fell back to this Mac. */
export const CLOUD_FALLBACK_TEXT = 'Databricks dictation was not available. I used this Mac.';

/** The card update that shows the fallback notice on the MOG. */
export function cloudFallbackCard(mogId: MogId, segmentId: string): CardUpdate {
  return {
    mogId,
    blurbs: [{ id: `stt-cloud-${segmentId}`, text: CLOUD_FALLBACK_TEXT }],
    chips: [],
  };
}

/** The card update that shows an STT failure on the MOG. */
export function sttFailureCard(mogId: MogId, f: SttFailure): CardUpdate {
  return { mogId, blurbs: [{ id: `stt-${f.segmentId}`, text: f.userText }], chips: [] };
}

/** The `transcribe` dependency of the capture lifecycle. It never throws. */
export function createLifecycleTranscriber(
  stt: SttProvider,
  showCard: (mogId: MogId, update: CardUpdate) => Promise<unknown>,
  /** Dictation words: fixes the text before the route, so dictation and ⌥G both get it. */
  correct: (text: string) => string = (t) => t,
  /**
   * ADR 0123: the Settings choice "Dictation engine: Databricks". When it is on and the
   * transcript came from the local model, the cloud call failed (offline, timeout, refusal), so
   * the MOG shows a short notice. The text is the local text and goes on as usual.
   */
  cloudDictation: () => boolean = () => false,
): (seg: SegmentRef) => Promise<Transcribed> {
  return async (seg) => {
    try {
      const t = await stt.transcribe(seg, { app: 'bottle', mog: seg.mogId, op: 'asr' });
      if (cloudDictation() && t.provider === 'local_parakeet' && t.text.trim() !== '') {
        try {
          await showCard(seg.mogId, cloudFallbackCard(seg.mogId, seg.segmentId));
        } catch {
          // A closed card shows nothing; the local text still routes.
        }
      }
      // ADR 0113: `remember` is Rust's. Its text is never corrected or touched here.
      return t.remember ? { text: correct(t.text), remember: t.remember } : correct(t.text);
    } catch (e) {
      const f =
        e instanceof SttFailure ? e : new SttFailure('could_not_transcribe', seg.segmentId, false);
      try {
        await showCard(seg.mogId, sttFailureCard(seg.mogId, f));
      } catch {
        // A closed card shows nothing; the utterance still routes no text.
      }
      return '';
    }
  };
}
