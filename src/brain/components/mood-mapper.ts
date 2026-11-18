/**
 * T11 MoodMapper (4.3, hidden TS brain window). Implements the 4.7 `MoodMapper`.
 *
 * Responsibilities: Pure function from capture and request states to mood and overlays.
 *
 * Priority (4.8, UX-01a): paused > capture > error > success > answering > processing >
 * dozing > idle. A paused or dozing MOG is `sleeping`; capture is `listening`. The error and
 * success windows end at `errorUntil` and `successUntil` (3 s and 2.5 s; 1.2 s after Inserted).
 *
 * The brain owns the mood; view windows only render it (UX-01b). See ADR 0018 for the inputs,
 * the agent `set_mood` slot (UX-41) and the 400 ms hold (UX-05a).
 */
import type { CaptureState, Mood, MogId, MoodMapper } from '../../shared/interfaces';

type MoodInput = Parameters<MoodMapper>[0];
type MoodOutput = ReturnType<MoodMapper>;
type Overlays = MoodOutput['overlays'];

/** The 7 moods of the Rive file (UX-02a). */
export const MOODS: readonly Mood[] = [
  'idle',
  'listening',
  'processing',
  'answering',
  'success',
  'error',
  'sleeping',
];

/** 4.8: error shows 3 s, success 2.5 s, success after an Inserted (UC1) 1.2 s. */
export const ERROR_MS = 3000;
export const SUCCESS_MS = 2500;
export const INSERTED_MS = 1200;
/** 4.8: a blurb reveal is `answering` for 1.2 s or less. */
export const BLURB_REVEAL_MS = 1200;
/** ADR 0018: an agent mood other than error or success ends after 10 s. */
export const AGENT_MOOD_MS = 10_000;
/** UX-05a: each mood holds for 400 ms or more. */
export const MOOD_HOLD_MS = 400;
/** 4.8: a chip offered below this router confidence shows `attention`. */
const CHIP_ATTENTION_BELOW = 0.7;

/** Region A (4.8): the MOG lifecycle. TODO(#98): take the MogManager lifecycle type when #98 adds it. */
export type Lifecycle =
  'defined' | 'starting' | 'ready' | 'dozing' | 'paused' | 'login_required' | 'offline' | 'stopped';

/** Region C (4.8): the request phases that pick a mood (C0-C19, plus Inserted for UC1). */
export const REQUEST_PHASES = [
  'utterance',
  'transcribing',
  'cleaning',
  'routing',
  'preview',
  'agent_turn',
  'acknowledged',
  'working',
  'rate_limited',
  'soft_notice',
  'partial',
  'blurb_reveal',
  'chip_offered',
  'needs_input',
  'ask_uncertain',
  'memory_answer',
  'completed',
  'inserted',
  'failed',
  'gave_up',
  'cancelled',
  'late_finished',
  'queued_offline',
] as const;
export type RequestPhase = (typeof REQUEST_PHASES)[number];

/** One request of this MOG. `at` is when the request entered `phase` (ms, brain clock). */
export interface RequestView {
  phase: RequestPhase;
  at: number;
  /** A failure the user must act on (4.8: `attention` if the user must act). */
  userMustAct?: boolean;
  /** Router confidence for `chip_offered`. Missing means low. */
  confidence?: number;
}

/** Region A: the MOG status and its badges. `since` is when the MOG entered `lifecycle`. */
export interface MogStatus {
  lifecycle: Lifecycle;
  since: number;
  hop?: string;
  localOnly?: boolean;
  /**
   * UX-39b (ADR 0029): when the last lifecycle change that is not a move between `ready` and
   * `offline` happened. Offline is a badge, so that move does not end an agent mood. Missing
   * means `since`. TODO(#98): the MogManager lifecycle keeps this time.
   */
  moodSince?: number;
}

/**
 * Everything the mood depends on. `capture` is the last `capture:state` (`capture-mood.ts`, #551).
 * TODO(#39): `requests` come from the OperationTracker and the pipeline.
 * TODO(#113): `agentMood` comes from the main agent `set_mood` tool.
 */
export interface MoodSources {
  mog: MogStatus;
  capture: CaptureState;
  requests: readonly RequestView[];
  /** The last `set_mood` of the main agent for this MOG (UX-41). */
  agentMood?: AgentMood;
}

/** One `set_mood` call of the main agent and when it came (ms, brain clock). */
export interface AgentMood {
  mood: Mood;
  at: number;
}

const PROCESSING: ReadonlySet<RequestPhase> = new Set([
  'utterance',
  'transcribing',
  'cleaning',
  'routing',
  'preview',
  'agent_turn',
  'acknowledged',
  'working',
  'rate_limited',
  'soft_notice',
]);
const ANSWERING_FOR: Partial<Record<RequestPhase, number>> = {
  partial: Infinity,
  blurb_reveal: BLURB_REVEAL_MS,
};
const SUCCESS_FOR: Partial<Record<RequestPhase, number>> = {
  completed: SUCCESS_MS,
  memory_answer: SUCCESS_MS,
  inserted: INSERTED_MS,
};
// GaveUp (C16) is not a failure: its card says "I will check again later" (Appendix E).
const ERROR_FOR: ReadonlySet<RequestPhase> = new Set(['failed']);

