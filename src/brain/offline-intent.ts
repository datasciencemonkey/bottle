/**
 * The RTR-15c offline heuristic and the router fallback that uses it (#163, ADR 0092).
 *
 * - RTR-15c: if the router model is not available, the offline heuristic decides the intent
 *   with a question mark or a wh-word. The router model runs on the MOG's home profile
 *   (RTR-10a, RTR-10b), so with no home profile, or when the model call fails, it is not
 *   available.
 * - RTR-01b, B5: UC1 dictation then still works on first run, with no profile and no sign-in.
 *   A plain sentence is `dictation` and goes on to insertion or Copy.
 * - B1: an offline question has a confidence below the preview bar, so it is a chip. It goes
 *   out only after a click ("Offline questions go to the outbox only after a click").
 *
 * TODO(#143): the RTR-15a command grammar and the RTR-15b fallback classifier extend this module.
 */
import type { Intent, MogId, ProfileName, RouterDecision } from '../shared/interfaces';
import type { RouterInput } from './components/router-step';
import type { UtteranceRouteDeps } from './utterance-route';

/** The router input profile of a MOG with no usable home profile. Bottle never picks one. */
export const NO_HOME: ProfileName = '';

/** Below `ASK_CONFIDENCE` (0.7): an offline question gets a chip, never the 1.5 s preview. */
export const OFFLINE_CONFIDENCE = 0.5;

const WH_WORDS = new Set(['what', 'when', 'where', 'which', 'who', 'whom', 'whose', 'why', 'how']);

/** RTR-15c: a question mark, or a wh-word as the first word, gives `question`. */
export function offlineIntent(text: string): Extract<Intent, 'question' | 'dictation' | 'noise'> {
  const t = text.trim();
  if (t === '') return 'noise';
  if (t.includes('?')) return 'question';
  const first = /^[^\p{L}]*(\p{L}+)/u.exec(t)?.[1]?.toLowerCase() ?? '';
  return WH_WORDS.has(first) ? 'question' : 'dictation';
}

/** A router decision from the heuristic alone. It has no hint, no command and no split. */
export function offlineDecision(text: string): RouterDecision {
  const intent = offlineIntent(text);
  const cleaned = text.trim();
  return {
    intent,
    cleaned_text: cleaned,
    genie_question: intent === 'question' ? cleaned : '',
    target_profile_hint: '',
    confidence: OFFLINE_CONFIDENCE,
    follow_up: false,
    topic_label: '',
    part_questions: [],
    part_profile_hints: [],
    command: 'none',
    command_arg: '',
    context_note: '',
    injection_suspected: false,
    directed_mog_id: '',
  };
}

/**
 * The route deps with the RTR-15c fallback: no home profile skips the model call, and a failed
 * model call (offline, signed out, no endpoint) gives the heuristic decision.
 */
export function withOfflineFallback(deps: UtteranceRouteDeps): UtteranceRouteDeps {
  return {
    ...deps,
    async route(text: string, mogId: MogId, input: RouterInput) {
      if (input.profile === NO_HOME) return offlineDecision(text);
      try {
        return await deps.route(text, mogId, input);
      } catch {
        return offlineDecision(text);
      }
    },
  };
}
