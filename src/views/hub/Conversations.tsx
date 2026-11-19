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
} from 'react';
import {
  asConversationsOpen,
  asHubConversations,
  CONVERSATIONS_VIEW_ID,
  HUB_CONVERSATIONS_EVENT,
  HUB_CONVERSATIONS_OPEN_EVENT,
  threadOf,
  type ConversationRow,
  type HubConversations,
  type ThreadComposerThread,
} from '../../shared/hub-conversations';
import { paletteColorOf } from '../../shared/mog-palette';
import { SafeMarkdown } from '../card/safe-markdown';
import { ConversationsSkeleton } from '../ui/motion-effects';
import { emitView } from '../view-ipc';
import { motion } from 'motion/react';

const DAY_MS = 86_400_000;

/** #ui-4: the small colour dot next to a MOG's name, the same colour as its lamp and card. */
function MogDot({ row }: { row: ConversationRow }) {
  const c = paletteColorOf(row.mogColorVariant, row.mogId);
  return (
    <span
      className="conv-mog-dot"
      data-color={c.id}
      style={{ background: c.hex }}
      aria-hidden="true"
    />
  );
}

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
  rows: ConversationRow[];
}

function groupByDay(rows: ConversationRow[], now: number): DayGroup[] {
  const out: DayGroup[] = [];
  for (const r of rows) {
    const label = dayLabel(r.askedAt, now);
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

/** The list the brain sends, and the selection a double-click asks for. */
function useConversations(onOpen: () => void) {
  const [list, setList] = useState<HubConversations | null>(null);
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const openRef = useRef(onOpen);
  useEffect(() => {
    openRef.current = onOpen;
  }, [onOpen]);
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
          if (l) setList(l);
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
  return { list, selectedId, setSelectedId };
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

/** One exchange of a thread: its question, meta line, answer and key numbers (#145). */
function Exchange({ row, headingId }: { row: ConversationRow; headingId?: string | undefined }) {
  const failed = row.status === 'failed';
  const trust = useMemo(
    () => ({ profileHost: row.workspaceHost, allowedAppUrls: [] as string[] }),
    [row.workspaceHost],
  );
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
      {row.answerMd ? (
        // TODO(#523): open a trusted link in the browser once the hub has the link opener. Until then a
        // click never navigates the hub window; the link text stays selectable.
        <div
          className="conv-answer"
          onClick={(e) => {
            if ((e.target as Element).closest('a')) e.preventDefault();
          }}
        >
          <SafeMarkdown text={row.answerMd} trust={trust} />
        </div>
      ) : (
        <p className="conv-answer conv-answer--none">
          {failed ? 'Genie could not answer this question.' : 'Genie sent no answer text.'}
        </p>
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

/**
 * The thread detail (#145, #103): every exchange that shares one Genie `conversationId`, oldest
 * first (question, answer, question, answer), then the reply composer.
 */
function Detail({
  thread,
  focusRef,
  composer,
}: {
  thread: ConversationRow[];
  focusRef: React.RefObject<HTMLElement | null>;
  composer?: ((thread: ThreadComposerThread) => ReactNode) | undefined;
}) {
  const first = thread[0];
  const latest = thread.at(-1);
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
      {thread.map((row) => (
        <Exchange
          key={row.requestId}
          row={row}
          headingId={row === first ? `conv-q-${first.requestId}` : undefined}
        />
      ))}
      <ComposerSlot latest={latest} composer={composer} />
    </motion.article>
  );
}

function Message({ title, line }: { title: string; line?: string }) {
  return (
    <div className="conv-message" role="status">
      <p className="conv-message-title">{title}</p>
      {line && <p className="conv-message-line">{line}</p>}
    </div>
  );
}

export function Conversations({
  active,
  onOpen,
  now = Date.now,
  composer,
}: {
  active: boolean;
  onOpen: () => void;
  now?: () => number;
  /** #145: the reply composer of the selected thread's detail, or no composer at all. */
  composer?: ((thread: ThreadComposerThread) => ReactNode) | undefined;
}) {
  const { list, selectedId, setSelectedId } = useConversations(onOpen);
  const listRef = useRef<HTMLDivElement>(null);
  const detailRef = useRef<HTMLElement | null>(null);
  const baseId = useId();
  const optionId = (requestId: string) => `${baseId}-${requestId}`;

  // Ask again when the area shows (Forget or Wipe may have changed the store). The first ask goes
  // after the listeners are on.
  const shown = useRef(active);
  useEffect(() => {
    if (active && !shown.current) askForList();
    shown.current = active;
  }, [active]);

  const allRows = useMemo(() => list?.rows ?? [], [list]);
  // #145: the list shows one entry per thread (its latest exchange); the detail shows them all.
  const entries = useMemo(() => threadsNewestFirst(allRows), [allRows]);
  const selected = entries.find((r) => r.requestId === selectedId) ?? entries[0];
  const selectedThread = useMemo(
    () => (selected ? threadOf(allRows, selected) : undefined),
    [allRows, selected],
  );

  const select = useCallback(
    (r: ConversationRow | undefined) => {
      if (r) setSelectedId(r.requestId);
    },
    [setSelectedId],
  );

  // Keep the selected option in view.
  useEffect(() => {
    if (!selected) return;
    const el = listRef.current?.querySelector<HTMLElement>(
      `[data-request="${CSS.escape(selected.requestId)}"]`,
    );
    el?.scrollIntoView?.({ block: 'nearest' });
  }, [selected]);

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
    }
  };

  // Read the clock on each render, so "Today" moves on after midnight.
  const groups = groupByDay(entries, now());

  let body;
  if (list === null) {
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
      />
    );
  } else if (entries.length === 0) {
    body = (
      <Message title="No conversations yet." line="Hold Right Option and tap G to ask Genie." />
    );
  } else {
    body = (
      <div className="conv-panes">
        <div
          ref={listRef}
          className="conv-list"
          role="listbox"
          aria-label="Conversations"
          tabIndex={0}
          aria-activedescendant={selected ? optionId(selected.requestId) : undefined}
          onKeyDown={onKeyDown}
        >
          {groups.map((g) => (
            <div key={`${g.label}-${g.rows[0]?.requestId}`} role="group" aria-label={g.label}>
              <div className="conv-day" aria-hidden="true">
                {g.label}
              </div>
              {g.rows.map((r) => {
                const isSel = r === selected;
                return (
                  // `layout` (ui-1, stands in for Magic UI's AnimatedList): the row animates to
                  // its new position instead of jumping, when the list re-sorts or grows.
                  <motion.div
                    key={r.requestId}
                    layout="position"
                    id={optionId(r.requestId)}
                    role="option"
                    aria-selected={isSel}
                    data-request={r.requestId}
                    className="conv-row"
                    onClick={() => {
                      select(r);
                      listRef.current?.focus();
                    }}
                  >
                    <span className="conv-row-q">{r.question || 'Question not saved'}</span>
                    <span className="conv-row-meta">
                      <span className="conv-mog">
                        <MogDot row={r} />
                        {r.mogName}
                      </span>
                      {' · '}
                      {timeOf(r.askedAt)}
                      {r.profile && ` · ${r.profile}`}
                      {r.status === 'failed' && (
                        <>
                          {' · '}
                          <span className="conv-status--failed">Failed</span>
                        </>
                      )}
                    </span>
                  </motion.div>
                );
              })}
            </div>
          ))}
        </div>
        {selectedThread && (
          <Detail
            key={selected?.requestId}
            thread={selectedThread}
            focusRef={detailRef}
            // [review] the area can stay mounted (`hidden`) while the hub shows another view: the
            // composer must unmount with it, so its `thread.active` announcement always goes
            // with it, and a ⌥G never becomes a follow-up in a thread the user cannot see.
            composer={active ? composer : undefined}
          />
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
    </section>
  );
}
