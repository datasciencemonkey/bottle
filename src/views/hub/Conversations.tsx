/**
 * The hub Conversations area (#103, #40): the local history of Ask Genie. Thin view (ARC-2): it
 * renders the list the brain sends on `hub:conversations` and asks again with
 * `conversations.ready` each time the area shows. It runs no store statement.
 * - A listbox, newest first, under day headers. Up, Down, Home and End move the selection; Enter
 *   moves focus to the detail.
 * - The detail: the question, the meta line, the answer through the card's allowlist markdown
 *   renderer (SEC-25a), and the key numbers.
 * - `hub:conversations.open` (a double-click on the MOG or the card) shows the area and selects
 *   that exchange, also when it comes before the list.
 * - #145: a thread is every exchange that shares one Genie `conversationId`. The list shows one
 *   entry per thread (its latest exchange); the detail shows the whole thread, oldest first,
 *   then the optional reply composer (`composer`), the hub→brain seam for a follow-up question.
 * - Agent threads (the MOG agent) list with the Genie threads: an "In progress" group on top, and
 *   the detail is `AgentDetail`. `hub:conversations.open {threadId}` selects one.
 * - Delete (the button, or Delete or Backspace on the list) and Clear failed hide at once and show
 *   one Undo toast for 8 s. The view sends `conversations.delete` after that, when the area goes
 *   away, or at once when a new delete starts (the Memory pattern).
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react';
import type { HubAgentView } from '../../shared/hub-agent';
import { MEMORY_UNDO_MS } from '../../shared/hub-memory';
import {
  asConversationsOpen,
  asHubConversations,
  CONVERSATIONS_VIEW_ID,
  CONVERSATIONS_DELETE_MAX,
  HUB_CONVERSATIONS_EVENT,
  HUB_CONVERSATIONS_OPEN_EVENT,
  threadOf,
  type ConversationRow,
  type HubConversations,
  type ThreadComposerThread,
} from '../../shared/hub-conversations';
import { paletteColorOf } from '../../shared/mog-palette';
import { Check, Copy, ExternalLink } from 'lucide-react';
import { LINK_OPEN, openInDatabricksUrl } from '../../shared/link-trust';
import { SafeMarkdown } from '../card/safe-markdown';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/tooltip';
import { ConversationsSkeleton } from '../ui/motion-effects';
import { emitView } from '../view-ipc';
import { motion, useReducedMotion } from 'motion/react';
import {
  AgentDetail,
  buildAgentEntries,
  firstQuestion,
  inProgress,
  isFailedThread,
  liveMeta,
  threadLine,
  threadOutcome,
  workspaceNames,
  type AgentEntry,
} from './AgentThread';
import { AgentComposer } from './ThreadComposer';

const DAY_MS = 86_400_000;
/** The most body rows of a table in a thread (the card shows fewer, `MAX_TABLE_ROWS`). */
const CONV_TABLE_ROWS = 200;

/** #ui-4: the small colour dot next to a MOG's name, the same colour as its lamp and card. */
function MogDot({
  row,
}: {
  row: { mogId?: string | undefined; mogName?: string; mogColorVariant?: string | undefined };
}) {
  const c = paletteColorOf(row.mogColorVariant, row.mogId ?? row.mogName ?? '');
  return (
    <span
      className="conv-mog-dot"
      data-color={c.id}
      style={{ background: c.hex }}
      aria-hidden="true"
    />
  );
}

/**
 * While the hub has no list, it asks again after these waits (0.5, 1, 2 and 4 s; the asks go at
 * 0.5, 1.5, 3.5 and 7.5 s). The brain may not listen yet, or the store may be locked.
 */
export const LIST_RETRY_BACKOFF_MS: readonly number[] = [500, 1000, 2000, 4000];
/** After this many ms with no list, the skeleton gives way to a message with Try again. */
export const LIST_GIVE_UP_MS = 10_000;

function askForList(): void {
  void emitView(CONVERSATIONS_VIEW_ID, 'click', { action: 'conversations.ready' }).catch(
    () => undefined,
  );
}

function startOfDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** "Today", "Yesterday", then the short date; the year only when it is not this year. */
function dayLabel(ms: number, now: number): string {
  const day = startOfDay(ms);
  const today = startOfDay(now);
  if (day === today) return 'Today';
  // Round: a daylight-saving day is 23 or 25 hours.
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
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });

