/**
 * The hub Recordings area (ADR 0103, refs #65), next to Conversations. Thin view (ARC-2): it
 * renders the list the brain sends on `hub:recordings` and the one recording it sends on
 * `hub:recording.detail`, and asks again with `recordings.ready` each time the area shows.
 * - A listbox, newest first, under day headers: the time, the length and the status. A failed
 *   recording has Retry. Up, Down, Home and End move the selection.
 * - The detail: the notes through the card's allowlist markdown renderer (no raw HTML, SEC-25a;
 *   no trusted host, so no link is live), the speaker turns with time stamps, and "Open in
 *   Databricks". The core opens that link; the view never builds or opens a URL.
 */
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from 'react';
import {
  asHubRecordingDetail,
  asHubRecordings,
  HUB_RECORDING_DETAIL_EVENT,
  HUB_RECORDINGS_EVENT,
  lengthOf,
  RECORDING_TEXT,
  recordingFileNames,
  rowActionOf,
  rowCanDeleteOf,
  stampOf,
  statusLabelOf,
  type HubRecordingDetail,
  type HubRecordings,
  type RecordingOpenWhat,
  type RecordingRow,
} from '../../shared/recording-view';
import { MAX_ANSWER_CHARS, SafeMarkdown } from '../card/safe-markdown';
import { RecordingAlert } from './RecordControl';
import { askRecording, sendRecording, useHubEvent } from './recording-ipc';

const DAY_MS = 86_400_000;
/** No host is trusted in recording notes: a link shows as text (SEC-25a). */
const NO_LINKS = { profileHost: '', allowedAppUrls: [] as string[] };

/**
 * The notes with a real heading scale: `#`/`##` and `###`+ lines become headings (plain text,
 * never HTML); every other block goes through the card's allowlist renderer. Lines in a code
 * fence stay code.
 */
