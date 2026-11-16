// @vitest-environment jsdom
import { act } from 'react';
import type { MascotRenderer } from './mascot';
import type { MicLevelFrame, MicLevelSource } from './level-meter';
import { RiveMascot } from './rive-mascot';
import { render, tauri, viewEmits } from '../test-tauri';
import { MogView, defaultMascot } from './MogView';

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
    setAttention: (a) => calls.push(`attention:${a}`),
    setHover: (h) => calls.push(`hover:${h}`),
    poke: () => calls.push('poke'),
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
    // UX-06a: the view computes the level from the mic frames, not from the brain's value.
    expect(calls).not.toContain('level:0.5');
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

  it('[V2] [ARC-2] [UX-08a] sends pointer input only through view_emit', async () => {
    const { m, calls } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await act(async () => target.click());
    await act(async () => target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    expect(calls).toEqual(['mount', 'poke', 'hover:true']);
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

  it('[UX-09a] the default renderer is the Rive mascot', () => {
    expect(defaultMascot()).toBeInstanceOf(RiveMascot);
  });

  it('[UX-03] [AT-37] binds the attention overlay of mog.view_state to the mascot cue', async () => {
    const { m, calls } = fakeMascot();
    await render(<MogView mogId={ID} createMascot={() => m} />);
    await tauri.emit('mog:view_state', {
      mogId: ID,
      mood: 'idle',
      overlays: { ...overlays, attention: true },
    });
    expect(calls).toEqual(['mount', 'mood:idle', 'attention:true']);
  });

  it('[UX-08a] pointer leave turns the mascot hover off', async () => {
    const { m, calls } = fakeMascot();
    const { el } = await render(<MogView mogId={ID} createMascot={() => m} />);
    const target = el.querySelector('[aria-label="MOG"]') as HTMLElement;
    await act(async () => target.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
    await act(async () =>
      target.dispatchEvent(
        new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body }),
      ),
    );
    expect(calls).toEqual(['mount', 'hover:true', 'hover:false']);
  });

  it('[UX-06a] [UX-06b] [AT-14] the level comes from the mic frames this MOG owns', async () => {
    const { m, calls } = fakeMascot();
    let push: (f: MicLevelFrame) => void = () => undefined;
    let subs = 0;
    const levels: MicLevelSource = {
      subscribe: (fn) => {
        subs++;
        push = fn;
        return () => {
          subs--;
        };
      },
    };
    const { root } = await render(<MogView mogId={ID} createMascot={() => m} levels={levels} />);
    expect(subs).toBe(1);
    act(() => {
      for (let t = 0; t <= 300; t += 10) push({ owner: OTHER, dbfs: -10, t });
    });
    expect(calls.filter((c) => c.startsWith('level:'))).toEqual([]);
    act(() => {
      for (let t = 400; t <= 700; t += 10) push({ owner: ID, dbfs: -10, t });
    });
    const shown = calls.filter((c) => c.startsWith('level:')).map((c) => Number(c.slice(6)));
    expect(shown.length).toBeGreaterThan(3);
    expect(shown.at(-1)).toBeGreaterThan(0.9);
    act(() => push({ owner: null, dbfs: -10, t: 710 }));
    expect(calls.at(-1)).toBe('level:0');
    act(() => root.unmount());
    expect(subs).toBe(0);
  });
});
