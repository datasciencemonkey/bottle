/**
 * An agent thread in Conversations (the MOG agent, slice 2a; plan "agent answers in
 * Conversations"). Thin view (ARC-2, ARC-4): it draws the stored threads of `hub:conversations`
 * (`agentThreads`) and the in-memory turns of `hub:agent` (`HubAgentView`) as one thread, and sends
 * one `AgentAction` for each click with `view_emit`.
 *
 * - Header: "Genie Code in sales · started 10:42 · 3 questions · 124k tokens" and [New session],
 *   or "This session ended. A reply starts a new session." when no session is open at all.
 * - Turns, oldest first: the question, a meta line, the answer of each workspace ("Answer from
 *   sales", open) and then "Note from your MOG" as one small muted block.
 * - The live turn is the card's `TurnBody` with [Stop]. Waiting turns show a muted row with [×].
 * - The in-memory turn wins over the stored turn with the same turn id.
 */
import { ChevronRightIcon, XIcon } from 'lucide-react';
import { motion } from 'motion/react';
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import { HUB_SCOPE, type AgentAction, type HubAgentView } from '../../shared/hub-agent';
import {
  CONVERSATIONS_VIEW_ID,
  type AgentThread,
  type AgentThreadPart,
  type AgentThreadTurn,
} from '../../shared/hub-conversations';
import type { AgentTurnView } from '../../shared/ipc';
import { LINK_OPEN, openInDatabricksUrl } from '../../shared/link-trust';
import {
  NOTE_LABEL,
  OPEN_IN_DATABRICKS,
  QUEUED_LABEL,
  SLOW_COPY,
  TurnBody,
  WAITING_COPY,
  WsChip,
  answerFromText,
  firstAnswerLine,
  stripe,
  tokensText,
  useNow,
} from '../card/AgentTurnCard';
import { SafeMarkdown } from '../card/safe-markdown';
import { Button } from '../ui/button';
import { emitView } from '../view-ipc';

function send(action: AgentAction): void {
  void emitView(HUB_SCOPE, 'click', action).catch(() => undefined);
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

const fullDateOf = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });

export const QUESTION_NOT_SAVED = 'Question not saved';
export const SESSION_ENDED_COPY = 'This session ended. A reply starts a new session.';
/** The queue holds at most this many waiting turns (`HubAgentView.waiting`). */
export const QUEUE_LIMIT = 3;

// --- The thread model: the stored turns and the in-memory turns as one list ---------------------

export type ShownTurn =
  { kind: 'stored'; turn: AgentThreadTurn } | { kind: 'mem'; turn: AgentTurnView };

export interface AgentEntry {
  /** The root UUID of the thread, in lower case. */
  threadId: string;
  /** Undefined only when neither a stored thread nor the Hub knows the MOG. */
  mogId: string | undefined;
  mogName: string;
  mogColorVariant?: string | undefined;
  /** The turns that ended, oldest first. */
  turns: ShownTurn[];
  live?: AgentTurnView | undefined;
  /** The turns that wait, in send order. */
  waiting: AgentTurnView[];
  /** Epoch ms of the newest stored turn, or of the view when nothing is stored yet. */
  at: number;
  /** A turn of this thread is in the live session of the agent. */
  inSession: boolean;
}

export type Outcome = 'answered' | 'failed' | 'stopped' | 'needs_you';

const turnIdOf = (uuid: string) => uuid.replaceAll('-', '').toLowerCase();

export const inProgress = (e: AgentEntry): boolean => e.live !== undefined || e.waiting.length > 0;

/**
 * A turn answered when its own status says so or when any Genie call of it answered: a stopped
 * or timed-out turn can still hold one answer. This is the rule of the brain's `failed` flag for
 * Clear failed (`hub-conversations-wiring.ts`: a `completed` turn row or part); a test keeps the
 * two equal.
 */
const memOutcome = (t: AgentTurnView): Outcome =>
  t.state === 'done' || t.calls.some((c) => c.state === 'ready')
    ? 'answered'
    : t.state === 'needs_you'
      ? 'needs_you'
      : t.state === 'stopped'
        ? 'stopped'
        : 'failed';

const storedOutcome = (t: AgentThreadTurn): Outcome =>
  t.status === 'answered' || t.parts.some((p) => p.status === 'answered') ? 'answered' : t.status;

const outcomeOf = (t: ShownTurn): Outcome =>
  t.kind === 'stored' ? storedOutcome(t.turn) : memOutcome(t.turn);

