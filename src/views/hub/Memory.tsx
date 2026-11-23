/**
 * The hub Memory area (local memory plan 9.1, slice 1; ADR 0112). Thin view (ARC-2): it renders
 * what the brain sends on `hub:memory` and `hub:memory.file`, and it runs no store statement.
 * - The tree: USER.md, then for each MOG its MEMORY.md and its daily notes, newest first.
 * - The list: the items of the selected file. A new (not yet rated) item has a "New" badge; a
 *   quarantined one has a mark.
 * - The view: the whole rendered file, or the selected item's section, as Rust renders it.
 * - Feedback: thumbs up (`memory.rate`); thumbs down deletes. A thumbs up on a quarantined item
 *   releases it (owner ruling 2026-11-21): the host checks a ticket of this click, so only the
 *   user can do it. The inline Release button sends the same click.
 * - Delete and thumbs down have no confirm step. The item hides at once and "Memory deleted"
 *   shows with Undo for 8 s. The view sends `memory.delete` only when the 8 s end, so Undo needs
 *   no new write. If the area goes away in the 8 s, the delete goes at once: the user asked for it.
 * - Add (S3) and edit (S4): `MemoryForm`. A write is a `requestId` request; the answer is
 *   `hub:memory.result`. A secret keeps the text in the form. A saved add or edit shows Undo for 8 s:
 *   undo of an add sends `memory.delete`, undo of an edit sends `memory.edit` with the old fields.
 * The view reaches the brain only through a {@link MemorySource}; tests pass a local one.
 */
import { listen } from '@tauri-apps/api/event';
import { motion, useReducedMotion } from 'motion/react';
import {
  CalendarDaysIcon,
  FileTextIcon,
  PencilIcon,
  PinIcon,
  PlusIcon,
  ShieldAlertIcon,
  ThumbsDownIcon,
  ThumbsUpIcon,
  UserIcon,
} from 'lucide-react';
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import {
  asHubMemory,
  asMemoryFileView,
  filesOf,
  HUB_MEMORY_EVENT,
  HUB_MEMORY_FILE_EVENT,
  HUB_MEMORY_ITEM_EVENT,
  HUB_MEMORY_RESULT_EVENT,
  isNew,
  KIND_LABEL,
  MEMORY_UNDO_MS,
  MEMORY_VIEW_ID,
  splitFile,
  USER_FILE_PATH,
  type EditableKind,
  type HubMemory,
  type MemoryAction,
  type MemoryFileRow,
  type MemoryFileView,
  type MemoryItemFields,
  type MemoryKindName,
  type MemoryResult,
} from '../../shared/hub-memory';
import { NO_LINKS } from '../../shared/link-trust';
import { paletteColorOf } from '../../shared/mog-palette';
import { SafeMarkdown } from '../card/safe-markdown';
import { FORGET_OPEN_TARGET } from '../../shared/forget-view';
import { Button } from '../ui/button';
import { emitView } from '../view-ipc';
import { MemoryForm, type MemoryFormValues } from './MemoryForm';

/** The seam between the view and the brain. */
export interface MemorySource {
  /**
   * Listen for the folders, the files, the raw fields of one item and the result of a write.
   * Returns the stop function.
   */
  subscribe(handlers: MemoryHandlers): () => void;
  /** Send one user action to the brain. */
  send(a: MemoryAction): void;
}

export interface MemoryHandlers {
  onList: (m: HubMemory) => void;
  onFile: (f: MemoryFileView) => void;
  onItem: (i: MemoryItemFields) => void;
  onResult: (r: MemoryResult) => void;
}

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

/** Check a `hub:memory.item` payload. */
export function asMemoryItem(p: unknown): MemoryItemFields | undefined {
  if (!isObj(p)) return undefined;
  const { mogId, itemId, kind, title, body, status, pinned, releasedAt } = p;
  if (
    typeof mogId !== 'string' ||
    typeof itemId !== 'string' ||
    typeof kind !== 'string' ||
    !(kind in KIND_LABEL) ||
    typeof title !== 'string' ||
    typeof body !== 'string' ||
    typeof status !== 'string' ||
    typeof pinned !== 'boolean'
  )
    return undefined;
  return {
    mogId,
    itemId,
    kind: kind as MemoryKindName,
    title,
    body,
    status: status as MemoryItemFields['status'],
    pinned,
    ...(typeof releasedAt === 'number' ? { releasedAt } : {}),
  };
}

