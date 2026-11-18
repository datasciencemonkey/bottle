// @vitest-environment jsdom
/**
 * AT-24 "Notice and attestation" (#27 part), section 15: "First run. Change the notice version.
 * ... The notice blocks capture until accepted." This file runs the notice part end to end with
 * doubles: the hub view, the brain wiring, T12 ConsentService and the capture_start gate. The
 * per-session attestation part of AT-24 is #64; the device run is a person's step (AT-24 is a
 * device test).
 */
import { act } from 'react';
import { HUB_NOTICE_EVENT, NOTICE_VERSION } from '../shared/hub-notice';
import { BrainBus } from '../brain/bus';
import { startCapture } from '../brain/capture-gate';
import {
  createConsentService,
  LEGACY_CONSENT_STORAGE_KEY,
} from '../brain/components/consent-service';
import { consentRecordsDouble } from '../test-support/consent-records';
import { wireNotice } from '../brain/notice-wiring';
import { render, tauri, viewEmits } from '../views/test-tauri';
import { Hub } from '../views/hub/Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../views/test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../views/test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../views/test-tauri')).windowModule);

const MOG = '00000000-0000-4000-8000-0000000000c1';
const args = { mogId: MOG, mode: 'ptt' } as const;
const flush = () => act(async () => new Promise((r) => setTimeout(r, 0)));

/** One brain (bus + T12 + wiring) that talks to the rendered hub through the Tauri double. */
async function app(records = consentRecordsDouble(), version = NOTICE_VERSION) {
  const bus = new BrainBus();
  // The brain's own Web Storage is the one the hub, MOG and card views share (same origin).
  const consent = createConsentService({ records, version, legacyStorage: window.localStorage });
  const send = (s: unknown) => tauri.emitTo('hub', HUB_NOTICE_EVENT, s);
  const { el } = await render(<Hub />);
  wireNotice(bus, consent, send);
  await flush();
  /** Relay each hub view_emit to the brain as `view:input`, as the core does. */
  const relay = async () => {
    for (const input of viewEmits())
      bus.dispatch({ event: 'view:input', payload: { from: 'hub', input } });
    tauri.invoke.mockClear();
    await flush();
  };
  const capture = () =>
    startCapture({ consent, invoke: async () => ({ ok: true }) as never }, args);
  return { el, relay, capture };
}

async function accept(el: HTMLElement) {
  const box = el.querySelector<HTMLInputElement>('input[type="checkbox"]');
  const btn = [...el.querySelectorAll('button')].find((b) => b.textContent === 'Continue');
  if (!box || !btn) throw new Error('the notice is not shown');
  await act(async () => box.click());
  await act(async () => btn.click());
}

describe('AT-24 notice part (#27)', () => {
  beforeEach(() => {
    tauri.reset();
    document.body.innerHTML = '';
    window.localStorage.clear();
    window.sessionStorage.clear();
  });

  it('[AT-24] [MB-08a] first run: the notice blocks capture until the user accepts it', async () => {
    const { el, relay, capture } = await app();
    await relay();
    expect(el.querySelector('[role="dialog"]')).not.toBeNull();
    await expect(capture()).rejects.toEqual({
      error: 'consent_required',
    });
    await accept(el);
    await relay();
    expect(el.querySelector('[role="dialog"]')).toBeNull();
    await expect(capture()).resolves.toEqual({ ok: true });
  });

  it('[AT-24] [MB-08b] a notice version change shows the notice and blocks capture again', async () => {
    const store = consentRecordsDouble();
    const first = await app(store, 1);
    await first.relay();
    await accept(first.el);
    await first.relay();
    await expect(first.capture()).resolves.toEqual({ ok: true });

    tauri.reset();
    document.body.innerHTML = '';
    const next = await app(store, 2);
    await next.relay();
    expect(next.el.querySelector('[role="dialog"]')?.textContent).toContain('Notice version 2');
    await expect(next.capture()).rejects.toEqual({
      error: 'consent_required',
    });
    await accept(next.el);
    await next.relay();
    await expect(next.capture()).resolves.toEqual({ ok: true });
  });

  it('[AT-24] [SEC-32a] an acceptance writes one record in the Rust store and none in view-reachable storage', async () => {
    const records = consentRecordsDouble(() => 1_794_916_800_000);
    const { el, relay, capture } = await app(records);
    await relay();
    await accept(el);
    await relay();
    await expect(capture()).resolves.toEqual({ ok: true });
    // One record, written through consent_notice_record with the version only (#430).
    expect(records.writes).toEqual([NOTICE_VERSION]);
    expect(records.stored).toEqual([
      {
        recordId: expect.any(String),
        kind: 'notice',
        noticeVersion: NOTICE_VERSION,
        sessionId: null,
        acceptedAt: 1_794_916_800_000,
      },
    ]);
    // Nothing a hub, MOG or card view can read or write holds a consent record.
    expect(window.localStorage.length).toBe(0);
    expect(window.sessionStorage.length).toBe(0);
    expect(document.cookie).toBe('');
  });

  it('[AT-24] [SEC-32a] the old Web Storage record is removed and does not open the gate', async () => {
    // A #27 build kept the record in Web Storage, where a view could have written it.
    window.localStorage.setItem(
      LEGACY_CONSENT_STORAGE_KEY,
      JSON.stringify({
        device: 'd',
        notices: [{ kind: 'notice', version: NOTICE_VERSION, at: 'x', user: null, device: 'd' }],
      }),
    );
    const { el, relay, capture } = await app();
    expect(window.localStorage.getItem(LEGACY_CONSENT_STORAGE_KEY)).toBeNull();
    await relay();
    expect(el.querySelector('[role="dialog"]')).not.toBeNull();
    await expect(capture()).rejects.toEqual({ error: 'consent_required' });
  });
});
