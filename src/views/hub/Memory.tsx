/**
 * The hub Memory area (local memory plan 9.1, slice 1; ADR 0112). Thin view (ARC-2): it renders
 * what the brain sends on `hub:memory` and `hub:memory.file`, and it runs no store statement.
 * - The tree: USER.md, then for each MOG its MEMORY.md and its daily notes, newest first.
 * - The list: the items of the selected file. A new (not yet rated) item has a "New" badge; a
 *   quarantined one has a mark.
 * - The view: the whole rendered file, or the selected item's section, as Rust renders it.
 * - Feedback: thumbs up (`memory.rate`); thumbs down deletes. A thumbs up on a quarantined item
 *   releases it (owner ruling 2026-11-21): the host checks a ticket of this click, so only the
 *   user can do it. Delete needs a confirm click. Both
 *   deletes show "Memory deleted" with Undo for 8 s, and the view sends `memory.delete` only
 *   when the 8 s end, so Undo needs no new write. If the area goes away in the 8 s, the delete
 *   goes at once: the user asked for it.
 * The view reaches the brain only through a {@link MemorySource}; tests pass a local one.
 */
import { listen } from '@tauri-apps/api/event';
import {
  CalendarDaysIcon,
  FileTextIcon,
  PinIcon,
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
  isNew,
  KIND_LABEL,
  MEMORY_UNDO_MS,
  MEMORY_VIEW_ID,
  splitFile,
  USER_FILE_PATH,
  type HubMemory,
  type MemoryAction,
  type MemoryFileRow,
  type MemoryFileView,
} from '../../shared/hub-memory';
import { NO_LINKS } from '../../shared/link-trust';
import { paletteColorOf } from '../../shared/mog-palette';
import { SafeMarkdown } from '../card/safe-markdown';
import { Button } from '../ui/button';
import { emitView } from '../view-ipc';

/** The seam between the view and the brain. */
export interface MemorySource {
  /** Listen for the folders and the files. Returns the stop function. */
  subscribe(onList: (m: HubMemory) => void, onFile: (f: MemoryFileView) => void): () => void;
  /** Send one user action to the brain. */
  send(a: MemoryAction): void;
}