/** Check a `hub:memory.result` payload. */
export function asMemoryResult(p: unknown): MemoryResult | undefined {
  if (!isObj(p) || typeof p['requestId'] !== 'string' || typeof p['ok'] !== 'boolean')
    return undefined;
  const { code, itemId, status } = p;
  return {
    requestId: p['requestId'],
    ok: p['ok'],
    ...(code === 'secret_refused' || code === 'invalid' || code === 'failed' ? { code } : {}),
    ...(typeof itemId === 'string' ? { itemId } : {}),
    ...(status === 'active' || status === 'quarantined' ? { status } : {}),
  };
}

/** The default source: `hub:memory` events in, `view_emit` clicks out. */
export const ipcMemorySource: MemorySource = {
  subscribe(h) {
    const stops: (() => void)[] = [];
    let stopped = false;
    const keep = (p: Promise<() => void>) =>
      void p.then((stop) => (stopped ? stop() : stops.push(stop))).catch(() => undefined);
    keep(
      listen(HUB_MEMORY_EVENT, (e) => {
        const m = asHubMemory(e.payload);
        if (m) h.onList(m);
      }),
    );
    keep(
      listen(HUB_MEMORY_FILE_EVENT, (e) => {
        const f = asMemoryFileView(e.payload);
        if (f) h.onFile(f);
      }),
    );
    keep(
      listen(HUB_MEMORY_ITEM_EVENT, (e) => {
        const i = asMemoryItem(e.payload);
        if (i) h.onItem(i);
      }),
    );
    keep(
      listen(HUB_MEMORY_RESULT_EVENT, (e) => {
        const r = asMemoryResult(e.payload);
        if (r) h.onResult(r);
      }),
    );
    return () => {
      stopped = true;
      for (const s of stops.splice(0)) s();
    };
  },
  send(a) {
    void emitView(MEMORY_VIEW_ID, 'click', a).catch(() => undefined);
  },
};

/** One tree entry: USER.md, or one file of one MOG. */
interface TreeFile {
  /** `''` for USER.md (shared). */
  mogId: string;
  path: string;
}

interface Pending {
  mogId: string;
  itemId: string;
  timer: ReturnType<typeof setTimeout>;
}

/** The one Undo toast. A delete also has a {@link Pending}; an add or an edit has only `undo`. */
interface Toast {
  key: number;
  text: string;
  undo?: () => void;
}

export const SECRET_MESSAGE = 'Not saved: this looks like a key or password.';
/** A write that gets no answer from the brain ends after this time. */
export const WRITE_TIMEOUT_MS = 15_000;
export const QUARANTINE_MESSAGE =
  'Genie and the agent will not see this, because it reads like an instruction.';
const FAILED_MESSAGE = 'Bottle could not save this. Try again.';

const SOURCE_LABEL: Readonly<Record<string, string>> = {
  you: 'You',
  others: 'Others in a call',
  genie: 'Genie',
  agent: 'Agent',
  confirmed: 'You (confirmed)',
};

const dateOf = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    day: 'numeric',
    month: 'short',
    hour: 'numeric',
    minute: '2-digit',
  });

