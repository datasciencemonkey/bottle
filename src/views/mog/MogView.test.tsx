// @vitest-environment jsdom
import { act } from 'react';
import type { MascotRenderer } from './mascot';
import { render, tauri, viewEmits } from '../test-tauri';
import { MogView } from './MogView';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const ID = '00000000-0000-4000-8000-000000000001';
const OTHER = '00000000-0000-4000-8000-000000000002';

function fakeMascot() {
  const calls: string[] = [];
  const m: MascotRenderer = {
    mount: () => calls.push('mount'),
    setMood: (mood) => calls.push(`mood:${mood}`),
    setLevel: (l) => calls.push(`level:${l}`),
    destroy: () => calls.push('destroy'),
  };
  return { m, calls };
}

const overlays = { attention: false, recording: false, offline: false, localOnly: false };

describe('V2 MOG view', () => {
  beforeEach(() => tauri.reset());

  it('[V2] mounts exactly one mascot instance and destroys it on unmount', async () => {
    const { m, calls } = fakeMascot();
    const factory = vi.fn(() => m);
    const { root } = await render(<MogView mogId={ID} createMascot={factory} />);
    expect(factory).toHaveBeenCalledTimes(1);
    expect(calls).toEqual(['mount']);
    act(() => root.unmount());
    expect(calls).toEqual(['mount', 'destroy']);
  });

  it('[V2] subscribes to mog:view_state and renders the mood the brain sends', async () => {
    const { m, calls } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    expect(tauri.listeners('mog:view_state')).toBe(1);
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'listening', overlays, level: 0.5 });
    expect(calls).toContain('mood:listening');
    expect(calls).toContain('level:0.5');
    expect(el.querySelector('[data-overlay="attention"]')).toBeNull();
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'error',
      overlays: { ...overlays, attention: true },
    });
    expect(el.querySelector('[data-overlay="attention"]')).not.toBeNull();
  });

  it('[V2] ignores view state for another MOG and malformed payloads', async () => {
    const { m, calls } = fakeMascot();
    await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', { mogId: OTHER, mood: 'error', overlays });
    await tauri.emit('mog:view_state', { mogId: ID, mood: 'not-a-mood', overlays });
    await tauri.emit('mog:view_state', null);
    expect(calls).toEqual(['mount']);
  });

  it('[V2] [ARC-2] sends pointer input only through view_emit', async () => {
    const { m } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await act(async () => target.click());
    await act(async () => target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    expect(viewEmits()).toEqual([
      { mogId: ID, kind: 'click' },
      { mogId: ID, kind: 'hover' },
    ]);
    expect(tauri.invoke.mock.calls.every(([c]) => c === 'view_emit')).toBe(true);
  });

  it('[V2] stops listening on unmount', async () => {
    const { m } = fakeMascot();
    const { root } = await render(<MogView mogId={ID} createMascot={() => m} />);
    act(() => root.unmount());
    expect(tauri.listeners('mog:view_state')).toBe(0);
  });
});
