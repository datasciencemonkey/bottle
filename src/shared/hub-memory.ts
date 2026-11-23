/**
 * The hub Memory area (local memory plan 9.1, slice 1; ADR 0112): the virtual folder
 * `/memories` in the OpenClaw-style layout.
 * - `/memories/USER.md`: the shared preferences across MOGs.
 * - `/memories/<mog-slug>/MEMORY.md`: the lasting memories of one MOG.
 * - `/memories/<mog-slug>/memory/<YYYY-MM-DD>.md`: the daily notes of one MOG.
 * Each item is one encrypted row; a file is a rendering of rows. `<file>#<slug>` names one item.
 *
 * The brain reads the rows (`mem.item.list`) and the rendered files (`mem.file.get`) from the
 * R10 store and sends them to the hub window only. The hub view never runs a store statement
 * (ARC-2). This file holds names and shape checks only, no state. It is not a contract file,
 * like `hub-conversations.ts`.
 */

/** The brain-to-hub event with the folders and their items. */
export const HUB_MEMORY_EVENT = 'hub:memory';

/** The brain-to-hub event with one rendered file or one item section. */
export const HUB_MEMORY_FILE_EVENT = 'hub:memory.file';

/** The `mogId` of the Memory `view_emit`. It cannot be a MOG id, because MOG ids are UUIDs. */
export const MEMORY_VIEW_ID = 'memory';

/** How long "Memory deleted" with Undo shows before the delete runs (plan 6.2). */
export const MEMORY_UNDO_MS = 8_000;

/** The path of the shared preferences. */
export const USER_FILE_PATH = '/memories/USER.md';

export const MEMORY_KINDS = [
  'fact',
  'definition',
  'decision',
  'open_question',
  'genie_answer',
  'preference',
] as const;
export type MemoryKindName = (typeof MEMORY_KINDS)[number];

export const MEMORY_LAYERS = ['user', 'durable', 'daily'] as const;
export type MemoryLayerName = (typeof MEMORY_LAYERS)[number];

/** The statuses. There is no `proposed`: every write saves at once (owner, 2026-11-21). */
export const MEMORY_STATUSES = [
  'active',
  'stale',
  'superseded',
  'resolved',
  'retracted',
  'quarantined',
  'removed',
] as const;
export type MemoryStatusName = (typeof MEMORY_STATUSES)[number];

export const KIND_LABEL: Readonly<Record<MemoryKindName, string>> = {
  fact: 'Fact',
  definition: 'Definition',
  decision: 'Decision',
  open_question: 'Open question',
  genie_answer: 'Genie answer',
  preference: 'Preference',
};

/** One item, as the list shows it. The body comes only with the file. */
export interface MemoryFileRow {
  itemId: string;
  kind: MemoryKindName;
  layer: MemoryLayerName;
  /** The file of the item, for example `/memories/sales/memory/2026-11-21.md`. */
  file: string;
  /** The item address, `<file>#<slug>`. */
  path: string;
  title: string;
  /** The summary line. */
  summary: string;
  status: MemoryStatusName;
  pinned: boolean;
  /** `you`, `others`, `genie`, `agent` or `confirmed`. */
  source: string;
  /** `pin`, `kept_answer`, … or `legacy`. */
  origin: string;
  tags: string[];
  /** 1 (thumbs up or an edit), 0 (not rated yet) or -1. */
  rating: -1 | 0 | 1;
  /** Epoch ms of the last rating or edit. */
  reviewedAt?: number;
  /** Epoch ms: the user released this item from quarantine ("You released this"). */
  releasedAt?: number;
  /** Epoch ms. */
  createdAt: number;
  /** Epoch ms. */
  updatedAt: number;
}

/** True if the user has not rated or edited the item yet: the hub shows "New". */
export function isNew(r: Pick<MemoryFileRow, 'rating' | 'reviewedAt'>): boolean {
  return r.rating === 0 && r.reviewedAt === undefined;
}

/** One MOG folder. */
export interface MemoryFolder {
  mogId: string;
  /** The folder name in the path (the MOG slug). */
  folder: string;
  mogName: string;
  /** `mog.ui.variant`, so the folder shows the MOG's colour. */
  mogColorVariant?: string;
  rows: MemoryFileRow[];
}

