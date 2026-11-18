import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { NOTICE_VERSION } from '../../shared/hub-notice';
import { consentRecordsDouble } from '../../test-support/consent-records';
import { core } from '../test-core';
import { createConsentService, ipcConsentRecords } from './consent-service';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-core')).eventModule);

const T0 = Date.UTC(2026, 10, 17, 10, 0, 0);
const MONTH_MS = 30 * 86_400_000;

function service(records = consentRecordsDouble(() => T0), version = NOTICE_VERSION) {
  return { t: createConsentService({ records, version, legacyStorage: null }), records };
}

describe('T12 ConsentService: first-run notice (#27) on the Rust consent records (#431)', () => {
  beforeEach(() => core.reset());

  it('[MB-08a] reports no accepted notice before the user accepts one', async () => {
    const { t, records } = service();
    expect(await t.noticeAccepted()).toBeNull();
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: false });
    expect(records.writes).toEqual([]);
  });

  it('[SEC-32a] [AT-24] an acceptance writes one record in the Rust store, and it outlives the service', async () => {
    const { t, records } = service();
    expect(await t.acceptNotice(NOTICE_VERSION)).toBe(true);
    expect(records.writes).toEqual([NOTICE_VERSION]);
    expect(records.stored).toHaveLength(1);
    expect(await t.noticeAccepted()).toEqual({ version: NOTICE_VERSION });
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: true });
    // A second click (or a second hub) writes no second record for the same version.
    expect(await t.acceptNotice(NOTICE_VERSION)).toBe(true);
    expect(records.writes).toEqual([NOTICE_VERSION]);
    // A new brain reads the record from the store, not from its own memory.
    expect(await service(records).t.noticeState()).toEqual({
      version: NOTICE_VERSION,
      accepted: true,
    });
  });

  it('[SEC-32a] [AT-24] the brain sends only the notice version; Rust fills user, device and time', async () => {
    core.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'consent_records_list'
        ? { records: [] }
        : { recordId: '00000000-0000-4000-8000-000000000001' },
    );
    const t = createConsentService({ records: ipcConsentRecords(), legacyStorage: null });
    await t.acceptNotice(NOTICE_VERSION);
    const writes = core.invoke.mock.calls.filter(([cmd]) => cmd === 'consent_notice_record');
    expect(writes).toEqual([
      ['consent_notice_record', { args: { noticeVersion: NOTICE_VERSION } }],
    ]);
    expect(core.invoke).toHaveBeenCalledWith('consent_records_list', { args: {} });
  });

  it('[SEC-32a] the default service reaches the records through the #430 IPC commands', async () => {
    core.invoke.mockImplementation(async () => ({
      records: [
        {
          recordId: '00000000-0000-4000-8000-000000000001',
          kind: 'notice',
          noticeVersion: NOTICE_VERSION,
          sessionId: null,
          acceptedAt: T0,
        },
      ],
    }));
    const t = createConsentService();
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: true });
    expect(core.invoke).toHaveBeenCalledWith('consent_records_list', { args: {} });
  });

  it('[SEC-32a] records() returns the stored content-free records, oldest first', async () => {
    const { t, records } = service();
    expect(await t.records()).toEqual([]);
    await t.acceptNotice(NOTICE_VERSION);
    const got = await t.records();
    expect(got).toEqual(records.stored);
    expect(got).toEqual([
      {
        recordId: expect.any(String),
        kind: 'notice',
        noticeVersion: NOTICE_VERSION,
        sessionId: null,
        acceptedAt: T0,
      },
    ]);
    // No user, device or MOG field reaches the brain.
    for (const r of got)
      expect(Object.keys(r).sort()).toEqual([
        'acceptedAt',
        'kind',
        'noticeVersion',
        'recordId',
        'sessionId',
      ]);
  });

  it('[SEC-15b] a record the 13-month purge removed is gone, and the notice shows again', async () => {
    const { t, records } = service();
    await t.acceptNotice(NOTICE_VERSION);
    records.purge(T0 + 14 * MONTH_MS - 13 * MONTH_MS);
    expect(await t.records()).toEqual([]);
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: false });
  });

  it('[MB-08b] an accepted older version is not the current version', async () => {
    const records = consentRecordsDouble(() => T0);
    await service(records, 1).t.acceptNotice(1);
    const next = service(records, 2).t;
    expect(await next.noticeAccepted()).toEqual({ version: 1 });
    expect(await next.noticeState()).toEqual({ version: 2, accepted: false });
    expect(await next.acceptNotice(2)).toBe(true);
    expect(await next.noticeState()).toEqual({ version: 2, accepted: true });
    // Each version keeps its own record.
    expect((await next.records()).map((r) => r.noticeVersion)).toEqual([1, 2]);
  });

  it('[MB-08b] refuses to accept a version that is not the current one', async () => {
    const { t, records } = service(undefined, 2);
    expect(await t.acceptNotice(1)).toBe(false);
    expect(await t.acceptNotice(3)).toBe(false);
    expect(await t.acceptNotice(0)).toBe(false);
    expect(await t.noticeState()).toEqual({ version: 2, accepted: false });
    expect(records.writes).toEqual([]);
  });

  it('[SEC-03] [MB-08a] a store that cannot write leaves the notice not accepted', async () => {
    const { t, records } = service();
    records.fail({ write: true });
    expect(await t.acceptNotice(NOTICE_VERSION)).toBe(false);
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: false });
  });

  it('[SEC-03] [MB-08a] a store that cannot read fails closed', async () => {
    const { t, records } = service();
    await t.acceptNotice(NOTICE_VERSION);
    records.fail({ read: true });
    expect(await t.noticeState()).toEqual({ version: NOTICE_VERSION, accepted: false });
    expect(await t.noticeAccepted()).toBeNull();
    await expect(t.records()).rejects.toEqual({ error: 'consent_storage_failed' });
  });

  it('[SEC-03] [MB-08a] a reply that is not the #430 shape fails closed', async () => {
    for (const bad of [
      undefined,
      {},
      { records: 'x' },
      { records: [{ kind: 'notice', noticeVersion: '1' }] },
      {
        records: [
          { recordId: 'r', kind: 'session', noticeVersion: 1, sessionId: null, acceptedAt: 0 },
        ],
      },
    ]) {
      core.reset();
      core.invoke.mockImplementation(async () => bad);
      const t = createConsentService({ records: ipcConsentRecords(), legacyStorage: null });
      expect(await t.noticeState(), JSON.stringify(bad)).toEqual({
        version: NOTICE_VERSION,
        accepted: false,
      });
    }
  });

  it('[SEC-32a] no app source keeps a consent record in Web Storage', () => {
    const root = resolve(import.meta.dirname, '../..');
    const files = (d: string): string[] =>
      readdirSync(d).flatMap((f) => {
        const p = join(d, f);
        if (statSync(p).isDirectory()) return files(p);
        return /\.tsx?$/.test(f) && !/\.test\.tsx?$/.test(f) ? [p] : [];
      });
    for (const f of files(root)) {
      const src = readFileSync(f, 'utf8');
      if (!/consent/i.test(src)) continue;
      // Web Storage, IndexedDB and cookies are shared by every view of the app origin.
      expect(
        /(local|session)Storage\.(setItem|getItem)|indexedDB|document\.cookie/.test(src),
        f,
      ).toBe(false);
    }
    // T12 only removes the old #27 key.
    const t12 = readFileSync(join(root, 'brain/components/consent-service.ts'), 'utf8');
    expect(t12).toMatch(/removeItem\(LEGACY_CONSENT_STORAGE_KEY\)/);
    expect(t12).not.toMatch(/setItem|getItem/);
  });
});
