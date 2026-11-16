import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { core } from './test-core';
import { BrainBus, emitToView, subscribeBrain } from './bus';

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
