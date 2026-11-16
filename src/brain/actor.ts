/**
 * The single brain-side actor (ARC-2).
 *
 * All shared queues run through one actor in the hidden brain window. Tasks on the same
 * queue key run one at a time, in order; this is how "one poll per conversation" is kept.
 * Tasks on different keys run independently. TODO(#226): T9 ConversationManager uses this.
 */
export class BrainActor {
  readonly #tails = new Map<string, Promise<unknown>>();
  readonly #pending = new Map<string, number>();

  /** Run `task` after every earlier task on `queue` has settled. */
  enqueue<T>(queue: string, task: () => Promise<T>): Promise<T> {
    const previous = this.#tails.get(queue) ?? Promise.resolve();
    this.#pending.set(queue, (this.#pending.get(queue) ?? 0) + 1);
    const run = previous.then(
      () => task(),
      () => task(),
    );
    const settled = run.finally(() => {
      const left = (this.#pending.get(queue) ?? 1) - 1;
      if (left === 0) {
        this.#pending.delete(queue);
        if (this.#tails.get(queue) === settled) this.#tails.delete(queue);
      } else {
        this.#pending.set(queue, left);
      }
    });
    // Keep the chain alive even when a task fails.
    this.#tails.set(
      queue,
      settled.catch(() => undefined),
    );
    return run;
  }

  /** Tasks queued or running on `queue`. */
  pending(queue: string): number {
    return this.#pending.get(queue) ?? 0;
  }
}

let instance: BrainActor | undefined;

/** The one actor of the brain window. */
export function getBrainActor(): BrainActor {
  instance ??= new BrainActor();
  return instance;
}
