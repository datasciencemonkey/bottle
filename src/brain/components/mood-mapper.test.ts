import type { CaptureState, Mood, MoodMapper } from '../../shared/interfaces';
import {
  createMoodMapper,
  ERROR_MS,
  holdMood,
  INSERTED_MS,
  mapMood,
  MOOD_HOLD_MS,
  MOODS,
  REQUEST_PHASES,
  SUCCESS_MS,
  type HeldMood,
  type MoodSources,
  type RequestPhase,
  type RequestView,
} from './mood-mapper';

const NOW = 1_000_000;
const mapper = createMoodMapper(() => NOW);

type State = Parameters<typeof mapper>[0];
const base: State = {
  paused: false,
  ownsCapture: false,
  answering: false,
  processing: false,
  dozing: false,
};

/** One condition per 4.8 priority level, from high to low. */
const LEVELS: { name: string; set: Partial<State>; mood: Mood }[] = [
  { name: 'paused', set: { paused: true }, mood: 'sleeping' },
  { name: 'capture', set: { ownsCapture: true }, mood: 'listening' },
  { name: 'error', set: { errorUntil: NOW + 3000 }, mood: 'error' },
  { name: 'success', set: { successUntil: NOW + 2500 }, mood: 'success' },
  { name: 'answering', set: { answering: true }, mood: 'answering' },
  { name: 'processing', set: { processing: true }, mood: 'processing' },
  { name: 'dozing', set: { dozing: true }, mood: 'sleeping' },
];

describe('T11 MoodMapper (4.8)', () => {
  it('[T11] [UX-01a] maps no condition to idle', () => {
    expect(mapper(base).mood).toBe('idle');
  });

  it.each(LEVELS)('[T11] [UX-01a] maps $name alone to $mood', ({ set, mood }) => {
    expect(mapper({ ...base, ...set }).mood).toBe(mood);
  });

  const pairs = LEVELS.slice(0, -1).map((hi, i) => ({ hi, lo: LEVELS[i + 1] ?? hi }));
  it.each(pairs)('[T11] [UX-01a] $hi.name wins over $lo.name', ({ hi, lo }) => {
    expect(mapper({ ...base, ...lo.set, ...hi.set }).mood).toBe(hi.mood);
  });

  it('[T11] [UX-01a] dozing wins over idle', () => {
    expect(mapper({ ...base, dozing: true }).mood).toBe('sleeping');
  });

  it('[T11] [UX-01a] the highest condition wins when all are set', () => {
    const all = LEVELS.reduce<State>((s, l) => ({ ...s, ...l.set }), base);
    expect(mapper(all).mood).toBe('sleeping');
    expect(mapper({ ...all, paused: false }).mood).toBe('listening');
  });

  it('[T11] [UX-01a] error holds only while errorUntil is in the future', () => {
    const at = (t: number) => createMoodMapper(() => t);
    const s = { ...base, errorUntil: NOW + 3000, processing: true };
    expect(at(NOW)(s).mood).toBe('error');
    expect(at(NOW + 2999)(s).mood).toBe('error');
    expect(at(NOW + 3000)(s).mood).toBe('processing');
    expect(at(NOW + 5000)(s).mood).toBe('processing');
  });

  it('[T11] [UX-01a] success holds only while successUntil is in the future', () => {
    const at = (t: number) => createMoodMapper(() => t);
    const s = { ...base, successUntil: NOW + 2500 };
    expect(at(NOW + 2499)(s).mood).toBe('success');
    expect(at(NOW + 2500)(s).mood).toBe('idle');
  });

  it('[T11] [UX-01a] an expired error falls through to success', () => {
    const s = { ...base, errorUntil: NOW - 1, successUntil: NOW + 1 };
    expect(mapper(s).mood).toBe('success');
  });

  it('[T11] reads the clock on every call', () => {
    let t = NOW;
    const m = createMoodMapper(() => t);
    const s = { ...base, errorUntil: NOW + 10 };
    expect(m(s).mood).toBe('error');
    t = NOW + 10;
    expect(m(s).mood).toBe('idle');
  });

  it('[T11] is deterministic and returns the minimal overlays', () => {
    const s = { ...base, processing: true };
    expect(mapper(s)).toEqual(mapper(s));
    expect(mapper(s).overlays).toEqual({
      attention: false,
      recording: false,
      offline: false,
      localOnly: false,
    });
  });

  it('[T11] defaults to the system clock', () => {
    const m = createMoodMapper();
    expect(m({ ...base, errorUntil: Date.now() + 60_000 }).mood).toBe('error');
    expect(m({ ...base, errorUntil: Date.now() - 1 }).mood).toBe('idle');
  });
});