/** What the area shows. `unavailable` is true when the brain could not read the store. */
export interface HubMemory {
  /** The number of shared preferences in USER.md (0 until slice 2). */
  userItems: number;
  folders: MemoryFolder[];
  unavailable?: true;
}

/** One rendered file or item section, as Rust renders it. */
export interface MemoryFileView {
  mogId: string;
  /** The file path, or the item address `<file>#<slug>`. */
  path: string;
  text: string;
}

/**
 * The hub Memory actions:
 * - `memory.ready` asks for the folders (when the area opens or shows again);
 * - `memory.file` asks for one rendered file, or one item when `path` has `#<slug>`;
 * - `memory.rate` is a thumbs up on one item;
 * - `memory.delete` deletes one item. The view sends it only after the Undo time
 *   ({@link MEMORY_UNDO_MS}), so an Undo never needs a new write. A thumbs down takes this path.
 */
export type MemoryAction =
  | { action: 'memory.ready' }
  | { action: 'memory.file'; mogId: string; path: string }
  | { action: 'memory.item'; mogId: string; itemId: string }
  | {
      action: 'memory.add';
      requestId: string;
      mogId: string;
      kind: EditableKind;
      title: string;
      body: string;
    }
  | {
      action: 'memory.edit';
      requestId: string;
      mogId: string;
      itemId: string;
      kind: MemoryKindName;
      title: string;
      body: string;
    }
  | { action: 'memory.rate'; requestId: string; mogId: string; itemId: string }
  | { action: 'memory.delete'; mogId: string; itemId: string };

/** The caps of a user write (ADR 0113, migration 0003). */
export const TITLE_MAX = 80;
export const BODY_MAX = 4_000;

/** The kinds a user can pick when they add or edit an item. A Genie answer stays one. */
export const EDITABLE_KINDS = ['fact', 'definition', 'decision', 'open_question'] as const;
export type EditableKind = (typeof EDITABLE_KINDS)[number];

/** Brain to hub: the raw fields of one item, for the editor (`memory.item`). */
export const HUB_MEMORY_ITEM_EVENT = 'hub:memory.item';
export interface MemoryItemFields {
  mogId: string;
  itemId: string;
  kind: MemoryKindName;
  title: string;
  body: string;
  status: MemoryStatusName;
  pinned: boolean;
  releasedAt?: number;
}

/** Brain to hub: the result of one add, edit or rating, by `requestId`. Content-free. */
export const HUB_MEMORY_RESULT_EVENT = 'hub:memory.result';
export interface MemoryResult {
  requestId: string;
  ok: boolean;
  code?: 'secret_refused' | 'invalid' | 'failed';
  itemId?: string;
  status?: 'active' | 'quarantined';
}

/** The files of one MOG folder: MEMORY.md first, then the daily notes, newest first. */
export interface MemoryFileEntry {
  path: string;
  /** `MEMORY.md` or the day `YYYY-MM-DD`. */
  label: string;
  layer: 'durable' | 'daily';
  items: number;
}

/** The files of `f`, from its rows. MEMORY.md is always there, also when it is empty. */
export function filesOf(f: MemoryFolder): MemoryFileEntry[] {
  const memory = `/memories/${f.folder}/MEMORY.md`;
  const days = new Map<string, number>();
  let durable = 0;
  for (const r of f.rows) {
    if (r.layer === 'durable') durable += 1;
    else if (r.layer === 'daily') days.set(r.file, (days.get(r.file) ?? 0) + 1);
  }
  const daily = [...days.entries()]
    .sort(([a], [b]) => (a < b ? 1 : a > b ? -1 : 0))
    .map(([path, items]) => ({
      path,
      label: path.slice(path.lastIndexOf('/') + 1, -'.md'.length),
      layer: 'daily' as const,
      items,
    }));
  return [{ path: memory, label: 'MEMORY.md', layer: 'durable', items: durable }, ...daily];
}

const KINDS: ReadonlySet<string> = new Set(MEMORY_KINDS);
const LAYERS: ReadonlySet<string> = new Set(MEMORY_LAYERS);
const STATUSES: ReadonlySet<string> = new Set(MEMORY_STATUSES);
const FILE = /^\/memories\/(?:USER\.md|[a-z0-9-]+\/(?:MEMORY\.md|memory\/\d{4}-\d{2}-\d{2}\.md))$/;

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

