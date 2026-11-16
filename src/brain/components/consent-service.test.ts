import { NOTICE_VERSION } from '../../shared/hub-notice';
import { createConsentService, memoryConsentStore, type ConsentStore } from './consent-service';

const AT = new Date('2026-11-16T10:00:00.000Z');

function service(store: ConsentStore = memoryConsentStore(), version = NOTICE_VERSION) {
  return createConsentService({
    store,
    version,
    now: () => AT,
    newId: () => 'device-1',
    user: () => null,
  });
}

describe('T12 ConsentService: first-run notice (#27)', () => {
  it('[MB-08a] reports no accepted notice before the user accepts one', async () => {
    const t = service();
    expect(await t.noticeAccepted()).toBeNull();
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: false });
  });

  it('[MB-08a] [SEC-32a] stores the record with version, time, user and device on accept', async () => {
    const store = memoryConsentStore();
    const t = service(store);
    expect(await t.acceptNotice(NOTICE_VERSION)).toBe(true);
    expect(await t.noticeAccepted()).toEqual({ version: NOTICE_VERSION });
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: true });
    expect(await t.noticeRecords()).toEqual([
      {
        kind: 'notice',
        version: NOTICE_VERSION,
        at: AT.toISOString(),
        user: null,
        device: 'device-1',
      },
    ]);
    // The record outlives the service object: a new brain reads it from the store.
    expect(await service(store).noticeState()).toEqual({ version: NOTICE_VERSION, accepted: true });
  });

  it('[MB-08b] an accepted older version is not the current version', async () => {
    const store = memoryConsentStore();
    await service(store, 1).acceptNotice(1);
    const next = service(store, 2);
    expect(await next.noticeAccepted()).toEqual({ version: 1 });
    expect(await next.noticeState()).toEqual({ version: 2, accepted: false });
    expect(await next.acceptNotice(2)).toBe(true);
    expect(await next.noticeState()).toEqual({ version: 2, accepted: true });
    // Each version keeps its own record, and the device ID stays the same.
    expect((await next.noticeRecords()).map((r) => [r.version, r.device])).toEqual([
      [1, 'device-1'],
      [2, 'device-1'],
    ]);
  });

  it('[MB-08b] refuses to accept a version that is not the current one', async () => {
    const t = service(memoryConsentStore(), 2);
    expect(await t.acceptNotice(1)).toBe(false);
    expect(await t.acceptNotice(3)).toBe(false);
    expect(await t.noticeState()).toEqual({ version: 2, accepted: false });
    expect(await t.noticeRecords()).toEqual([]);
  });

  it('[MB-08a] fails closed on a corrupt or foreign store value', async () => {
    for (const bad of [
      '{',
      'null',
      '[]',
      '{"device":"d","notices":[{"kind":"notice","version":"1"}]}',
      `{"device":"d","notices":[{"kind":"session","version":${NOTICE_VERSION}}]}`,
    ]) {
      const store = memoryConsentStore(bad);
      expect(await service(store).noticeState(), bad).toEqual({
        version: NOTICE_VERSION,
        accepted: false,
      });
    }
  });

  it('[MB-08a] a store that cannot write leaves the notice not accepted', async () => {
    const store: ConsentStore = {
      read: () => null,
      write: () => {
        throw new Error('quota');
      },
    };
    const t = service(store);
    expect(await t.acceptNotice(NOTICE_VERSION)).toBe(false);
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: false });
  });
});
