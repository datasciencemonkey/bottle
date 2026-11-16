/**
 * T11 MoodMapper (4.3, hidden TS brain window). Implements the 4.7 `MoodMapper`.
 *
 * Responsibilities: Pure function from capture and request states to mood and overlays.
 *
 * Priority (4.8, UX-01a): paused > capture > error > success > answering > processing >
 * dozing > idle. A paused or dozing MOG is `sleeping`; capture is `listening`. The error and
 * success windows end at `errorUntil` and `successUntil` (the caller sets 3 s and 2.5 s).
 */
import type { Mood, MoodMapper } from '../../shared/interfaces';

type MoodInput = Parameters<MoodMapper>[0];
type MoodOutput = ReturnType<MoodMapper>;

function pick(s: MoodInput, t: number): Mood {
  if (s.paused) return 'sleeping';
  if (s.ownsCapture) return 'listening';
  if (s.errorUntil !== undefined && s.errorUntil > t) return 'error';
  if (s.successUntil !== undefined && s.successUntil > t) return 'success';
  if (s.answering) return 'answering';
  if (s.processing) return 'processing';
  if (s.dozing) return 'sleeping';
  return 'idle';
}

/**
 * Build the MoodMapper with a clock. The result depends only on the input and `now()`.
 * TODO(#9): hold times, the overlay rules of 4.8 and the agent `set_mood` tool.
 */
export function createMoodMapper(now: () => number = Date.now): MoodMapper {
  return (s: MoodInput): MoodOutput => ({
    mood: pick(s, now()),
    overlays: { attention: false, recording: false, offline: false, localOnly: false },
  });
}
