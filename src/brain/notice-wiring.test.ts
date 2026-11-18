import { HUB_NOTICE_EVENT, HUB_SCOPE, NOTICE_VERSION } from '../shared/hub-notice';
import { BrainBus } from './bus';
import { core } from './test-core';
import { consentRecordsDouble } from '../test-support/consent-records';
import { createConsentService } from './components/consent-service';
import { sendHubNotice, wireNotice } from './notice-wiring';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG = '00000000-0000-4000-8000-000000000001';

function input(from: string, mogId: string, payload: unknown) {
  return { event: 'view:input', payload: { from, input: { mogId, kind: 'click', payload } } };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

async function setup() {
  const bus = new BrainBus();
  const consent = createConsentService({ records: consentRecordsDouble(), legacyStorage: null });
  const send = vi.fn(async () => undefined);
  wireNotice(bus, consent, send);
  await flush();
  return { bus, consent, send };
}

describe('brain wiring of the hub notice (#27)', () => {
  beforeEach(() => core.reset());

  it('[MB-08a] sends the notice state to the hub when wired and when the hub is ready', async () => {
    const { bus, send } = await setup();
    expect(send).toHaveBeenLastCalledWith({ version: NOTICE_VERSION, accepted: false });
    send.mockClear();
    bus.dispatch(input('hub', HUB_SCOPE, { action: 'notice.ready' }));
    await flush();
    expect(send).toHaveBeenCalledWith({ version: NOTICE_VERSION, accepted: false });
  });

  it('[MB-08a] accepts the notice only on a Continue click from the hub', async () => {
    const { bus, consent, send } = await setup();
    const accept = { action: 'notice.accept', noticeVersion: NOTICE_VERSION };
    // A MOG or card window cannot accept the notice for the user.
    bus.dispatch(input(`mog-${MOG}`, MOG, accept));
    bus.dispatch(input(`card-${MOG}`, MOG, accept));
    // A malformed payload does nothing.
    bus.dispatch(input('hub', HUB_SCOPE, { action: 'notice.accept', noticeVersion: '1' }));
    await flush();
    expect(await consent.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: false });
    bus.dispatch(input('hub', HUB_SCOPE, accept));
    await flush();
    expect(await consent.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: true });
    expect(send).toHaveBeenLastCalledWith({ version: NOTICE_VERSION, accepted: true });
  });

  it('[MB-08b] a Continue click for an old version does not accept the current one', async () => {
    const { bus, consent, send } = await setup();
    bus.dispatch(input('hub', HUB_SCOPE, { action: 'notice.accept', noticeVersion: 0 }));
    await flush();
    expect(await consent.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: false });
    expect(send).toHaveBeenLastCalledWith({ version: NOTICE_VERSION, accepted: false });
  });

  it('[MB-08a] [ARC-5] sends the notice only to the hub window', async () => {
    await sendHubNotice({ version: NOTICE_VERSION, accepted: false });
    expect(core.emitTo).toHaveBeenCalledWith('hub', HUB_NOTICE_EVENT, {
      version: NOTICE_VERSION,
      accepted: false,
    });
    expect(core.emit).not.toHaveBeenCalled();
  });
});