// ---------- #9: the 4.8 table over typed MOG, capture and request states ----------

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';
const T = 2_000_000;

/** A ready MOG with the mic closed and no request. */
const quiet: MoodSources = {
  mog: { lifecycle: 'ready', since: 0 },
  capture: { recording: false, paused: false, othersTrack: 'off' },
  requests: [],
};
const ptt: CaptureState = {
  owner: A,
  mode: 'ptt',
  recording: true,
  paused: false,
  othersTrack: 'off',
};
const req = (phase: RequestPhase, extra: Partial<RequestView> = {}): RequestView => ({
  phase,
  at: T,
  ...extra,
});
const withReq = (r: RequestView): MoodSources => ({ ...quiet, requests: [r] });
const off: ReturnType<MoodMapper>['overlays'] = {
  attention: false,
  recording: false,
  offline: false,
  localOnly: false,
};

/** Every row of the 4.8 table: pipeline or capture state -> mood and overlay. */
const ROWS: { row: string; src: MoodSources; mood: Mood; overlays?: Partial<typeof off> }[] = [
  { row: 'Ready, mic closed', src: quiet, mood: 'idle' },
  { row: 'PTT down / held', src: { ...quiet, capture: ptt }, mood: 'listening' },
  {
    row: 'hands-free',
    src: { ...quiet, capture: { ...ptt, mode: 'hands_free' } },
    mood: 'listening',
  },
  ...(
    [
      'transcribing',
      'cleaning',
      'routing',
      'preview',
      'agent_turn',
      'acknowledged',
      'working',
      'rate_limited',
      'soft_notice',
    ] as const
  ).map((p) => ({ row: `processing: ${p}`, src: withReq(req(p)), mood: 'processing' as Mood })),
  {
    row: 'Ask uncertain (C19)',
    src: withReq(req('ask_uncertain')),
    mood: 'idle',
    overlays: { attention: true },
  },
  { row: 'Inserted (UC1)', src: withReq(req('inserted')), mood: 'success' },
  { row: 'Partial', src: withReq(req('partial')), mood: 'answering' },
  { row: 'blurb reveal', src: withReq(req('blurb_reveal')), mood: 'answering' },
  { row: 'Completed', src: withReq(req('completed')), mood: 'success' },
  { row: 'memory answer', src: withReq(req('memory_answer')), mood: 'success' },
  { row: 'Failed', src: withReq(req('failed')), mood: 'error' },
  {
    row: 'Failed, user must act',
    src: withReq(req('failed', { userMustAct: true })),
    mood: 'error',
    overlays: { attention: true },
  },
  {
    row: 'login required',
    src: { ...quiet, mog: { lifecycle: 'login_required', since: T } },
    mood: 'error',
    overlays: { attention: true },
  },
  {
    row: 'Needs input',
    src: withReq(req('needs_input')),
    mood: 'idle',
    overlays: { attention: true },
  },
  {
    row: 'chip offered (conf < 0.7)',
    src: withReq(req('chip_offered', { confidence: 0.69 })),
    mood: 'idle',
    overlays: { attention: true },
  },
  {
    row: 'chip offered (conf >= 0.7)',
    src: withReq(req('chip_offered', { confidence: 0.7 })),
    mood: 'idle',
  },
  {
    row: 'Session recording',
    src: { ...quiet, capture: { ...ptt, mode: 'session', sessionId: 's1' } },
    mood: 'idle',
    overlays: { recording: true },
  },
  {
    row: 'Session recording, per request',
    src: {
      ...withReq(req('working')),
      capture: { ...ptt, mode: 'session', sessionId: 's1' },
    },
    mood: 'processing',
    overlays: { recording: true },
  },
  {
    row: 'Session paused',
    src: { ...quiet, capture: { ...ptt, mode: 'session', sessionId: 's1', paused: true } },
    mood: 'idle',
  },
  {
    row: 'MOG paused',
    src: { ...quiet, mog: { lifecycle: 'paused', since: 0 } },
    mood: 'sleeping',
  },
  {
    row: 'MOG dozing',
    src: { ...quiet, mog: { lifecycle: 'dozing', since: 0 } },
    mood: 'sleeping',
  },
  {
    row: 'Offline badge, per other rows',
    src: { ...withReq(req('working')), mog: { lifecycle: 'offline', since: 0 } },
    mood: 'processing',
    overlays: { offline: true },
  },
  {
    row: 'hop badge, per other rows',
    src: { ...withReq(req('partial')), mog: { lifecycle: 'ready', since: 0, hop: 'west' } },
    mood: 'answering',
    overlays: { hop: 'west' },
  },
  {
    row: 'local-only badge, per other rows',
    src: { ...quiet, mog: { lifecycle: 'ready', since: 0, localOnly: true } },
    mood: 'idle',
    overlays: { localOnly: true },
  },
];

