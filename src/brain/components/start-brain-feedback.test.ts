/**
 * #551: the started brain sends the MOG mood on `capture:state`, shows a refused start on the
 * MOG, and turns text insertion on (UX-01a, UX-05b, MB-08a, MOG-06a, MOG-06d).
 */
import { EVENTS } from '../../shared/ipc';
import { createCaptureLifecycle } from '../capture-lifecycle';
import { REFUSAL_TEXT } from '../capture-mood';
import { routeUtterance } from '../utterance-route';
import { createUtteranceHandler } from '../utterance-wiring';
import { core } from '../test-core';
import { startBrain } from './index';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);
vi.mock('../capture-lifecycle', async (orig) => {
  const m = await orig<typeof import('../capture-lifecycle')>();
  return { ...m, createCaptureLifecycle: vi.fn(m.createCaptureLifecycle) };
});
vi.mock('../utterance-wiring', async (orig) => {
  const m = await orig<typeof import('../utterance-wiring')>();
  return { ...m, createUtteranceHandler: vi.fn(m.createUtteranceHandler) };
});

const A = '00000000-0000-4000-8000-0000000000a1';
const VIEW_STATE = EVENTS['mog.view_state'].name;

const viewStates = (label: string) =>
  core.emitTo.mock.calls
    .filter(([to, ev]) => to === label && ev === VIEW_STATE)
    .map(([, , p]) => p as { mogId: string; mood: string; badges?: { text: string }[] });

describe('startBrain: visible PTT feedback (#551)', () => {
  it('[UX-01a] [UX-05b] a capture:state for A sends listening to mog-A and to the hub', () => {
    const brain = startBrain();
    brain.bus.dispatch({
      event: EVENTS['capture.state'].name,
      payload: { owner: A, mode: 'ptt', recording: true, paused: false, othersTrack: 'off' },
    });
    expect(viewStates(`mog-${A}`).at(-1)).toMatchObject({ mogId: A, mood: 'listening' });
    expect(viewStates('hub').at(-1)).toMatchObject({ mogId: A, mood: 'listening' });
  });

  it('[MB-08a] a refused capture start shows its reason on the MOG', () => {
    startBrain();
    const deps = vi.mocked(createCaptureLifecycle).mock.calls[0]?.[0];
    expect(deps?.onRefused).toBeTypeOf('function');
    const B = '00000000-0000-4000-8000-0000000000b2';
    deps?.onRefused?.({ mogId: B, mode: 'ptt', error: { error: 'mic_busy' } });
    expect(viewStates(`mog-${B}`).at(-1)).toMatchObject({
      mood: 'error',
      badges: [{ text: REFUSAL_TEXT.mic_busy.text }],
    });
  });

  it('[MOG-06a] [MOG-06d] insertion is on, and a no_permission or failed insert_text keeps the text for Copy', async () => {
    startBrain();
    const deps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
    expect(deps?.insertionOn(A)).toBe(true);
    const u = (insertionOn: boolean) => ({
      mogId: A,
      text: 'raw',
      forceAsk: false,
      insertionOn,
      input: {} as never,
    });
    const decision = { intent: 'dictation', cleaned_text: 'clean text' } as never;
    const route = async () => decision;
    for (const [insertText, reason] of [
      [async () => ({ result: 'no_permission' as const }), 'no_permission'],
      [() => Promise.reject(new Error('down')), 'host_failed'],
    ] as const) {
      const r = await routeUtterance(u(deps?.insertionOn(A) ?? false), { route, insertText });
      expect(r).toEqual({ route: 'insert_failed', reason, text: 'clean text' });
    }
  });
});
