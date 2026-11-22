/**
 * The brain side of the hub Memory area (local memory slice 1, ADR 0112, ADR 0113).
 *
 * - The hub sends `view_emit` clicks with `mogId: 'memory'` and a {@link MemoryAction}. The hub
 *   view never runs a store statement (ARC-2). Every event goes to the hub window only (ARC-5).
 * - `memory.ready` reads the rows of each MOG (`mem.item.list`) and sends `hub:memory`.
 *   `memory.file` and `memory.item` read one file or one item and send it back.
 * - `memory.add`, `memory.edit` and `memory.rate` are user writes. They run only with the
 *   `actionTicket` that the host minted for the click. With no ticket the brain makes no call
 *   and sends `hub:memory.result` with `ok: false, code: 'failed'`. The brain never makes a ticket.
 * - After each successful write the brain sends `hub:memory` again. `memory.delete` runs
 *   `mem.forget.items` (the view waited out the Undo time) and sends `hub:memory` again.
 * - A result holds a code and an id only: never a title, a body or a path. Nothing here logs.
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  asHubMemory,
  asMemoryFileView,
  EDITABLE_KINDS,
  HUB_MEMORY_EVENT,
  HUB_MEMORY_FILE_EVENT,
  HUB_MEMORY_ITEM_EVENT,
  HUB_MEMORY_RESULT_EVENT,
  MEMORY_KINDS,
  MEMORY_STATUSES,
  MEMORY_VIEW_ID,
  type EditableKind,
  type HubMemory,
  type MemoryAction,
  type MemoryFolder,
  type MemoryItemFields,
  type MemoryKindName,
  type MemoryResult,
  type MemoryStatusName,
  BODY_MAX,
  TITLE_MAX,
} from '../shared/hub-memory';
import { onViewInput, type BrainBus } from './bus';
import { writeCodeOf, type LocalMemoryService } from './components/memory-service';

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const REQUEST_ID = /^[A-Za-z0-9-]{1,64}$/;
const FILE_PATH = /^\/memories\/[A-Za-z0-9./#_-]{1,200}$/;
const KINDS: ReadonlySet<string> = new Set(MEMORY_KINDS);
const STATUSES: ReadonlySet<string> = new Set(MEMORY_STATUSES);
const EDITABLE: ReadonlySet<string> = new Set(EDITABLE_KINDS);

/** One defined MOG: its id, name and accent colour. */
export interface MemoryMog {
  mogId: string;
  name: string;
  colorVariant?: string;
  /** The backend of the MOG row, for the row that a MOG with no exchange yet needs. */
  backendKind: string;
}

export type HubMemoryOps = Pick<
  LocalMemoryService,
  | 'pinItem'
  | 'editItem'
  | 'rateItem'
  | 'readItem'
  | 'listItems'
  | 'ensureMog'
  | 'readFile'
  | 'forget'
>;

export interface HubMemoryDeps {
  memory: HubMemoryOps;
  mogs(): Promise<MemoryMog[]>;
  /** Send one event to the hub window only. Default: `emitTo('hub', …)`. */
  send?(event: string, payload: unknown): Promise<unknown>;
  /** Whether a hub window exists. Default: yes. */
  hubPresent?(): Promise<boolean>;
}

export interface HubMemoryWiring {
  /** Send `hub:memory` again. Never rejects. */
  refresh(): Promise<void>;
  /** Send `hub:memory` with no request from the hub, when a hub exists. Never rejects. */
  push(): Promise<void>;
  dispose(): void;
}

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
/** The length in code points, like the catalog caps. */
const len = (s: string): number => Array.from(s).length;
const isStr = (v: unknown): v is string => typeof v === 'string';

