/**
 * The hub Memory area (local memory plan 9.1, slice 1; ADR 0112): the memories of each MOG as
 * files in the virtual folder `/memories/<mog-slug>/<kind-folder>/<slug>.md`.
 *
 * The brain reads the rows (`mem.item.list`) and the rendered files (`mem.index.get`) from the
 * R10 store and sends them to the hub window only. The hub view never runs a store statement
 * (ARC-2): it renders what the brain sends and sends user input back with `view_emit`.
 * This file holds names and shape checks only, no state. It is not a contract file, like
 * `hub-conversations.ts`.
 */

/** The brain-to-hub event with the folders and their memories. */
export const HUB_MEMORY_EVENT = 'hub:memory';

/** The brain-to-hub event with one rendered memory file. */
export const HUB_MEMORY_FILE_EVENT = 'hub:memory.file';

/** The `mogId` of the Memory `view_emit`. It cannot be a MOG id, because MOG ids are UUIDs. */
export const MEMORY_VIEW_ID = 'memory';

/** How long "Memory deleted" with Undo shows before the delete runs (plan 6.2). */
export const MEMORY_UNDO_MS = 8_000;

/** The memory kinds (plan 5.1). There is no `person` kind (owner, Q7). */
export const MEMORY_KINDS = [
  'fact',
  'definition',
  'decision',
  'open_question',
  'genie_answer',
  'preference',
] as const;
export type MemoryKindName = (typeof MEMORY_KINDS)[number];

/** The kind folder of each kind, as in the path. */
export const KIND_FOLDER: Readonly<Record<MemoryKindName, string>> = {
  fact: 'facts',
  definition: 'definitions',
  decision: 'decisions',
  open_question: 'open-questions',
  genie_answer: 'answers',
  preference: 'preferences',
};

/** The label of each kind folder in the tree. */
export const KIND_LABEL: Readonly<Record<MemoryKindName, string>> = {
  fact: 'Facts',
  definition: 'Definitions',
  decision: 'Decisions',
  open_question: 'Open questions',
  genie_answer: 'Answers',
  preference: 'Preferences',
};

export const MEMORY_STATUSES = [
  'proposed',
  'active',
  'stale',
  'superseded',
  'resolved',
  'retracted',
  'quarantined',
  'removed',
] as const;
export type MemoryStatusName = (typeof MEMORY_STATUSES)[number];

/** One memory, as the list shows it. The body comes only with the file. */
export interface MemoryFileRow {
  itemId: string;
  kind: MemoryKindName;
  /** The full path, `/memories/<folder>/<kind-folder>/<slug>.md`. */
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
  /** Epoch ms. */
  createdAt: number;
  /** Epoch ms. */
  updatedAt: number;
  /** The Genie exchange of a kept answer, if any (its request id), for a link to Conversations. */
  requestId?: string;
}

/** One MOG folder. */
export interface MemoryFolder {
  mogId: string;
  /** The folder name in the path (the MOG slug). */
  folder: string;
  mogName: string;
  /** `mog.ui.variant`, so the folder shows the MOG's colour. */
  mogColorVariant?: string;
  /** Pinned first, then by the last use (the order of `mem.item.list`). */
  rows: MemoryFileRow[];
}

/** What the area shows. `unavailable` is true when the brain could not read the store. */
export interface HubMemory {
  folders: MemoryFolder[];
  unavailable?: true;
}

/** One rendered file: the frontmatter and the body, as Rust renders them. */
export interface MemoryFileView {
  mogId: string;
  itemId: string;
  path: string;
  /** The whole file text. */
  text: string;
}

/**
 * The hub Memory actions:
 * - `memory.ready` asks for the folders (when the area opens or shows again);
 * - `memory.file` asks for the rendered file of one memory;
 * - `memory.delete` deletes one memory. The view sends it only after the confirm click and the
 *   Undo time ([`MEMORY_UNDO_MS`]), so an Undo never needs a new write.
 */
export type MemoryAction =
  | { action: 'memory.ready' }
  | { action: 'memory.file'; mogId: string; itemId: string }
  | { action: 'memory.delete'; mogId: string; itemId: string };

const KINDS: ReadonlySet<string> = new Set(MEMORY_KINDS);
const STATUSES: ReadonlySet<string> = new Set(MEMORY_STATUSES);

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

const isStr = (v: unknown): v is string => typeof v === 'string';
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

function asRow(v: unknown): MemoryFileRow | undefined {
  if (!isObj(v)) return undefined;
  const { itemId, kind, path, title, summary, status, pinned, source, origin, tags } = v;
  if (
    !isStr(itemId) ||
    !isStr(kind) ||
    !KINDS.has(kind) ||
    !isStr(path) ||
    !path.startsWith('/memories/') ||
    !isStr(title) ||
    !isStr(summary) ||
    !isStr(status) ||
    !STATUSES.has(status) ||
    typeof pinned !== 'boolean' ||
    !isStr(source) ||
    !isStr(origin) ||
    !Array.isArray(tags) ||
    !tags.every(isStr) ||
    !isNum(v.createdAt) ||
    !isNum(v.updatedAt)
  )
    return undefined;
  return {
    itemId,
    kind: kind as MemoryKindName,
    path,
    title,
    summary,
    status: status as MemoryStatusName,
    pinned,
    source,
    origin,
    tags: tags as string[],
    createdAt: v.createdAt,
    updatedAt: v.updatedAt,
    ...(isStr(v.requestId) ? { requestId: v.requestId } : {}),
  };
}

/** Check a `hub:memory` payload. Bad rows are dropped; a bad payload is `undefined`. */
export function asHubMemory(p: unknown): HubMemory | undefined {
  if (!isObj(p) || !Array.isArray(p.folders)) return undefined;
  const folders: MemoryFolder[] = [];
  for (const f of p.folders) {
    if (!isObj(f) || !isStr(f.mogId) || !isStr(f.folder) || !isStr(f.mogName)) continue;
    if (!Array.isArray(f.rows)) continue;
    folders.push({
      mogId: f.mogId,
      folder: f.folder,
      mogName: f.mogName,
      ...(isStr(f.mogColorVariant) ? { mogColorVariant: f.mogColorVariant } : {}),
      rows: f.rows.map(asRow).filter((r): r is MemoryFileRow => r !== undefined),
    });
  }
  return { folders, ...(p.unavailable === true ? { unavailable: true as const } : {}) };
}

/** Check a `hub:memory.file` payload. */
export function asMemoryFileView(p: unknown): MemoryFileView | undefined {
  if (!isObj(p)) return undefined;
  const { mogId, itemId, path, text } = p;
  if (!isStr(mogId) || !isStr(itemId) || !isStr(path) || !isStr(text)) return undefined;
  return { mogId, itemId, path, text };
}

/** The frontmatter and the body of a rendered file. A file with no frontmatter is all body. */
export function splitFile(text: string): { fields: [string, string][]; body: string } {
  const m = /^---\n([\s\S]*?)\n---\n?([\s\S]*)$/.exec(text);
  if (!m) return { fields: [], body: text };
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
  return { fields, body: m[2] ?? '' };
}
