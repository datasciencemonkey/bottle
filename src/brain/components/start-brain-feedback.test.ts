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

// The test setup clears mocks before each test, so keep the deps of the one startBrain call.
let brain: ReturnType<typeof startBrain>;
let lifecycleDeps: Parameters<typeof createCaptureLifecycle>[0] | undefined;
let utteranceDeps: Parameters<typeof createUtteranceHandler>[0] | undefined;
beforeAll(() => {
  brain = startBrain();
  lifecycleDeps = vi.mocked(createCaptureLifecycle).mock.calls[0]?.[0];
  utteranceDeps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
});

describe('startBrain: visible PTT feedback (#551)', () => {
  it('[UX-01a] [UX-05b] a capture:state for A sends listening to mog-A and to the hub', async () => {
    brain.bus.dispatch({
      event: EVENTS['capture.state'].name,
      payload: { owner: A, mode: 'ptt', recording: true, paused: false, othersTrack: 'off' },
    });
    // UX-05b: the MOG window gets it in the same task; the hub copy follows.
    expect(viewStates(`mog-${A}`).at(-1)).toMatchObject({ mogId: A, mood: 'listening' });
    await Promise.resolve();
    expect(viewStates('hub').at(-1)).toMatchObject({ mogId: A, mood: 'listening' });
  });

  it('[MB-08a] a refused capture start shows its reason on the MOG', () => {
    const deps = lifecycleDeps;
    expect(deps?.onRefused).toBeTypeOf('function');
    const B = '00000000-0000-4000-8000-0000000000b2';
    deps?.onRefused?.({ mogId: B, mode: 'ptt', error: { error: 'mic_busy' } });
    expect(viewStates(`mog-${B}`).at(-1)).toMatchObject({
      mood: 'error',
      badges: [{ text: REFUSAL_TEXT.mic_busy.text }],
    });
  });

  it('[MOG-06a] [MOG-06d] insertion is on, and a no_permission or failed insert_text keeps the text for Copy', async () => {
    const deps = utteranceDeps;
    expect(deps?.insertionOn(A)).toBe(true);
    const u = (insertionOn: boolean) => ({
      mogId: A,
      text: 'raw',
      leaseId: 'lease-1',
      forceAsk: false,
      insertionOn,
      input: {} as never,
    });
    const decision = {
      intent: 'dictation',
      cleaned_text: 'clean text',
      genie_question: '',
    } as never;
    const route = async () => decision;
    for (const [insertText, reason] of [
      [async () => ({ result: 'no_permission' as const }), 'no_permission'],
      [() => Promise.reject(new Error('down')), 'host_failed'],
    ] as const) {
      const r = await routeUtterance(u(deps?.insertionOn(A) ?? false), { route, insertText });
      expect(r).toMatchObject({
        route: 'chip',
        dictated: { route: 'insert_failed', reason, text: 'clean text' },
      });
    }
  });
});