/** The default source: `hub:memory` events in, `view_emit` clicks out. */
export const ipcMemorySource: MemorySource = {
  subscribe(onList, onFile) {
    const stops: (() => void)[] = [];
    let stopped = false;
    const keep = (p: Promise<() => void>) =>
      void p.then((stop) => (stopped ? stop() : stops.push(stop))).catch(() => undefined);
    keep(
      listen(HUB_MEMORY_EVENT, (e) => {
        const m = asHubMemory(e.payload);
        if (m) onList(m);
      }),
    );
    keep(
      listen(HUB_MEMORY_FILE_EVENT, (e) => {
        const f = asMemoryFileView(e.payload);
        if (f) onFile(f);
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

function Marks({ row }: { row: MemoryFileRow }) {
  return (
    <>
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
    </>
  );
}

const sameFile = (a: TreeFile | undefined, b: TreeFile) =>
  a !== undefined && a.mogId === b.mogId && a.path === b.path;

export interface MemoryAreaProps {
  /** The area shows. Each time it shows, it asks for the folders again. */
  active: boolean;
  source?: MemorySource;
  /** The clock for the day labels. Tests pass a fixed one. */
  now?: () => number;
}

export function MemoryArea({ active, source = ipcMemorySource, now = Date.now }: MemoryAreaProps) {
  const [model, setModel] = useState<HubMemory | undefined>(undefined);
  const [view, setView] = useState<MemoryFileView | undefined>(undefined);
  const [picked, setPicked] = useState<TreeFile | undefined>(undefined);
  const [itemId, setItemId] = useState<string | undefined>(undefined);
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState<Pending | undefined>(undefined);
  const [rated, setRated] = useState<ReadonlySet<string>>(new Set());
  const pendingRef = useRef<Pending | undefined>(undefined);
  useEffect(() => {
    pendingRef.current = pending;
  }, [pending]);

  useEffect(() => source.subscribe(setModel, setView), [source]);

  useEffect(() => {
    if (active) source.send({ action: 'memory.ready' });
  }, [active, source]);

  // The delete goes when the area goes away: the user asked for it.
  useEffect(
    () => () => {
      const p = pendingRef.current;
      if (!p) return;
      clearTimeout(p.timer);
      source.send({ action: 'memory.delete', mogId: p.mogId, itemId: p.itemId });
    },
    [source],
  );

  // The rows the view shows: a pending delete hides its row; a thumbs up shows at once.
  const folders = useMemo(
    () =>
      (model?.folders ?? []).map((f) => ({
        ...f,
        rows: f.rows
          .filter((r) => r.itemId !== pending?.itemId)
          .map((r) =>
            rated.has(r.itemId)
              ? {
                  ...r,
                  rating: 1 as const,
                  reviewedAt: 0,
                  status: r.status === 'quarantined' ? ('active' as const) : r.status,
                }
              : r,
          ),
      })),
    [model, pending, rated],
  );

  const tree = useMemo<TreeFile[]>(
    () => [
      { mogId: '', path: USER_FILE_PATH },
      ...folders.flatMap((f) => filesOf(f).map((e) => ({ mogId: f.mogId, path: e.path }))),
    ],
    [folders],
  );

  // Until the user picks a file: the newest daily note of the first MOG, else its MEMORY.md.
  const sel = useMemo<TreeFile | undefined>(() => {
    if (picked && tree.some((t) => sameFile(picked, t))) return picked;
    const first = folders[0];
    if (!first) return tree[0];
    const files = filesOf(first);
    const entry = files.find((e) => e.layer === 'daily') ?? files[0];
    return entry ? { mogId: first.mogId, path: entry.path } : tree[0];
  }, [picked, tree, folders]);

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

  // Ask for the rendered file or item each time the selection changes.
  useEffect(() => {
    if (!active || !sel || !viewPath) return;
    source.send({ action: 'memory.file', mogId: sel.mogId, path: viewPath });
  }, [active, sel, viewPath, source]);

  const pickFile = useCallback((t: TreeFile) => {
    setPicked(t);
    setItemId(undefined);
    setConfirming(false);
  }, []);

  const pickRow = useCallback((r: MemoryFileRow) => {
    setItemId(r.itemId);
    setConfirming(false);
  }, []);

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
    setPending(p);
    setConfirming(false);
    setItemId(undefined);
  }, [sel, row, source]);

  const undo = useCallback(() => {
    const p = pendingRef.current;
    if (!p) return;
    clearTimeout(p.timer);
    setPending(undefined);
  }, []);

  const rate = useCallback(() => {
    if (!sel?.mogId || !row) return;
    source.send({ action: 'memory.rate', mogId: sel.mogId, itemId: row.itemId });
    setRated((s) => new Set([...s, row.itemId]));
  }, [sel, row, source]);

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
      else if (e.key === 'Escape') setItemId(undefined);
      else return;
      e.preventDefault();
      if (next) pickRow(next);
    },
    [rows, itemId, pickRow],
  );

  const treeItem = (t: TreeFile, label: string, count: number, icon: ReactNode) => (
    <li
      key={t.path}
      role="treeitem"
      aria-selected={sameFile(sel, t)}
      data-file={t.path}
      className="mem-tree-file"
    >
      <button type="button" tabIndex={-1} className="mem-tree-row" onClick={() => pickFile(t)}>
        {icon}
        <span className="mem-tree-name">{label}</span>
        <span className="mem-count">{count}</span>
      </button>
    </li>
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
  } else if (folders.every((f) => f.rows.length === 0) && model.userItems === 0) {
    body = (
      <div className="mem-message" data-memory-empty>
        <p className="mem-empty-title">No memories yet</p>
        <p>
          Click <strong>Remember this</strong> on a Genie answer, or hold the talk key and say
          “remember this:” and then the fact. Each one goes into today’s note.
        </p>
      </div>
    );
  } else {
    const parts = view && view.path === viewPath ? splitFile(view.text) : undefined;
    body = (
      <div className="mem-panes">
        <nav className="mem-tree" aria-label="Memory files">
          <ul role="tree" tabIndex={0} onKeyDown={onTreeKey} aria-label="Memory files">
            {treeItem(
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
                    {f.mogName}
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
        <div
          className="mem-list"
          role="listbox"
          tabIndex={0}
          aria-label="Memories in this file"
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
              role="option"
              aria-selected={r.itemId === itemId}
              className="mem-row"
              data-item={r.itemId}
              data-status={r.status}
              onClick={() => pickRow(r)}
            >
              <span className="mem-row-title">
                {r.pinned && <PinIcon className="mem-pin size-3" aria-label="Pinned" role="img" />}
                {r.title}
              </span>
              <span className="mem-row-meta">
                <Marks row={r} />
                <span>{KIND_LABEL[r.kind]}</span>
              </span>
            </div>
          ))}
        </div>
        <section className="mem-file" aria-label="Memory file" data-memory-file>
          <article className="mem-doc">
            <header className="mem-doc-head">
              <code className="mem-path" data-memory-path>
                {viewPath}
              </code>
              {row && (
                <p className="mem-doc-meta">
                  <span>{SOURCE_LABEL[row.source] ?? row.source}</span>
                  <span>{dateOf(row.createdAt)}</span>
                  <Marks row={row} />
                </p>
              )}
            </header>
            {row?.status === 'quarantined' && (
              <p className="mem-note" role="note" data-memory-quarantine-note>
                This memory reads like an instruction to an assistant. Bottle keeps it, but never
                gives it to Genie or the agent. If you trust it, click Useful: then it is a normal
                memory.
              </p>
            )}
            {parts ? (
              <>
                <div className="mem-doc-body">
                  {row?.status === 'quarantined' ? (
                    <pre className="mem-plain">{parts.body.trim()}</pre>
                  ) : (
                    <SafeMarkdown text={parts.body} trust={NO_LINKS} />
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
                {confirming ? (
                  <>
                    <span className="mem-confirm-text">Delete this memory?</span>
                    <Button type="button" variant="destructive" size="sm" onClick={startDelete}>
                      Delete
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() => setConfirming(false)}
                    >
                      Cancel
                    </Button>
                  </>
                ) : (
                  <>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      aria-pressed={row.rating === 1}
                      disabled={row.rating === 1}
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
                      onClick={() => setConfirming(true)}
                    >
                      Delete…
                    </Button>
                  </>
                )}
              </footer>
            )}
          </article>
        </section>
      </div>
    );
  }

  return (
    <section className="mem" hidden={!active} aria-label="Memory" data-memory-area>
      {body}
      {pending && (
        <div className="mem-toast" role="status" data-memory-undo>
          <span>Memory deleted</span>
          <Button type="button" variant="link" size="sm" onClick={undo}>
            Undo
          </Button>
        </div>
      )}
    </section>
  );
}