/** Check a hub click payload. Anything that is not a well-formed action is `undefined`. */
export function asMemoryAction(p: unknown): MemoryAction | undefined {
  if (!isObj(p) || !isStr(p['action'])) return undefined;
  const { mogId, itemId, requestId, kind, title, body, path } = p;
  switch (p['action']) {
    case 'memory.ready':
      return { action: 'memory.ready' };
    case 'memory.file':
      return isStr(mogId) && UUID.test(mogId) && isStr(path) && FILE_PATH.test(path)
        ? { action: 'memory.file', mogId, path }
        : undefined;
    case 'memory.item':
    case 'memory.delete':
      return isStr(mogId) && UUID.test(mogId) && isStr(itemId) && UUID.test(itemId)
        ? { action: p['action'], mogId, itemId }
        : undefined;
    case 'memory.rate':
      return isStr(requestId) &&
        REQUEST_ID.test(requestId) &&
        isStr(mogId) &&
        UUID.test(mogId) &&
        isStr(itemId) &&
        UUID.test(itemId)
        ? { action: 'memory.rate', requestId, mogId, itemId }
        : undefined;
    case 'memory.add':
      return isStr(requestId) &&
        REQUEST_ID.test(requestId) &&
        isStr(mogId) &&
        UUID.test(mogId) &&
        isStr(kind) &&
        EDITABLE.has(kind) &&
        isStr(title) &&
        isStr(body)
        ? { action: 'memory.add', requestId, mogId, kind: kind as EditableKind, title, body }
        : undefined;
    case 'memory.edit':
      return isStr(requestId) &&
        REQUEST_ID.test(requestId) &&
        isStr(mogId) &&
        UUID.test(mogId) &&
        isStr(itemId) &&
        UUID.test(itemId) &&
        isStr(kind) &&
        KINDS.has(kind) &&
        isStr(title) &&
        isStr(body)
        ? {
            action: 'memory.edit',
            requestId,
            mogId,
            itemId,
            kind: kind as MemoryKindName,
            title,
            body,
          }
        : undefined;
    default:
      return undefined;
  }
}

const FOLDER = /^[a-z0-9-]{1,72}$/;

/**
 * Read the folders of all MOGs. A MOG that fails to read is left out; the others still show.
 * The list is `unavailable` only if the MOG list fails or every MOG fails.
 */
export async function readMemory(
  memory: Pick<HubMemoryOps, 'listItems' | 'ensureMog'>,
  mogs: () => Promise<MemoryMog[]>,
): Promise<HubMemory> {
  const unavailable = { userItems: 0, folders: [], unavailable: true as const };
  try {
    const defined = await mogs();
    const raw: MemoryFolder[] = [];
    const user = new Set<string>();
    for (const m of defined) {
      try {
        await memory.ensureMog({ mogId: m.mogId, name: m.name, backendKind: m.backendKind });
        const { rows, folder } = await memory.listItems(m.mogId);
        if (folder === undefined || !FOLDER.test(folder)) continue;
        for (const r of rows) {
          if (isObj(r) && r['layer'] === 'user' && isStr(r['itemId'])) user.add(r['itemId']);
        }
        raw.push({
          mogId: m.mogId,
          folder,
          mogName: m.name,
          ...(m.colorVariant !== undefined ? { mogColorVariant: m.colorVariant } : {}),
          rows: rows.filter((r) => !(isObj(r) && r['layer'] === 'user')) as MemoryFolder['rows'],
        });
      } catch {
        // This MOG shows no folder. The others do.
      }
    }
    if (defined.length > 0 && raw.length === 0) return unavailable;
    return asHubMemory({ userItems: user.size, folders: raw }) ?? unavailable;
  } catch {
    return unavailable;
  }
}

