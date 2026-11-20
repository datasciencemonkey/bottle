import { BrainBus } from './bus';
import { wireMogLook } from './mog-look-relay';

const send = (bus: BrainBus, payload: unknown) => bus.dispatch({ event: 'mog:look', payload });

describe('lamp-alive: the look relay', () => {
  it('sends { dx, dy } to the one MOG the core named, never the ID in the payload', () => {
    const bus = new BrainBus();
    const got: unknown[][] = [];
    wireMogLook(bus, (id, look) => void got.push([id, look]));
    expect(send(bus, { mogId: 'a1', dx: 0.5, dy: -0.25 })).toBe(true);
    expect(got).toEqual([['a1', { dx: 0.5, dy: -0.25 }]]);
  });

  it('clamps a vector to the unit disc and drops a bad payload or ID', () => {
    const bus = new BrainBus();
    const got: unknown[][] = [];
    wireMogLook(bus, (id, look) => void got.push([id, look]));
    send(bus, { mogId: 'a1', dx: 3, dy: 4 });
    send(bus, { mogId: 'a1', dx: 'x', dy: 0 });
    send(bus, { mogId: 7, dx: 0, dy: 0 });
    send(bus, null);
    expect(got).toHaveLength(1);
    const [, look] = got[0] as [string, { dx: number; dy: number }];
    expect(Math.hypot(look.dx, look.dy)).toBeCloseTo(1);
  });

  it('survives a sender that throws (an ID that is not a window label)', () => {
    const bus = new BrainBus();
    wireMogLook(bus, () => {
      throw new TypeError('label');
    });
    expect(() => send(bus, { mogId: 'a b', dx: 0, dy: 0 })).not.toThrow();
  });
});
