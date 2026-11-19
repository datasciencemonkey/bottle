/**
 * The hub MOG list (#26, MB-01, MB-03a, MB-03b). Thin view (ARC-2): it renders the list the brain
 * sends on `hub:mogs` and sends one hub action per user action with `view_emit`. The brain checks
 * every value and writes every result text. The view keeps only form and dialog state.
 * TODO(#407): the event name and the actions move into the contract.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import {
  asHubMogList,
  HUB_MOGS_EVENT,
  HUB_SCOPE,
  type HubMogList,
  type HubMogRow,
  type MogAction,
} from '../../shared/hub-mogs';
import { FORGET_OPEN_TARGET } from '../../shared/forget-view';
import { emitView } from '../view-ipc';

function send(action: MogAction): void {
  void emitView(HUB_SCOPE, 'click', action).catch(() => undefined);
}

/** The list the brain sends to this window, or null before the brain answers. */
function useHubMogs(): { list: HubMogList | null; seq: number } {
  const [state, setState] = useState<{ list: HubMogList | null; seq: number }>({
    list: null,
    seq: 0,
  });
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    // ARC-5: only events sent to this window.
    const target = getCurrentWindow().label;
    void listen<unknown>(
      HUB_MOGS_EVENT,
      (e) => {
        const l = asHubMogList(e.payload);
        // `seq` counts notices, so the same text twice is a new alert (read again).
        if (l) setState((s) => ({ list: l, seq: l.notice ? s.seq + 1 : s.seq }));
      },
      { target },
    ).then((u) => {
      if (gone) u();
      else {
        off = u;
        send({ action: 'mogs.ready' });
      }
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return state;
}

function CreateForm() {
  const [name, setName] = useState('');
  const [home, setHome] = useState('');
  const id = useId();
  return (
    <form
      aria-label="Create a MOG"
      className="hub-form hub-create"
      onSubmit={(e) => {
        e.preventDefault();
        send({ action: 'mog.create', name, home });
        setName('');
      }}
    >
      <label htmlFor={`${id}-name`}>Name</label>
      <input
        id={`${id}-name`}
        value={name}
        maxLength={64}
        placeholder="MOG 1"
        onChange={(e) => setName(e.target.value)}
      />
      {/* TODO(#17, #163): pick the home profile from the ProfileService list. */}
      <label htmlFor={`${id}-home`}>Home profile</label>
      <input id={`${id}-home`} value={home} required onChange={(e) => setHome(e.target.value)} />
      <button type="submit" className="hub-primary">
        Create MOG
      </button>
    </form>
  );
}

function EditForm({ row, onDone }: { row: HubMogRow; onDone: () => void }) {
  const [name, setName] = useState(row.name);
  const [home, setHome] = useState(row.home);
  const [persona, setPersona] = useState(row.persona);
  const id = useId();
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') onDone();
  };
  return (
    <form
      data-edit={row.mogId}
      aria-label={`Edit ${row.name}`}
      className="hub-form hub-edit"
      onKeyDown={onKey}
      onSubmit={(e) => {
        e.preventDefault();
        send({ action: 'mog.edit', mogId: row.mogId, name, home, persona });
        onDone();
      }}
    >
      <label htmlFor={`${id}-name`}>Name of {row.name}</label>
      <input
        id={`${id}-name`}
        value={name}
        maxLength={64}
        autoFocus
        onChange={(e) => setName(e.target.value)}
      />
      <label htmlFor={`${id}-home`}>Home profile of {row.name}</label>
      <input id={`${id}-home`} value={home} onChange={(e) => setHome(e.target.value)} />
      <label htmlFor={`${id}-persona`}>Persona instructions</label>
      <textarea
        id={`${id}-persona`}
        value={persona}
        maxLength={2000}
        onChange={(e) => setPersona(e.target.value)}
      />
      <div className="hub-actions">
        <button type="submit" className="hub-primary">
          Save
        </button>
        <button type="button" onClick={onDone}>
          Cancel
        </button>
      </div>
    </form>
  );
}

function DeleteDialog({
  row,
  onDone,
  onDeleted,
}: {
  row: HubMogRow;
  onDone: () => void;
  onDeleted: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  const del = useRef<HTMLButtonElement>(null);
  const id = useId();
  useEffect(() => cancel.current?.focus(), []);
  return (
    <div
      role="alertdialog"
      aria-modal="true"
      className="hub-dialog"
      aria-labelledby={`${id}-title`}
      aria-describedby={`${id}-text`}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onDone();
        if (e.key === 'Tab') {
          // aria-modal: Tab and Shift+Tab move between the two buttons only.
          e.preventDefault();
          (document.activeElement === cancel.current ? del : cancel).current?.focus();
        }
      }}
    >
      <p id={`${id}-title`} className="hub-dialog-title">
        Delete {row.name}?
      </p>
      <p id={`${id}-text`}>
        Bottle removes the settings of this MOG. Its memory and history stay until you use Wipe in
        Settings.
      </p>
      <div className="hub-actions">
        <button
          type="button"
          className="hub-danger"
          ref={del}
          onClick={() => {
            send({ action: 'mog.delete', mogId: row.mogId, confirmed: true });
            onDeleted();
          }}
        >
          Delete
        </button>
        <button type="button" ref={cancel} onClick={onDone}>
          Cancel
        </button>
      </div>
    </div>
  );
}