interface DayGroup {
  label: string;
  rows: ListEntry[];
}

function groupByDay(rows: ListEntry[], now: number): DayGroup[] {
  const out: DayGroup[] = [];
  for (const r of rows) {
    const label = dayLabel(r.at, now);
    const last = out.at(-1);
    if (last && last.label === label) last.rows.push(r);
    else out.push({ label, rows: [r] });
  }
  return out;
}

/**
 * `all` (newest first), collapsed to its latest exchange of each thread (#145): one list entry
 * per Genie `conversationId`, in the order its newest exchange comes in `all`. A row with no
 * `conversationId` is its own thread, so it never merges with an unrelated blank one.
 */
function threadsNewestFirst(all: ConversationRow[]): ConversationRow[] {
  const seen = new Set<string>();
  const out: ConversationRow[] = [];
  for (const r of all) {
    const key = r.conversationId === '' ? `request:${r.requestId}` : r.conversationId;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(r);
  }
  return out;
}

/** One entry of the list: a Genie thread (its latest exchange) or an agent thread. */
interface ListEntry {
  /** The option id and the selection: a `requestId`, or `a:<threadId>` for an agent thread. */
  id: string;
  /** The key `conversations.delete` takes: `c:`, `r:` or `a:`. */
  deleteKey: string;
  at: number;
  failed: boolean;
  inProgress: boolean;
  row?: ConversationRow;
  agent?: AgentEntry;
}

/** A Genie thread is failed when no exchange of it was answered. */
function genieEntries(all: ConversationRow[]): ListEntry[] {
  const answered = new Set<string>();
  const keyOf = (r: ConversationRow) =>
    r.conversationId === '' ? `request:${r.requestId}` : r.conversationId;
  for (const r of all) if (r.status !== 'failed') answered.add(keyOf(r));
  return threadsNewestFirst(all).map((r) => ({
    id: r.requestId,
    deleteKey: r.conversationId === '' ? `r:${r.requestId}` : `c:${r.conversationId}`,
    at: r.askedAt,
    failed: !answered.has(keyOf(r)),
    inProgress: false,
    row: r,
  }));
}

const agentKey = (threadId: string) => `a:${threadId}`;

/** The list the brain sends, and the selection a double-click asks for. */
function useConversations(onOpen: () => void, onList: () => void) {
  const [list, setList] = useState<HubConversations | null>(null);
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const openRef = useRef(onOpen);
  const listRef = useRef(onList);
  useEffect(() => {
    openRef.current = onOpen;
    listRef.current = onList;
  }, [onOpen, onList]);
  useEffect(() => {
    const offs: (() => void)[] = [];
    let gone = false;
    // ARC-5: only events sent to this window.
    const target = getCurrentWindow().label;
    const keep = (u: () => void) => (gone ? u() : offs.push(u));
    void Promise.all([
      listen<unknown>(
        HUB_CONVERSATIONS_EVENT,
        (e) => {
          const l = asHubConversations(e.payload);
          if (l) {
            setList(l);
            listRef.current();
          }
        },
        { target },
      ).then(keep),
      listen<unknown>(
        HUB_CONVERSATIONS_OPEN_EVENT,
        (e) => {
          const o = asConversationsOpen(e.payload);
          if (!o) return;
          // A double-click selects its exchange, also before the list comes.
          if (o.requestId) setSelectedId(o.requestId);
          // An agent thread opens by its id, also before the store has it.
          else if (o.threadId) setSelectedId(agentKey(o.threadId.toLowerCase()));
          // Ack, so the brain stops holding this open.
          void emitView(CONVERSATIONS_VIEW_ID, 'click', { action: 'conversations.opened' }).catch(
            () => undefined,
          );
          openRef.current();
        },
        { target },
      ).then(keep),
    ]).then(() => {
      if (!gone) askForList();
    });
    return () => {
      gone = true;
      for (const u of offs) u();
    };
  }, []);
  // Never wait on a skeleton forever: ask again with a backoff while there is no list, and give up
  // after LIST_GIVE_UP_MS. A list that arrives at any point ends both (the effect cleans up).
  const [timedOut, setTimedOut] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const waiting = list === null;
  useEffect(() => {
    if (!waiting) return;
    const timers: ReturnType<typeof setTimeout>[] = [];
    let at = 0;
    for (const wait of LIST_RETRY_BACKOFF_MS) {
      at += wait;
      timers.push(setTimeout(askForList, at));
    }
    timers.push(setTimeout(() => setTimedOut(true), LIST_GIVE_UP_MS));
    return () => {
      for (const t of timers) clearTimeout(t);
    };
  }, [waiting, attempt]);
  const tryAgain = useCallback(() => {
    setTimedOut(false);
    setAttempt((n) => n + 1);
    askForList();
  }, []);
  return { list, selectedId, setSelectedId, timedOut: waiting && timedOut, tryAgain };
}

