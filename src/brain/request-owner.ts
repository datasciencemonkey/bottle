/**
 * The MOG that captures an utterance owns its request and its answer (#73, MOG-13a, MOG-13b,
 * ADR 0058).
 *
 * At `hotkey:ptt` down the capture lifecycle calls `begin(mogId)` with the MOG that Rust
 * latched. That MOG stays the owner until `release`, also when the user interacts with another
 * MOG and the talk-key target moves. The request gets the owner's `mogId` and tags, and the
 * answer goes to the owner's card only.
 * The utterance hook (`utterance-wiring.ts`, #59) calls `begin` and `release`.
 * The T3 ask gate (#142) calls `release` when a preview ends or a chip is offered.
 * TODO(#563): the agent runtime calls `stampRequest` and `deliverAnswer`.
 */
import type { AskRequest, MogId } from '../shared/interfaces';
import type { CardUpdate } from '../shared/ipc';
import { sendCardUpdate } from './bus';

/** One captured utterance and the MOG that owns it. */
export interface OwnedUtterance {
  utteranceId: string;
  mogId: MogId;
}

/** The request fields that do not depend on the owner. */
export type UnownedRequest = Omit<AskRequest, 'mogId' | 'tags'>;

/** The answer fields of a card update; the owner fills in `mogId`. */
export type OwnedAnswer = Omit<CardUpdate, 'mogId'>;

export interface RequestOwnerDeps {
  /** True while the MOG window is open (the talk-key target tracker). */
  isOpen: (mogId: MogId) => boolean;
  newId: () => string;
  sendCard: (mogId: MogId, update: CardUpdate) => Promise<unknown>;
}

export interface RequestOwner {
  /** Start an utterance for the capturing MOG. Undefined if that MOG is not open. */
  begin(mogId: MogId): OwnedUtterance | undefined;
  /** The owner of an utterance, if it is still live. */
  ownerOf(utteranceId: string): MogId | undefined;
  /** The request for an utterance, with the owner's `mogId` and tags (MOG-13a). */
  stampRequest(utteranceId: string, req: UnownedRequest): AskRequest | undefined;
  /** Show the answer on the owner's card (MOG-13b). False if the utterance is not live. */
  deliverAnswer(utteranceId: string, answer: OwnedAnswer): Promise<boolean>;
  /** The utterance was cancelled: it has no owner now. */
  cancel(utteranceId: string): void;
  /** The exchange is done: drop the owner. */
  release(utteranceId: string): void;
}

export function createRequestOwner(deps: RequestOwnerDeps): RequestOwner {
  const owners = new Map<string, MogId>();
  return {
    begin(mogId) {
      if (!deps.isOpen(mogId)) return undefined;
      const utteranceId = deps.newId();
      owners.set(utteranceId, mogId);
      return { utteranceId, mogId };
    },
    ownerOf: (id) => owners.get(id),
    stampRequest(id, req) {
      const mogId = owners.get(id);
      if (mogId === undefined) return undefined;
      return { ...req, mogId, tags: { app: 'bottle', mog: mogId, op: 'genie' } };
    },
    async deliverAnswer(id, answer) {
      const mogId = owners.get(id);
      if (mogId === undefined) return false;
      await deps.sendCard(mogId, { ...answer, mogId });
      return true;
    },
    cancel: (id) => void owners.delete(id),
    release: (id) => void owners.delete(id),
  };
}

/** The production card send: `card.update` to the owner's card window only (ARC-5). */
export function sendOwnedCard(mogId: MogId, update: CardUpdate): Promise<void> {
  return sendCardUpdate(mogId, update);
}