function pick(s: MoodInput, t: number, agentMood?: Mood): Mood {
  if (s.paused) return 'sleeping';
  if (s.ownsCapture) return 'listening';
  if (s.errorUntil !== undefined && s.errorUntil > t) return 'error';
  if (s.successUntil !== undefined && s.successUntil > t) return 'success';
  // UX-41: the agent mood ranks below capture, error and success. UX-07: never `listening`.
  if (agentMood !== undefined) return agentMood;
  if (s.answering) return 'answering';
  if (s.processing) return 'processing';
  if (s.dozing) return 'sleeping';
  return 'idle';
}

/**
 * Build the MoodMapper with a clock. The result depends only on the input and `now()`.
 * Overlays are not in the 4.7 input, so this form returns none; `mapMood` derives them.
 */
export function createMoodMapper(now: () => number = Date.now): MoodMapper {
  return (s: MoodInput): MoodOutput => ({
    mood: pick(s, now()),
    overlays: { attention: false, recording: false, offline: false, localOnly: false },
  });
}

const latest = (a: number | undefined, b: number): number => (a === undefined ? b : Math.max(a, b));

/** UX-07: only the owner of an open PTT or hands-free capture is `listening`. */
function ownsListeningCapture(mogId: MogId, c: CaptureState): boolean {
  return (
    c.owner === mogId && c.recording && !c.paused && (c.mode === 'ptt' || c.mode === 'hands_free')
  );
}

/**
 * UX-41 (ADR 0018): the agent mood while it is live. A newer request phase or lifecycle state
 * replaces it. An agent error lasts 3 s, a success 2.5 s and any other mood 10 s.
 */
function liveAgentMood(src: MoodSources, now: number): Mood | undefined {
  const a = src.agentMood;
  if (!a || a.mood === 'listening') return undefined;
  // UX-39b (ADR 0029): a move between ready and offline is a badge change and does not end it.
  const lifecycleAt = src.mog.moodSince ?? src.mog.since;
  if (lifecycleAt > a.at || src.requests.some((r) => r.at > a.at)) return undefined;
  const lasts = a.mood === 'error' ? ERROR_MS : a.mood === 'success' ? SUCCESS_MS : AGENT_MOOD_MS;
  return a.at + lasts > now ? a.mood : undefined;
}

/**
 * UX-01a: the mood and overlays of one MOG, a pure function of its status, the capture state
 * and its request states at time `now`. The caller applies `holdMood` before it renders.
 */
export function mapMood(mogId: MogId, src: MoodSources, now: number): MoodOutput {
  const { mog, capture, requests } = src;
  const input: MoodInput = {
    paused: mog.lifecycle === 'paused',
    ownsCapture: ownsListeningCapture(mogId, capture),
    answering: false,
    processing: false,
    dozing: mog.lifecycle === 'dozing',
  };
  let attention = false;
  if (mog.lifecycle === 'login_required') {
    input.errorUntil = mog.since + ERROR_MS;
    attention = true;
  }
  for (const r of requests) {
    if (PROCESSING.has(r.phase)) input.processing = true;
    const answer = ANSWERING_FOR[r.phase];
    if (answer !== undefined && r.at + answer > now) input.answering = true;
    const ok = SUCCESS_FOR[r.phase];
    if (ok !== undefined) input.successUntil = latest(input.successUntil, r.at + ok);
    if (ERROR_FOR.has(r.phase)) {
      input.errorUntil = latest(input.errorUntil, r.at + ERROR_MS);
      if (r.userMustAct) attention = true;
    }
    if (r.phase === 'needs_input' || r.phase === 'ask_uncertain') attention = true;
    if (r.phase === 'chip_offered' && !((r.confidence ?? 0) >= CHIP_ATTENTION_BELOW)) {
      attention = true;
    }
  }
  const overlays: Overlays = {
    attention,
    recording:
      capture.owner === mogId && capture.mode === 'session' && capture.recording && !capture.paused,
    offline: mog.lifecycle === 'offline',
    localOnly: mog.localOnly === true,
  };
  // TODO(#100): the session "PAUSED" pill and dashed ring (4.8) need an overlay in the contract.
  if (mog.hop !== undefined) overlays.hop = mog.hop;
  return { mood: pick(input, now, liveAgentMood(src, now)), overlays };
}

/** The mood a MOG shows and when it started showing. */
export interface HeldMood {
  mood: Mood;
  since: number;
}

/**
 * UX-05a: keep the shown mood for `MOOD_HOLD_MS` before a change. When a change must wait,
 * `recheckAt` says when to call again. `listening` shows at once so key-down feedback stays
 * under 100 ms (UX-05b, ADR 0018). `listening` -> `processing` (the user ends the capture)
 * also shows at once, so key-up feedback stays within 100 ms (NFR-01c, UX-36, ADR 0031).
 */
export function holdMood(
  prev: HeldMood | undefined,
  target: Mood,
  now: number,
): { shown: HeldMood; recheckAt?: number } {
  if (!prev) return { shown: { mood: target, since: now } };
  if (prev.mood === target) return { shown: prev };
  const end = prev.since + MOOD_HOLD_MS;
  const userAction =
    target === 'listening' || (prev.mood === 'listening' && target === 'processing');
  if (userAction || now >= end) return { shown: { mood: target, since: now } };
  return { shown: prev, recheckAt: end };
}
