import { createSimScheduler, FRAME_MS } from '../../test-support/sim-scheduler';
import { FrameProbe } from './frame-probe';

function setup(max?: number) {
  const s = createSimScheduler(0);
  const probe = new FrameProbe({
    now: s.clock.now,
    requestFrame: (cb) => s.requestFrame(cb),
    ...(max === undefined ? {} : { max }),
  });
  return { s, probe };
}

describe('MOG view frame probe (#13, ADR 0031)', () => {
  it('[UX-36] records the frame after a traced view state, with all the times', async () => {
    const { s, probe } = setup();
    await s.runUntil(40);
    probe.onApplied('processing', { seq: 7, eventAt: 30, sentAt: 31 });
    expect(probe.samples()).toEqual([]);
    await s.runUntil(100);
    expect(probe.samples()).toEqual([
      {
        seq: 7,
        mood: 'processing',
        eventAt: 30,
        sentAt: 31,
        receivedAt: 40,
        frameAt: 3 * FRAME_MS,
      },
    ]);
  });

  it('[UX-36] does nothing for a view state with no trace or a bad trace', async () => {
    const { s, probe } = setup();
    const raf = vi.spyOn(s, 'requestFrame');
    probe.onApplied('idle', undefined);
    probe.onApplied('idle', { seq: 1, eventAt: 0, sentAt: 0, text: 'no' });
    await s.runUntil(100);
    expect(raf).not.toHaveBeenCalled();
    expect(probe.samples()).toEqual([]);
  });

  it('[UX-05b] [UX-36] [AT-21] reports both rows from the samples', async () => {
    const { s, probe } = setup();
    for (let i = 0; i < 20; i++) {
      const t0 = i * 1000;
      await s.runUntil(t0 + 30);
      probe.onApplied('listening', { seq: i, eventAt: t0 + 25, sentAt: t0 + 26, keyDownAt: t0 });
      await s.runUntil(t0 + 500);
    }
    const r = probe.report();
    expect(r['UX-05b'].n).toBe(20);
    expect(r['UX-05b'].verdict).toBe('pass');
    expect(r['UX-05b'].p95).toBeLessThan(30 + FRAME_MS);
    expect(r['UX-36'].n).toBe(20);
    expect(r['UX-36'].verdict).toBe('pass');
  });

  it('[UX-05b] only listening samples with a key-down time count for UX-05b', async () => {
    const { s, probe } = setup();
    probe.onApplied('processing', { seq: 1, eventAt: 0, sentAt: 0, keyDownAt: 0 });
    probe.onApplied('listening', { seq: 2, eventAt: 0, sentAt: 0 });
    await s.runUntil(100);
    expect(probe.report()['UX-05b'].n).toBe(0);
    expect(probe.report()['UX-36'].n).toBe(2);
  });

  it('[UX-36] [AT-21] a state whose frame has not come yet counts as over the limit', () => {
    const { probe } = setup();
    probe.onApplied('error', { seq: 1, eventAt: 0, sentAt: 0 });
    const r = probe.report()['UX-36'];
    expect(r.n).toBe(1);
    expect(r.p95).toBe(Infinity);
  });

  it('[UX-36] keeps at most `max` samples, the newest ones', async () => {
    const { s, probe } = setup(3);
    for (let i = 0; i < 5; i++) {
      probe.onApplied('idle', { seq: i, eventAt: 0, sentAt: 0 });
      await s.runUntil(s.clock.now() + 50);
    }
    expect(probe.samples().map((x) => x.seq)).toEqual([2, 3, 4]);
  });

  it('[UX-36] the report is JSON with no text but mood names', async () => {
    const { s, probe } = setup();
    probe.onApplied('answering', { seq: 1, eventAt: 0, sentAt: 1 });
    await s.runUntil(50);
    const json = JSON.parse(probe.toJSON()) as { samples: unknown[]; rows: unknown };
    expect(json.samples).toHaveLength(1);
    expect(Object.keys(json.rows as object)).toEqual(['UX-05b', 'UX-36']);
  });
});