function KeyNumbers({ items }: { items: ConversationRow['keyNumbers'] }) {
  if (items.length === 0) return null;
  return (
    <ul className="conv-numbers" aria-label="Key numbers">
      {items.map((n, i) => (
        <li key={i} data-key-number>
          <span className="conv-number">{n.text}</span>
          {n.label && <span className="conv-number-label">{n.label}</span>}
        </li>
      ))}
    </ul>
  );
}

/** How long the "Copy Markdown" button shows its check mark (ADR 0124 amendment). An optimistic,
 * view-local state: the direct answer has no brain-sent copy note to reuse (unlike the agent
 * turn's `copyNote`, `copy-answer-note.tsx`). */
const COPY_MARKDOWN_CHECK_MS = 1_500;
export const COPY_MARKDOWN_LABEL = 'Copy as Markdown';

/** One exchange of a thread: its question, meta line, answer and key numbers (#145). */
function Exchange({ row, headingId }: { row: ConversationRow; headingId?: string | undefined }) {
  const failed = row.status === 'failed';
  const trust = useMemo(
    () => ({ profileHost: row.workspaceHost, allowedAppUrls: [] as string[] }),
    [row.workspaceHost],
  );
  const databricksUrl = openInDatabricksUrl(row.deepLink, row.workspaceHost);
  // #523: the hub never opens a URL. The click goes to the brain with the exchange it belongs to;
  // the brain finds the workspace itself, checks the URL again, and Rust opens it.
  const openLink = useCallback(
    (url: string) => {
      void emitView(CONVERSATIONS_VIEW_ID, 'click', {
        target: LINK_OPEN,
        url,
        requestId: row.requestId,
      }).catch(() => undefined);
    },
    [row.requestId],
  );
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(copyTimer.current), []);
  const copyMarkdown = useCallback(() => {
    void emitView(CONVERSATIONS_VIEW_ID, 'click', {
      action: 'conversations.copy_markdown',
      requestId: row.requestId,
    }).catch(() => undefined);
    setCopied(true);
    clearTimeout(copyTimer.current);
    copyTimer.current = setTimeout(() => setCopied(false), COPY_MARKDOWN_CHECK_MS);
  }, [row.requestId]);
  return (
    <div className="conv-exchange" data-exchange={row.requestId}>
      <h2 id={headingId} className="conv-question">
        {row.question || 'Question not saved'}
      </h2>
      {/* The parts never break inside; the dot between them is drawn in CSS. */}
      <p className="conv-meta">
        <span
          className={failed ? 'conv-meta-part conv-status--failed' : 'conv-meta-part conv-status'}
        >
          {failed ? 'Failed' : 'Answered'}
        </span>
        <time className="conv-meta-part" dateTime={new Date(row.askedAt).toISOString()}>
          {fullDateOf(row.askedAt)}
        </time>
        {/* One profile is one workspace; the host is in the tooltip. */}
        <span className="conv-meta-part" title={row.workspaceHost || undefined}>
          {row.profile}
          {row.workspaceHost && <span className="conv-sr">, {row.workspaceHost}</span>}
        </span>
        <span className="conv-meta-part conv-mog">
          <MogDot row={row} />
          {row.mogName}
        </span>
      </p>
      {databricksUrl !== undefined && (
        <p className="conv-open">
          <a
            className="conv-databricks"
            data-action="open-in-databricks"
            href={databricksUrl}
            rel="noopener noreferrer"
            title={databricksUrl}
            onClick={(e) => {
              e.preventDefault();
              if (e.detail > 1) return;
              openLink(databricksUrl);
            }}
          >
            Open in Databricks
            <ExternalLink size={11} strokeWidth={2} aria-hidden="true" />
          </a>
        </p>
      )}
      {row.answerMd ? (
        // #523: a trusted link goes to the brain (`openLink`); a click never navigates the hub.
        <div
          className="conv-answer"
          onClick={(e) => {
            if ((e.target as Element).closest('a')) e.preventDefault();
          }}
        >
          <SafeMarkdown
            text={row.answerMd}
            trust={trust}
            onOpenLink={openLink}
            maxTableRows={CONV_TABLE_ROWS}
          />
        </div>
      ) : (
        <p className="conv-answer conv-answer--none">
          {failed ? 'Genie could not answer this question.' : 'Genie sent no answer text.'}
        </p>
      )}
      {!failed && row.answerMd !== '' && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="size-6"
              data-action="copy-markdown"
              aria-label={COPY_MARKDOWN_LABEL}
              onClick={copyMarkdown}
            >
              {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{COPY_MARKDOWN_LABEL}</TooltipContent>
        </Tooltip>
      )}
      <KeyNumbers items={row.keyNumbers} />
    </div>
  );
}

