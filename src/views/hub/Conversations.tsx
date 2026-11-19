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
} from 'react';
import {
  asConversationsOpen,
  asHubConversations,
  CONVERSATIONS_VIEW_ID,
  HUB_CONVERSATIONS_EVENT,
  HUB_CONVERSATIONS_OPEN_EVENT,
  type ConversationRow,
  type HubConversations,
} from '../../shared/hub-conversations';
import type { ReactNode } from 'react';
import { SafeMarkdown } from '../card/safe-markdown';
import { ConversationsSkeleton } from '../ui/motion-effects';
import { emitView } from '../view-ipc';
import { motion } from 'motion/react';

/** A thread the composer slot hands to its caller (ui-1, #103): the minimal shape it needs. */
export interface ComposerThread {
  requestId: string;
  profile: string;
}

const DAY_MS = 86_400_000;

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

/**
 * The sticky composer slot (ui-1, #103): an optional `composer` render prop fills it; otherwise
 * it shows only a static hint, never a dead input. impl-followup owns the real composer; this
 * component only owns the slot's layout and fallback.
 */
function ComposerSlot({
  row,
  composer,
}: {
  row: ConversationRow;
  composer?: ((thread: ComposerThread) => ReactNode) | undefined;
}) {
  return (
    <div data-slot="thread-composer" className="conv-composer">
      {composer ? (
        composer({ requestId: row.requestId, profile: row.profile })
      ) : (
        <p className="conv-composer-hint">Hold Right Option and tap G to ask by voice.</p>
      )}
    </div>
  );
}

function Detail({
  row,
  focusRef,
  composer,
}: {
  row: ConversationRow;
  focusRef: React.RefObject<HTMLElement | null>;
  composer?: ((thread: ComposerThread) => ReactNode) | undefined;
}) {
  const failed = row.status === 'failed';
  const trust = useMemo(
    () => ({ profileHost: row.workspaceHost, allowedAppUrls: [] as string[] }),
    [row.workspaceHost],
  );
  return (
    // BlurFade (ui-1, stands in for Magic UI's BlurFade): the newly selected thread fades and
    // un-blurs in, instead of popping in with no transition.
    <motion.article
      ref={focusRef}
      className="conv-detail"
      data-conversation-detail
      tabIndex={-1}
      aria-labelledby={`conv-q-${row.requestId}`}
      initial={{ opacity: 0, filter: 'blur(4px)' }}
      animate={{ opacity: 1, filter: 'blur(0px)' }}
      transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
    >
      <h2 id={`conv-q-${row.requestId}`} className="conv-question">
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
        <span className="conv-meta-part">{row.mogName}</span>
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
      <ComposerSlot row={row} composer={composer} />
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
  /** Fills the sticky composer slot under the detail pane (ui-1, #103); see `ComposerThread`. */
  composer?: ((thread: ComposerThread) => ReactNode) | undefined;
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

  const rows = useMemo(() => list?.rows ?? [], [list]);
  const selected = rows.find((r) => r.requestId === selectedId) ?? rows[0];

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
    if (rows.length === 0) return;
    const i = selected ? rows.indexOf(selected) : 0;
    const to: Record<string, number> = {
      ArrowDown: Math.min(i + 1, rows.length - 1),
      ArrowUp: Math.max(i - 1, 0),
      Home: 0,
      End: rows.length - 1,
    };
    if (e.key in to) {
      e.preventDefault();
      select(rows[to[e.key] as number]);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      detailRef.current?.focus();
    }
  };

  // Read the clock on each render, so "Today" moves on after midnight.
  const groups = groupByDay(rows, now());

  let body;
  if (list === null) {
    // ui-1: a visible skeleton instead of a blank aria-busy div (the ui-audit "loading" finding).
    body = (
      <div className="conv-message conv-message--loading" aria-busy="true">
        <ConversationsSkeleton />
      </div>
    );
  } else if (list.unavailable && rows.length === 0) {
    body = (
      <Message
        title="Bottle cannot read your saved conversations right now."
        line="Your history is still on this device. Try again in a moment."
      />
    );
  } else if (rows.length === 0) {
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
        {selected && (
          <Detail
            key={selected.requestId}
            row={selected}
            focusRef={detailRef}
            composer={composer}
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
