/**
 * The hub Memory area (local memory plan 9.1, slice 1; ADR 0112). Thin view (ARC-2): it renders
 * the folders that the brain sends on `hub:memory` and the files it sends on `hub:memory.file`,
 * and it runs no store statement.
 * - The tree: one folder for each MOG, then its kind folders (Facts, Answers, …) with counts.
 * - The list: the memories of the selected folder, pinned first, then by the last use. A
 *   proposed or quarantined memory has a mark.
 * - The file: the path, the frontmatter fields and the body, as Rust renders them.
 * - Delete: a confirm click, then "Memory deleted" with Undo for 8 s. The view sends
 *   `memory.delete` only when the 8 s end, so Undo needs no new write. If the area goes away in
 *   the 8 s, the delete goes at once: the user asked for it.
 * The view reaches the brain only through a {@link MemorySource}; tests pass a local one.
 */
import { listen } from '@tauri-apps/api/event';
import { FileTextIcon, FolderIcon, PinIcon, ShieldAlertIcon } from 'lucide-react';
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
  HUB_MEMORY_EVENT,
  HUB_MEMORY_FILE_EVENT,
  KIND_FOLDER,
  KIND_LABEL,
  MEMORY_KINDS,
  MEMORY_UNDO_MS,
  MEMORY_VIEW_ID,
  splitFile,
  type HubMemory,
  type MemoryAction,
  type MemoryFileRow,
  type MemoryFileView,
  type MemoryFolder,
  type MemoryKindName,
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

/** The selection of the tree: a MOG folder, and a kind folder in it or all kinds. */
interface TreeSel {
  mogId: string;
  kind?: MemoryKindName;
}

interface Pending {
  mogId: string;
  itemId: string;
  title: string;
  timer: ReturnType<typeof setTimeout>;
}

const ORIGIN_LABEL: Readonly<Record<string, string>> = {
  pin: 'Remember this',
  kept_answer: 'Kept answer',
  correction: 'Your edit',
  dictation_word: 'Dictation word',
  call_notes: 'Call notes',
  agent: 'Agent',
  merge: 'Merge',
  legacy: 'Earlier version of Bottle',
};

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
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });

function StatusMark({ row }: { row: MemoryFileRow }) {
  if (row.status === 'quarantined')
    return (
      <span className="mem-mark mem-mark--quarantined" data-mark="quarantined">
        <ShieldAlertIcon className="size-3" aria-hidden="true" />
        Quarantined
      </span>
    );
  if (row.status === 'proposed')
    return (
      <span className="mem-mark" data-mark="proposed">
        Suggested
      </span>
    );
  if (row.status === 'stale')
    return (
      <span className="mem-mark" data-mark="stale">
        Not used lately
      </span>
    );
  return null;
}

function kindsOf(f: MemoryFolder): { kind: MemoryKindName; count: number }[] {
  return MEMORY_KINDS.map((kind) => ({
    kind,
    count: f.rows.filter((r) => r.kind === kind).length,
  })).filter((k) => k.count > 0);
}

/** The tree items in order, for the arrow keys. */
function treeItems(folders: MemoryFolder[], open: ReadonlySet<string>): TreeSel[] {
  return folders.flatMap((f) => [
    { mogId: f.mogId },
    ...(open.has(f.mogId) ? kindsOf(f).map(({ kind }) => ({ mogId: f.mogId, kind })) : []),
  ]);
}

const sameSel = (a: TreeSel | undefined, b: TreeSel) =>
  a !== undefined && a.mogId === b.mogId && a.kind === b.kind;

export interface MemoryAreaProps {
  /** The area shows. Each time it shows, it asks for the folders again. */
  active: boolean;
  source?: MemorySource;
}