/** "Stopped" when the last turn stopped, "Failed" when no turn answered, else "answered". */
export function threadOutcome(e: AgentEntry): Outcome {
  const outcomes = e.turns.map(outcomeOf);
  const last = outcomes.at(-1);
  if (last === 'stopped' || last === 'needs_you') return last;
  return outcomes.includes('answered') ? 'answered' : 'failed';
}

/**
 * A finished thread with no answered turn and no turn that waits for the user: what Clear failed
 * removes. A thread that needs the user (a sign-in) is not failed: Settings fixes it.
 */
export const isFailedThread = (e: AgentEntry): boolean => {
  if (inProgress(e) || e.turns.length === 0) return false;
  const outcomes = e.turns.map(outcomeOf);
  return !outcomes.includes('answered') && !outcomes.includes('needs_you');
};

/** The first line of the first answered part of the newest turn that has one, or ''. */
export function turnLine(t: ShownTurn): string {
  if (t.kind === 'mem') return firstAnswerLine(t.turn);
  for (const p of t.turn.parts) {
    const line = p.status === 'answered' ? p.answerMd.split('\n').find((l) => l.trim() !== '') : '';
    if (line !== undefined && line !== '') return line.replace(/^[#>*\-\s]+/, '').trim();
  }
  return '';
}

/** One line for a list: the answer of the newest turn that has one (never the agent note). */
export function threadLine(e: AgentEntry): string {
  for (const t of [...e.turns].reverse()) {
    const line = turnLine(t);
    if (line !== '') return line;
  }
  return '';
}

export function firstQuestion(e: AgentEntry): string {
  const t = e.turns[0] ?? (e.live ? { kind: 'mem' as const, turn: e.live } : undefined);
  const q = t?.turn.question ?? e.waiting[0]?.question ?? '';
  return q === '' ? QUESTION_NOT_SAVED : q;
}

/** The workspace names of the parts of a thread, once each. */
export function workspaceNames(e: AgentEntry): string[] {
  const names = new Set<string>();
  for (const t of e.turns) {
    if (t.kind === 'stored') for (const p of t.turn.parts) names.add(p.workspace.name);
    else for (const c of t.turn.calls) names.add(c.workspace.name);
  }
  return [...names];
}

/**
 * The threads Conversations shows: each stored thread, plus the threads that only the in-memory
 * view knows (a first turn that has not reached the store). `mogId` is the MOG the Hub knows,
 * for a thread that no stored row names yet.
 */
export function buildAgentEntries(
  stored: readonly AgentThread[] | undefined,
  agent: HubAgentView | undefined,
  mogId: string | undefined,
  nowMs: number,
): AgentEntry[] {
  const byId = new Map<string, AgentEntry>();
  for (const t of stored ?? []) {
    const id = t.threadId.toLowerCase();
    byId.set(id, {
      threadId: id,
      mogId: t.mogId,
      mogName: t.mogName,
      mogColorVariant: t.mogColorVariant,
      turns: t.turns.map((turn) => ({ kind: 'stored', turn })),
      waiting: [],
      at: t.turns.at(-1)?.askedAt ?? nowMs,
      inSession: false,
    });
  }
  if (!agent) return [...byId.values()];
  const home = [...byId.values()][0];
  const links = new Map(agent.threads.map((l) => [l.turnId, l.threadId.toLowerCase()]));
  const entryOf = (turnId: string): AgentEntry | undefined => {
    const id = links.get(turnId);
    if (id === undefined) return undefined;
    let e = byId.get(id);
    if (!e) {
      e = {
        threadId: id,
        mogId: home?.mogId ?? mogId,
        mogName: home?.mogName ?? 'Your MOG',
        mogColorVariant: home?.mogColorVariant,
        turns: [],
        waiting: [],
        at: nowMs,
        inSession: false,
      };
      byId.set(id, e);
    }
    if (agent.session.state !== 'none') e.inSession = true;
    return e;
  };
  // Oldest first: `earlier` is newest first. A stored turn with the same id gives way.
  for (const turn of [...agent.earlier].reverse()) {
    const e = entryOf(turn.turnId);
    if (!e) continue;
    const at = e.turns.findIndex(
      (s) => s.kind === 'stored' && turnIdOf(s.turn.turnUuid) === turn.turnId,
    );
    if (at >= 0) e.turns[at] = { kind: 'mem', turn };
    else e.turns.push({ kind: 'mem', turn });
  }
  if (agent.live) {
    const e = entryOf(agent.live.turnId);
    if (e) e.live = agent.live;
  }
  for (const turn of agent.waiting) entryOf(turn.turnId)?.waiting.push(turn);
  return [...byId.values()];
}

/** The meta line of an "In progress" row: a pill or a state, with a live dot while it runs. */
export function liveMeta(e: AgentEntry): { pill?: string; text: string; running: boolean } {
  const t = e.live;
  if (!t) return { pill: QUEUED_LABEL, text: 'Sends next', running: false };
  if (t.state === 'slow') return { text: SLOW_COPY, running: true };
  if (t.state === 'running' && t.calls.length > 0)
    return { text: `Asking · ${t.used} of ${t.budget} Genie calls`, running: true };
  return { text: 'Planning…', running: true };
}

// --- Text moved from the old Agent panel ---------------------------------------------------------

/** "Genie Code in sales · started 10:42 · 3 questions · 124k tokens". */
export function sessionText(v: HubAgentView): string {
  const t = v.live ?? v.waiting[0] ?? v.earlier[0];
  const ws = t?.agentWorkspace.name ?? v.session.agentProfile ?? 'your workspace';
  const parts = [`Genie Code in ${ws}`];
  if (v.session.startedAt !== undefined)
    parts.push(
      `started ${new Date(v.session.startedAt).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`,
    );
  parts.push(plural(v.session.turns, 'question', 'questions'));
  if (v.session.tokens > 0) parts.push(tokensText(v.session.tokens));
  return parts.join(' · ');
}

/** Why the agent cannot run, in one sentence, with where the question goes instead. */
export function notReadyText(g: HubAgentView['genieCode']): string {
  // The brain sends a status with no pinned version while it has not read the real one yet.
  if (g.pinned === '') return 'Checking Genie Code…';
  const home = 'your home workspace';
  const tail = `Questions that name another workspace go to ${home} only.`;
  switch (g.binary) {
    case 'not_installed':
      return `Genie Code is not installed. ${tail}`;
    case 'installing':
      return `Genie Code is still installing. ${tail}`;
    case 'unsupported_os':
      return `Genie Code is not available on this Mac yet. ${tail}`;
    case 'wrong_version':
      return `Genie Code is not the version that Bottle needs. ${tail}`;
    case 'ready':
      break;
  }
  switch (g.agentProfileState) {
    case 'not_eligible':
      return `Genie Code is not available in your agent workspace. ${tail}`;
    case 'signed_out':
      return `Your sign-in to your agent workspace expired. ${tail}`;
    case 'no_home':
      return `Choose a home workspace first. ${tail}`;
    default:
      return `Genie Code cannot run on this profile. ${tail}`;
  }
}

export const agentReady = (g: HubAgentView['genieCode'] | undefined): boolean =>
  g === undefined || (g.binary === 'ready' && g.agentProfileState === 'ready');

export function NewSessionConfirm({
  live,
  onConfirm,
  onCancel,
}: {
  live: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const id = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  // The safe choice has the focus when the question opens.
  useEffect(() => cancel.current?.focus(), []);
  return (
    <div
      role="group"
      aria-labelledby={id}
      data-agent-new-session-confirm
      className="agent-confirm"
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <p id={id} className="m-0">
        {live
          ? 'Start a new session? The live question stops. Your conversations stay.'
          : 'Start a new session? The waiting questions are removed. Your conversations stay.'}
      </p>
      <div className="flex gap-[var(--mog-space-2)]">
        <Button type="button" size="sm" data-confirm-new-session onClick={onConfirm}>
          New session
        </Button>
        <Button ref={cancel} type="button" size="sm" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

/** The copy of a waiting turn: "Sends when the answer above lands" only with an answer above. */
export const SENDS_NEXT_COPY = 'Sends after the current question';

export function WaitingRow({ turn, copy = WAITING_COPY }: { turn: AgentTurnView; copy?: string }) {
  return (
    <li className="agent-queued-row" data-agent-waiting={turn.turnId}>
      <span className="conv-queued-text">
        <span className="agent-waiting-q" title={turn.question}>
          {turn.question}
        </span>
        <span className="agent-sub">{copy}</span>
      </span>
      <button
        type="button"
        className="agent-x"
        data-agent-remove={turn.turnId}
        aria-label={`Remove: ${turn.question}`}
        onClick={(e) => {
          // The next row's button takes the focus, so the queue clears in a row from the keyboard.
          const li = e.currentTarget.closest('li');
          const near = li?.nextElementSibling ?? li?.previousElementSibling;
          const target = near?.querySelector<HTMLElement>('[data-agent-remove]');
          send({ action: 'agent.remove_queued', turnId: turn.turnId });
          if (target) target.focus();
          else
            (e.currentTarget.closest('[data-conversation-detail]') as HTMLElement | null)?.focus();
        }}
      >
        <XIcon className="size-3.5" aria-hidden="true" />
      </button>
    </li>
  );
}

// --- The turns -----------------------------------------------------------------------------------

const openAgentLink = (url: string) =>
  void emitView(HUB_SCOPE, 'click', { target: LINK_OPEN, url }).catch(() => undefined);

/** #523: the hub never opens a URL. The click goes to the brain with the part it belongs to. */
const openPartLink = (url: string, requestId: string) =>
  void emitView(CONVERSATIONS_VIEW_ID, 'click', {
    target: LINK_OPEN,
    url,
    requestId,
  }).catch(() => undefined);

function StoredPart({
  part,
  turnFailed,
}: {
  part: AgentThreadPart;
  /** The turn shows "Failed" already: the part adds no second tag. */
  turnFailed: boolean;
}) {
  const trust = useMemo(
    () => ({ profileHost: part.workspaceHost, allowedAppUrls: [] as string[] }),
    [part.workspaceHost],
  );
  const url = openInDatabricksUrl(part.deepLink, part.workspaceHost);
  const failed = part.status === 'failed';
  return (
    <li className="agent-part" data-agent-part={part.requestId} style={stripe(part.workspace)}>
      <div className="agent-part-head">
        {part.answerMd ? (
          <h3 className="agent-part-title">
            <span className="agent-chip-dot" aria-hidden="true" />
            {answerFromText(part.workspace.name)}
          </h3>
        ) : (
          <WsChip chip={part.workspace} />
        )}
        {failed && !turnFailed && <span className="agent-tag">Failed</span>}
      </div>
      {part.answerMd ? (
        <div
          className="agent-part-body"
          data-answer
          onClick={(e) => {
            if ((e.target as Element).closest('a')) e.preventDefault();
          }}
        >
          <SafeMarkdown
            text={part.answerMd}
            trust={trust}
            onOpenLink={(u) => openPartLink(u, part.requestId)}
          />
        </div>
      ) : (
        <p className="agent-part-body conv-answer--none">
          {failed ? 'Genie could not answer this question.' : 'Genie sent no answer text.'}
        </p>
      )}
      {part.keyNumbers.length > 0 && (
        <ul className="agent-keys" aria-label="Key numbers">
          {part.keyNumbers.map((n, i) => (
            <li key={i}>{n.label ? `${n.text} ${n.label}` : n.text}</li>
          ))}
        </ul>
      )}
      {url !== undefined && (
        <a
          className="agent-databricks"
          href={url}
          rel="noopener noreferrer"
          data-action="open-in-databricks"
          onClick={(e) => {
            e.preventDefault();
            if (e.detail > 1) return;
            openPartLink(url, part.requestId);
          }}
        >
          {OPEN_IN_DATABRICKS}
        </a>
      )}
    </li>
  );
}

/**
 * A turn read from the store: the question, the meta line, the answer of each workspace (open,
 * first) and the note of the MOG after them. The stored turn does not say whether the agent saw
 * every answer, so the answers are always first: a note that only refers to them is one small
 * muted block, never the only thing the user can read.
 */
function StoredTurn({
  turn,
  headingId,
}: {
  turn: AgentThreadTurn;
  headingId?: string | undefined;
}) {
  const [open, setOpen] = useState(true);
  const trust = useMemo(() => ({ profileHost: '', allowedAppUrls: [] as string[] }), []);
  const listId = `agent-parts-${turn.turnUuid}`;
  const hasNote = turn.summaryMd !== '';
  const shown = !hasNote || open;
  const names = [...new Map(turn.parts.map((p) => [p.workspace.name, p.workspace])).values()];
  const word =
    turn.status === 'answered' ? 'Answered' : turn.status === 'stopped' ? 'Stopped' : 'Failed';
  const failed = turn.status === 'failed';
  return (
    <section className="conv-exchange" data-agent-stored-turn={turn.turnUuid}>
      <h2 id={headingId} className="conv-question">
        {turn.question || QUESTION_NOT_SAVED}
      </h2>
      <p className="conv-meta">
        <span
          className={failed ? 'conv-meta-part conv-status--failed' : 'conv-meta-part conv-status'}
        >
          {word}
        </span>
        <time className="conv-meta-part" dateTime={new Date(turn.askedAt).toISOString()}>
          {fullDateOf(turn.askedAt)}
        </time>
        {/* One part: its heading names the workspace already. */}
        {names.length > 0 && turn.parts.length > 1 && (
          <span className="conv-meta-part conv-chips">
            {names.map((c) => (
              <WsChip key={c.name} chip={c} />
            ))}
          </span>
        )}
      </p>
      {turn.parts.length === 0 && !hasNote && (
        <p className="conv-answer conv-answer--none">
          {turn.status === 'stopped'
            ? 'You stopped this question before an answer came.'
            : 'Genie could not answer this question.'}
        </p>
      )}
      {turn.parts.length > 0 && (
        <>
          {hasNote && (
            <button
              type="button"
              className="agent-fold agent-fold--chevron"
              aria-expanded={open}
              aria-controls={listId}
              data-action="parts"
              onClick={() => setOpen((o) => !o)}
            >
              <ChevronRightIcon
                className={open ? 'size-3.5 rotate-90' : 'size-3.5'}
                aria-hidden="true"
              />
              {open
                ? 'Hide answers'
                : `Answers from ${plural(names.length, 'workspace', 'workspaces')}`}
            </button>
          )}
          <ul id={listId} className="agent-parts" aria-label="Answers" hidden={!shown}>
            {turn.parts.map((p) => (
              <StoredPart key={p.requestId} part={p} turnFailed={failed} />
            ))}
          </ul>
        </>
      )}
      {hasNote && (
        <div className="agent-note agent-note--after" data-agent-note>
          <p className="agent-note-label">{NOTE_LABEL}</p>
          <div className="agent-note-body">
            <SafeMarkdown text={turn.summaryMd} trust={trust} />
          </div>
        </div>
      )}
    </section>
  );
}

const openSettings = (
  <Button
    type="button"
    variant="outline"
    size="sm"
    data-agent-open-settings
    onClick={() => send({ action: 'agent.open_settings' })}
  >
    Open Settings
  </Button>
);

/** A turn of this session that ended: the card's body, with its action when it needs the user. */
function MemTurn({
  turn,
  now,
  headingId,
}: {
  turn: AgentTurnView;
  now: number;
  headingId?: string | undefined;
}) {
  return (
    <section className="conv-exchange" data-agent-earlier={turn.turnId} data-state={turn.state}>
      <h2 id={headingId} className="conv-question">
        {turn.question}
      </h2>
      <TurnBody
        turn={turn}
        now={now}
        showQuestion={false}
        onOpenLink={openAgentLink}
        actions={turn.state === 'needs_you' ? openSettings : undefined}
      />
    </section>
  );
}

// --- The detail ----------------------------------------------------------------------------------

export function AgentDetail({
  entry,
  agent,
  active,
  focusRef,
  toolbar,
  composer,
}: {
  entry: AgentEntry;
  agent: HubAgentView | undefined;
  active: boolean;
  focusRef: RefObject<HTMLElement | null>;
  toolbar: ReactNode;
  composer: ReactNode;
}) {
  const sessionLine = useRef<HTMLParagraphElement>(null);
  const [confirm, setConfirm] = useState(false);
  const counting = entry.live?.calls.some((c) => c.state === 'asking') ?? false;
  const now = useNow(active && counting);
  const toSession = useCallback(
    () => requestAnimationFrame(() => sessionLine.current?.focus()),
    [],
  );
  // New session stops a live question and clears the queue: it asks first.
  const needsConfirm = agent?.live !== undefined || (agent?.waiting.length ?? 0) > 0;
  const newSession = () => {
    send({ action: 'agent.new_session' });
    setConfirm(false);
    toSession();
  };
  const first = `agent-q-${entry.threadId}`;

  // A new turn lands below the fold: scroll the pane to its end (not on first show).
  const count = entry.turns.length + entry.waiting.length + (entry.live ? 1 : 0);
  const seen = useRef(count);
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (count > seen.current) {
      const list = scrollRef.current;
      list?.scrollTo?.({ top: list.scrollHeight, behavior: 'smooth' });
    }
    seen.current = count;
  }, [count]);

  const live = entry.live;
  // A thread with only queued turns has no question heading yet: its first waiting turn gives it.
  const queuedOnly = entry.turns.length === 0 && !live && entry.waiting[0] !== undefined;
  // A session is open (for this thread or another one): nothing says "ended" then. A reply joins
  // the open session, so the line would be false.
  const noSession = agent !== undefined && agent.session.state === 'none' && !entry.inSession;
  const newSessionButton = agent !== undefined && entry.inSession && (
    <Button
      type="button"
      variant="outline"
      size="sm"
      data-agent-new-session
      aria-expanded={confirm}
      onClick={() => (needsConfirm ? setConfirm(true) : newSession())}
    >
      New session
    </Button>
  );
  const hasActions = Boolean(newSessionButton) || Boolean(toolbar);
  return (
    <motion.article
      ref={focusRef}
      className="conv-detail"
      data-conversation-detail
      data-agent-thread={entry.threadId}
      tabIndex={-1}
      aria-labelledby={first}
      initial={{ opacity: 0, filter: 'blur(4px)' }}
      animate={{ opacity: 1, filter: 'blur(0px)' }}
      transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
    >
      <div ref={scrollRef} className="conv-thread">
        {(agent !== undefined || hasActions) && (
          <div className="agent-session-row">
            {agent !== undefined && entry.inSession ? (
              <p ref={sessionLine} tabIndex={-1} data-agent-session className="agent-session">
                {sessionText(agent)}
              </p>
            ) : noSession ? (
              <p data-agent-ended className="agent-session">
                {SESSION_ENDED_COPY}
              </p>
            ) : (
              <span />
            )}
            {hasActions && (
              <span className="agent-session-actions">
                {newSessionButton}
                {toolbar}
              </span>
            )}
          </div>
        )}
        {confirm && (
          <NewSessionConfirm
            live={agent?.live !== undefined}
            onConfirm={newSession}
            onCancel={() => setConfirm(false)}
          />
        )}
        {entry.turns.map((t, i) =>
          t.kind === 'stored' ? (
            <StoredTurn
              key={t.turn.turnUuid}
              turn={t.turn}
              headingId={i === 0 ? first : undefined}
            />
          ) : (
            <MemTurn
              key={t.turn.turnId}
              turn={t.turn}
              now={now}
              headingId={i === 0 ? first : undefined}
            />
          ),
        )}
        {queuedOnly && (
          <section className="conv-exchange" data-agent-queued-only>
            <h2 id={first} className="conv-question">
              {entry.waiting[0]?.question}
            </h2>
            <p className="conv-meta">
              <span className="agent-pill">{QUEUED_LABEL}</span>
              <span className="conv-meta-part">{SENDS_NEXT_COPY}</span>
              <button
                type="button"
                className="agent-x"
                data-agent-remove={entry.waiting[0]?.turnId}
                aria-label={`Remove: ${entry.waiting[0]?.question}`}
                onClick={() =>
                  send({ action: 'agent.remove_queued', turnId: entry.waiting[0]?.turnId ?? '' })
                }
              >
                Remove
              </button>
            </p>
          </section>
        )}
        {live && (
          <section className="conv-exchange" data-agent-live aria-label="Live question">
            <h2
              id={entry.turns.length === 0 ? first : undefined}
              className="conv-question"
              title={live.question}
            >
              {live.question}
            </h2>
            <TurnBody
              turn={live}
              now={now}
              showQuestion={false}
              onOpenLink={openAgentLink}
              live
              actions={
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  data-agent-stop={live.turnId}
                  aria-label={`Stop: ${live.question}`}
                  onClick={() => {
                    send({ action: 'agent.stop', turnId: live.turnId });
                    toSession();
                  }}
                >
                  Stop
                </Button>
              }
            />
          </section>
        )}
        {entry.waiting.length > 0 && (
          <ul className="agent-queued conv-queued" aria-label="Waiting questions">
            {entry.waiting.map((t, i) =>
              queuedOnly && i === 0 ? null : (
                <WaitingRow
                  key={t.turnId}
                  turn={t}
                  copy={entry.turns.length > 0 || live || i > 0 ? WAITING_COPY : SENDS_NEXT_COPY}
                />
              ),
            )}
          </ul>
        )}
      </div>
      {composer}
    </motion.article>
  );
}
