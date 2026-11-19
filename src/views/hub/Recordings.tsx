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
  STATUS_LABEL,
  stampOf,
  type HubRecordingDetail,
  type HubRecordings,
  type RecordingRow,
} from '../../shared/recording-view';
import { SafeMarkdown } from '../card/safe-markdown';
import { RecordingAlert } from './RecordControl';
import { sendRecording, useHubEvent } from './recording-ipc';

const DAY_MS = 86_400_000;
/** No host is trusted in recording notes: a link shows as text (SEC-25a). */
const NO_LINKS = { profileHost: '', allowedAppUrls: [] as string[] };

const askForList = () => sendRecording({ action: 'recordings.ready' });

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

function Status({ row }: { row: Pick<RecordingRow, 'status'> }) {
  const busy = row.status !== 'done' && row.status !== 'failed';
  return (
    <span className="rec-status" data-status={row.status}>
      <span className="rec-status-dot" aria-hidden="true" data-busy={busy || undefined} />
      {STATUS_LABEL[row.status]}
    </span>
  );
}

const PENDING_TEXT: Partial<Record<RecordingRow['status'], string>> = {
  recording: 'This call is still recording.',
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
            <Status row={{ status }} />
          </p>
        </div>
        {status !== 'recording' && (
          <button type="button" data-recording-open onClick={open}>
            Open in Databricks
          </button>
        )}
      </header>
      {status === 'failed' && error && (
        <RecordingAlert notice={error} onRetry={retry} onOpen={open} onSettings={onOpenSettings} />
      )}
      {PENDING_TEXT[status] && <p className="rec-pending">{PENDING_TEXT[status]}</p>}
      {d === undefined && status === 'done' && (
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
            <SafeMarkdown text={d.notesMd} trust={NO_LINKS} />
          </div>
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

  // Ask for the detail of the selected recording, and again when its status changes.
  const status = selected?.status;
  useEffect(() => {
    if (selectedId && status) sendRecording({ action: 'recordings.detail', id: selectedId });
  }, [selectedId, status]);

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
        <p className="rec-message-title">Bottle can't read your recordings right now.</p>
        <p className="rec-message-line">They are safe in your volume. Try again in a moment.</p>
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
          aria-activedescendant={selected ? `${baseId}-${selected.id}` : undefined}
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
                  data-recording-row={r.id}
                  className="rec-row"
                  onClick={() => {
                    setSelectedId(r.id);
                    listRef.current?.focus();
                  }}
                >
                  <span className="rec-row-main">
                    <span className="rec-row-time">{timeOf(r.startedAt)}</span>
                    <span className="rec-row-len">{lengthOf(r.durationS)}</span>
                  </span>
                  <span className="rec-row-side">
                    <Status row={r} />
                    {r.status === 'failed' && (
                      <button
                        type="button"
                        className="rec-link"
                        data-recording-retry
                        onClick={(e) => {
                          e.stopPropagation();
                          sendRecording({ action: 'recordings.retry', id: r.id });
                        }}
                      >
                        Retry
                      </button>
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
    <section className="rec-area" aria-label="Recordings" hidden={!active}>
      {header && <div className="rec-bar">{header}</div>}
      {body}
    </section>
  );
}