/**
 * The sticky composer slot (ui-1, #103): an optional `composer` render prop fills it. A thread
 * with no Genie conversation to continue (a row saved before #584), or a build with no composer,
 * shows only a static hint, never a dead input.
 */
function ComposerSlot({
  latest,
  composer,
}: {
  latest: ConversationRow;
  composer?: ((thread: ThreadComposerThread) => ReactNode) | undefined;
}) {
  return (
    <div data-slot="thread-composer" className="conv-composer">
      {composer && latest.conversationId !== '' ? (
        composer({
          conversationId: latest.conversationId,
          exchangeId: latest.requestId,
          profile: latest.profile,
        })
      ) : (
        <p className="conv-composer-hint">Hold Right Option and tap G to ask by voice.</p>
      )}
    </div>
  );
}

/** The Delete button at the top right of a detail. A thread in progress has none: stop it first. */
function DetailToolbar({
  onDelete,
  hint,
  bare = false,
}: {
  onDelete: () => void;
  hint: string;
  /** No bar around the button: an agent thread puts it in its session row. */
  bare?: boolean;
}) {
  const button = (
    <Button
      type="button"
      variant="outline"
      size="sm"
      data-action="delete"
      title={hint}
      aria-label="Delete conversation"
      onClick={onDelete}
    >
      Delete
    </Button>
  );
  return bare ? button : <div className="conv-toolbar conv-detail-bar">{button}</div>;
}

const DELETE_HINT_TAIL =
  "stays in Genie until you Wipe or Forget with 'Also delete Genie conversations'.";
const GENIE_DELETE_HINT = `Deletes it from this Mac. The Genie conversation ${DELETE_HINT_TAIL}`;
const AGENT_DELETE_HINT = `Deletes it from this Mac. The Genie conversations ${DELETE_HINT_TAIL.replace('stays', 'stay')}`;

/**
 * The thread detail (#145, #103): every exchange that shares one Genie `conversationId`, oldest
 * first (question, answer, question, answer), then the reply composer.
 */
function Detail({
  thread,
  focusRef,
  composer,
  toolbar,
}: {
  thread: ConversationRow[];
  focusRef: React.RefObject<HTMLElement | null>;
  composer?: ((thread: ThreadComposerThread) => ReactNode) | undefined;
  toolbar: ReactNode;
}) {
  const first = thread[0];
  const latest = thread.at(-1);
  // A new exchange lands below the fold: scroll the pane to its end so the new answer and the
  // composer show (not on first show, which starts at the question).
  const count = thread.length;
  const seen = useRef(count);
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (count > seen.current) {
      const list = scrollRef.current;
      list?.scrollTo?.({ top: list.scrollHeight, behavior: 'smooth' });
    }
    seen.current = count;
  }, [count]);
  if (!first || !latest) return null;
  return (
    // BlurFade (ui-1): the newly selected thread fades and un-blurs in.
    <motion.article
      ref={focusRef}
      className="conv-detail"
      data-conversation-detail
      tabIndex={-1}
      aria-labelledby={`conv-q-${first.requestId}`}
      initial={{ opacity: 0, filter: 'blur(4px)' }}
      animate={{ opacity: 1, filter: 'blur(0px)' }}
      transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
    >
      {toolbar}
      <div ref={scrollRef} className="conv-thread">
        {thread.map((row) => (
          <Exchange
            key={row.requestId}
            row={row}
            headingId={row === first ? `conv-q-${first.requestId}` : undefined}
          />
        ))}
      </div>
      <ComposerSlot latest={latest} composer={composer} />
    </motion.article>
  );
}

