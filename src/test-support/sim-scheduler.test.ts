import { createSimScheduler, FRAME_MS } from './sim-scheduler';

describe('sim scheduler (#13 test support)', () => {
  it('[UX-36] runs timers in time order on the sim clock', async () => {
    const s = createSimScheduler();
    const seen: string[] = [];
    s.at(30, () => void seen.push(`b@${s.clock.now()}`));
    s.at(10, () => void seen.push(`a@${s.clock.now()}`));
    const cancel = s.at(20, () => void seen.push('cancelled'));
    cancel();
    await s.runUntil(25);
    expect(seen).toEqual(['a@10']);
    expect(s.clock.now()).toBe(25);
    await s.runUntil(100);
    expect(seen).toEqual(['a@10', 'b@30']);
  });

  it('[UX-36] a frame callback runs on the next vsync after now', async () => {
    const s = createSimScheduler();
    await s.runUntil(FRAME_MS); // exactly on a vsync: the frame is the next one
    let at = -1;
    s.requestFrame(() => (at = s.clock.now()));
    await s.runUntil(100);
    expect(at).toBeCloseTo(2 * FRAME_MS, 6);
  });
});