function Notes({ text }: { text: string }) {
  const out: React.ReactNode[] = [];
  let chunk: string[] = [];
  let fence = false;
  const flush = () => {
    if (chunk.join('').trim() !== '')
      out.push(<SafeMarkdown key={`m${out.length}`} text={chunk.join('\n')} trust={NO_LINKS} />);
    chunk = [];
  };
  for (const line of text.slice(0, MAX_ANSWER_CHARS).split(/\r?\n/)) {
    if (line.startsWith('```')) fence = !fence;
    const h = fence ? null : /^(#{1,6})\s+(.*)$/.exec(line);
    if (!h) {
      chunk.push(line);
      continue;
    }
    flush();
    const words = (h[2] ?? '').replace(/[*`_]/g, '').trim();
    out.push(
      (h[1] ?? '').length <= 2 ? (
        <h4 key={`h${out.length}`} className="rec-h2">
          {words}
        </h4>
      ) : (
        <h5 key={`h${out.length}`} className="rec-h3">
          {words}
        </h5>
      ),
    );
  }
  flush();
  return <>{out}</>;
}

/** The banner of Recordings when recordings cannot upload: the reason and one button. */
function StorageBanner({
  reason,
  onOpenSettings,
}: {
  reason: string;
  onOpenSettings: (() => void) | undefined;
}) {
  return (
    <div className="rec-alert rec-banner" role="alert" data-recording-banner>
      <p className="rec-banner-title">{RECORDING_TEXT.cantUpload}</p>
      <p className="rec-banner-reason">{reason}</p>
      {onOpenSettings && (
        <button type="button" data-recording-banner-action onClick={onOpenSettings}>
          {RECORDING_TEXT.chooseWhere}
        </button>
      )}
    </div>
  );
}

/** Upload all, and its progress while it runs. */
function UploadAll({
  state,
  blocked,
}: {
  state: NonNullable<HubRecordings['uploadAll']>;
  blocked: boolean;
}) {
  const { running, done, total, failed } = state;
  return (
    <div className="rec-upload-all" data-recording-upload-all>
      <button
        type="button"
        disabled={running || blocked}
        onClick={() => sendRecording({ action: 'recordings.upload_all' })}
      >
        {running ? 'Uploading…' : `Upload all (${total})`}
      </button>
      {running && (
        <span className="rec-upload-progress" role="status" data-recording-upload-progress>
          {Math.min(done + 1, total)} of {total}
        </span>
      )}
      {!running && failed > 0 && (
        <span className="rec-upload-progress" role="status">
          {failed} did not upload
        </span>
      )}
    </div>
  );
}

const BATCH_LABEL = {
  queued: 'Queued',
  running: 'Uploading…',
  done: 'Uploaded',
  failed: 'Did not upload',
} as const;

const OPEN_LABEL: Record<RecordingOpenWhat, string> = {
  transcript: 'Open transcript in Databricks',
  notes: 'Open notes in Databricks',
  folder: 'Open folder in Databricks',
};

/**
 * The three links of a finished recording. Databricks has no file deep link, so the transcript
 * and notes links open the folder, and the file name shows so the user knows what to look for.
 */
function FileLinks({ row, detail }: { row: RecordingRow; detail?: boolean }) {
  const files = recordingFileNames(row.name);
  const open = (target: RecordingOpenWhat) => (e: React.MouseEvent) => {
    e.stopPropagation();
    sendRecording({ action: 'recordings.open', id: row.id, target });
  };
  const name: Record<RecordingOpenWhat, string | undefined> = {
    transcript: files.transcript,
    notes: files.notes,
    folder: undefined,
  };
  return (
    <div className={detail ? 'rec-links rec-links--detail' : 'rec-links'} data-recording-links>
      {(['transcript', 'notes', 'folder'] as const).map((t) => (
        <span key={t} className="rec-link-item">
          <button type="button" className="rec-link" data-recording-link={t} onClick={open(t)}>
            {OPEN_LABEL[t]}
          </button>
          {detail && name[t] && (
            <code className="rec-file" data-recording-file={t}>
              {name[t]}
            </code>
          )}
        </span>
      ))}
      {detail && (
        <p className="rec-hint" data-recording-links-hint>
          Databricks opens the folder. Find the highlighted file there.
        </p>
      )}
    </div>
  );
}

const askForList = () => askRecording({ action: 'recordings.ready' });

function startOfDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

function dayLabel(ms: number, now: number): string {
  const day = startOfDay(ms);
  const today = startOfDay(now);
  if (day === today) return 'Today';
  if (Math.round((today - day) / DAY_MS) === 1) return 'Yesterday';
  const sameYear = new Date(ms).getFullYear() === new Date(now).getFullYear();
  return new Date(ms).toLocaleDateString(undefined, {
    day: 'numeric',
    month: 'short',
    ...(sameYear ? {} : { year: 'numeric' }),
  });
}

const timeOf = (ms: number) =>
  new Date(ms).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });

const fullDateOf = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    hour: 'numeric',
    minute: '2-digit',
  });

function groupByDay(rows: RecordingRow[], now: number) {
  const out: { label: string; rows: RecordingRow[] }[] = [];
  for (const r of rows) {
    const label = dayLabel(r.startedAt, now);
    const last = out.at(-1);
    if (last && last.label === label) last.rows.push(r);
    else out.push({ label, rows: [r] });
  }
  return out;
}

type StatusRow = Pick<
  RecordingRow,
  'status' | 'partsUploaded' | 'partsTotal' | 'onDeviceOnly' | 'waitingNetwork'
>;

/** The tone of a status chip: the words differ from the stage for the device and network cases. */
function toneOf(row: StatusRow): string {
  if (row.onDeviceOnly) return 'device';
  if (row.waitingNetwork) return 'waiting';
  return row.status;
}

function Status({ row }: { row: StatusRow }) {
  const busy =
    !row.onDeviceOnly &&
    !row.waitingNetwork &&
    (row.status === 'uploading' || row.status === 'transcribing' || row.status === 'writing_notes');
  return (
    <span className="rec-status" data-status={toneOf(row)} data-recording-status>
      <span className="rec-status-dot" aria-hidden="true" data-busy={busy || undefined} />
      {statusLabelOf(row)}
    </span>
  );
}

const PENDING_TEXT: Partial<Record<RecordingRow['status'], string>> = {
  recording: 'This call is still recording. Bottle uploads it in parts while you talk.',
  uploading: 'Bottle is uploading this call. The notes show here when they are ready.',
  transcribing: 'Bottle is transcribing this call. The notes show here when they are ready.',
  writing_notes: 'Bottle is writing the notes. They show here in a moment.',
};

function Detail({
  row,
  detail,
  onOpenSettings,
}: {
  row: RecordingRow;
  detail: HubRecordingDetail | undefined;
  onOpenSettings: (() => void) | undefined;
}) {
  const d = detail?.id === row.id ? detail : undefined;
  const status = d?.status ?? row.status;
  const error = d?.error ?? row.error;
  const open = () => sendRecording({ action: 'recordings.open', id: row.id });
  const retry = () => sendRecording({ action: 'recordings.retry', id: row.id });
  const canDelete = rowCanDeleteOf({ ...row, status });
  // The id the question is open for: another selection starts with no question open.
  const [confirmId, setConfirmId] = useState<string | undefined>();
  const confirming = confirmId === row.id;
  const setConfirming = (on: boolean) => setConfirmId(on ? row.id : undefined);
  const remove = () => {
    setConfirming(false);
    sendRecording({ action: 'recordings.delete', id: row.id });
  };
  const action = rowActionOf(row.onDeviceOnly ? { status, onDeviceOnly: true } : { status });
  // A failed recording whose error already offers Retry shows it in the alert, once.
  const alertHasRetry = error?.action?.target === 'retry';
  return (
    <article
      className="rec-detail"
      data-recording-detail
      tabIndex={-1}
      aria-labelledby={`rec-t-${row.id}`}
    >
      <header className="rec-detail-head">
        <div>
          <h2 id={`rec-t-${row.id}`} className="rec-detail-title">
            {fullDateOf(row.startedAt)}
          </h2>
          <p className="rec-detail-meta">
            <span>{lengthOf(row.durationS)}</span>
            <Status row={{ ...row, status }} />
          </p>
        </div>
        <div className="rec-actions">
          {action === 'upload' && (
            <button type="button" data-recording-upload onClick={retry}>
              Upload
            </button>
          )}
          {action === 'retry' && !alertHasRetry && (
            <button type="button" data-recording-retry onClick={retry}>
              Retry
            </button>
          )}
          {canDelete && !confirming && (
            <button type="button" data-recording-delete onClick={() => setConfirming(true)}>
              Delete
            </button>
          )}
          {row.canOpen && status !== 'done' && (
            <button type="button" data-recording-open onClick={open}>
              Open in volume
            </button>
          )}
        </div>
      </header>
      {canDelete && confirming && (
        <div
          className="rec-confirm"
          role="group"
          aria-label="Delete this recording?"
          data-recording-confirm
        >
          <p>Delete this recording? It is only on this Mac and cannot be recovered.</p>
          <button
            type="button"
            className="rec-confirm-delete"
            data-recording-delete-yes
            onClick={remove}
            autoFocus
          >
            Delete
          </button>
          <button type="button" data-recording-delete-no onClick={() => setConfirming(false)}>
            Cancel
          </button>
        </div>
      )}
      {status === 'done' && !row.onDeviceOnly && <FileLinks row={row} detail />}
      {row.onDeviceOnly && (
        <p className="rec-pending" data-recording-device>
          This recording is only on this device. Upload it to your volume to get the transcript and
          the notes.
        </p>
      )}
      {row.onDeviceOnly && row.error && (
        <RecordingAlert notice={row.error} onSettings={onOpenSettings} />
      )}
      {row.waitingNetwork && (
        <p className="rec-pending" data-recording-waiting>
          Bottle can't reach your workspace. It keeps the audio on this device and uploads again
          when the network is back.
        </p>
      )}
      {status === 'failed' && error && !row.onDeviceOnly && (
        <RecordingAlert notice={error} onRetry={retry} onOpen={open} onSettings={onOpenSettings} />
      )}
      {!row.onDeviceOnly && !row.waitingNetwork && PENDING_TEXT[status] && (
        <p className="rec-pending">{PENDING_TEXT[status]}</p>
      )}
      {d === undefined && status === 'done' && !row.onDeviceOnly && (
        <div className="rec-skeleton rec-skeleton--tall" aria-busy="true" />
      )}
      {d?.unavailable && (
        <p className="rec-pending">
          Bottle can't read this recording right now. Try again in a moment.
        </p>
      )}
      {d?.notesMd && (
        <section className="rec-notes" aria-label="Notes">
          <h3 className="rec-sub">Notes</h3>
          <div className="rec-md">
            <Notes text={d.notesMd} />
          </div>
        </section>
      )}
      {d && d.turns.length === 0 && d.transcriptMd && (
        <section className="rec-transcript" aria-label="Transcript">
          <h3 className="rec-sub">Transcript</h3>
          <pre className="rec-turn-text" data-transcript-raw>
            {d.transcriptMd}
          </pre>
        </section>
      )}
      {d && d.turns.length > 0 && (
        <section className="rec-transcript" aria-label="Transcript">
          <h3 className="rec-sub">Transcript</h3>
          <ol className="rec-turns">
            {d.turns.map((t, i) => (
              <li key={i} data-turn className="rec-turn">
                <p className="rec-turn-head">
                  <span className="rec-speaker">{t.speaker}</span>
                  <time className="rec-stamp">{stampOf(t.startS)}</time>
                </p>
                <p className="rec-turn-text">{t.text}</p>
              </li>
            ))}
          </ol>
        </section>
      )}
    </article>
  );
}

export function Recordings({
  active,
  now = Date.now,
  header,
  onOpenSettings,
}: {
  active: boolean;
  now?: () => number;
  /** "Open Settings" in an error moves the hub to Settings. */
  onOpenSettings?: () => void;
  /** The Record control, above the list. */
  header?: React.ReactNode;
}) {
  const [list, setList] = useState<HubRecordings | null>(null);
  const [detail, setDetail] = useState<HubRecordingDetail | undefined>();
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const listRef = useRef<HTMLDivElement>(null);
  const baseId = useId();

  const onList = useCallback((p: unknown) => {
    const l = asHubRecordings(p);
    if (l) setList(l);
  }, []);
  const onDetail = useCallback((p: unknown) => {
    const d = asHubRecordingDetail(p);
    if (d) setDetail(d);
  }, []);
  useHubEvent(HUB_RECORDINGS_EVENT, onList, askForList);
  useHubEvent(HUB_RECORDING_DETAIL_EVENT, onDetail);

  // Ask again when the area shows. The first ask goes once the listener is on.
  const shown = useRef(active);
  useEffect(() => {
    if (active && !shown.current) askForList();
    shown.current = active;
  }, [active]);

  const rows = useMemo(() => list?.rows ?? [], [list]);
  const selected = rows.find((r) => r.id === selectedId);
  // With nothing selected, the first row is the active option, so the focus ring has a place.
  const current = selected ?? rows[0];

  // Ask for the detail of the selected recording, and again when its status changes.
  const status = selected?.status;
  const deviceOnly = selected?.onDeviceOnly === true;
  useEffect(() => {
    // A recording that is only on the device has no files in the volume to read.
    if (selectedId && status && !deviceOnly)
      sendRecording({ action: 'recordings.detail', id: selectedId });
  }, [selectedId, status, deviceOnly]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (rows.length === 0) return;
    const i = selected ? rows.indexOf(selected) : -1;
    const to: Record<string, number> = {
      ArrowDown: Math.min(i + 1, rows.length - 1),
      ArrowUp: Math.max(i - 1, 0),
      Home: 0,
      End: rows.length - 1,
    };
    if (e.key in to) {
      e.preventDefault();
      setSelectedId(rows[to[e.key] as number]?.id);
    }
  };

  const groups = groupByDay(rows, now());
  let body;
  if (list === null) {
    body = (
      <div className="rec-panes">
        <div className="rec-list" aria-busy="true" aria-label="Loading recordings">
          {[0, 1, 2].map((i) => (
            <div key={i} className="rec-skeleton rec-skeleton--row" />
          ))}
        </div>
      </div>
    );
  } else if (list.unavailable && rows.length === 0) {
    body = (
      <div className="rec-message" role="status">
        {list.error ? (
          <p className="rec-message-title">{list.error.text}</p>
        ) : (
          <>
            <p className="rec-message-title">Bottle can't read your recordings right now.</p>
            <p className="rec-message-line">They are safe in your volume. Try again in a moment.</p>
          </>
        )}
      </div>
    );
  } else if (rows.length === 0) {
    body = (
      <div className="rec-message" role="status">
        <p className="rec-message-title">No recordings yet.</p>
        <p className="rec-message-line">Record a call. Bottle writes the notes when it ends.</p>
      </div>
    );
  } else {
    body = (
      <div className="rec-panes">
        <div
          ref={listRef}
          className="rec-list"
          role="listbox"
          aria-label="Recordings"
          tabIndex={0}
          aria-activedescendant={current ? `${baseId}-${current.id}` : undefined}
          onKeyDown={onKeyDown}
        >
          {groups.map((g) => (
            <div key={`${g.label}-${g.rows[0]?.id}`} role="group" aria-label={g.label}>
              <div className="rec-day" aria-hidden="true">
                {g.label}
              </div>
              {g.rows.map((r) => (
                <div
                  key={r.id}
                  id={`${baseId}-${r.id}`}
                  role="option"
                  aria-selected={r === selected}
                  data-active={r === current || undefined}
                  data-recording-row={r.id}
                  className="rec-row"
                  onClick={() => {
                    setSelectedId(r.id);
                    listRef.current?.focus();
                  }}
                >
                  <span className="rec-row-main">
                    <span className="rec-row-time">{timeOf(r.startedAt)}</span>
                    <span className="rec-row-len">
                      {lengthOf(r.durationS)}
                      {r.error &&
                        (r.onDeviceOnly || r.status === 'failed' || r.status === 'recording') && (
                          <span className="rec-row-reason" data-recording-reason>
                            {' · '}
                            {r.error.text}
                          </span>
                        )}
                    </span>
                  </span>
                  <span className="rec-row-side">
                    {r.batch ? (
                      <span
                        className="rec-status"
                        data-status={
                          r.batch === 'done'
                            ? 'done'
                            : r.batch === 'failed'
                              ? 'failed'
                              : 'uploading'
                        }
                        data-recording-batch={r.batch}
                      >
                        <span
                          className="rec-status-dot"
                          aria-hidden="true"
                          data-busy={r.batch === 'running' || undefined}
                        />
                        {BATCH_LABEL[r.batch]}
                      </span>
                    ) : (
                      <Status row={r} />
                    )}
                  </span>
                </div>
              ))}
            </div>
          ))}
        </div>
        {selected ? (
          <Detail
            key={selected.id}
            row={selected}
            detail={detail}
            onOpenSettings={onOpenSettings}
          />
        ) : (
          <div className="rec-message">
            <p className="rec-message-line">Pick a recording to read its notes.</p>
          </div>
        )}
      </div>
    );
  }

  return (
    <section
      className="rec-area"
      data-hub-view="recordings"
      aria-label="Recordings"
      hidden={!active}
    >
      {header && <div className="rec-bar">{header}</div>}
      {list?.storageProblem && (
        <StorageBanner reason={list.storageProblem.reason} onOpenSettings={onOpenSettings} />
      )}
      {list?.uploadAll && (
        <UploadAll
          state={list.uploadAll}
          blocked={list.storageProblem ? !list.storageProblem.canUpload : false}
        />
      )}
      {body}
    </section>
  );
}
