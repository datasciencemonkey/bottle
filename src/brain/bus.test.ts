import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import type { MogViewState } from '../shared/ipc';
import { core } from './test-core';
import {
  BrainBus,
  emitToView,
  onViewInput,
  sendCardUpdate,
  sendMicLevel,
  sendMogViewState,
  subscribeBrain,
} from './bus';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const MOG = '00000000-0000-4000-8000-000000000001';

describe('brain Channel (ADR 0003, decision 3)', () => {
  beforeEach(() => core.reset());

  it('[ARC-2] subscribes once with brain_subscribe and a Channel', async () => {
    const bus = new BrainBus();
    await expect(subscribeBrain(bus)).resolves.toBe(true);
    expect(core.invoke).toHaveBeenCalledTimes(1);
    const [cmd, args] = core.invoke.mock.calls[0] ?? [];
    expect(cmd).toBe('brain_subscribe');
    expect(args).toEqual({ args: { onEvent: core.channels[0] } });
  });

  it('[ARC-2] delivers Channel messages to the handlers of their event', async () => {
    const bus = new BrainBus();
    const got: unknown[] = [];
    bus.on('view:input', (p) => got.push(p));
    await subscribeBrain(bus);
    core.channels[0]?.onmessage({ event: 'view:input', payload: { mogId: MOG, kind: 'click' } });
    core.channels[0]?.onmessage({ event: 'hotkey:ptt', payload: {} });
    expect(got).toEqual([{ mogId: MOG, kind: 'click' }]);
  });

  it('[ARC-2] reports a refused subscription without throwing', async () => {
    core.invoke.mockRejectedValueOnce({ error: 'not_implemented', command: 'brain_subscribe' });
    await expect(subscribeBrain(new BrainBus())).resolves.toBe(false);
  });

  it('[ARC-2] dispatches only core-to-brain events and drops malformed messages', () => {
    const bus = new BrainBus();
    const seen: string[] = [];
    bus.on('agent:tool_call', () => seen.push('tool'));
    expect(bus.dispatch({ event: 'agent:tool_call', payload: {} })).toBe(true);
    expect(bus.dispatch({ event: 'mog:view_state', payload: {} })).toBe(false);
    expect(bus.dispatch({ event: 'nope', payload: {} })).toBe(false);
    expect(bus.dispatch(null)).toBe(false);
    expect(bus.dispatch('view:input')).toBe(false);
    expect(seen).toEqual(['tool']);
  });

  it('[ARC-2] a handler can unsubscribe', () => {
    const bus = new BrainBus();
    const seen: number[] = [];
    const off = bus.on('net:status', () => seen.push(1));
    bus.dispatch({ event: 'net:status', payload: {} });
    off();
    bus.dispatch({ event: 'net:status', payload: {} });
    expect(seen).toEqual([1]);
  });

  it('[ARC-2] sends brain-to-view events with emitTo to one window label', async () => {
    await emitToView(`mog-${MOG}`, 'mog.view_state', { mogId: MOG });
    await emitToView(`card-${MOG}`, 'card.update', { mogId: MOG });
    expect(core.emitTo.mock.calls).toEqual([
      [`mog-${MOG}`, 'mog:view_state', { mogId: MOG }],
      [`card-${MOG}`, 'card:update', { mogId: MOG }],
    ]);
    expect(core.emit).not.toHaveBeenCalled();
  });

  it('[ARC-2] brain sources never emit globally or listen to Tauri events', () => {
    const dir = import.meta.dirname;
    const walk = (d: string): string[] =>
      readdirSync(d).flatMap((f) => {
        const p = join(d, f);
        return statSync(p).isDirectory() ? walk(p) : [p];
      });
    const sources = walk(dir).filter(
      (f) => /\.tsx?$/.test(f) && !/\.test\.tsx?$/.test(f) && !f.endsWith('test-core.ts'),
    );
    expect(sources.length).toBeGreaterThan(15);
    for (const f of sources) {
      const text = readFileSync(f, 'utf8');
      const name = relative(dir, f);
      expect(text, name).not.toMatch(/\bemit\s*\(/);
      expect(text, name).not.toMatch(/\blisten\s*[(<]/);
      expect(text, name).not.toMatch(
        /import\s*\{[^}]*\b(emit|listen)\b[^}]*\}\s*from\s*'@tauri-apps\/api\/event'/,
      );
    }
  });
});

