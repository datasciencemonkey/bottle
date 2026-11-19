/** ADR 0105: the hub side of the store gate in the brain. */
import {
  STORE_CHECK_TARGET,
  STORE_READY_TARGET,
  STORE_RETRY_TARGET,
  STORE_VIEW_ID,
} from '../shared/store-view';
import { BrainBus } from './bus';
import { createStoreGate } from './store-gate';
import { isTauriMac, wireStoreGate } from './store-wiring';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const flush = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};
const hub = (bus: BrainBus, target: string, from = 'hub') =>
  bus.dispatch({
    event: 'view:input',
    payload: { from, input: { mogId: STORE_VIEW_ID, kind: 'click', payload: { target } } },
  });

function setup(explain = true) {
  const bus = new BrainBus();
  const gate = createStoreGate();
  const sent: unknown[] = [];
  const onRetry = vi.fn();
  wireStoreGate(bus, gate, { explain, send: async (m) => void sent.push(m), onRetry });
  return { bus, gate, sent, onRetry };
}

describe('store wiring (ADR 0105)', () => {
  it('[MEM-08b] the hub open gets the reason; only its ready lets the store open', async () => {
    const t = setup();
    const fn = vi.fn(async () => 1);
    const p = t.gate.run(fn);
    hub(t.bus, STORE_CHECK_TARGET);
    await flush();
    expect(t.sent.at(-1)).toEqual({ state: 'explain' });
    expect(fn).not.toHaveBeenCalled();
    // Another window cannot say that the hub shows the reason.
    hub(t.bus, STORE_READY_TARGET, 'mog-0b0e8a52-6f7e-4a57-9a43-2f3b6f8f4c11');
    await flush();
    expect(fn).not.toHaveBeenCalled();
    hub(t.bus, STORE_READY_TARGET);
    await expect(p).resolves.toBe(1);
    expect(t.sent.at(-1)).toEqual({ state: 'hidden' });
  });

  it('[MEM-08b] with no hub, a talk-key press lets the store open', async () => {
    const t = setup();
    const p = t.gate.run(async () => 2);
    t.bus.dispatch({
      event: 'hotkey:ptt',
      payload: { mogId: '0b0e8a52-6f7e-4a57-9a43-2f3b6f8f4c11', phase: 'down' },
    });
    await expect(p).resolves.toBe(2);
  });

  it('[MEM-08b] Try again opens the store again and asks the brain to read once more', async () => {
    const t = setup(false);
    await expect(t.gate.run(async () => Promise.reject({ error: 'host_failed' }))).rejects.toEqual({
      error: 'host_failed',
    });
    await flush();
    expect(t.sent.at(-1)).toEqual({ state: 'off' });
    hub(t.bus, STORE_RETRY_TARGET);
    await flush();
    expect(t.onRetry).toHaveBeenCalledTimes(1);
    expect(t.sent.at(-1)).toEqual({ state: 'hidden' });
  });

  it('asks for the reason only in the macOS app (not in tests or other systems)', () => {
    expect(isTauriMac({ userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)' }, {})).toBe(
      false,
    );
    expect(
      isTauriMac(
        { userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)' },
        { __TAURI_INTERNALS__: {} },
      ),
    ).toBe(true);
    expect(
      isTauriMac({ userAgent: 'Mozilla/5.0 (Windows NT 10.0)' }, { __TAURI_INTERNALS__: {} }),
    ).toBe(false);
  });
});
