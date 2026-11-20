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
 *   MOG or the card). The brain holds the selection until the hub acks it
 *   (`conversations.opened`) or asks for the list, so a hub that opens late still selects it, once.
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

/** One defined MOG: its id, name and accent colour. */
export interface ConversationMog {
  mogId: string;
  name: string;
  /** #ui-4: `mog.ui.variant`. */
  colorVariant?: string;
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
  /**
   * #523: the profile and host of a listed exchange, from the last list the brain sent (never
   * from a view). The link opener trusts this, not the click.
   */
  workspaceOf(requestId: string): { profile: string; host: string } | undefined;
  /** Open Conversations, with this exchange selected when given. Never rejects. */
  openConversations(requestId?: string): Promise<void>;
  /**
   * The MOG and the profile that own a thread's `conversationId` (genie-followup, #145): a
   * follow-up keeps the MOG and the workspace of the first question, read from the stored
   * exchanges — never from the hub view, which only names the conversation it wants, not the
   * workspace it may use (ARC-2). `undefined` for a blank id or one no stored exchange carries.
   * Never rejects.
   */
  findThread(
    conversationId: string,
  ): Promise<{ mogId: string; profile: string; host: string } | undefined>;
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
    ...(mog.colorVariant !== undefined ? { mogColorVariant: mog.colorVariant } : {}),
    question: str(o['questionUser']),
    answerMd: str(o['finalAnswerMd']),
    keyNumbers: keyNumbersOf(o['keyNumbers']),
    askedAt,
    profile: str(o['profileName']),
    workspaceHost: str(o['workspaceHost']),
    deepLink: str(o['deepLink']),
    status: o['status'] === 'completed' ? 'answered' : 'failed',
    conversationId: str(o['conversationId']),
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
  /**
   * The owner of each conversation, from the last list read (#145). A conversation never changes
   * its MOG or its workspace, so a send takes it from here with no store round trip. Every
   * publish replaces it, so a Forget or a Wipe (which refresh the list) drops a gone thread.
   */
  let owners = new Map<string, { mogId: string; profile: string; host: string }>();
  let known = new Map<string, { profile: string; host: string }>();
  const remember = (rows: readonly ConversationRow[]) => {
    owners = new Map(
      rows
        .filter((r) => r.conversationId !== '')
        .map((r) => [
          r.conversationId,
          { mogId: r.mogId, profile: r.profile, host: r.workspaceHost },
        ]),
    );
  };
  const publish = async () => {
    try {
      const list = await readConversations(deps);
      if (!list.unavailable) remember(list.rows);
      known = new Map(
        list.rows.map((r) => [r.requestId, { profile: r.profile, host: r.workspaceHost }]),
      );
      await deps.send(list);
    } catch {
      // No hub is open. The hub asks again with conversations.ready when it opens.
    }
  };
  const off = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== CONVERSATIONS_VIEW_ID || input.kind !== 'click')
      return;
    const action = asConversationsAction(input.payload);
    if (!action) return;
    if (action.action === 'conversations.opened') {
      held = undefined;
      return;
    }
    void (async () => {
      await publish();
      const open = held;
      held = undefined;
      if (open) await deps.sendOpen(open).catch(() => undefined);
    })();
  });
  return {
    refresh: publish,
    workspaceOf: (requestId) => known.get(requestId),
    async openConversations(requestId) {
      const open = requestId ? { requestId } : {};
      held = open;
      try {
        await deps.sendOpen(open);
      } catch {
        // No hub is open yet: the held selection goes when the hub asks.
      }
    },
    async findThread(conversationId) {
      if (conversationId === '') return undefined;
      const cached = owners.get(conversationId);
      if (cached) return cached;
      try {
        const { rows } = await readConversations(deps);
        const row = rows.find((r) => r.conversationId === conversationId);
        return row
          ? { mogId: row.mogId, profile: row.profile, host: row.workspaceHost }
          : undefined;
      } catch {
        return undefined;
      }
    },
    dispose: off,
  };
}
