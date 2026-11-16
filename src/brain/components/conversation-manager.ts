/**
 * T9 ConversationManager (4.3, hidden TS brain window).
 *
 * Responsibilities: One PollActor per conversation (D31/D55 cadence), D49 topic reset, offline
 * outbox replay, SplitCoordinator (v1).
 */
import type { MogId } from '../../shared/interfaces';
import { getBrainActor, type BrainActor } from '../actor';
import { NotImplementedError } from '../errors';

export interface ConversationManager {
  /** Run `task` on the poll queue of `conversationId`: one poll per conversation at a time. */
  enqueuePoll<T>(conversationId: string, task: () => Promise<T>): Promise<T>;
  resetTopic(mogId: MogId): void;
  replayOutbox(): Promise<number>;
}

const C = 'ConversationManager';

export class ConversationManagerStub implements ConversationManager {
  readonly #actor: BrainActor;

  constructor(actor: BrainActor) {
    this.#actor = actor;
  }

  // The serial queue is the shared brain actor (ARC-2). TODO(#31): the G-POLL cadence.
  enqueuePoll<T>(conversationId: string, task: () => Promise<T>): Promise<T> {
    return this.#actor.enqueue(`poll:${conversationId}`, task);
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
): ConversationManager {
  return new ConversationManagerStub(actor);
}