describe('brain-to-view routing (#8)', () => {
  beforeEach(() => core.reset());

  it('[MOG-08b] [ARC-5] sendMogViewState emits mog:view_state to the mog-<id> window and the hub only', async () => {
    const state: MogViewState = {
      mogId: MOG,
      mood: 'idle',
      overlays: { attention: false, recording: false, offline: false, localOnly: false },
    };
    await sendMogViewState(MOG, state);
    // The hub (V1) lists every MOG with its mood, so it gets a copy (ADR 0008, decision 4).
    expect(core.emitTo.mock.calls).toEqual([
      [`mog-${MOG}`, 'mog:view_state', state],
      ['hub', 'mog:view_state', state],
    ]);
    expect(core.emit).not.toHaveBeenCalled();
  });

  it('[MOG-08b] [ARC-5] sendCardUpdate emits card:update to the card-<id> window only', async () => {
    const update = { mogId: MOG, blurbs: [], chips: [] };
    await sendCardUpdate(MOG, update);
    expect(core.emitTo.mock.calls).toEqual([[`card-${MOG}`, 'card:update', update]]);
  });

  it('[UX-06a] [ARC-5] sendMicLevel emits mog:level to the mog-<id> window only', async () => {
    const frame = { owner: MOG, dbfs: -30, t: 5 };
    await sendMicLevel(MOG, frame);
    // UX-06b: when the mic closes, the MOG that owned it gets one frame with no owner.
    await sendMicLevel(MOG, { owner: null, dbfs: -120, t: 6 });
    expect(core.emitTo.mock.calls).toEqual([
      [`mog-${MOG}`, 'mog:level', frame],
      [`mog-${MOG}`, 'mog:level', { owner: null, dbfs: -120, t: 6 }],
    ]);
    expect(core.emit).not.toHaveBeenCalled();
  });

  it('[MOG-08b] [ARC-3] refuses a MOG ID that is not label-safe and sends nothing', async () => {
    for (const bad of ['', 'a/b', 'a b', '*', 'x'.repeat(65)]) {
      await expect(sendMogViewState(bad, {})).rejects.toThrow(/window label/);
      await expect(sendCardUpdate(bad, {})).rejects.toThrow(/window label/);
    }
    expect(core.emitTo).not.toHaveBeenCalled();
  });

  it('[MOG-08b] [ARC-5] onViewInput gives typed view:input from the brain Channel', () => {
    const bus = new BrainBus();
    const got: unknown[] = [];
    onViewInput(bus, (i) => got.push(i));
    bus.dispatch({
      event: 'view:input',
      payload: { from: `mog-${MOG}`, input: { mogId: MOG, kind: 'poked' } },
    });
    bus.dispatch({
      event: 'view:input',
      payload: { from: 'hub', input: { mogId: MOG, kind: 'click' } },
    });
    expect(got).toEqual([
      { from: `mog-${MOG}`, mogId: MOG, kind: 'poked', payload: undefined },
      { from: 'hub', mogId: MOG, kind: 'click', payload: undefined },
    ]);
  });

  it('[MOG-08b] [ARC-5] onViewInput drops malformed input and input for another MOG', () => {
    const bus = new BrainBus();
    const got: unknown[] = [];
    const off = onViewInput(bus, (i) => got.push(i));
    for (const payload of [
      null,
      { from: `mog-${MOG}` },
      { from: `mog-${MOG}`, input: { mogId: 'other', kind: 'poked' } },
      { from: `card-${MOG}`, input: { mogId: MOG, kind: 'nope' } },
      { from: 'brain', input: { mogId: MOG, kind: 'click' } },
    ])
      bus.dispatch({ event: 'view:input', payload });
    expect(got).toEqual([]);
    off();
    bus.dispatch({
      event: 'view:input',
      payload: { from: 'hub', input: { mogId: MOG, kind: 'click' } },
    });
    expect(got).toEqual([]);
  });
});
