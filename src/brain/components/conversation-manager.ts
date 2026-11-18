/**
 * T9 ConversationManager (4.3, hidden TS brain window).
 *
 * Responsibilities: One PollActor per conversation (D31/D55 cadence), D49 topic reset, offline
 * outbox replay, SplitCoordinator (v1).
 */
import type { MogId, ProfileName } from '../../shared/interfaces';
import { call } from '../../shared/ipc';
import { getBrainActor, type BrainActor } from '../actor';
import { NotImplementedError } from '../errors';
import {
  GIVE_UP_MS,
  POLL_TIMEOUT_MS,
  createInflightAsks,
  createMemoryPollJournal,
  runPollActor,
  type PollEnv,
  type PollJournal,
  type PollOutcome,
  type PollTarget,
  type PollUpdate,
} from './poll-actor';

export interface ConversationManager {
  /** Run `task` on the poll queue of `conversationId`: one poll per conversation at a time. */
  enqueuePoll<T>(conversationId: string, task: () => Promise<T>): Promise<T>;
  /**
   * Start the PollActor of an accepted ask (#31, ADR 0073): G-POLL cadence, the G-08 floor,
   * polls on the queue of the conversation. The ask counts as in flight on its profile from
   * this call until the loop ends.
   */
  poll(
    target: PollTarget,
    signal?: AbortSignal,
  ): AsyncGenerator<PollUpdate, PollOutcome, undefined>;
  /** G-08: the asks in flight on `profile`. */
  inflightAsks(profile: ProfileName): number;
  /** ARC-F10: the IDs and profile of every open poll loop. */
  readonly journal: PollJournal;
  resetTopic(mogId: MogId): void;
  replayOutbox(): Promise<number>;
}

const C = 'ConversationManager';

function systemSleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) return resolve();
    const t = setTimeout(resolve, ms);
    signal?.addEventListener(
      'abort',
      () => {
        clearTimeout(t);
        resolve();
      },
      { once: true },
    );
  });
}

// One journal and one in-flight count for the whole brain window, whoever builds a manager.
const brainJournal = createMemoryPollJournal();
const brainInflight = createInflightAsks();

function defaultPollEnv(): PollEnv {
  return {
    call: (args) => call('mcp_call', args),
    // Epoch milliseconds, so `askedAt` still means something after a restart (ARC-F10).
    now: () => Date.now(),
    sleep: systemSleep,
    random: Math.random,
    journal: brainJournal,
    inflight: brainInflight,
    giveUpMs: GIVE_UP_MS,
    pollTimeoutMs: POLL_TIMEOUT_MS,
  };
}

export class ConversationManagerStub implements ConversationManager {
  readonly #actor: BrainActor;
  readonly #env: PollEnv;

  constructor(actor: BrainActor, env: Partial<PollEnv> = {}) {
    this.#actor = actor;
    this.#env = { ...defaultPollEnv(), ...env };
  }

  get journal(): PollJournal {
    return this.#env.journal;
  }

  // The serial queue is the shared brain actor (ARC-2).
  enqueuePoll<T>(conversationId: string, task: () => Promise<T>): Promise<T> {
    return this.#actor.enqueue(`poll:${conversationId}`, task);
  }

  poll(
    target: PollTarget,
    signal?: AbortSignal,
  ): AsyncGenerator<PollUpdate, PollOutcome, undefined> {
    // Count the ask now, not on the first `next()`, so asks started together see each other.
    // It stops counting when the loop ends, throws, or the caller calls `return()`.
    const remove = this.#env.inflight.add(target.profile);
    let counted = true;
    const release = () => {
      if (counted) remove();
      counted = false;
    };
    const loop = runPollActor(
      target,
      this.#env,
      (cid, task) => this.enqueuePoll(cid, task),
      signal,
    );
    const gen = (async function* () {
      try {
        return yield* loop;
      } finally {
        release();
      }
    })();
    const stop = gen.return.bind(gen);
    gen.return = (v) => {
      release();
      return stop(v);
    };
    return gen;
  }

  inflightAsks(profile: ProfileName): number {
    return this.#env.inflight.count(profile);
  }

  resetTopic(_mogId: MogId): void {
    throw new NotImplementedError(C, 'resetTopic', 145); // TODO(#145)
  }
  async replayOutbox(): Promise<number> {
    throw new NotImplementedError(C, 'replayOutbox', 41); // TODO(#41)
  }
}

export function createConversationManager(
  actor: BrainActor = getBrainActor(),
  env: Partial<PollEnv> = {},
): ConversationManager {
  return new ConversationManagerStub(actor, env);
}