/** Wire the hub Memory area. */
export function wireHubMemory(bus: BrainBus, deps: HubMemoryDeps): HubMemoryWiring {
  const send = deps.send ?? ((event, payload) => emitTo('hub', event, payload));
  const out = async (event: string, payload: unknown) => {
    try {
      await send(event, payload);
    } catch {
      // No hub is open. The hub asks again with memory.ready when it opens.
    }
  };
  const publish = async () => {
    await out(HUB_MEMORY_EVENT, await readMemory(deps.memory, deps.mogs));
  };
  const result = (r: MemoryResult) => out(HUB_MEMORY_RESULT_EVENT, r);

  /** One user write. No ticket: no call. Success: a result, then the list again. */
  const write = async (
    requestId: string,
    ticket: string | undefined,
    run: (ticket: string) => Promise<{ itemId: string; status?: string | undefined }>,
  ) => {
    if (ticket === undefined) {
      await result({ requestId, ok: false, code: 'failed' });
      return;
    }
    let done: { itemId: string; status?: string | undefined };
    try {
      done = await run(ticket);
    } catch (e) {
      await result({ requestId, ok: false, code: writeCodeOf(e) });
      return;
    }
    const status =
      done.status === 'active' || done.status === 'quarantined' ? done.status : undefined;
    await result({
      requestId,
      ok: true,
      itemId: done.itemId,
      ...(status !== undefined ? { status } : {}),
    });
    await publish();
  };

  const handle = async (a: MemoryAction, ticket: string | undefined) => {
    switch (a.action) {
      case 'memory.ready':
        await publish();
        return;
      case 'memory.file': {
        const file = await deps.memory.readFile(a.mogId, a.path).catch(() => undefined);
        const view = file && asMemoryFileView({ mogId: a.mogId, path: a.path, text: file.text });
        if (view) await out(HUB_MEMORY_FILE_EVENT, view);
        return;
      }
      case 'memory.item': {
        const item = await deps.memory.readItem(a.mogId, a.itemId).catch(() => undefined);
        if (!item || !KINDS.has(item.kind) || !STATUSES.has(item.status)) return;
        const fields: MemoryItemFields = {
          mogId: a.mogId,
          itemId: a.itemId,
          kind: item.kind as MemoryKindName,
          title: item.title,
          body: item.body,
          status: item.status as MemoryStatusName,
          pinned: item.pinned,
          ...(item.releasedAt !== undefined ? { releasedAt: item.releasedAt } : {}),
        };
        await out(HUB_MEMORY_ITEM_EVENT, fields);
        return;
      }
      case 'memory.add':
        if (len(a.title) > TITLE_MAX || len(a.body) > BODY_MAX || a.body.trim() === '') {
          await result({ requestId: a.requestId, ok: false, code: 'invalid' });
          return;
        }
        await write(a.requestId, ticket, async (t) => {
          // A MOG with no exchange yet has no store row: register it first.
          const m = (await deps.mogs()).find((x) => x.mogId === a.mogId);
          if (m)
            await deps.memory
              .ensureMog({ mogId: m.mogId, name: m.name, backendKind: m.backendKind })
              .catch(() => undefined);
          return deps.memory.pinItem(
            { mogId: a.mogId, itemType: a.kind, origin: 'added', title: a.title, body: a.body },
            t,
          );
        });
        return;
      case 'memory.edit':
        if (
          a.kind === 'preference' ||
          len(a.title) > TITLE_MAX ||
          len(a.body) > BODY_MAX ||
          a.title.trim() === '' ||
          a.body.trim() === ''
        ) {
          await result({ requestId: a.requestId, ok: false, code: 'invalid' });
          return;
        }
        if (a.kind === 'genie_answer') {
          // Only a Genie answer stays one. No other kind becomes one.
          const was = await deps.memory.readItem(a.mogId, a.itemId).catch(() => undefined);
          if (was?.kind !== 'genie_answer') {
            await result({ requestId: a.requestId, ok: false, code: 'invalid' });
            return;
          }
        }
        await write(a.requestId, ticket, async (t) => ({
          itemId: a.itemId,
          ...(await deps.memory.editItem(
            {
              mogId: a.mogId,
              itemId: a.itemId,
              itemType: a.kind,
              title: a.title,
              body: a.body,
            },
            t,
          )),
        }));
        return;
      case 'memory.rate':
        await write(a.requestId, ticket, async (t) => ({
          itemId: a.itemId,
          ...(await deps.memory.rateItem({ mogId: a.mogId, itemId: a.itemId }, t)),
        }));
        return;
      case 'memory.delete':
        try {
          await deps.memory.forget({ mogId: a.mogId, itemIds: [a.itemId] });
        } catch {
          // The row stays. The next list shows it again.
        }
        await publish();
        return;
    }
  };

  const off = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== MEMORY_VIEW_ID || input.kind !== 'click') return;
    const action = asMemoryAction(input.payload);
    if (!action) return;
    void handle(action, input.actionTicket).catch(() => undefined);
  });
  return {
    refresh: publish,
    async push() {
      try {
        if (deps.hubPresent && !(await deps.hubPresent())) return;
      } catch {
        return;
      }
      await publish();
    },
    dispose: off,
  };
}
