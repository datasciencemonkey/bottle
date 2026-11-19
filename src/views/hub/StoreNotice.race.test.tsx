// @vitest-environment jsdom
/**
 * ADR 0105 review: Tauri registers a listener only after an IPC round trip. If the hub asked
 * before its listener existed, a fast brain reply was lost, the hub never said `store.ready`,
 * and every store call waited. The hub must ask only once it listens.
 */
import { act } from 'react';
import { render, tauri, viewEmits } from '../test-tauri';
import {
  STORE_CHECK_TARGET,
  STORE_EVENT,
  STORE_READY_TARGET,
  STORE_TEXT,
  STORE_VIEW_ID,
} from '../../shared/store-view';
import { StoreNotice } from './StoreNotice';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);
// Like Tauri: the listener exists only after a round trip.
vi.mock('@tauri-apps/api/event', async () => {
  const m = (await import('../test-tauri')).eventModule;
  return {
    listen: async (...a: Parameters<typeof m.listen>) => {
      await new Promise((r) => setTimeout(r, 0));
      return m.listen(...a);
    },
  };
});

const flush = async () => {
  for (let i = 0; i < 5; i++) await act(async () => new Promise((r) => setTimeout(r, 0)));
};

describe('Store notice start race (ADR 0105)', () => {
  beforeEach(() => tauri.reset());

  it('[MEM-08b] a reply that comes at once is not lost, so store calls do not wait forever', async () => {
    // The brain answers the check at once (it is already armed).
    tauri.invoke.mockImplementation(async (cmd: string, a?: unknown) => {
      const args = (a as { args?: { payload?: { target?: string } } } | undefined)?.args;
      if (cmd === 'view_emit' && args?.payload?.target === STORE_CHECK_TARGET)
        void tauri.emitTo('hub', STORE_EVENT, { state: 'explain' });
      return {};
    });
    const { el, root } = await render(<StoreNotice />);
    await flush();
    expect(el.textContent).toContain(STORE_TEXT.explain);
    const store = viewEmits().filter((e) => (e as { mogId?: string }).mogId === STORE_VIEW_ID);
    expect(store.map((e) => (e as { payload: { target: string } }).payload.target)).toEqual([
      STORE_CHECK_TARGET,
      STORE_READY_TARGET,
    ]);
    await act(async () => root.unmount());
  });
});