describe('T11 mapMood (#9, 4.8 table)', () => {
  it.each(ROWS)('[UX-01a] 4.8 row "$row" -> $mood', ({ src, mood, overlays }) => {
    expect(mapMood(A, src, T)).toEqual({ mood, overlays: { ...off, ...overlays } });
  });

  /** Region C (4.8): the mood of each request phase alone. `Record` makes this exhaustive. */
  const PHASE_MOOD: Record<RequestPhase, Mood> = {
    utterance: 'processing',
    transcribing: 'processing',
    cleaning: 'processing',
    routing: 'processing',
    preview: 'processing',
    agent_turn: 'processing',
    acknowledged: 'processing',
    working: 'processing',
    rate_limited: 'processing',
    soft_notice: 'processing',
    partial: 'answering',
    blurb_reveal: 'answering',
    chip_offered: 'idle',
    needs_input: 'idle',
    ask_uncertain: 'idle',
    memory_answer: 'success',
    completed: 'success',
    inserted: 'success',
    failed: 'error',
    gave_up: 'error',
    cancelled: 'idle',
    late_finished: 'idle',
    queued_offline: 'idle',
  };

  it('[UX-01a] maps every request phase (exhaustive)', () => {
    expect([...REQUEST_PHASES].sort()).toEqual(Object.keys(PHASE_MOOD).sort());
    for (const p of REQUEST_PHASES) {
      expect(mapMood(A, withReq(req(p)), T).mood, p).toBe(PHASE_MOOD[p]);
    }
  });

  it('[UX-01a] is pure: same input and time give the same output, input unchanged', () => {
    const src = withReq(req('failed', { userMustAct: true }));
    const copy = structuredClone(src);
    expect(mapMood(A, src, T)).toEqual(mapMood(A, src, T));
    expect(src).toEqual(copy);
  });

  /** One source per priority level, high to low (paused > capture > error > ... > idle). */
  const LADDER: { name: string; mood: Mood; src: (s: MoodSources) => MoodSources }[] = [
    {
      name: 'paused',
      mood: 'sleeping',
      src: (s) => ({ ...s, mog: { ...s.mog, lifecycle: 'paused' } }),
    },
    { name: 'capture', mood: 'listening', src: (s) => ({ ...s, capture: ptt }) },
    {
      name: 'error',
      mood: 'error',
      src: (s) => ({ ...s, requests: [...s.requests, req('failed')] }),
    },
    {
      name: 'success',
      mood: 'success',
      src: (s) => ({ ...s, requests: [...s.requests, req('completed')] }),
    },
    {
      name: 'answering',
      mood: 'answering',
      src: (s) => ({ ...s, requests: [...s.requests, req('partial')] }),
    },
    {
      name: 'processing',
      mood: 'processing',
      src: (s) => ({ ...s, requests: [...s.requests, req('working')] }),
    },
    {
      name: 'dozing',
      mood: 'sleeping',
      src: (s) => ({ ...s, mog: { ...s.mog, lifecycle: 'dozing' } }),
    },
  ];
  const ALL_PAIRS = LADDER.flatMap((hi, i) => LADDER.slice(i + 1).map((lo) => ({ hi, lo })));

  it.each(ALL_PAIRS)('[UX-01a] $hi.name wins over $lo.name', ({ hi, lo }) => {
    // Apply the low level first so that a later lifecycle write (paused over dozing) is the high one.
    expect(mapMood(A, hi.src(lo.src(quiet)), T).mood).toBe(hi.mood);
  });

  it('[UX-01a] any level wins over idle', () => {
    for (const l of LADDER) expect(mapMood(A, l.src(quiet), T).mood).toBe(l.mood);
    expect(mapMood(A, quiet, T).mood).toBe('idle');
  });

  it('[UX-01a] error lasts 3 s, then the next level shows', () => {
    const src: MoodSources = { ...quiet, requests: [req('failed'), req('working')] };
    expect(mapMood(A, src, T + ERROR_MS - 1).mood).toBe('error');
    expect(mapMood(A, src, T + ERROR_MS).mood).toBe('processing');
  });

  it('[UX-01a] success lasts 2.5 s after Completed, 1.2 s after Inserted', () => {
    expect(SUCCESS_MS).toBe(2500);
    expect(INSERTED_MS).toBe(1200);
    const done = withReq(req('completed'));
    expect(mapMood(A, done, T + SUCCESS_MS - 1).mood).toBe('success');
    expect(mapMood(A, done, T + SUCCESS_MS).mood).toBe('idle');
    const ins = withReq(req('inserted'));
    expect(mapMood(A, ins, T + INSERTED_MS - 1).mood).toBe('success');
    expect(mapMood(A, ins, T + INSERTED_MS).mood).toBe('idle');
  });

  it('[UX-01a] login required shows error for 3 s, then attention stays', () => {
    const src: MoodSources = { ...quiet, mog: { lifecycle: 'login_required', since: T } };
    expect(ERROR_MS).toBe(3000);
    expect(mapMood(A, src, T + ERROR_MS).mood).toBe('idle');
    expect(mapMood(A, src, T + ERROR_MS).overlays.attention).toBe(true);
  });

  it('[UX-07] only the capture owner shows listening; the other MOG keeps its mood', () => {
    const shared: MoodSources = { ...withReq(req('working')), capture: ptt };
    expect(mapMood(A, shared, T).mood).toBe('listening');
    expect(mapMood(B, shared, T).mood).toBe('processing');
    expect(mapMood(B, { ...quiet, capture: ptt }, T).mood).toBe('idle');
  });

  it('[UX-07] a capture with no owner, a closed mic or a paused capture is not listening', () => {
    const noOwner: CaptureState = {
      mode: 'ptt',
      recording: true,
      paused: false,
      othersTrack: 'off',
    };
    expect(mapMood(A, { ...quiet, capture: noOwner }, T).mood).toBe('idle');
    expect(mapMood(A, { ...quiet, capture: { ...ptt, recording: false } }, T).mood).toBe('idle');
    expect(mapMood(A, { ...quiet, capture: { ...ptt, paused: true } }, T).mood).toBe('idle');
  });

  it('[UX-07] a meeting session is idle + recording, never listening, and only for its owner', () => {
    const session: CaptureState = { ...ptt, mode: 'session', sessionId: 's1' };
    expect(mapMood(A, { ...quiet, capture: session }, T).mood).toBe('idle');
    expect(mapMood(B, { ...quiet, capture: session }, T).overlays.recording).toBe(false);
  });

  it('[UX-41] agent set_mood applies when nothing above it has priority', () => {
    for (const p of ['partial', 'working'] as const) {
      expect(mapMood(A, { ...withReq(req(p)), agentMood: 'answering' }, T).mood).toBe('answering');
    }
    expect(mapMood(A, { ...quiet, agentMood: 'processing' }, T).mood).toBe('processing');
    const dozing: MoodSources = { ...quiet, mog: { lifecycle: 'dozing', since: 0 } };
    expect(mapMood(A, { ...dozing, agentMood: 'idle' }, T).mood).toBe('idle');
  });

  it.each([
    { name: 'capture', src: { ...quiet, capture: ptt }, mood: 'listening' as Mood },
    { name: 'error', src: withReq(req('failed')), mood: 'error' as Mood },
    { name: 'success', src: withReq(req('completed')), mood: 'success' as Mood },
    {
      name: 'paused',
      src: { ...quiet, mog: { lifecycle: 'paused' as const, since: 0 } },
      mood: 'sleeping' as Mood,
    },
  ])('[UX-41] [AT-42] a set_mood during $name has no effect', ({ src, mood }) => {
    for (const agentMood of MOODS) {
      expect(mapMood(A, { ...src, agentMood }, T).mood).toBe(mood);
    }
  });

  it('[UX-41] [UX-07] the agent can never set listening', () => {
    expect(mapMood(A, { ...quiet, agentMood: 'listening' }, T).mood).toBe('idle');
    expect(mapMood(A, { ...withReq(req('working')), agentMood: 'listening' }, T).mood).toBe(
      'processing',
    );
  });
});