const isStr = (v: unknown): v is string => typeof v === 'string';
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

function asRow(v: unknown): MemoryFileRow | undefined {
  if (!isObj(v)) return undefined;
  const { itemId, kind, layer, file, path, title, summary, status, pinned, source, origin } = v;
  const { tags, rating } = v;
  if (
    !isStr(itemId) ||
    !isStr(kind) ||
    !KINDS.has(kind) ||
    !isStr(layer) ||
    !LAYERS.has(layer) ||
    !isStr(file) ||
    !FILE.test(file) ||
    !isStr(path) ||
    !path.startsWith(`${file}#`) ||
    !isStr(title) ||
    !isStr(summary) ||
    !isStr(status) ||
    !STATUSES.has(status) ||
    typeof pinned !== 'boolean' ||
    !isStr(source) ||
    !isStr(origin) ||
    !Array.isArray(tags) ||
    !tags.every(isStr) ||
    (rating !== -1 && rating !== 0 && rating !== 1) ||
    !isNum(v.createdAt) ||
    !isNum(v.updatedAt)
  )
    return undefined;
  return {
    itemId,
    kind: kind as MemoryKindName,
    layer: layer as MemoryLayerName,
    file,
    path,
    title,
    summary,
    status: status as MemoryStatusName,
    pinned,
    source,
    origin,
    tags: tags as string[],
    rating,
    ...(isNum(v.reviewedAt) ? { reviewedAt: v.reviewedAt } : {}),
    ...(isNum(v.releasedAt) ? { releasedAt: v.releasedAt } : {}),
    createdAt: v.createdAt,
    updatedAt: v.updatedAt,
  };
}

/** Check a `hub:memory` payload. Bad rows are dropped; a bad payload is `undefined`. */
export function asHubMemory(p: unknown): HubMemory | undefined {
  if (!isObj(p) || !Array.isArray(p.folders)) return undefined;
  const folders: MemoryFolder[] = [];
  for (const f of p.folders) {
    if (!isObj(f) || !isStr(f.mogId) || !isStr(f.folder) || !isStr(f.mogName)) continue;
    if (!/^[a-z0-9-]+$/.test(f.folder) || !Array.isArray(f.rows)) continue;
    const prefix = `/memories/${f.folder}/`;
    folders.push({
      mogId: f.mogId,
      folder: f.folder,
      mogName: f.mogName,
      ...(isStr(f.mogColorVariant) ? { mogColorVariant: f.mogColorVariant } : {}),
      rows: f.rows
        .map(asRow)
        .filter((r): r is MemoryFileRow => r !== undefined && r.file.startsWith(prefix)),
    });
  }
  return {
    userItems: isNum(p.userItems) ? p.userItems : 0,
    folders,
    ...(p.unavailable === true ? { unavailable: true as const } : {}),
  };
}

/** Check a `hub:memory.file` payload. */
export function asMemoryFileView(p: unknown): MemoryFileView | undefined {
  if (!isObj(p)) return undefined;
  const { mogId, path, text } = p;
  if (!isStr(mogId) || !isStr(path) || !path.startsWith('/memories/') || !isStr(text))
    return undefined;
  return { mogId, path, text };
}

/**
 * The frontmatter and the body of a rendered file. The hidden metadata lines of the items
 * (`<!-- memory … -->`) leave the body: they are for an import, not for the eye.
 */
export function splitFile(text: string): { fields: [string, string][]; body: string } {
  const m = /^---\n([\s\S]*?)\n---\n?([\s\S]*)$/.exec(text);
  const strip = (b: string) => b.replace(/^<!-- memory [^\n]*-->\n?/gm, '');
  if (!m) return { fields: [], body: strip(text) };
  const fields: [string, string][] = [];
  for (const line of (m[1] ?? '').split('\n')) {
    const at = line.indexOf(':');
    if (at <= 0) continue;
    const key = line.slice(0, at).trim();
    let value = line.slice(at + 1).trim();
    // The renderer writes strings as JSON strings (valid YAML).
    if (value.startsWith('"')) {
      try {
        const parsed: unknown = JSON.parse(value);
        if (typeof parsed === 'string') value = parsed;
      } catch {
        // Keep the raw text.
      }
    }
    fields.push([key, value]);
  }
  return { fields, body: strip(m[2] ?? '') };
}