/** "Today", "Yesterday" or the date of a daily note, from its `YYYY-MM-DD` label. */
function dayLabel(day: string, now: number): string {
  const [y, m, d] = day.split('-').map(Number);
  if (!y || !m || !d) return day;
  const t = new Date(y, m - 1, d).getTime();
  const n = new Date(now);
  const today = new Date(n.getFullYear(), n.getMonth(), n.getDate()).getTime();
  const diff = Math.round((today - t) / 86_400_000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  return new Date(t).toLocaleDateString(undefined, {
    day: 'numeric',
    month: 'short',
    ...(new Date(t).getFullYear() === n.getFullYear() ? {} : { year: 'numeric' }),
  });
}

function Marks({ row, pinned = false }: { row: MemoryFileRow; pinned?: boolean }) {
  return (
    <>
      {pinned && row.pinned && (
        <span className="mem-mark" data-mark="pinned">
          <PinIcon className="size-3" aria-hidden="true" />
          Pinned
        </span>
      )}
      {isNew(row) && (
        <span className="mem-mark mem-mark--new" data-mark="new">
          New
        </span>
      )}
      {row.status === 'quarantined' && (
        <span className="mem-mark mem-mark--quarantined" data-mark="quarantined">
          <ShieldAlertIcon className="size-3" aria-hidden="true" />
          Quarantined
        </span>
      )}
      {row.status === 'stale' && (
        <span className="mem-mark" data-mark="stale">
          Not used lately
        </span>
      )}
      {row.releasedAt !== undefined && (
        <span className="mem-mark" data-mark="released">
          You released this
        </span>
      )}
    </>
  );
}

/** The item section without its `## Title` line: the title shows in the heading. */
const withoutHeading = (body: string) => body.replace(/^\s*## [^\n]*\n?/, '');

/** The pane heading of a whole file: `MEMORY.md`, or the day of a daily note. */
function fileLabel(path: string | undefined, nowMs: number): string {
  const name = path?.slice(path.lastIndexOf('/') + 1) ?? '';
  return name.endsWith('.md') && /^\d{4}-/.test(name) ? dayLabel(name.slice(0, -3), nowMs) : name;
}

const fileDomId = (path: string) => `mem-file-${path}`;
const itemDomId = (id: string) => `mem-item-${id}`;

const sameFile = (a: TreeFile | undefined, b: TreeFile) =>
  a !== undefined && a.mogId === b.mogId && a.path === b.path;

export interface MemoryAreaProps {
  /** The area shows. Each time it shows, it asks for the folders again. */
  active: boolean;
  source?: MemorySource;
  /** The clock for the day labels. Tests pass a fixed one. */
  now?: () => number;
  /** The MOG the "Forget memory…" button asks the brain to forget (MEM-32a). None: no button. */
  forgetMogId?: string | undefined;
}

/** What the right pane shows instead of the file: the add form or the edit form. */
type Compose =
  | { mode: 'add'; mogId: string }
  | { mode: 'edit'; mogId: string; itemId: string; from: MemoryItemFields };

let toastKey = 0;

export function MemoryArea({
  active,
  source = ipcMemorySource,
  now = Date.now,
  forgetMogId,
}: MemoryAreaProps) {
  const reduce = useReducedMotion();
  const [model, setModel] = useState<HubMemory | undefined>(undefined);
  const [view, setView] = useState<MemoryFileView | undefined>(undefined);
  const [fields, setFields] = useState<MemoryItemFields | undefined>(undefined);
  const [picked, setPicked] = useState<TreeFile | undefined>(undefined);
  const [pickedItem, setItemId] = useState<string | undefined>(undefined);
  const [pending, setPending] = useState<Pending | undefined>(undefined);
  /** Local changes on top of the model, so a click shows at once (rate, release, edit). */
  const [patch, setPatch] = useState<Readonly<Record<string, Partial<MemoryFileRow>>>>({});
  /** Items the user removed (the Undo of an add). */
  const [gone, setGone] = useState<ReadonlySet<string>>(new Set());
  const [compose, setCompose] = useState<Compose | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | undefined>(undefined);
  const [toast, setToast] = useState<Toast | undefined>(undefined);
  /** The new item to select once the list has it. */
  const [want, setWant] = useState<string | undefined>(undefined);
  const pendingRef = useRef<Pending | undefined>(undefined);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const requests = useRef(new Map<string, (r: MemoryResult) => void>());
  /** The request the form waits for. A result of any other request never touches the form. */
  const activeReq = useRef<string | undefined>(undefined);
  const busyRef = useRef(false);
  const writeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const listRef = useRef<HTMLDivElement>(null);
  const treeRef = useRef<HTMLUListElement>(null);
  useEffect(() => {
    pendingRef.current = pending;
  }, [pending]);

  useEffect(
    () =>
      source.subscribe({
        onList: (m) => {
          // The brain's list is the truth again: local changes on top of it end.
          setPatch({});
          setModel(m);
        },
        onFile: setView,
        onItem: setFields,
        onResult: (r) => {
          const done = requests.current.get(r.requestId);
          requests.current.delete(r.requestId);
          done?.(r);
        },
      }),
    [source],
  );

  useEffect(() => {
    if (active) source.send({ action: 'memory.ready' });
  }, [active, source]);

  // The delete goes when the area goes away or the window closes: the user asked for it.
  useEffect(() => {
    const flush = () => {
      const p = pendingRef.current;
      if (!p) return;
      pendingRef.current = undefined;
      clearTimeout(p.timer);
      source.send({ action: 'memory.delete', mogId: p.mogId, itemId: p.itemId });
    };
    window.addEventListener('pagehide', flush);
    return () => {
      window.removeEventListener('pagehide', flush);
      clearTimeout(toastTimer.current);
      clearTimeout(writeTimer.current);
      flush();
    };
  }, [source]);

  // One toast at a time. A new toast ends a pending delete first (it goes at once), so the
  // "Memory deleted" toast never hides an add or edit Undo.
  const showToast = useCallback(
    (text: string, undo?: () => void) => {
      const p = pendingRef.current;
      if (p) {
        pendingRef.current = undefined;
        clearTimeout(p.timer);
        source.send({ action: 'memory.delete', mogId: p.mogId, itemId: p.itemId });
        setPending(undefined);
      }
      clearTimeout(toastTimer.current);
      toastKey += 1;
      setToast({ key: toastKey, text, ...(undo ? { undo } : {}) });
      toastTimer.current = setTimeout(() => setToast(undefined), MEMORY_UNDO_MS);
    },
    [source],
  );

  // The rows the view shows: a pending delete hides its row; local changes show at once.
  const folders = useMemo(
    () =>
      (model?.folders ?? []).map((f) => ({
        ...f,
        rows: f.rows
          .filter((r) => r.itemId !== pending?.itemId && !gone.has(r.itemId))
          .map((r) => (patch[r.itemId] ? { ...r, ...patch[r.itemId] } : r)),
      })),
    [model, pending, patch, gone],
  );

  // USER.md shows only when it has items (S8).
  const showUser = (model?.userItems ?? 0) > 0;
  const tree = useMemo<TreeFile[]>(
    () => [
      ...(showUser ? [{ mogId: '', path: USER_FILE_PATH }] : []),
      ...folders.flatMap((f) => filesOf(f).map((e) => ({ mogId: f.mogId, path: e.path }))),
    ],
    [folders, showUser],
  );

  // A new item is selected as soon as the list has it.
  const wantRow = useMemo(
    () =>
      want
        ? folders
            .flatMap((f) => f.rows.map((row) => ({ mogId: f.mogId, row })))
            .find((x) => x.row.itemId === want)
        : undefined,
    [want, folders],
  );
  const itemId = wantRow ? wantRow.row.itemId : pickedItem;

  // Until the user picks a file: the newest daily note of the first MOG, else its MEMORY.md.
  const sel = useMemo<TreeFile | undefined>(() => {
    if (wantRow) return { mogId: wantRow.mogId, path: wantRow.row.file };
    if (picked && tree.some((t) => sameFile(picked, t))) return picked;
    const first = folders[0];
    if (!first) return tree[0];
    const files = filesOf(first);
    const entry = files.find((e) => e.layer === 'daily') ?? files[0];
    return entry ? { mogId: first.mogId, path: entry.path } : tree[0];
  }, [picked, tree, folders, wantRow]);

  const rows = useMemo(
    () =>
      sel?.mogId
        ? (folders.find((f) => f.mogId === sel.mogId)?.rows ?? []).filter(
            (r) => r.file === sel.path,
          )
        : [],
    [folders, sel],
  );
  const row = rows.find((r) => r.itemId === itemId);
  const viewPath = row ? row.path : sel?.path;
  const itemFields = row && fields?.itemId === row.itemId ? fields : undefined;
  const releasedAt = row?.releasedAt ?? itemFields?.releasedAt;

  // Ask for the rendered file or item each time the selection changes.
  useEffect(() => {
    if (!active || !sel || !viewPath) return;
    source.send({ action: 'memory.file', mogId: sel.mogId, path: viewPath });
  }, [active, sel, viewPath, source]);

  // Ask for the raw fields of the selected item (the editor and the Released mark).
  const rowId = row?.itemId;
  useEffect(() => {
    if (!active || !sel?.mogId || !rowId) return;
    source.send({ action: 'memory.item', mogId: sel.mogId, itemId: rowId });
  }, [active, sel?.mogId, rowId, source]);

  const pickFile = useCallback((t: TreeFile) => {
    setWant(undefined);
    setPicked(t);
    setItemId(undefined);
    setCompose(undefined);
  }, []);

  const pickRow = useCallback((r: MemoryFileRow) => {
    setWant(undefined);
    setItemId(r.itemId);
    setCompose(undefined);
  }, []);

  const request = useCallback((done: (r: MemoryResult) => void) => {
    const requestId = crypto.randomUUID();
    requests.current.set(requestId, done);
    return requestId;
  }, []);

  /** A write the form waits for. `done(r, live)`: `live` is false when the form moved on. */
  const formWrite = useCallback(
    (done: (r: MemoryResult, live: boolean) => void) => {
      const id = request((r) => {
        const live = activeReq.current === id;
        if (live) {
          activeReq.current = undefined;
          busyRef.current = false;
          clearTimeout(writeTimer.current);
          setBusy(false);
        }
        done(r, live);
      });
      activeReq.current = id;
      clearTimeout(writeTimer.current);
      writeTimer.current = setTimeout(() => {
        if (activeReq.current !== id) return;
        activeReq.current = undefined;
        busyRef.current = false;
        setBusy(false);
        setFormError(FAILED_MESSAGE);
      }, WRITE_TIMEOUT_MS);
      return id;
    },
    [request],
  );

  const startDelete = useCallback(() => {
    if (!sel?.mogId || !row) return;
    // A second delete sends the first one at once.
    const prev = pendingRef.current;
    if (prev) {
      clearTimeout(prev.timer);
      source.send({ action: 'memory.delete', mogId: prev.mogId, itemId: prev.itemId });
    }
    const p: Pending = {
      mogId: sel.mogId,
      itemId: row.itemId,
      timer: setTimeout(() => {
        source.send({ action: 'memory.delete', mogId: p.mogId, itemId: p.itemId });
        setPending((cur) => (cur?.itemId === p.itemId ? undefined : cur));
      }, MEMORY_UNDO_MS),
    };
    clearTimeout(toastTimer.current);
    setToast(undefined);
    setPending(p);
    setCompose(undefined);
    setItemId(undefined);
    // The row is gone: the keyboard stays in the list.
    listRef.current?.focus();
  }, [sel, row, source]);

  const undoDelete = useCallback(() => {
    const p = pendingRef.current;
    if (!p) return;
    clearTimeout(p.timer);
    setPending(undefined);
  }, []);

  const rate = useCallback(() => {
    if (!sel?.mogId || !row) return;
    const id = row.itemId;
    const mogId = sel.mogId;
    const requestId = request((r) => {
      if (r.ok) {
        source.send({ action: 'memory.ready' });
        return;
      }
      // The brain refused: the local change goes and the user sees why.
      setPatch((p) => {
        return Object.fromEntries(Object.entries(p).filter(([k]) => k !== id));
      });
      showToast(FAILED_MESSAGE);
    });
    source.send({ action: 'memory.rate', requestId, mogId, itemId: id });
    setPatch((p) => ({
      ...p,
      [id]: {
        ...p[id],
        rating: 1,
        reviewedAt: 0,
        status: row.status === 'quarantined' ? 'active' : row.status,
        ...(row.status === 'quarantined' ? { releasedAt: now() } : {}),
      },
    }));
  }, [sel, row, source, now, request, showToast]);

  const resetForm = useCallback(() => {
    activeReq.current = undefined;
    busyRef.current = false;
    clearTimeout(writeTimer.current);
    setFormError(undefined);
    setBusy(false);
  }, []);

  const openAdd = useCallback(
    (mogId: string) => {
      resetForm();
      setCompose({ mode: 'add', mogId });
    },
    [resetForm],
  );

  const openEdit = useCallback(() => {
    if (!sel?.mogId || !row || !itemFields) return;
    resetForm();
    setCompose({ mode: 'edit', mogId: sel.mogId, itemId: row.itemId, from: itemFields });
  }, [sel, row, itemFields, resetForm]);

  const closeForm = useCallback(() => {
    resetForm();
    setCompose(undefined);
  }, [resetForm]);

  const failure = (code: MemoryResult['code']) =>
    code === 'secret_refused' ? SECRET_MESSAGE : FAILED_MESSAGE;

  const submit = useCallback(
    (v: MemoryFormValues) => {
      if (!compose || busyRef.current) return;
      busyRef.current = true;
      setBusy(true);
      setFormError(undefined);
      if (compose.mode === 'add') {
        const { mogId } = compose;
        const requestId = formWrite((r, live) => {
          if (!r.ok || !r.itemId) {
            if (live) setFormError(failure(r.code));
            return;
          }
          const itemId = r.itemId;
          if (live) {
            setCompose(undefined);
            setWant(itemId);
          }
          source.send({ action: 'memory.ready' });
          // Even when the form moved on, the memory was saved: the Undo stays.
          showToast(
            r.status === 'quarantined' ? 'Memory saved, but hidden from Genie' : 'Memory saved',
            () => {
              source.send({ action: 'memory.delete', mogId, itemId });
              setGone((g) => new Set([...g, itemId]));
              setWant(undefined);
              setItemId((cur) => (cur === itemId ? undefined : cur));
              setToast(undefined);
            },
          );
        });
        source.send({
          action: 'memory.add',
          requestId,
          mogId,
          kind: v.kind as EditableKind,
          title: v.title,
          body: v.body,
        });
        return;
      }
      const { mogId, itemId, from } = compose;
      const filePath = row?.path ?? sel?.path ?? '';
      const requestId = formWrite((r, live) => {
        if (!r.ok) {
          if (live) setFormError(failure(r.code));
          return;
        }
        const status = r.status ?? 'active';
        if (live) setCompose(undefined);
        setFields({ ...from, kind: v.kind, title: v.title, body: v.body, status });
        setPatch((p) => ({
          ...p,
          [itemId]: {
            ...p[itemId],
            title: v.title || p[itemId]?.title || from.title,
            kind: v.kind,
            status,
            rating: 1,
            reviewedAt: 0,
          },
        }));
        source.send({ action: 'memory.ready' });
        source.send({ action: 'memory.file', mogId, path: filePath });
        showToast('Memory updated', () => {
          // Undo is a new edit click, so it has its own request.
          const undoId = request((u) => {
            if (u.ok) {
              source.send({ action: 'memory.ready' });
              source.send({ action: 'memory.item', mogId, itemId });
              return;
            }
            showToast('Could not undo. Try again.');
            source.send({ action: 'memory.item', mogId, itemId });
          });
          source.send({
            action: 'memory.edit',
            requestId: undoId,
            mogId,
            itemId,
            kind: from.kind,
            title: from.title,
            body: from.body,
          });
          setFields({ ...from });
          setPatch((p) => ({
            ...p,
            [itemId]: { ...p[itemId], title: from.title, kind: from.kind, status: from.status },
          }));
          setToast(undefined);
        });
      });
      source.send({
        action: 'memory.edit',
        requestId,
        mogId,
        itemId,
        kind: v.kind,
        title: v.title,
        body: v.body,
      });
    },
    [compose, formWrite, request, source, showToast, row?.path, sel?.path],
  );

  const onTreeKey = useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      const at = tree.findIndex((t) => sameFile(sel, t));
      let next: TreeFile | undefined;
      if (e.key === 'ArrowDown') next = tree[Math.min(tree.length - 1, at + 1)];
      else if (e.key === 'ArrowUp') next = tree[Math.max(0, at - 1)];
      else if (e.key === 'Home') next = tree[0];
      else if (e.key === 'End') next = tree.at(-1);
      else return;
      e.preventDefault();
      if (next) pickFile(next);
    },
    [tree, sel, pickFile],
  );

  const onListKey = useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      const at = rows.findIndex((r) => r.itemId === itemId);
      let next: MemoryFileRow | undefined;
      if (e.key === 'ArrowDown') next = rows[Math.min(rows.length - 1, at + 1)];
      else if (e.key === 'ArrowUp') next = rows[Math.max(0, at - 1)];
      else if (e.key === 'Home') next = rows[0];
      else if (e.key === 'End') next = rows.at(-1);
      else if (e.key === 'Escape') {
        setWant(undefined);
        setItemId(undefined);
      } else return;
      e.preventDefault();
      if (next) pickRow(next);
    },
    [rows, itemId, pickRow],
  );

  const treeItem = (t: TreeFile, label: string, count: number, icon: ReactNode) => (
    <li
      key={t.path}
      role="treeitem"
      id={fileDomId(t.path)}
      aria-selected={sameFile(sel, t)}
      data-file={t.path}
      className="mem-tree-file"
    >
      <button
        type="button"
        tabIndex={-1}
        className="mem-tree-row"
        onClick={() => {
          pickFile(t);
          treeRef.current?.focus();
        }}
      >
        {icon}
        <span className="mem-tree-name">{label}</span>
        <span className="mem-count">{count}</span>
      </button>
    </li>
  );

  const selFolder = folders.find((f) => f.mogId === sel?.mogId);
  const newButton = (mogId: string, name: string, variant: 'default' | 'outline') => (
    <Button
      type="button"
      variant={variant}
      size="sm"
      onClick={() => openAdd(mogId)}
      aria-label={`New memory in ${name}`}
      data-action="new-memory"
      data-mog={mogId}
    >
      <PlusIcon aria-hidden="true" />
      New memory
    </Button>
  );

  let body: ReactNode;
  if (!model) {
    body = (
      <p className="mem-message" data-memory-loading>
        Loading your memories…
      </p>
    );
  } else if (model.unavailable) {
    body = (
      <p className="mem-message" role="status" data-memory-unavailable>
        Bottle cannot read your memories right now. They stay safe in the encrypted store on this
        Mac.
      </p>
    );
  } else if (folders.every((f) => f.rows.length === 0) && model.userItems === 0 && !compose) {
    body = (
      <div className="mem-message mem-empty" data-memory-empty>
        <p className="mem-empty-title">No memories yet</p>
        <p>
          Add one here, click <strong>Remember this</strong> on a Genie answer, or hold the talk key
          and say “remember this:” and then the fact. Each one goes into today’s note.
        </p>
        <div className="mem-empty-actions">
          {folders.map((f, i) => newButton(f.mogId, f.mogName, i === 0 ? 'default' : 'outline'))}
        </div>
      </div>
    );
  } else {
    const parts = view && view.path === viewPath ? splitFile(view.text) : undefined;
    // A quarantined body is never Markdown: on the item and in the whole-file view.
    const quarantined = row
      ? row.status === 'quarantined'
      : rows.some((r) => r.status === 'quarantined');
    const itemQuarantined = row?.status === 'quarantined';
    const bodyText = parts ? (row ? withoutHeading(parts.body) : parts.body) : '';
    const heading = row ? row.title : fileLabel(sel?.path, now());
    const composeRow =
      compose?.mode === 'edit'
        ? { kind: compose.from.kind, title: compose.from.title, body: compose.from.body }
        : { kind: 'fact' as MemoryKindName, title: '', body: '' };
    const composeFolder = compose && folders.find((f) => f.mogId === compose.mogId);
    body = (
      <div className="mem-panes">
        <nav className="mem-tree" aria-label="Memory files">
          <ul
            ref={treeRef}
            role="tree"
            tabIndex={0}
            onKeyDown={onTreeKey}
            aria-label="Memory files"
            aria-activedescendant={sel ? fileDomId(sel.path) : undefined}
          >
            {showUser &&
              treeItem(
                { mogId: '', path: USER_FILE_PATH },
                'USER.md',
                model.userItems,
                <UserIcon className="size-3.5" aria-hidden="true" />,
              )}
            {folders.map((f) => {
              const c = paletteColorOf(f.mogColorVariant, f.mogId);
              const files = filesOf(f);
              const [memory, ...days] = files;
              return (
                <li key={f.mogId} role="treeitem" aria-expanded="true" data-folder={f.folder}>
                  <span className="mem-tree-folder">
                    <span
                      className="mem-dot"
                      style={{ background: c.hex }}
                      data-color={c.id}
                      aria-hidden="true"
                    />
                    <span className="mem-tree-folder-name">{f.mogName}</span>
                    <button
                      type="button"
                      tabIndex={-1}
                      className="mem-tree-add"
                      aria-label={`New memory in ${f.mogName}`}
                      title="New memory"
                      data-action="new-memory-folder"
                      data-mog={f.mogId}
                      onClick={() => openAdd(f.mogId)}
                    >
                      <PlusIcon className="size-3.5" aria-hidden="true" />
                    </button>
                  </span>
                  <ul role="group">
                    {memory &&
                      treeItem(
                        { mogId: f.mogId, path: memory.path },
                        'MEMORY.md',
                        memory.items,
                        <FileTextIcon className="size-3.5" aria-hidden="true" />,
                      )}
                    {days.map((d) =>
                      treeItem(
                        { mogId: f.mogId, path: d.path },
                        dayLabel(d.label, now()),
                        d.items,
                        <CalendarDaysIcon className="size-3.5" aria-hidden="true" />,
                      ),
                    )}
                  </ul>
                </li>
              );
            })}
          </ul>
        </nav>
        <div className="mem-list-pane">
          {selFolder && (
            <div className="mem-list-head">
              {newButton(selFolder.mogId, selFolder.mogName, 'outline')}
            </div>
          )}
          <div
            ref={listRef}
            className="mem-list"
            role="listbox"
            tabIndex={0}
            aria-label="Memories in this file"
            aria-activedescendant={row ? itemDomId(row.itemId) : undefined}
            onKeyDown={onListKey}
          >
            {rows.length === 0 && (
              <p className="mem-message mem-message--quiet">
                {sel?.path === USER_FILE_PATH
                  ? 'Shared preferences come in a later version.'
                  : 'Nothing here yet.'}
              </p>
            )}
            {rows.map((r) => (
              <div
                key={r.itemId}
                id={itemDomId(r.itemId)}
                role="option"
                aria-selected={r.itemId === itemId}
                className="mem-row"
                data-item={r.itemId}
                data-status={r.status}
                onClick={() => {
                  pickRow(r);
                  listRef.current?.focus();
                }}
              >
                <span className="mem-row-title">
                  {r.pinned && (
                    <PinIcon className="mem-pin size-3" aria-label="Pinned" role="img" />
                  )}
                  {r.title}
                </span>
                <span className="mem-row-meta">
                  <Marks row={r} />
                  <span>{KIND_LABEL[r.kind]}</span>
                </span>
              </div>
            ))}
          </div>
        </div>
        <section className="mem-file" aria-label="Memory file" data-memory-file>
          {compose ? (
            <article className="mem-doc">
              <header className="mem-doc-head">
                <p className="mem-form-title">
                  {compose.mode === 'add'
                    ? `New memory${composeFolder ? ` in ${composeFolder.mogName}` : ''}`
                    : 'Edit memory'}
                </p>
              </header>
              <MemoryForm
                key={compose.mode === 'edit' ? compose.itemId : `add-${compose.mogId}`}
                mode={compose.mode}
                initial={composeRow}
                busy={busy}
                error={formError}
                onSubmit={submit}
                onCancel={closeForm}
              />
            </article>
          ) : (
            <article className="mem-doc">
              <header className="mem-doc-head">
                {heading && (
                  <h3 className="mem-doc-title" data-memory-heading>
                    {heading}
                  </h3>
                )}
                <code className="mem-path" data-memory-path>
                  {viewPath}
                </code>
                {row && (
                  <p className="mem-doc-meta">
                    <span>{SOURCE_LABEL[row.source] ?? row.source}</span>
                    <span>{dateOf(row.createdAt)}</span>
                    <Marks row={releasedAt === undefined ? row : { ...row, releasedAt }} pinned />
                  </p>
                )}
              </header>
              {itemQuarantined && (
                <p className="mem-note" role="note" data-memory-quarantine-note>
                  <span>{QUARANTINE_MESSAGE}</span>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={rate}
                    data-action="release"
                    aria-label="Release this memory to Genie and the agent"
                  >
                    Release
                  </Button>
                </p>
              )}
              {parts ? (
                <>
                  <div className="mem-doc-body">
                    {quarantined ? (
                      <pre className="mem-plain">{bodyText.trim()}</pre>
                    ) : (
                      <SafeMarkdown text={bodyText} trust={NO_LINKS} />
                    )}
                  </div>
                  <details className="mem-front">
                    <summary>File details</summary>
                    <dl>
                      {parts.fields.map(([k, v]) => (
                        <div key={k} className="mem-field">
                          <dt>{k}</dt>
                          <dd>{v}</dd>
                        </div>
                      ))}
                    </dl>
                  </details>
                </>
              ) : (
                <p className="mem-message mem-message--quiet" data-memory-file-loading>
                  Loading the file…
                </p>
              )}
              {row && (
                <footer className="mem-doc-actions">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={row.rating === 1 && row.status === 'active'}
                    onClick={rate}
                    data-action="thumbs-up"
                  >
                    <ThumbsUpIcon aria-hidden="true" />
                    Useful
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={startDelete}
                    title="Not useful: deletes this memory"
                    data-action="thumbs-down"
                  >
                    <ThumbsDownIcon aria-hidden="true" />
                    Not useful
                  </Button>
                  <span className="mem-spacer" />
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={!itemFields}
                    onClick={openEdit}
                    data-action="edit"
                  >
                    <PencilIcon aria-hidden="true" />
                    {itemFields ? 'Edit' : 'Loading…'}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={startDelete}
                    data-action="delete"
                  >
                    Delete
                  </Button>
                </footer>
              )}
            </article>
          )}
        </section>
      </div>
    );
  }

  const toastText = pending ? 'Memory deleted' : toast?.text;
  const toastKeyNow = pending ? -1 : toast?.key;

  return (
    <section className="mem" hidden={!active} aria-label="Memory" data-memory-area>
      {forgetMogId !== undefined && (
        <div className="mem-head">
          {/* MEM-32a: the brain opens the Forget dialog for this MOG (ForgetDialogHost). */}
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-forget-open
            onClick={() =>
              void emitView(forgetMogId, 'click', { target: FORGET_OPEN_TARGET }).catch(
                () => undefined,
              )
            }
          >
            Forget memory…
          </Button>
        </div>
      )}
      {body}
      <div role="status" className="mem-toast-live">
        {toastText !== undefined && (
          <motion.div
            key={toastKeyNow}
            className="mem-toast"
            data-memory-undo
            initial={reduce ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
          >
            <span>{toastText}</span>
            {(pending || toast?.undo) && (
              <Button
                type="button"
                variant="link"
                size="sm"
                onClick={pending ? undoDelete : toast?.undo}
                data-action="undo"
              >
                Undo
              </Button>
            )}
          </motion.div>
        )}
      </div>
    </section>
  );
}
