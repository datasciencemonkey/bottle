// @vitest-environment jsdom
import { act } from 'react';
import { render, tauri, viewEmits } from '../test-tauri';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';
const overlays = { attention: false, recording: false, offline: false, localOnly: false };

describe('V1 hub view', () => {
  beforeEach(() => tauri.reset());

  it('[V1] shows the product name and the Labs label', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('h1')?.textContent).toBe('Bottle');
    expect(el.textContent).toContain('Databricks Labs / experimental');
  });

  it('[V1] subscribes to mog:view_state and lists each MOG with its mood', async () => {
    const { el } = await render(<Hub />);
    expect(tauri.listeners('mog:view_state')).toBe(1);
    await tauri.emit('mog:view_state', { mogId: A, mood: 'idle', overlays });
    await tauri.emit('mog:view_state', { mogId: B, mood: 'processing', overlays });
    await tauri.emit('mog:view_state', { mogId: A, mood: 'sleeping', overlays });
    await tauri.emit('mog:view_state', { mogId: 7 });
    const rows = [...el.querySelectorAll('li[data-mog]')].map((li) => [
      li.getAttribute('data-mog'),
      li.getAttribute('data-mood'),
    ]);
    expect(rows).toEqual([
      [A, 'sleeping'],
      [B, 'processing'],
    ]);
  });

  it('[V1] [ARC-2] sends a MOG selection only through view_emit', async () => {
    const { el } = await render(<Hub />);
    await tauri.emit('mog:view_state', { mogId: A, mood: 'idle', overlays });
    await act(async () => (el.querySelector(`li[data-mog="${A}"] button`) as HTMLElement).click());
    expect(viewEmits()).toEqual([{ mogId: A, kind: 'click' }]);
  });
});
