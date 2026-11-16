import { BrainActor, getBrainActor } from './actor';

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe('BrainActor', () => {
  it('[ARC-2] the brain has exactly one actor', () => {
    expect(getBrainActor()).toBe(getBrainActor());
  });

  it('[ARC-2] runs tasks on one queue one at a time, in order', async () => {
    const actor = new BrainActor();
    const log: string[] = [];
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const first = actor.enqueue('conv-1', async () => {
      log.push('first:start');
      await gate;
      log.push('first:end');
      return 1;
    });
    const second = actor.enqueue('conv-1', async () => {
      log.push('second:start');
      return 2;
    });
    await tick();
    expect(log).toEqual(['first:start']);
    expect(actor.pending('conv-1')).toBe(2);
    release();
    await expect(first).resolves.toBe(1);
    await expect(second).resolves.toBe(2);
    expect(log).toEqual(['first:start', 'first:end', 'second:start']);
    expect(actor.pending('conv-1')).toBe(0);
  });

  it('[ARC-2] different queues do not block each other', async () => {
    const actor = new BrainActor();
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const slow = actor.enqueue('conv-1', () => gate);
    const fast = actor.enqueue('conv-2', async () => 'done');
    await expect(fast).resolves.toBe('done');
    release();
    await slow;
  });

  it('[ARC-2] a failed task does not stop the queue', async () => {
    const actor = new BrainActor();
    const failed = actor.enqueue('q', async () => {
      throw new Error('boom');
    });
    const next = actor.enqueue('q', async () => 'ok');
    await expect(failed).rejects.toThrow('boom');
    await expect(next).resolves.toBe('ok');
  });
});
