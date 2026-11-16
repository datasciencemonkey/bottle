import { createSimClock } from './clock';
import { BrainBus } from './bus';
import { asPttEvent, onPtt } from './ptt';

const MOG = '00000000-0000-4000-8000-000000000001';

describe('hotkey:ptt in the brain (#6 payload, #13)', () => {
  it('[UX-05b] parses the { mogId, phase } payload of hotkey_host and stamps the arrival', () => {
    expect(asPttEvent({ mogId: MOG, phase: 'down' }, 12)).toEqual({
      mogId: MOG,
      phase: 'down',
      at: 12,
    });
    expect(asPttEvent({ mogId: MOG, phase: 'up' }, 0)?.phase).toBe('up');
    expect(asPttEvent({ mogId: MOG, phase: 'cancel' }, 0)?.phase).toBe('cancel');
    expect(asPttEvent({ mogId: MOG, phase: 'tap' }, 0)).toBeUndefined();
    expect(asPttEvent({ phase: 'down' }, 0)).toBeUndefined();
    expect(asPttEvent(null, 0)).toBeUndefined();
  });

  it('[CAP-14a] parses the double_tap phase of section 4.5', () => {
    expect(asPttEvent({ mogId: MOG, phase: 'double_tap' }, 7)).toEqual({
      mogId: MOG,
      phase: 'double_tap',
      at: 7,
    });
  });

  it('[UX-05b] a key-down on the Channel reaches the handler in the same task, with the brain time', () => {
    const bus = new BrainBus();
    const clock = createSimClock(500);
    const got: unknown[] = [];
    const off = onPtt(bus, clock.now, (e) => got.push(e));
    expect(bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: MOG, phase: 'down' } })).toBe(
      true,
    );
    expect(got).toEqual([{ mogId: MOG, phase: 'down', at: 500 }]);
    bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: MOG, phase: 'bogus' } });
    off();
    bus.dispatch({ event: 'hotkey:ptt', payload: { mogId: MOG, phase: 'up' } });
    expect(got).toHaveLength(1);
  });
});
