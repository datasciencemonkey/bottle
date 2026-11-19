// @vitest-environment jsdom
/** ADR 0105 (MEM-08b): the hub says why macOS asks for the password, and when history is off. */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import { render as mount, tauri, viewEmits } from '../test-tauri';
import {
  STORE_CHECK_TARGET,
  STORE_EVENT,
  STORE_READY_TARGET,
  STORE_RETRY_TARGET,
  STORE_TEXT,
  STORE_VIEW_ID,
} from '../../shared/store-view';
import { Hub } from './Hub';
import { StoreNotice } from './StoreNotice';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

let roots: Root[] = [];
async function render(node: Parameters<typeof mount>[0]) {
  const r = await mount(node);
  roots.push(r.root);
  return r;
}
afterEach(async () => {
  for (const r of roots) await act(async () => r.unmount());
  roots = [];
});

const emit = (target: string) => ({ mogId: STORE_VIEW_ID, kind: 'click', payload: { target } });
const storeEmits = () =>
  viewEmits().filter((e) => (e as { mogId?: string }).mogId === STORE_VIEW_ID);

describe('Store notice (hub)', () => {
  beforeEach(() => tauri.reset());

  it('[MEM-08b] asks on open; shows the reason, then tells the brain it shows', async () => {
    const { el } = await render(<StoreNotice />);
    expect(storeEmits()).toEqual([emit(STORE_CHECK_TARGET)]);
    expect(el.textContent).toBe('');
    await tauri.emitTo('hub', STORE_EVENT, { state: 'explain' });
    expect(el.textContent).toContain(STORE_TEXT.explain);
    expect(el.querySelectorAll('button')).toHaveLength(0);
    expect(storeEmits()).toEqual([emit(STORE_CHECK_TARGET), emit(STORE_READY_TARGET)]);
    await tauri.emitTo('hub', STORE_EVENT, { state: 'hidden' });
    expect(el.textContent).toBe('');
  });

  it('[MEM-08b] a refused key shows "History is off" with Try again', async () => {
    const { el } = await render(<StoreNotice />);
    await tauri.emitTo('hub', STORE_EVENT, { state: 'off' });
    expect(el.textContent).toContain(STORE_TEXT.off);
    const buttons = [...el.querySelectorAll('button')];
    expect(buttons.map((b) => b.textContent)).toEqual(['Try again']);
    await act(async () => buttons[0]?.click());
    expect(storeEmits().at(-1)).toEqual(emit(STORE_RETRY_TARGET));
  });

  it('[MEM-08b] the hub shows it before and after the legal notice', async () => {
    const { el } = await render(<Hub />);
    await tauri.emitTo('hub', STORE_EVENT, { state: 'off' });
    expect(el.textContent).toContain(STORE_TEXT.off);
    await tauri.emitTo('hub', 'hub:notice', { version: 1, accepted: false });
    expect(el.textContent).toContain(STORE_TEXT.off);
  });
});
