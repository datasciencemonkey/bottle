import type { Mood } from '../../shared/interfaces';
import { createMoodMapper } from './mood-mapper';

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

  const pairs = LEVELS.slice(0, -1).map((hi, i) => ({ hi, lo: LEVELS[i + 1]! }));
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
