/**
 * The brain side of the hub Conversations list (#103, #40).
 *
 * - The brain reads the newest exchanges of each defined MOG from the R10 store
 *   (`mem.exchange.recent` through `mem_local_exec`, MEM-01: one MOG per read), merges them
 *   newest first and sends the list to the hub window only (ARC-5). The hub view never runs a
 *   store statement.
 * - The hub sends `conversations.ready` when Conversations opens or shows again. The brain sends
 *   the list again after each saved exchange (`refresh`).
 * - `openConversations(requestId)` opens Conversations on one exchange (a double-click on the
 *   MOG or the card). The brain holds the selection until the hub asks, so a hub that opens late
 *   still selects it, once.
 * - A failed read sends an empty list marked `unavailable`, with no error text.
 * - Rows of a deleted MOG stay in the store until Wipe; the list shows defined MOGs only.
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  asConversationsAction,
  CONVERSATIONS_MAX,
  CONVERSATIONS_VIEW_ID,
  HUB_CONVERSATIONS_EVENT,
  HUB_CONVERSATIONS_OPEN_EVENT,
  type ConversationKeyNumber,
  type ConversationRow,
  type HubConversations,
} from '../shared/hub-conversations';
import { onViewInput, type BrainBus } from './bus';
import type { LocalMemoryStore } from './components/memory-store';

/** One defined MOG: its id and name. */
export interface ConversationMog {
  mogId: string;
  name: string;
}

export interface ConversationsReadDeps {
  store: LocalMemoryStore;
  mogs(): Promise<ConversationMog[]>;
}

export interface ConversationsWireDeps extends ConversationsReadDeps {
  send(list: HubConversations): Promise<unknown>;
  sendOpen(open: { requestId?: string }): Promise<unknown>;
}

export interface HubConversationsWiring {
  /** Send the list to the hub again. Never rejects. */
  refresh(): Promise<void>;
  /** Open Conversations, with this exchange selected when given. Never rejects. */
  openConversations(requestId?: string): Promise<void>;
  dispose(): void;
}

/** Send the list to the hub window only. */
export function sendHubConversations(list: HubConversations): Promise<void> {
  return emitTo('hub', HUB_CONVERSATIONS_EVENT, list);
}

/** Send an open request to the hub window only. */
export function sendHubConversationsOpen(open: { requestId?: string }): Promise<void> {
  return emitTo('hub', HUB_CONVERSATIONS_OPEN_EVENT, open);
}

const str = (v: unknown): string => (typeof v === 'string' ? v : '');

function keyNumbersOf(raw: unknown): ConversationKeyNumber[] {
  let v: unknown;
  try {
    v = typeof raw === 'string' ? JSON.parse(raw) : raw;
  } catch {
    return [];
  }
  if (!Array.isArray(v)) return [];
  return v.flatMap((n: unknown) => {
    if (typeof n !== 'object' || n === null) return [];
    const { text, label } = n as Record<string, unknown>;
    if (typeof text !== 'string' || text === '') return [];
    return [typeof label === 'string' && label !== '' ? { text, label } : { text }];
  });
}

function rowOf(r: unknown, mog: ConversationMog): ConversationRow | undefined {
  if (typeof r !== 'object' || r === null) return undefined;
  const o = r as Record<string, unknown>;
  const requestId = str(o['requestId']);
  const askedAt = o['askedAt'];
  // The brain saves a row only when a request ends, so a pending row is not a conversation yet.
  if (requestId === '' || typeof askedAt !== 'number' || o['status'] === 'pending') {
    return undefined;
  }
  return {
    requestId,
    mogId: mog.mogId,
    mogName: mog.name,
    question: str(o['questionUser']),
    answerMd: str(o['finalAnswerMd']),
    keyNumbers: keyNumbersOf(o['keyNumbers']),
    askedAt,
    profile: str(o['profileName']),
    workspaceHost: str(o['workspaceHost']),
    status: o['status'] === 'completed' ? 'answered' : 'failed',
  };
}

/** Read the merged list: the newest `CONVERSATIONS_MAX` exchanges of all MOGs. */
export async function readConversations(deps: ConversationsReadDeps): Promise<HubConversations> {
  try {
    const mogs = await deps.mogs();
    const per = await Promise.all(
      mogs.map(async (m) => {
        const { rows } = await deps.store.exec('mem.exchange.recent', {
          mogId: m.mogId,
          limit: CONVERSATIONS_MAX,
        });
        return rows.flatMap((r) => rowOf(r, m) ?? []);
      }),
    );
    const rows = per
      .flat()
      .sort((a, b) => b.askedAt - a.askedAt || (a.requestId < b.requestId ? 1 : -1))
      .slice(0, CONVERSATIONS_MAX);
    return { rows };
  } catch {
    return { rows: [], unavailable: true };
  }
}

/** Wire the hub Conversations list. */
export function wireHubConversations(
  bus: BrainBus,
  deps: ConversationsWireDeps,
): HubConversationsWiring {
  let held: { requestId?: string } | undefined;
  const publish = async () => {
    try {
      await deps.send(await readConversations(deps));
    } catch {
      // No hub is open. The hub asks again with conversations.ready when it opens.
    }
  };
  const off = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== CONVERSATIONS_VIEW_ID || input.kind !== 'click')
      return;
    if (!asConversationsAction(input.payload)) return;
    void (async () => {
      await publish();
      const open = held;
      held = undefined;
      if (open) await deps.sendOpen(open).catch(() => undefined);
    })();
  });
  return {
    refresh: publish,
    async openConversations(requestId) {
      const open = requestId ? { requestId } : {};
      held = open;
      try {
        await deps.sendOpen(open);
      } catch {
        // No hub is open yet: the held selection goes when the hub asks.
      }
    },
    dispose: off,
  };
}
