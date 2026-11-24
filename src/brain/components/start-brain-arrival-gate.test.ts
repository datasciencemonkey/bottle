/**
 * startBrain, UX slice 1: an Ask Genie arrival does not open the card while the talk key is held.
 * It applies 1.2 s after the paste of the hold (or after the key-up of an ⌥G ask). The failing
 * `mcp_call` of the R5 stub ends the request at once, so the failure is the arrival here.
 */
import { core } from '../test-core';
import { NOTICE_VERSION } from '../../shared/hub-notice';
import { routerDecision } from '../../test-support/model-invoke';
import { createUtteranceHandler, type UtteranceHandlerDeps } from '../utterance-wiring';

import { startBrain, type Brain } from './index';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);
vi.mock('../hub-mogs-wiring', () => ({ wireHubMogs: vi.fn() }));
vi.mock('../notice-wiring', () => ({ wireNotice: vi.fn() }));
vi.mock('../level-stream', async (orig) => ({
  ...(await orig<typeof import('../level-stream')>()),
  wireLevelStream: vi.fn(() => () => undefined),
  subscribeLevels: vi.fn(async () => true),
}));
vi.mock('../utterance-wiring', async (orig) => {
  const real = await orig<typeof import('../utterance-wiring')>();
  return { ...real, createUtteranceHandler: vi.fn(real.createUtteranceHandler) };
});

const MOG = '00000000-0000-4000-8000-0000000005a3';
import { workspaceList } from '../../test-support/workspace-list';

const HOME = 'fake-home';

let deps: UtteranceHandlerDeps | undefined;
let brain: Brain | undefined;

type CardPayload = { blurbs: { text: string }[]; chips: { id: string; label: string }[] };
const cards = () =>
  core.emitTo.mock.calls
    .filter(([target, event]) => target === `card-${MOG}` && event === 'card:update')
    .map(([, , payload]) => payload as CardPayload);
const settle = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};
const commands = () => core.invoke.mock.calls.map(([c]) => c as string);
const genieAsks = () =>
  core.invoke.mock.calls.filter(
    ([c, a]) => c === 'mcp_call' && (a as { args: { tool: string } }).args.tool === 'genie_ask',
  );

/** The host as on main: a home profile, and an `mcp_call` that R5 cannot run yet (#16). */
function hostWithStubR5() {
  core.invoke.mockImplementation(async (cmd) => {
    if (cmd === 'config_read') return { json: { name: 'Finance', binding: { home: HOME } } };
    if (cmd === 'workspace_list') return workspaceList(HOME);
    if (cmd === 'mcp_call') return Promise.reject({ error: 'not_implemented' });
    return {};
  });
}

/** The utterance hook's ask: the router input first (it learns the home profile), then the chip. */
async function offer(question: string, utteranceId: string) {
  if (!deps) throw new Error('startBrain made no utterance handler');
  await deps.routerInput(MOG);
  deps.onAsk(
    {
      route: 'chip',
      decision: routerDecision({ genie_question: question, cleaned_text: question }),
      warning: undefined,
    },
    { utteranceId, mogId: MOG },
  );
}

function press(chipId: string) {
  brain?.bus.dispatch({
    event: 'view:input',
    payload: {
      from: `card-${MOG}`,
      input: { mogId: MOG, kind: 'chip_click', payload: { chipId } },
    },
  });
}

/** The host as above, with the first-run notice accepted: a talk-key start is not refused. */
function hostAcceptingNotice() {
  core.invoke.mockImplementation(async (cmd) => {
    if (cmd === 'config_read') return { json: { name: 'Finance', binding: { home: HOME } } };
    if (cmd === 'workspace_list') return workspaceList(HOME);
    if (cmd === 'mcp_call') return Promise.reject({ error: 'not_implemented' });
    if (cmd === 'consent_records_list')
      return {
        records: [
          {
            recordId: 'rec-1',
            kind: 'notice',
            noticeVersion: NOTICE_VERSION,
            sessionId: null,
            acceptedAt: 1,
          },
        ],
      };
    if (cmd === 'capture_start')
      return {
        owner: MOG,
        mode: 'ptt',
        recording: true,
        paused: false,
        othersTrack: 'off',
        leaseId: 'lease-gate',
      };
    return {};
  });
}

const SEG = { segmentId: 'seg-gate', leaseId: 'lease-gate' };
const ptt = (phase: string, extra: object = {}) =>
  brain?.bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: MOG, phase, ...extra } });
/** `window_card_show` calls after the offer chip: the arrivals only. */
let base = 0;
const shows = () => commands().filter((c) => c === 'window_card_show').length - base;
const failureOnCard = () =>
  cards()
    .at(-1)
    ?.blurbs.some((b) => /\nCode [A-Z_]+$/.test(b.text)) ?? false;