describe('T11 holdMood (#9, UX-05a)', () => {
  it('[UX-05a] the hold is 400 ms', () => {
    expect(MOOD_HOLD_MS).toBe(400);
  });

  it('[UX-05a] the first mood shows at once', () => {
    expect(holdMood(undefined, 'idle', T)).toEqual({ shown: { mood: 'idle', since: T } });
  });

  it('[UX-05a] the same mood keeps its start time', () => {
    const prev = { mood: 'processing' as Mood, since: T };
    expect(holdMood(prev, 'processing', T + 10)).toEqual({ shown: prev });
  });

  const LATER: Mood[] = MOODS.filter((m) => m !== 'listening');
  it.each(LATER)('[UX-05a] a change to %s within 400 ms waits until the hold ends', (next) => {
    const prev = { mood: (next === 'idle' ? 'processing' : 'idle') as Mood, since: T };
    for (const dt of [0, 1, 200, MOOD_HOLD_MS - 1]) {
      expect(holdMood(prev, next, T + dt)).toEqual({ shown: prev, recheckAt: T + MOOD_HOLD_MS });
    }
    expect(holdMood(prev, next, T + MOOD_HOLD_MS)).toEqual({
      shown: { mood: next, since: T + MOOD_HOLD_MS },
    });
  });

  it('[UX-05a] no mood shows for less than 400 ms over a fast sequence', () => {
    // A request that flips mood every 50 ms: every shown mood still lasts 400 ms or more.
    const targets: Mood[] = ['processing', 'answering', 'processing', 'success', 'error', 'idle'];
    let held: HeldMood | undefined;
    const shown: HeldMood[] = [];
    for (let t = 0; t <= 3000; t += 50) {
      const target = targets[Math.min(targets.length - 1, Math.floor(t / 50))] ?? 'idle';
      held = holdMood(held, target, T + t).shown;
      if (shown.at(-1)?.mood !== held.mood) shown.push(held);
    }
    for (let i = 1; i < shown.length; i++) {
      const a = shown[i - 1];
      const b = shown[i];
      expect(b && a && b.since - a.since).toBeGreaterThanOrEqual(MOOD_HOLD_MS);
    }
    expect(shown.at(-1)?.mood).toBe('idle');
  });

  it('[UX-05a] [UX-05b] listening shows at once, so key-down is not delayed by the hold', () => {
    const prev = { mood: 'processing' as Mood, since: T };
    expect(holdMood(prev, 'listening', T + 50)).toEqual({
      shown: { mood: 'listening', since: T + 50 },
    });
  });

  it('[UX-05a] listening is held for 400 ms like any mood', () => {
    const prev = { mood: 'listening' as Mood, since: T };
    expect(holdMood(prev, 'processing', T + 100)).toEqual({
      shown: prev,
      recheckAt: T + MOOD_HOLD_MS,
    });
  });
});