function Message({
  title,
  line,
  action,
  focusRef,
}: {
  title: string;
  line?: string;
  action?: ReactNode;
  /** The empty state takes the focus when a Clear failed emptied the list. */
  focusRef?: RefObject<HTMLDivElement | null>;
}) {
  return (
    <div
      ref={focusRef}
      className="conv-message"
      role="status"
      {...(focusRef ? { tabIndex: -1 } : {})}
    >
      <p className="conv-message-title">{title}</p>
      {line && <p className="conv-message-line">{line}</p>}
      {action}
    </div>
  );
}

/** One thread waiting for its 8 s: the keys to delete and the timer. */
interface Pending {
  keys: string[];
  /** Clear failed: the brain skips a thread that was answered since. */
  onlyFailed?: true;
  timer: ReturnType<typeof setTimeout>;
  /** The thread to select again on Undo. */
  restore: string | undefined;
}

let toastKey = 0;

/** The row of one agent thread in the list: the question and one meta line. */
function AgentRowBody({ entry }: { entry: AgentEntry }) {
  const names = workspaceNames(entry);
  const outcome = threadOutcome(entry);
  if (inProgress(entry)) {
    const m = liveMeta(entry);
    return (
      <>
        <span className="conv-row-q">{firstQuestion(entry)}</span>
        <span className="conv-row-meta">
          {m.running && <span className="conv-live-dot" data-live-dot aria-hidden="true" />}
          {m.pill && <span className="agent-pill">{m.pill}</span>}
          {m.text}
        </span>
      </>
    );
  }
  return (
    <>
      <span className="conv-row-q">{firstQuestion(entry)}</span>
      <span className="conv-row-meta">
        <span className="conv-mog">
          <MogDot row={entry} />
          {entry.mogName}
        </span>
        {' · '}
        {timeOf(entry.at)}
        {names.length > 0 && ` · ${names.join(', ')}`}
        {outcome !== 'answered' && (
          <>
            {' · '}
            <span
              className={
                outcome === 'failed'
                  ? 'conv-status--failed'
                  : outcome === 'needs_you'
                    ? 'conv-status--needs'
                    : undefined
              }
            >
              {outcome === 'failed' ? 'Failed' : outcome === 'needs_you' ? 'Needs you' : 'Stopped'}
            </span>
          </>
        )}
      </span>
    </>
  );
}

