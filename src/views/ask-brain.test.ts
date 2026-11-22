/** `askBrain`: a view ask refused while the brain has no Channel is tried again with a backoff. */
import { askBrain, ASK_BRAIN_BACKOFF_MS } from './view-ipc';
import { tauri, viewEmits } from './test-tauri';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('./test-tauri')).windowModule);

describe('askBrain (launch race)', () => {
  beforeEach(() => {
    tauri.reset();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    tauri.invoke.mockImplementation(async () => ({}));
  });

  it('does not repeat an ask the brain took', async () => {
    askBrain('hub', 'click', { action: 'mogs.ready' });
    await vi.advanceTimersByTimeAsync(20_000);
    expect(viewEmits()).toHaveLength(1);
  });

  it('repeats a refused ask at 0.5, 1, 2 and 4 s after the last, then stops', async () => {
    tauri.invoke.mockImplementation(async () => {
      throw { error: 'host_failed' };
    });
    askBrain('hub', 'click', { target: 'store.check' });
    expect(viewEmits()).toHaveLength(1);
    let n = 1;
    for (const wait of ASK_BRAIN_BACKOFF_MS) {
      await vi.advanceTimersByTimeAsync(wait - 1);
      expect(viewEmits()).toHaveLength(n);
      await vi.advanceTimersByTimeAsync(1);
      expect(viewEmits()).toHaveLength(++n);
    }
    await vi.advanceTimersByTimeAsync(60_000);
    expect(viewEmits()).toHaveLength(n);
  });

  it('stops at the first try the brain takes', async () => {
    let calls = 0;
    tauri.invoke.mockImplementation(async () => {
      if (++calls < 3) throw { error: 'host_failed' };
      return {};
    });
    askBrain('hub', 'ready');
    await vi.advanceTimersByTimeAsync(60_000);
    expect(viewEmits()).toHaveLength(3);
  });
});