describe('startBrain arrival gate (UX slice 1)', () => {
  beforeAll(() => {
    brain = startBrain();
    deps = vi.mocked(createUtteranceHandler).mock.calls[0]?.[0];
  });
  beforeEach(() => core.reset());
  afterEach(() => vi.useRealTimers());

  it('[AC-1.1] [AC-1.5] an arrival in a talk-key hold waits, then applies 1.2 s after the key-up of an ask', async () => {
    hostAcceptingNotice();
    brain?.talkTarget.viewReady(MOG);
    await offer('What was churn last quarter?', 'utt-gate-1');
    await settle();
    base = commands().filter((c) => c === 'window_card_show').length;
    vi.useFakeTimers();
    ptt('down');
    press('ask_genie:utt-gate-1');
    await vi.advanceTimersByTimeAsync(50);
    expect(genieAsks()).toHaveLength(1);
    // The failure arrived during the hold: no card show, no failure text.
    expect(shows()).toBe(0);
    expect(failureOnCard()).toBe(false);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(shows()).toBe(0);

    ptt('up', { ask: true });
    await vi.advanceTimersByTimeAsync(1_199);
    expect(shows()).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(shows()).toBe(1);
    expect(failureOnCard()).toBe(true);
  });

  it('[AC-1.4] a held arrival waits for the paste, then 1.2 s more', async () => {
    hostAcceptingNotice();
    brain?.talkTarget.viewReady(MOG);
    await offer('What was churn last quarter?', 'utt-gate-2');
    await settle();
    base = commands().filter((c) => c === 'window_card_show').length;
    vi.useFakeTimers();
    ptt('down');
    press('ask_genie:utt-gate-2');
    await vi.advanceTimersByTimeAsync(50);
    ptt('up');
    await vi.advanceTimersByTimeAsync(3_000);
    expect(shows()).toBe(0);
    // The route of the dictation ended with an insertion.
    deps?.onPasteResolved?.(true, SEG);
    await vi.advanceTimersByTimeAsync(1_199);
    expect(shows()).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(shows()).toBe(1);
  });

  it('[AC-1.6] with no paste the arrival applies 5 s after the key-up', async () => {
    hostAcceptingNotice();
    brain?.talkTarget.viewReady(MOG);
    await offer('What was churn last quarter?', 'utt-gate-3');
    await settle();
    base = commands().filter((c) => c === 'window_card_show').length;
    vi.useFakeTimers();
    ptt('down');
    press('ask_genie:utt-gate-3');
    await vi.advanceTimersByTimeAsync(50);
    ptt('up');
    await vi.advanceTimersByTimeAsync(4_999);
    expect(shows()).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(shows()).toBe(1);
  });

  it('[AC-1.8] hands-free listening holds the arrival too', async () => {
    hostWithStubR5();
    brain?.talkTarget.viewReady(MOG);
    await offer('What was churn last quarter?', 'utt-gate-4');
    await settle();
    base = commands().filter((c) => c === 'window_card_show').length;
    vi.useFakeTimers();
    brain?.bus.dispatch({
      event: 'capture:state',
      payload: {
        owner: MOG,
        mode: 'hands_free',
        recording: true,
        paused: false,
        othersTrack: 'off',
      },
    });
    press('ask_genie:utt-gate-4');
    await vi.advanceTimersByTimeAsync(10_000);
    expect(shows()).toBe(0);
    brain?.bus.dispatch({
      event: 'capture:state',
      payload: { mode: 'hands_free', recording: false, paused: false, othersTrack: 'off' },
    });
    deps?.onPasteResolved?.(false, SEG);
    await vi.advanceTimersByTimeAsync(1_200);
    expect(shows()).toBe(1);
  });

  it('[AC-1.5] a final segment with empty STT text applies 1.2 s after the key-up', async () => {
    hostAcceptingNotice();
    brain?.talkTarget.viewReady(MOG);
    await offer('What was churn last quarter?', 'utt-gate-5');
    await settle();
    base = commands().filter((c) => c === 'window_card_show').length;
    vi.useFakeTimers();
    ptt('down');
    press('ask_genie:utt-gate-5');
    await vi.advanceTimersByTimeAsync(50);
    ptt('up');
    // The segment is final, and the STT step gives no text (no utterance reaches the route).
    brain?.bus.dispatch({
      event: 'capture:segment',
      payload: { segmentId: 'seg-empty', mogId: MOG, leaseId: 'lease-gate', final: true },
    });
    await vi.advanceTimersByTimeAsync(1_199);
    expect(shows()).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(shows()).toBe(1);
  });

  it('[AC-1.5] a refused start applies 1.2 s after the key-up, not 5 s', async () => {
    hostWithStubR5();
    brain?.talkTarget.viewReady(MOG);
    await offer('What was churn last quarter?', 'utt-gate-6');
    await settle();
    base = commands().filter((c) => c === 'window_card_show').length;
    vi.useFakeTimers();
    ptt('down');
    press('ask_genie:utt-gate-6');
    await vi.advanceTimersByTimeAsync(50);
    // The first-run notice is not accepted here, so the consent gate refuses the start.
    // The refusal shows its own card; count only what comes after it.
    base = commands().filter((c) => c === 'window_card_show').length;
    ptt('up');
    await vi.advanceTimersByTimeAsync(1_199);
    expect(shows()).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(shows()).toBe(1);
  });
});