export function Conversations({
  active,
  onOpen,
  now = Date.now,
  composer,
  agent,
  agentMogId,
}: {
  active: boolean;
  onOpen: () => void;
  now?: () => number;
  /** #145: the reply composer of the selected thread's detail, or no composer at all. */
  composer?: ((thread: ThreadComposerThread) => ReactNode) | undefined;
  /** The orchestrator agent view (`hub:agent`): the in-memory turns of the agent threads. */
  agent?: HubAgentView | undefined;
  /** The MOG that the Hub knows, for a follow-up in a thread that the store does not have yet. */
  agentMogId?: string | undefined;
}) {
  // The keys of the threads that the view hides: the pending ones (8 s) and the ones sent, until
  // the next list shows the truth. A list that comes in the 8 s never shows a pending one again.
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set());
  const sentRef = useRef<ReadonlySet<string>>(new Set());
  const onList = useCallback(() => {
    const sent = sentRef.current;
    if (sent.size === 0) return;
    sentRef.current = new Set();
    setHidden((h) => new Set([...h].filter((k) => !sent.has(k))));
  }, []);
  const { list, selectedId, setSelectedId, timedOut, tryAgain } = useConversations(onOpen, onList);
  const listRef = useRef<HTMLDivElement>(null);
  const detailRef = useRef<HTMLElement | null>(null);
  const baseId = useId();
  const optionId = (id: string) => `${baseId}-${id}`;
  const reduce = useReducedMotion();

  // Ask again when the area shows (Forget or Wipe may have changed the store). The first ask goes
  // after the listeners are on.
  const shown = useRef(active);
  useEffect(() => {
    if (active && !shown.current) askForList();
    shown.current = active;
  }, [active]);

  const allRows = useMemo(() => list?.rows ?? [], [list]);
  const storedAgent = list?.agentThreads;
  const agentList = useMemo(
    () => buildAgentEntries(storedAgent, agent, agentMogId, now()),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the clock only stamps a thread the store lacks
    [storedAgent, agent, agentMogId],
  );
  // #145: the list shows one entry per thread (its latest exchange); the detail shows them all.
  // The agent threads in progress come first (their own group), then all threads newest first.
  const { running, entries } = useMemo(() => {
    const agents: ListEntry[] = agentList.map((a) => ({
      id: agentKey(a.threadId),
      deleteKey: agentKey(a.threadId),
      at: a.at,
      failed: isFailedThread(a),
      inProgress: inProgress(a),
      agent: a,
    }));
    const geniePart = genieEntries(allRows).filter((e) => !hidden.has(e.deleteKey));
    const agentPart = agents.filter((e) => !hidden.has(e.deleteKey)).sort((a, b) => b.at - a.at);
    const run = agentPart.filter((e) => e.inProgress);
    // Both lists are newest first: merge them and keep the order the brain gave the Genie threads.
    const rest: ListEntry[] = [];
    const later = agentPart.filter((e) => !e.inProgress);
    let g = 0;
    let a = 0;
    while (g < geniePart.length || a < later.length) {
      const ge = geniePart[g];
      const ag = later[a];
      if (ge && (!ag || ge.at >= ag.at)) {
        rest.push(ge);
        g += 1;
      } else if (ag) {
        rest.push(ag);
        a += 1;
      }
    }
    return { running: run, entries: [...run, ...rest] };
  }, [allRows, agentList, hidden]);
  const selected = entries.find((e) => e.id === selectedId) ?? entries[0];
  const emptyRef = useRef<HTMLDivElement>(null);
  const selectedThread = useMemo(
    () => (selected?.row ? threadOf(allRows, selected.row) : undefined),
    [allRows, selected],
  );
  const failedEntries = entries.filter((e) => e.failed && !e.inProgress);

  const select = useCallback(
    (e: ListEntry | undefined) => {
      if (e) setSelectedId(e.id);
    },
    [setSelectedId],
  );

  // Keep the selected option in view.
  useEffect(() => {
    if (!selected) return;
    const el = listRef.current?.querySelector<HTMLElement>(
      `[data-request="${CSS.escape(selected.id)}"]`,
    );
    el?.scrollIntoView?.({ block: 'nearest' });
  }, [selected]);

  // --- Delete and Undo (the Memory pattern): hide at once, send after 8 s ---
  const pendingRef = useRef<Pending | undefined>(undefined);
  const [toast, setToast] = useState<{ key: number; text: string } | undefined>(undefined);
  const flush = useCallback(() => {
    const p = pendingRef.current;
    if (!p) return;
    pendingRef.current = undefined;
    clearTimeout(p.timer);
    sentRef.current = new Set([...sentRef.current, ...p.keys]);
    for (let i = 0; i < p.keys.length; i += CONVERSATIONS_DELETE_MAX) {
      void emitView(CONVERSATIONS_VIEW_ID, 'click', {
        action: 'conversations.delete',
        threads: p.keys.slice(i, i + CONVERSATIONS_DELETE_MAX),
        ...(p.onlyFailed ? { onlyFailed: true } : {}),
      }).catch(() => undefined);
    }
    setToast(undefined);
  }, []);
  // The delete goes when the area goes away or the window closes: the user asked for it.
  useEffect(() => {
    window.addEventListener('pagehide', flush);
    return () => {
      window.removeEventListener('pagehide', flush);
      flush();
    };
  }, [flush]);

  const startDelete = (
    keys: string[],
    text: string,
    next: ListEntry | undefined,
    onlyFailed?: true,
  ) => {
    if (keys.length === 0) return;
    // A new delete sends the one before it at once; one toast at a time.
    flush();
    const restore = selected?.id;
    pendingRef.current = {
      keys,
      ...(onlyFailed ? { onlyFailed } : {}),
      restore,
      timer: setTimeout(flush, MEMORY_UNDO_MS),
    };
    setHidden((h) => new Set([...h, ...keys]));
    toastKey += 1;
    setToast({ key: toastKey, text });
    setSelectedId(next?.id);
    // The row is gone: the keyboard stays in the list.
    listRef.current?.focus();
  };

  const undoDelete = () => {
    const p = pendingRef.current;
    if (!p) return;
    pendingRef.current = undefined;
    clearTimeout(p.timer);
    setHidden((h) => new Set([...h].filter((k) => !p.keys.includes(k))));
    setToast(undefined);
    if (p.restore !== undefined) setSelectedId(p.restore);
    listRef.current?.focus();
  };

  const deleteSelected = () => {
    if (!selected || selected.inProgress) return;
    const i = entries.indexOf(selected);
    startDelete([selected.deleteKey], 'Conversation deleted', entries[i + 1] ?? entries[i - 1]);
  };

  // Clear failed emptied the list: the focus goes to the empty state, not to the body.
  const emptied = list !== null && entries.length === 0 && toast !== undefined;
  useEffect(() => {
    if (emptied) emptyRef.current?.focus();
  }, [emptied]);

  const clearFailed = () => {
    const keys = failedEntries.map((e) => e.deleteKey);
    const n = keys.length;
    const gone = new Set(keys);
    startDelete(
      keys,
      n === 1 ? '1 failed conversation cleared' : `${n} failed conversations cleared`,
      entries.find((e) => !gone.has(e.deleteKey) && e.id === selected?.id) ??
        entries.find((e) => !gone.has(e.deleteKey)),
      true,
    );
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (entries.length === 0) return;
    const i = selected ? entries.indexOf(selected) : 0;
    const to: Record<string, number> = {
      ArrowDown: Math.min(i + 1, entries.length - 1),
      ArrowUp: Math.max(i - 1, 0),
      Home: 0,
      End: entries.length - 1,
    };
    if (e.key in to) {
      e.preventDefault();
      select(entries[to[e.key] as number]);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      detailRef.current?.focus();
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      deleteSelected();
    }
  };

  // Read the clock on each render, so "Today" moves on after midnight.
  const groups = groupByDay(
    entries.filter((e) => !e.inProgress),
    now(),
  );

  const optionOf = (r: ListEntry) => {
    const isSel = r === selected;
    return (
      // `layout` (ui-1, stands in for Magic UI's AnimatedList): the row animates to
      // its new position instead of jumping, when the list re-sorts or grows.
      <motion.div
        key={r.id}
        layout="position"
        id={optionId(r.id)}
        role="option"
        aria-selected={isSel}
        data-request={r.id}
        {...(r.inProgress ? { 'data-in-progress': '' } : {})}
        {...(r.agent && !r.inProgress && threadLine(r.agent) !== ''
          ? { title: threadLine(r.agent) }
          : {})}
        className="conv-row"
        onClick={() => {
          select(r);
          listRef.current?.focus();
        }}
      >
        {r.agent ? (
          <AgentRowBody entry={r.agent} />
        ) : r.row ? (
          <>
            <span className="conv-row-q">{r.row.question || 'Question not saved'}</span>
            <span className="conv-row-meta">
              <span className="conv-mog">
                <MogDot row={r.row} />
                {r.row.mogName}
              </span>
              {' · '}
              {timeOf(r.row.askedAt)}
              {r.row.profile && ` · ${r.row.profile}`}
              {r.row.status === 'failed' && (
                <>
                  {' · '}
                  <span className="conv-status--failed">Failed</span>
                </>
              )}
            </span>
          </>
        ) : null}
      </motion.div>
    );
  };

  // The Undo toast sits in the list column, so it never covers the composer (P1-3). With no list
  // column (the empty and error states) it sits at the foot of the area.
  const toastLive = (
    <div role="status" className="conv-toast-live">
      {toast !== undefined && (
        <motion.div
          key={toast.key}
          className="conv-toast"
          data-conversations-undo
          initial={reduce ? false : { opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
        >
          <span>{toast.text}</span>
          <Button
            type="button"
            variant="link"
            size="sm"
            className="text-[var(--mog-lid)] hover:text-[var(--mog-text)]"
            data-action="undo"
            onClick={undoDelete}
          >
            Undo
          </Button>
        </motion.div>
      )}
    </div>
  );
  let inListColumn = false;
  let body;
  if (list === null && timedOut) {
    body = (
      <Message
        title="Bottle cannot read your saved conversations right now."
        line="Your history is still on this device. Try again in a moment."
        action={
          <button type="button" className="conv-message-action" onClick={tryAgain}>
            Try again
          </button>
        }
      />
    );
  } else if (list === null) {
    // ui-1: a visible skeleton instead of a blank aria-busy div (the ui-audit "loading" finding).
    body = (
      <div className="conv-message conv-message--loading" aria-busy="true">
        <ConversationsSkeleton />
      </div>
    );
  } else if (list.unavailable && entries.length === 0) {
    body = (
      <Message
        title="Bottle cannot read your saved conversations right now."
        line="Your history is still on this device. Try again in a moment."
        action={
          <button type="button" className="conv-message-action" onClick={tryAgain}>
            Try again
          </button>
        }
      />
    );
  } else if (entries.length === 0) {
    body = (
      <Message
        title="No conversations yet."
        line="Hold Right Option and tap G to ask Genie."
        focusRef={emptyRef}
      />
    );
  } else {
    const detailKey = selected?.row
      ? selected.row.conversationId || selected.row.requestId
      : selected?.id;
    inListColumn = true;
    body = (
      <div className="conv-panes">
        <div className="conv-listcol">
          {failedEntries.length > 0 && (
            <div className="conv-toolbar conv-list-bar">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="text-muted-foreground hover:text-foreground"
                data-action="clear-failed"
                onClick={clearFailed}
              >
                {`Clear failed (${failedEntries.length})`}
              </Button>
            </div>
          )}
          <div
            ref={listRef}
            className={toast !== undefined ? 'conv-list conv-list-toast' : 'conv-list'}
            role="listbox"
            aria-label="Conversations"
            tabIndex={0}
            aria-activedescendant={selected ? optionId(selected.id) : undefined}
            onKeyDown={onKeyDown}
          >
            {running.length > 0 && (
              <div role="group" aria-label="In progress">
                <div className="conv-day" aria-hidden="true">
                  In progress
                </div>
                {running.map(optionOf)}
              </div>
            )}
            {groups.map((g) => (
              <div key={`${g.label}-${g.rows[0]?.id}`} role="group" aria-label={g.label}>
                <div className="conv-day" aria-hidden="true">
                  {g.label}
                </div>
                {g.rows.map(optionOf)}
              </div>
            ))}
          </div>
          {toastLive}
        </div>
        {selected?.agent ? (
          <AgentDetail
            // One thread keeps one pane.
            key={detailKey}
            entry={selected.agent}
            agent={agent}
            active={active}
            focusRef={detailRef}
            toolbar={
              selected.inProgress ? null : (
                <DetailToolbar bare onDelete={deleteSelected} hint={AGENT_DELETE_HINT} />
              )
            }
            // The composer unmounts with the area, so its `thread.active_agent` announcement
            // always goes with it (the same rule as the Genie composer below).
            composer={
              active ? (
                <div data-slot="thread-composer" className="conv-composer">
                  <AgentComposer
                    threadId={selected.agent.threadId}
                    mogId={selected.agent.mogId ?? agentMogId}
                    agent={agent}
                  />
                </div>
              ) : null
            }
          />
        ) : (
          selectedThread && (
            <Detail
              // One thread keeps one pane: a new exchange (the latest `requestId`) must not remount
              // it, which would reset its scroll and re-announce the thread to the brain.
              key={detailKey}
              thread={selectedThread}
              focusRef={detailRef}
              toolbar={<DetailToolbar onDelete={deleteSelected} hint={GENIE_DELETE_HINT} />}
              // [review] the area can stay mounted (`hidden`) while the hub shows another view: the
              // composer must unmount with it, so its `thread.active` announcement always goes
              // with it, and a ⌥G never becomes a follow-up in a thread the user cannot see.
              composer={active ? composer : undefined}
            />
          )
        )}
      </div>
    );
  }

  return (
    <section
      className="conv"
      data-hub-view="conversations"
      aria-label="Conversations"
      hidden={!active}
    >
      {body}
      {!inListColumn && toastLive}
    </section>
  );
}
