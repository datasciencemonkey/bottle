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
import { createConsentService, memoryConsentStore } from '../brain/components/consent-service';
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
async function app(store = memoryConsentStore(), version = NOTICE_VERSION) {
  const bus = new BrainBus();
  const consent = createConsentService({
    store,
    version,
    now: () => new Date(0),
    newId: () => 'd',
  });
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
    const store = memoryConsentStore();
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
});
