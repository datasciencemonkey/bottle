/** ADR 0105: the brain opens the local store once, after the hub explains the macOS prompt. */
import { STORE_TEXT } from '../shared/store-view';
import { createStoreGate, type StoreView } from './store-gate';

const flush = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};
const FAIL = { error: 'consent_storage_failed' };

function armed(explain = true) {
  const views: StoreView[] = [];
  const gate = createStoreGate();
  gate.arm({ explain, onChange: (v) => views.push(v) });
  return { gate, views };
}

describe('store gate (ADR 0105)', () => {
  it('unarmed it passes every call through', async () => {
    const gate = createStoreGate();
    await expect(gate.run(async () => 7)).resolves.toBe(7);
    await expect(gate.run(async () => Promise.reject(FAIL))).rejects.toEqual(FAIL);
    await expect(gate.run(async () => 8)).resolves.toBe(8);
  });

  it('[MEM-08b] on macOS no store call runs before the hub shows the reason', async () => {
    const { gate, views } = armed();
    expect(views.at(-1)).toEqual({ state: 'explain' });
    const fn = vi.fn(async () => 'ok');
    const p = gate.run(fn);
    await flush();
    expect(fn).not.toHaveBeenCalled();
    gate.allow();
    await expect(p).resolves.toBe('ok');
    expect(views.at(-1)).toEqual({ state: 'hidden' });
  });

  it('[MEM-08b] the first call opens the store alone; the others wait for it (one prompt)', async () => {
    const { gate } = armed(false);
    let release: (v: string) => void = () => undefined;
    const first = vi.fn(() => new Promise<string>((r) => (release = r)));
    const second = vi.fn(async () => 'b');
    const a = gate.run(first);
    const b = gate.run(second);
    await flush();
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).not.toHaveBeenCalled();
    release('a');
    await expect(a).resolves.toBe('a');
    await expect(b).resolves.toBe('b');
  });

  it('[MEM-08b] a refused key turns history off; later calls fail at once, with no new prompt', async () => {
    const { gate, views } = armed(false);
    let reject: (e: unknown) => void = () => undefined;
    const first = vi.fn(() => new Promise<string>((_, r) => (reject = r)));
    const queued = vi.fn(async () => 'never');
    const a = gate.run(first);
    const b = gate.run(queued);
    await flush();
    reject({ error: 'host_failed' });
    await expect(a).rejects.toEqual({ error: 'host_failed' });
    await expect(b).rejects.toEqual({ error: 'host_failed' });
    const later = vi.fn(async () => 'never');
    await expect(gate.run(later)).rejects.toEqual({ error: 'host_failed' });
    expect(queued).not.toHaveBeenCalled();
    expect(later).not.toHaveBeenCalled();
    expect(views.at(-1)).toEqual({ state: 'off' });
  });

  it('[MEM-08b] Try again opens the store once more', async () => {
    const { gate, views } = armed(false);
    await expect(gate.run(async () => Promise.reject(FAIL))).rejects.toEqual(FAIL);
    gate.retry();
    expect(views.at(-1)).toEqual({ state: 'hidden' });
    await expect(gate.run(async () => 'ok')).resolves.toBe('ok');
  });

  it('an error that is not a store failure does not turn history off', async () => {
    const { gate, views } = armed(false);
    await expect(gate.run(async () => Promise.reject({ error: 'invalid_args' }))).rejects.toEqual({
      error: 'invalid_args',
    });
    await expect(gate.run(async () => 'ok')).resolves.toBe('ok');
    expect(views.map((v) => v.state)).not.toContain('off');
  });

  it('the texts', () => {
    expect(STORE_TEXT.explain).toBe(
      'Bottle keeps your history encrypted on this Mac. macOS will ask for your password once so Bottle can unlock its key. Choose Always Allow.',
    );
    expect(STORE_TEXT.off).toBe('History is off: Bottle could not unlock its key.');
    expect(STORE_TEXT.retry).toBe('Try again');
  });
});
