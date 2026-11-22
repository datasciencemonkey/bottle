/**
 * "Remember this" (local memory slice 1, ADR 0113): the card button and the voice phrase.
 *
 * - Card: one click saves the Genie answer as a pinned `genie_answer` with origin `pin`. The
 *   title is the question (80), the text is the summary (600), the body is the answer (4000).
 * - Voice: Rust parsed the phrase from its own transcript and minted the ticket. The brain saves
 *   one pinned `fact` with `text = remember.text` and does not paste. It never parses the phrase.
 * - Each save sends `memory:saved` (content-free) to that MOG's card only.
 * - Undo (`memory.undo`) forgets an item that this MOG saved in the last {@link MEMORY_UNDO_MS}.
 * - No ticket: no call, and the card gets `failed`. The brain never makes a ticket.
 * - Nothing here logs. No question, answer, text or error detail leaves this module.
 */
import { emitTo } from '@tauri-apps/api/event';
import { MEMORY_UNDO_MS } from '../shared/hub-memory';
import type { MemorySaved, MogId } from '../shared/interfaces';
import { EVENTS } from '../shared/ipc';
import { viewLabel } from './bus';
import type { LocalMemoryService } from './components/memory-service';
import { writeCodeOf } from './components/memory-service';
import type { LocalMemoryStore } from './components/memory-store';

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const TITLE_MAX = 80;
const TEXT_MAX = 600;
const BODY_MAX = 4_000;
const TITLE_FALLBACK = 'Genie answer';

const cut = (s: string, max: number): string => {
  const cps = Array.from(s);
  return cps.length <= max ? s : cps.slice(0, max).join('');
};
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

export interface RememberDeps {
  memory: Pick<LocalMemoryService, 'pinItem' | 'forget'>;
  /** Reads the exchange of a card (`mem.exchange.get`, `mem.exchange.recent`). */
  store: LocalMemoryStore;
  /** Send `memory:saved` to the card of `mogId` only. */
  send(mogId: MogId, saved: MemorySaved): Promise<unknown>;
  /** Show the card of `mogId` (a voice save shows its line there). Optional. */
  showCard?(mogId: MogId): Promise<unknown>;
  now?: () => number;
}

export interface RememberFlow {
  /** The card button. `requestId` names the exchange; `ticket` is the host's. */
  card(mogId: MogId, requestId: string | undefined, ticket: string | undefined): Promise<void>;
  /** The voice phrase, with the host's ticket and the text of Rust's own transcript. */
  voice(mogId: MogId, text: string, ticket: string): Promise<void>;
  /** `memory.undo`: forget the newest item saved for `mogId` in the last 8 s. Never rejects. */
  undo(mogId: MogId): Promise<void>;
}

/** Send `memory:saved` to the card window of `mogId` only. */
export function sendMemorySaved(mogId: MogId, saved: MemorySaved): Promise<void> {
  return emitTo(viewLabel('card', mogId), EVENTS['memory.saved'].name, saved);
}

export function createRememberFlow(deps: RememberDeps): RememberFlow {
  const now = deps.now ?? (() => Date.now());
  const savedAt = new Map<MogId, { itemId: string; at: number }[]>();
  const reply = async (mogId: MogId, saved: Omit<MemorySaved, 'mogId' | 'undoMs'>) => {
    const ok = saved.outcome === 'saved' || saved.outcome === 'quarantined';
    try {
      await deps.send(mogId, { mogId, ...saved, undoMs: ok ? MEMORY_UNDO_MS : 0 });
    } catch {
      // No card window: nothing to show.
    }
  };
  const save = async (
    mogId: MogId,
    write: Parameters<RememberDeps['memory']['pinItem']>[0],
    ticket: string,
  ) => {
    try {
      const { itemId, status } = await deps.memory.pinItem(write, ticket);
      const list = (savedAt.get(mogId) ?? []).filter((e) => now() - e.at <= MEMORY_UNDO_MS);
      savedAt.set(mogId, [...list, { itemId, at: now() }]);
      await reply(mogId, {
        itemId,
        outcome: status === 'quarantined' ? 'quarantined' : 'saved',
      });
    } catch (e) {
      await reply(mogId, {
        outcome: writeCodeOf(e) === 'secret_refused' ? 'secret_refused' : 'failed',
      });
    }
  };
  /** The exchange of a card: the ids, the question and the answer, never logged. */
  const exchangeOf = async (mogId: MogId, requestId: string) => {
    const { rows } = await deps.store.exec('mem.exchange.get', { mogId, requestId });
    const x = rows[0] as Record<string, unknown> | undefined;
    if (!x || !UUID.test(str(x['exchangeId']))) return undefined;
    let profile = '';
    try {
      const recent = await deps.store.exec('mem.exchange.recent', { mogId, limit: 50 });
      const r = recent.rows.find(
        (o) => str((o as Record<string, unknown>)['requestId']) === requestId,
      );
      profile = str((r as Record<string, unknown> | undefined)?.['profileName']);
    } catch {
      // The profile is a nice-to-have link.
    }
    return { x, profile };
  };
  return {
    async card(mogId, requestId, ticket) {
      if (ticket === undefined || requestId === undefined || !UUID.test(requestId)) {
        await reply(mogId, { outcome: 'failed' });
        return;
      }
      let found: Awaited<ReturnType<typeof exchangeOf>>;
      try {
        found = await exchangeOf(mogId, requestId);
      } catch {
        found = undefined;
      }
      const answer = str(found?.x['finalAnswerMd']);
      const text = cut(str(found?.x['answerSummary']).trim() || answer.trim(), TEXT_MAX);
      if (!found || text === '') {
        await reply(mogId, { outcome: 'failed' });
        return;
      }
      const { x, profile } = found;
      const host = str(x['workspaceHost']);
      const wsId = x['workspaceId'];
      const space = str(x['spaceId']);
      await save(
        mogId,
        {
          mogId,
          itemType: 'genie_answer',
          origin: 'pin',
          title: cut(str(x['questionUser']).trim(), TITLE_MAX) || TITLE_FALLBACK,
          text,
          body: cut(answer, BODY_MAX),
          sourceExchangeId: str(x['exchangeId']),
          ...(host !== '' ? { workspaceHost: host } : {}),
          ...(typeof wsId === 'number' || typeof wsId === 'string' ? { workspaceId: wsId } : {}),
          ...(space !== '' ? { spaceId: space } : {}),
          ...(profile !== '' ? { profile } : {}),
        },
        ticket,
      );
    },
    async voice(mogId, text, ticket) {
      try {
        await deps.showCard?.(mogId);
      } catch {
        // The card may be open already.
      }
      await save(mogId, { mogId, itemType: 'fact', origin: 'pin', text }, ticket);
    },
    async undo(mogId) {
      const list = savedAt.get(mogId) ?? [];
      const last = list[list.length - 1];
      if (!last || now() - last.at > MEMORY_UNDO_MS) return;
      savedAt.set(mogId, list.slice(0, -1));
      try {
        await deps.memory.forget({ mogId, itemIds: [last.itemId] });
      } catch {
        // Nothing to say: the item stays, and the user can delete it in the hub.
      }
    },
  };
}