function Row({ row, onDeleted }: { row: HubMogRow; onDeleted: () => void }) {
  const [mode, setMode] = useState<'view' | 'edit' | 'delete'>('view');
  const editBtn = useRef<HTMLButtonElement>(null);
  const deleteBtn = useRef<HTMLButtonElement>(null);
  // WCAG 2.4.3: when the form or the dialog closes, focus goes back to the button that opened it.
  const closeEdit = () => {
    setMode('view');
    editBtn.current?.focus();
  };
  const closeDelete = () => {
    setMode('view');
    deleteBtn.current?.focus();
  };
  return (
    <li data-mog-row={row.mogId} data-running={String(row.running)} className="hub-row">
      <span className="hub-row-name">{row.name}</span>{' '}
      <span className="hub-row-state">{row.running ? 'Running' : 'Stopped'}</span>
      <span className="hub-row-actions">
        {row.running ? (
          <button
            type="button"
            aria-label={`Stop ${row.name}`}
            onClick={() => send({ action: 'mog.stop', mogId: row.mogId })}
          >
            Stop
          </button>
        ) : (
          <button
            type="button"
            aria-label={`Start ${row.name}`}
            onClick={() => send({ action: 'mog.start', mogId: row.mogId })}
          >
            Start
          </button>
        )}
        <button
          type="button"
          ref={editBtn}
          aria-label={`Edit ${row.name}`}
          onClick={() => setMode('edit')}
        >
          Edit
        </button>
        {/* The less used actions sit behind one "More" entry point; they stay in the row. */}
        <details className="hub-more">
          <summary aria-label={`More actions for ${row.name}`}>More</summary>
          <span className="hub-more-items">
            <button
              type="button"
              aria-label={`Duplicate ${row.name}`}
              onClick={() => send({ action: 'mog.duplicate', mogId: row.mogId })}
            >
              Duplicate
            </button>
            {/* #123 (MEM-32a): the brain opens the Forget dialog for this MOG. */}
            <button
              type="button"
              data-forget-open
              aria-label={`Forget the memory of ${row.name}`}
              onClick={() =>
                void emitView(row.mogId, 'click', { target: FORGET_OPEN_TARGET }).catch(
                  () => undefined,
                )
              }
            >
              Forget memory
            </button>
            <button
              type="button"
              ref={deleteBtn}
              aria-label={`Delete ${row.name}`}
              onClick={() => setMode('delete')}
            >
              Delete
            </button>
          </span>
        </details>
      </span>
      {mode === 'edit' && <EditForm row={row} onDone={closeEdit} />}
      {mode === 'delete' && (
        <DeleteDialog
          row={row}
          onDone={closeDelete}
          onDeleted={() => {
            setMode('view');
            onDeleted();
          }}
        />
      )}
    </li>
  );
}

/** The hub section that lists, creates, edits, duplicates, starts, stops and deletes MOGs. */
export function MogList() {
  const { list, seq } = useHubMogs();
  const id = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const notice = list?.notice;
  return (
    <section aria-labelledby={`${id}-h`} className="hub-section hub-mogs">
      <h2 id={`${id}-h`} ref={heading} tabIndex={-1}>
        MOGs
      </h2>
      <p role="status" className="hub-muted">
        {list ? `${list.runningCount} of ${list.maxRunning} MOGs are running.` : ''}
      </p>
      <p role="alert" key={seq} className="hub-error">
        {notice?.kind === 'error' ? notice.text : ''}
      </p>
      <p aria-live="polite">{notice?.kind === 'info' ? notice.text : ''}</p>
      {list && list.unreadable.length > 0 && (
        <p>{`${list.unreadable.length} MOG configs cannot be read. Bottle keeps the files.`}</p>
      )}
      <ul aria-label="Defined MOGs" className="hub-rows">
        {(list?.mogs ?? []).map((r) => (
          <Row key={r.mogId} row={r} onDeleted={() => heading.current?.focus()} />
        ))}
      </ul>
      <CreateForm />
    </section>
  );
}