export function MemoryArea({ active, source = ipcMemorySource }: MemoryAreaProps) {
  const [model, setModel] = useState<HubMemory | undefined>(undefined);
  const [file, setFile] = useState<MemoryFileView | undefined>(undefined);
  const [picked, setSel] = useState<TreeSel | undefined>(undefined);
  // Folders start open; a click or ArrowLeft closes one.
  const [closed, setClosed] = useState<ReadonlySet<string>>(new Set());
  const [itemId, setItemId] = useState<string | undefined>(undefined);
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState<Pending | undefined>(undefined);
  const pendingRef = useRef<Pending | undefined>(undefined);
  useEffect(() => {
    pendingRef.current = pending;
  }, [pending]);

  useEffect(() => source.subscribe(setModel, setFile), [source]);

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

  const folders = useMemo(() => {
    const hidden = pending?.itemId;
    return (model?.folders ?? []).map((f) => ({
      ...f,
      rows: f.rows.filter((r) => r.itemId !== hidden),
    }));
  }, [model, pending]);

  // The first folder is selected until the user picks one, or when the picked one goes away.
  const sel = useMemo<TreeSel | undefined>(() => {
    if (picked && folders.some((f) => f.mogId === picked.mogId)) return picked;
    const first = folders[0];
    return first ? { mogId: first.mogId } : undefined;
  }, [picked, folders]);
  const open = useMemo(
    () => new Set(folders.map((f) => f.mogId).filter((id) => !closed.has(id))),
    [folders, closed],
  );
  const setOpen = useCallback((mogId: string, isOpen: boolean) => {
    setClosed((c) => {
      const n = new Set(c);
      if (isOpen) n.delete(mogId);
      else n.add(mogId);
      return n;
    });
  }, []);

  const folder = folders.find((f) => f.mogId === sel?.mogId);
  const rows = useMemo(
    () => (folder ? folder.rows.filter((r) => !sel?.kind || r.kind === sel.kind) : []),
    [folder, sel],
  );
  const row = rows.find((r) => r.itemId === itemId);

  const pick = useCallback(
    (r: MemoryFileRow) => {
      if (!folder) return;
      setItemId(r.itemId);
      setConfirming(false);
      source.send({ action: 'memory.file', mogId: folder.mogId, itemId: r.itemId });
    },
    [folder, source],
  );

  const finishDelete = useCallback(
    (p: Pending) => {
      source.send({ action: 'memory.delete', mogId: p.mogId, itemId: p.itemId });
      setPending((cur) => (cur?.itemId === p.itemId ? undefined : cur));
    },
    [source],
  );

  const startDelete = useCallback(() => {
    if (!folder || !row) return;
    // A second delete sends the first one at once.
    const prev = pendingRef.current;
    if (prev) {
      clearTimeout(prev.timer);
      source.send({ action: 'memory.delete', mogId: prev.mogId, itemId: prev.itemId });
    }
    const p: Pending = {
      mogId: folder.mogId,
      itemId: row.itemId,
      title: row.title,
      timer: setTimeout(() => finishDelete(p), MEMORY_UNDO_MS),
    };
    setPending(p);
    setConfirming(false);
    setItemId(undefined);
    setFile(undefined);
  }, [folder, row, finishDelete, source]);

  const undo = useCallback(() => {
    const p = pendingRef.current;
    if (!p) return;
    clearTimeout(p.timer);
    setPending(undefined);
  }, []);

  const onTreeKey = useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      const items = treeItems(folders, open);
      const at = items.findIndex((t) => sameSel(sel, t));
      const go = (i: number) => {
        const t = items[Math.max(0, Math.min(items.length - 1, i))];
        if (t) {
          setSel(t);
          setItemId(undefined);
        }
      };
      if (e.key === 'ArrowDown') go(at + 1);
      else if (e.key === 'ArrowUp') go(at - 1);
      else if (e.key === 'Home') go(0);
      else if (e.key === 'End') go(items.length - 1);
      else if (e.key === 'ArrowRight' && sel && !sel.kind) setOpen(sel.mogId, true);
      else if (e.key === 'ArrowLeft' && sel) {
        if (sel.kind) setSel({ mogId: sel.mogId });
        else setOpen(sel.mogId, false);
      } else return;
      e.preventDefault();
    },
    [folders, open, sel, setOpen],
  );

  const onListKey = useCallback(
    (e: KeyboardEvent<HTMLElement>) => {
      const at = rows.findIndex((r) => r.itemId === itemId);
      let next: MemoryFileRow | undefined;
      if (e.key === 'ArrowDown') next = rows[Math.min(rows.length - 1, at + 1)];
      else if (e.key === 'ArrowUp') next = rows[Math.max(0, at - 1)];
      else if (e.key === 'Home') next = rows[0];
      else if (e.key === 'End') next = rows.at(-1);
      else return;
      e.preventDefault();
      if (next) pick(next);
    },
    [rows, itemId, pick],
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
  } else if (folders.every((f) => f.rows.length === 0)) {
    body = (
      <div className="mem-message" data-memory-empty>
        <p className="mem-empty-title">No memories yet</p>
        <p>
          Click <strong>Remember this</strong> on a Genie answer, or hold the talk key and say
          “remember this:” and then the fact.
        </p>
      </div>
    );
  } else {
    body = (
      <div className="mem-panes">
        <nav className="mem-tree" aria-label="Memory folders">
          <ul role="tree" tabIndex={0} onKeyDown={onTreeKey} aria-label="Memory folders">
            {folders.map((f) => {
              const c = paletteColorOf(f.mogColorVariant, f.mogId);
              const isOpen = open.has(f.mogId);
              return (
                <li
                  key={f.mogId}
                  role="treeitem"
                  aria-expanded={isOpen}
                  aria-selected={sameSel(sel, { mogId: f.mogId })}
                  data-folder={f.folder}
                >
                  <button
                    type="button"
                    tabIndex={-1}
                    className="mem-tree-row"
                    onClick={() => {
                      setSel({ mogId: f.mogId });
                      setItemId(undefined);
                      // A click on the selected open folder closes it.
                      setOpen(f.mogId, !(isOpen && sameSel(sel, { mogId: f.mogId })));
                    }}
                  >
                    <span
                      className="mem-dot"
                      style={{ background: c.hex }}
                      data-color={c.id}
                      aria-hidden="true"
                    />
                    <span className="mem-tree-name">{f.mogName}</span>
                    <span className="mem-count">{f.rows.length}</span>
                  </button>
                  {isOpen && (
                    <ul role="group">
                      {kindsOf(f).map(({ kind, count }) => (
                        <li
                          key={kind}
                          role="treeitem"
                          aria-selected={sameSel(sel, { mogId: f.mogId, kind })}
                          data-kind-folder={KIND_FOLDER[kind]}
                        >
                          <button
                            type="button"
                            tabIndex={-1}
                            className="mem-tree-row mem-tree-row--kind"
                            onClick={() => {
                              setSel({ mogId: f.mogId, kind });
                              setItemId(undefined);
                            }}
                          >
                            <FolderIcon className="size-3.5" aria-hidden="true" />
                            <span className="mem-tree-name">{KIND_LABEL[kind]}</span>
                            <span className="mem-count">{count}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              );
            })}
          </ul>
        </nav>
        <div
          className="mem-list"
          role="listbox"
          tabIndex={0}
          aria-label="Memories"
          onKeyDown={onListKey}
        >
          {rows.map((r) => (
            <div
              key={r.itemId}
              role="option"
              aria-selected={r.itemId === itemId}
              className="mem-row"
              data-item={r.itemId}
              data-status={r.status}
              onClick={() => pick(r)}
            >
              <span className="mem-row-title">
                {r.pinned && <PinIcon className="mem-pin size-3" aria-label="Pinned" role="img" />}
                {r.title}
              </span>
              <span className="mem-row-meta">
                <StatusMark row={r} />
                <span className="mem-row-path">{r.path.split('/').slice(-2).join('/')}</span>
              </span>
            </div>
          ))}
        </div>
        <section className="mem-file" aria-label="Memory file" data-memory-file>
          {row ? (
            <FileView
              row={row}
              file={file?.itemId === row.itemId ? file : undefined}
              confirming={confirming}
              onDelete={() => setConfirming(true)}
              onCancel={() => setConfirming(false)}
              onConfirm={startDelete}
            />
          ) : (
            <p className="mem-message mem-message--quiet">
              <FileTextIcon className="size-4" aria-hidden="true" />
              Select a memory to see its file.
            </p>
          )}
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

interface FileViewProps {
  row: MemoryFileRow;
  file: MemoryFileView | undefined;
  confirming: boolean;
  onDelete(): void;
  onCancel(): void;
  onConfirm(): void;
}

function FileView({ row, file, confirming, onDelete, onCancel, onConfirm }: FileViewProps) {
  const parts = file ? splitFile(file.text) : undefined;
  const quarantined = row.status === 'quarantined';
  return (
    <article className="mem-doc">
      <header className="mem-doc-head">
        <code className="mem-path" data-memory-path>
          {row.path}
        </code>
        <h3 className="mem-doc-title">{row.title}</h3>
        <p className="mem-doc-meta">
          <span>{SOURCE_LABEL[row.source] ?? row.source}</span>
          <span>{ORIGIN_LABEL[row.origin] ?? row.origin}</span>
          <span>{dateOf(row.createdAt)}</span>
          <StatusMark row={row} />
        </p>
      </header>
      {quarantined && (
        <p className="mem-note" role="note" data-memory-quarantine-note>
          This memory looks like an instruction to an assistant, so Bottle quarantined it. Recall
          and the agent never use it. You can delete it.
        </p>
      )}
      {parts ? (
        <>
          <div className="mem-doc-body">
            {quarantined ? (
              <pre className="mem-plain">{parts.body.trimEnd()}</pre>
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
      <footer className="mem-doc-actions">
        {confirming ? (
          <>
            <span className="mem-confirm-text">Delete this memory?</span>
            <Button type="button" variant="destructive" size="sm" onClick={onConfirm}>
              Delete
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
              Cancel
            </Button>
          </>
        ) : (
          <Button type="button" variant="outline" size="sm" onClick={onDelete}>
            Delete…
          </Button>
        )}
      </footer>
    </article>
  );
}
