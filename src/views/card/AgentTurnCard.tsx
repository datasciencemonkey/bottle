/**
 * Slice 2a: the view of one agent turn (plan Amendment 2 A2, "What the user sees"). The card
 * shows it in compact form with Stop and "Open in Conversations"; hub Conversations shows the
 * same body for the live turn and for each earlier turn (`AgentThread.tsx`). Thin view (ARC-2): the brain
 * sends the `AgentTurnView`; this file draws it and sends clicks with `view_emit`.
 *
 * - Header: "Planning with Genie Code in sales…", then "Asking 2 workspaces · 3 of 10 Genie
 *   calls", "Still working…" after 4 min, and a fixed line for each end.
 * - One row for each Genie call: colour stripe, workspace chip, the question on one line (the full
 *   text in `title`), and the state (Queued, "Asking finance… 0:12", Ready, No answer, Needs you,
 *   Failed, Stopped). The same names as the direct path and the UX proposal.
 * - The answers of the workspaces come first and open, each under "Answer from <workspace>", when
 *   the turn has no combined note (the cross-workspace answer is off, or the note is missing). The
 *   note of the MOG then comes after them as one small muted block. With a combined note the note
 *   comes first ("Combined by your MOG", with the chips of its sources) and the parts fold under
 *   it; a part withheld from the agent stays in view with "Not shown to the agent".
 *   A part of an earlier turn says so.
 * - A queued turn shows the pill "Queued" and "Sends when the answer above lands".
 * Colours, sizes and curves come from `tokens.css` (`agent-run.css`).
 */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { AFTER_HOVER_MS, CARD_DONE, CARD_MOG_HOVER_EVENT } from '../../shared/genie-status';
import type { MogId } from '../../shared/interfaces';
import type {
  AgentCallView,
  AgentPartView,
  AgentTurnView,
  AgentWsChip,
  CardSection,
} from '../../shared/ipc';
import { LINK_OPEN, openInDatabricksUrl } from '../../shared/link-trust';
import { paletteColorOf } from '../../shared/mog-palette';
import { emitView } from '../view-ipc';
import { SafeMarkdown } from './safe-markdown';
import type { CSSProperties, ReactNode } from 'react';

export const QUEUED_LABEL = 'Queued';
export const WAITING_COPY = 'Sends when the answer above lands';
export const SLOW_COPY = 'Still working…';
export const NOTE_LABEL = 'Note from your MOG';
export const COMBINED_LABEL = 'Combined by your MOG';
export const OPEN_IN_DATABRICKS = 'Open in Databricks';
export const CLOSE_LABEL = 'Close';
export const FROM_EARLIER_COPY = 'From an earlier answer';
export const NOT_SHOWN_COPY = 'Not shown to the agent';
/** A finished card fades after this time with no pointer or focus on it. */
export const DONE_SHOW_MS = 20_000;
/** The fade of a finished card (the same 300 ms as the direct card). */
export const FADE_MS = 300;

export const isLiveState = (s: AgentTurnView['state']): boolean =>
  s === 'planning' || s === 'running' || s === 'slow';
export const isOpenState = (s: AgentTurnView['state']): boolean => s === 'queued' || isLiveState(s);

/** "41k tokens", "850 tokens". */
export function tokensText(n: number): string {
  return n >= 1000 ? `${Math.round(n / 1000)}k tokens` : `${n} tokens`;
}

/** "m:ss". */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** The workspaces that a turn asked, once each. */
const askedNames = (t: AgentTurnView) => [...new Set(t.calls.map((c) => c.workspace.name))];

/** "sales and finance", "sales, finance +1". */
function nameList(names: string[]): string {
  if (names.length <= 2) return names.join(' and ');
  return `${names.slice(0, 2).join(', ')} +${names.length - 2}`;
}

/** The header line of a turn. The reason of an end is the `notice`, not this line. */
export function turnHeadline(t: AgentTurnView): string {
  const asked = askedNames(t).length;
  const calls = `${t.used} of ${t.budget} Genie calls`;
  switch (t.state) {
    case 'queued':
      return QUEUED_LABEL;
    case 'planning':
      return `Planning with Genie Code in ${t.agentWorkspace.name}…`;
    case 'running':
    case 'slow':
      return asked === 0
        ? `Planning with Genie Code in ${t.agentWorkspace.name}…`
        : `Asking ${plural(asked, 'workspace', 'workspaces')} · ${calls}`;
    case 'done': {
      const ready = t.calls.filter((c) => c.state === 'ready');
      if (t.calls.length === 0) return `Answered by Genie Code in ${t.agentWorkspace.name}`;
      const count =
        ready.length < t.calls.length
          ? `${ready.length} of ${t.calls.length} answered`
          : plural(t.calls.length, 'Genie call', 'Genie calls');
      const names = [...new Set(ready.map((c) => c.workspace.name))];
      return names.length === 0
        ? `No answers · ${count}`
        : `Answered from ${nameList(names)} · ${count}`;
    }
    case 'stopped':
      return 'Stopped';
    case 'timed_out':
      return 'Timed out';
    case 'offline':
      return 'Offline';
    case 'failed':
      return 'Could not finish';
    case 'needs_you':
      return 'Needs you';
  }
}

/** The state of a call row as words. */
export function callLabel(c: AgentCallView, now: number): string {
  switch (c.state) {
    case 'queued':
      return 'Queued';
    case 'asking':
      return c.startedAt === undefined
        ? `Asking ${c.workspace.name}…`
        : `Asking ${c.workspace.name}… ${clock(now - c.startedAt)}`;
    case 'ready':
      return 'Ready';
    case 'no_answer':
      return 'No answer';
    case 'needs_you':
      return 'Needs you';
    case 'failed':
      return 'Failed';
    case 'stopped':
      return 'Stopped';
  }
}

/** The footer line of a finished turn. */
export function footerText(t: AgentTurnView): string | undefined {
  if (t.tokens === undefined) return undefined;
  return `Planned by Genie Code in ${t.agentWorkspace.name} · ${tokensText(t.tokens)}`;
}

/** A 1 s tick while something on screen counts time. */
export function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const first = setTimeout(() => setNow(Date.now()), 0);
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      clearTimeout(first);
      clearInterval(t);
    };
  }, [active]);
  return now;
}

export const stripe = (chip: AgentWsChip): CSSProperties =>
  ({ '--agent-ws-color': paletteColorOf(chip.colorVariant, chip.name).hex }) as CSSProperties;

export function WsChip({ chip }: { chip: AgentWsChip }) {
  return (
    <span className="agent-chip" data-agent-chip style={stripe(chip)}>
      <span className="agent-chip-dot" aria-hidden="true" />
      {chip.name}
    </span>
  );
}

export function CallRows({ calls, now }: { calls: AgentCallView[]; now: number }) {
  if (calls.length === 0) return null;
  return (
    <ul className="agent-calls" aria-label="Genie calls">
      {calls.map((c) => (
        <li
          key={c.callId}
          className="agent-call"
          data-agent-call={c.callId}
          data-state={c.state}
          style={stripe(c.workspace)}
        >
          <WsChip chip={c.workspace} />
          <span className="agent-call-question" title={c.question}>
            {c.question}
          </span>
          <span className="agent-call-state">{callLabel(c, now)}</span>
        </li>
      ))}
    </ul>
  );
}

const sectionOf = <I extends CardSection['id']>(sections: CardSection[], id: I) =>
  sections.find((s): s is Extract<CardSection, { id: I }> => s.id === id);

/** The heading of an answer that comes first: "Answer from sales". */
export const answerFromText = (workspace: string): string => `Answer from ${workspace}`;

function Part({
  part,
  onOpenLink,
  titled = false,
}: {
  part: AgentPartView;
  onOpenLink?: ((url: string) => void) | undefined;
  /** The answer has its own heading "Answer from <workspace>" instead of the chip. */
  titled?: boolean;
}) {
  const answer = sectionOf(part.sections, 'answer');
  const keys = sectionOf(part.sections, 'keyNumbers');
  const footer = sectionOf(part.sections, 'footer');
  const trust = useMemo(
    () => ({ profileHost: footer?.workspaceHost ?? '', allowedAppUrls: [] as string[] }),
    [footer?.workspaceHost],
  );
  const url = openInDatabricksUrl(footer?.deepLink, footer?.workspaceHost);
  return (
    <li className="agent-part" data-agent-part={part.callId} style={stripe(part.workspace)}>
      <div className="agent-part-head">
        {titled && answer ? (
          <h3 className="agent-part-title">
            <span className="agent-chip-dot" aria-hidden="true" />
            {answerFromText(part.workspace.name)}
          </h3>
        ) : (
          <WsChip chip={part.workspace} />
        )}
        {part.fromEarlier && <span className="agent-tag">{FROM_EARLIER_COPY}</span>}
        {part.notShownToAgent && <span className="agent-tag">{NOT_SHOWN_COPY}</span>}
      </div>
      {answer && (
        <div className="agent-part-body" data-answer>
          <SafeMarkdown text={answer.md} trust={trust} {...(onOpenLink ? { onOpenLink } : {})} />
        </div>
      )}
      {keys && keys.items.length > 0 && (
        <ul className="agent-keys" aria-label="Key numbers">
          {keys.items.map((k, i) => (
            <li key={i}>{k}</li>
          ))}
        </ul>
      )}
      {url !== undefined && onOpenLink && (
        <a
          className="genie-toggle agent-databricks"
          href={url}
          rel="noopener noreferrer"
          data-action="open-in-databricks"
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onOpenLink(url);
          }}
        >
          {OPEN_IN_DATABRICKS}
        </a>
      )}
    </li>
  );
}

/** The parts of a turn: the cited ones, then the fold "1 more answer". */
export function TurnParts({
  turn,
  onOpenLink,
  answersFirst = false,
  combined = false,
}: {
  turn: AgentTurnView;
  onOpenLink?: ((url: string) => void) | undefined;
  /** Every answer shows open under "Answer from <workspace>"; none folds. */
  answersFirst?: boolean;
  /**
   * The note is a combined answer (D1 on): the parts of the workspaces fold under it, except a
   * part that was withheld from the agent, which stays in view with its tag.
   */
  combined?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const foldIt = (p: AgentPartView) => (combined ? p.notShownToAgent !== true : p.folded === true);
  const shown = turn.parts.filter((p) => answersFirst || !foldIt(p));
  const folded = answersFirst ? [] : turn.parts.filter(foldIt);
  const foldId = `agent-fold-${turn.turnId}`;
  if (turn.parts.length === 0) return null;
  return (
    <>
      {shown.length > 0 && (
        <ul className="agent-parts" aria-label="Answers">
          {shown.map((p) => (
            <Part key={p.callId} part={p} onOpenLink={onOpenLink} titled={answersFirst} />
          ))}
        </ul>
      )}
      {folded.length > 0 && (
        <>
          <button
            type="button"
            className="genie-toggle agent-fold"
            aria-expanded={open}
            aria-controls={foldId}
            data-action="fold"
            onClick={() => setOpen((o) => !o)}
          >
            {open
              ? 'Hide'
              : combined
                ? plural(folded.length, 'workspace answer', 'workspace answers')
                : plural(folded.length, 'more answer', 'more answers')}
          </button>
          <ul id={foldId} className="agent-parts" aria-label="More answers" hidden={!open}>
            {folded.map((p) => (
              <Part key={p.callId} part={p} onOpenLink={onOpenLink} />
            ))}
          </ul>
        </>
      )}
    </>
  );
}

export interface TurnBodyProps {
  turn: AgentTurnView;
  now: number;
  onOpenLink?: ((url: string) => void) | undefined;
  /** The question line is shown (the panel shows it as its own heading). */
  showQuestion?: boolean;
  /** Controls that go in the footer row (the card chips, or the panel's Stop). */
  actions?: ReactNode;
  /** The headline is a live region. Only the live turn (and the card) sets it. */
  live?: boolean;
  /** The caller draws the footer itself (the card keeps it outside its scroll region). */
  footerOutside?: boolean;
}

/** The footer of a turn: the planner and the tokens, then the controls. */
export function TurnFooter({ turn, actions }: { turn: AgentTurnView; actions?: ReactNode }) {
  const footer = footerText(turn);
  if (footer === undefined && !actions) return null;
  return (
    <footer className="agent-foot">
      {footer !== undefined && (
        <span className="agent-foot-text" data-agent-footer>
          {footer}
        </span>
      )}
      {actions}
    </footer>
  );
}

/** The call rows of a turn that ended sit behind one toggle: the answer comes before the process. */
function CallsOf({ turn, now }: { turn: AgentTurnView; now: number }) {
  const [open, setOpen] = useState(false);
  const id = `agent-calls-${turn.turnId}`;
  if (isOpenState(turn.state) || turn.state === 'needs_you' || turn.calls.length === 0)
    return <CallRows calls={turn.calls} now={now} />;
  return (
    <>
      <button
        type="button"
        className="genie-toggle agent-fold"
        aria-expanded={open}
        aria-controls={id}
        data-action="calls"
        onClick={() => setOpen((o) => !o)}
      >
        {`${open ? 'Hide' : 'Show'} ${plural(turn.calls.length, 'Genie call', 'Genie calls')}`}
      </button>
      <div id={id} hidden={!open}>
        <CallRows calls={turn.calls} now={now} />
      </div>
    </>
  );
}

/**
 * The workspace answers come first, open, when the agent has no combined note of them: the turn
 * has no summary, or the summary is the agent's own note (the cross-workspace answer is off).
 * Else (D1 on) the combined note comes first and the parts fold under it.
 */
export const answersFirstOf = (turn: AgentTurnView): boolean =>
  turn.parts.length > 0 && (turn.summary === undefined || turn.summary.kind !== 'combined');

/** The first line of the first answer of a turn, for a list: never the note of the agent. */
export function firstAnswerLine(turn: AgentTurnView): string {
  for (const p of turn.parts) {
    const md = sectionOf(p.sections, 'answer')?.md ?? '';
    const line = md.split('\n').find((l) => l.trim() !== '');
    if (line !== undefined) return line.replace(/^[#>*\-\s]+/, '').trim();
  }
  return '';
}

/** The body of a turn, the same in the card and in hub Conversations. */
export function TurnBody({
  turn,
  now,
  onOpenLink,
  showQuestion = true,
  actions,
  live = false,
  footerOutside = false,
}: TurnBodyProps) {
  const queued = turn.state === 'queued';
  // On an ended turn the answer comes before the process.
  const answerFirst = !isOpenState(turn.state) && turn.state !== 'needs_you';
  const answersFirst = answersFirstOf(turn);
  const combined = turn.summary?.kind === 'combined';
  const note = turn.summary && (
    <div className={answersFirst ? 'agent-note agent-note--after' : 'agent-note'} data-agent-note>
      <p className="agent-note-label">{combined ? COMBINED_LABEL : NOTE_LABEL}</p>
      {combined && turn.summary.workspaces.length > 0 && (
        <p className="agent-note-chips" aria-label="Combined from" data-agent-note-chips>
          {turn.summary.workspaces.map((c) => (
            <WsChip key={c.name} chip={c} />
          ))}
        </p>
      )}
      <div className="agent-note-body">
        <SafeMarkdown
          text={turn.summary.md}
          trust={{ profileHost: '', allowedAppUrls: [] }}
          {...(onOpenLink ? { onOpenLink } : {})}
        />
      </div>
    </div>
  );
  return (
    <div className="agent-turn" data-agent-turn={turn.turnId} data-state={turn.state}>
      {showQuestion && (
        <p className="genie-question agent-question" data-genie-question title={turn.question}>
          {turn.question}
        </p>
      )}
      <p
        className="agent-headline"
        data-agent-headline
        {...(live ? { role: 'status', 'aria-live': 'polite' as const } : {})}
      >
        {queued && <span className="agent-pill">{QUEUED_LABEL}</span>}
        {queued ? <span className="agent-sub">{WAITING_COPY}</span> : turnHeadline(turn)}
        {turn.state === 'slow' && <span className="agent-slow">{SLOW_COPY}</span>}
      </p>
      {turn.notice !== undefined && (
        <p className="agent-notice" data-agent-notice>
          {turn.notice}
        </p>
      )}
      {!answerFirst && <CallsOf turn={turn} now={now} />}
      {!answersFirst && note}
      <TurnParts
        turn={turn}
        onOpenLink={onOpenLink}
        answersFirst={answersFirst}
        combined={combined}
      />
      {answersFirst && note}
      {answerFirst && <CallsOf turn={turn} now={now} />}
      {!footerOutside && <TurnFooter turn={turn} actions={actions} />}
    </div>
  );
}

export interface AgentTurnCardProps {
  mogId: MogId;
  turn: AgentTurnView;
  /** The chips of the compact card: Stop (or Remove), Open Settings, Open in Conversations. */
  chips: { id: string; label: string }[];
  /** The turns that wait under the live one: `agent-queued:<turnId>` chips, the label is the question. */
  queued?: { id: string; label: string }[];
  /** The final height of the card with its frame, so the window can fit it (`fit-window.ts`). */
  onSize?: ((px: number) => Promise<void> | void) | undefined;
  identity?: { name: string; colorVariant?: string } | undefined;
}

/** Listen to one brain event sent to this window only (ARC-5). `fn` must be stable. */
function useWindowEvent(name: string, fn: (payload: unknown) => void) {
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(name, (e) => fn(e.payload), { target: getCurrentWindow().label }).then(
      (u) => {
        if (gone) u();
        else off = u;
      },
    );
    return () => {
      gone = true;
      off?.();
    };
  }, [name, fn]);
}

const chipClass = (c: { id: string; label: string }, turn: AgentTurnView) =>
  // One main action: Stop on a live turn, Open Settings on a turn that needs the user.
  (c.id.startsWith('agent-stop:') && c.label === 'Stop') ||
  (c.id.startsWith('agent-settings:') && turn.state === 'needs_you')
    ? 'popup-action'
    : 'genie-toggle';

/** The card of an agent turn. It never takes focus (ADR 0102). */
export function AgentTurnCard({
  mogId,
  turn,
  chips,
  queued = [],
  onSize,
  identity,
}: AgentTurnCardProps) {
  const ref = useRef<HTMLDivElement>(null);
  const open = isOpenState(turn.state);
  // A card that asks the user for something never fades by itself.
  const sticky = turn.state === 'needs_you';
  const counting = turn.calls.some((c) => c.state === 'asking');
  const now = useNow(counting);
  const accent = identity && paletteColorOf(identity.colorVariant, mogId).hex;
  const [vis, setVis] = useState<'shown' | 'fading'>('shown');

  const openLink = useCallback(
    (url: string) =>
      void emitView(mogId, 'click', { target: LINK_OPEN, url }).catch(() => undefined),
    [mogId],
  );
  const done = useCallback(
    () => void emitView(mogId, 'click', { target: CARD_DONE }).catch(() => undefined),
    [mogId],
  );

  // The window fits the content after each change.
  useLayoutEffect(() => {
    if (!onSize) return;
    const frame = ref.current?.parentElement;
    void onSize(frame ? frame.getBoundingClientRect().height : 0);
  });

  // A finished card fades (300 ms) after 20 s, unless the pointer, the keyboard or the MOG holds it.
  // After a hold it stays at least 2.5 s more.
  const holds = useRef({ pointer: false, focus: false, mog: false });
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const arm = useCallback(
    (ms: number) => {
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        setVis('fading');
        timer.current = setTimeout(done, FADE_MS);
      }, ms);
    },
    [done],
  );
  useEffect(() => {
    clearTimeout(timer.current);
    const h = holds.current;
    if (!open && !sticky && !h.pointer && !h.focus && !h.mog) arm(DONE_SHOW_MS);
    return () => clearTimeout(timer.current);
  }, [open, sticky, arm, turn.turnId, turn.state]);
  const hold = useCallback(
    (via: 'pointer' | 'focus' | 'mog', on: boolean) => {
      const h = holds.current;
      h[via] = on;
      if (h.pointer || h.focus || h.mog) {
        clearTimeout(timer.current);
        setVis('shown');
      } else if (!open && !sticky) arm(AFTER_HOVER_MS);
    },
    [open, sticky, arm],
  );
  const onMogHover = useCallback(
    (p: unknown) => {
      const on = typeof p === 'object' && p !== null && (p as { on?: unknown }).on;
      if (typeof on === 'boolean') hold('mog', on);
    },
    [hold],
  );
  useWindowEvent(CARD_MOG_HOVER_EVENT, onMogHover);

  const chipButtons = (
    <span className="agent-actions">
      {chips.map((c) => (
        <button
          key={c.id}
          type="button"
          className={chipClass(c, turn)}
          data-chip={c.id}
          onClick={() =>
            void emitView(mogId, 'chip_click', { chipId: c.id }).catch(() => undefined)
          }
        >
          {c.label}
        </button>
      ))}
    </span>
  );

  return (
    <div
      ref={ref}
      className="genie genie-answer agent-card"
      data-genie="agent"
      data-agent-card={turn.state}
      data-state={open || sticky ? 'shown' : vis}
      style={accent ? ({ '--mog-id-color': accent } as CSSProperties) : undefined}
      onMouseEnter={() => hold('pointer', true)}
      onMouseLeave={() => hold('pointer', false)}
      onFocus={() => hold('focus', true)}
      onBlur={() => hold('focus', false)}
    >
      {identity && (
        <span className="genie-identity">
          <span className="genie-identity-dot" aria-hidden="true" />
          {identity.name}
        </span>
      )}
      <button
        type="button"
        className="popup-dismiss genie-dismiss"
        data-action="dismiss"
        aria-label={CLOSE_LABEL}
        onClick={done}
      >
        <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
          <path d="M2 2l6 6M8 2l-6 6" />
        </svg>
      </button>
      <div className="agent-card-body" tabIndex={0} aria-label="Agent turn">
        <TurnBody turn={turn} now={now} onOpenLink={openLink} live footerOutside />
      </div>
      {queued.length > 0 && (
        <>
          <p className="agent-sub agent-queued-copy">{WAITING_COPY}</p>
          <ul className="agent-queued" aria-label="Waiting questions">
            {queued.map((q) => {
              const turnId = q.id.slice('agent-queued:'.length);
              return (
                <li key={q.id} className="agent-queued-row" data-agent-queued={turnId}>
                  <span className="agent-pill">{QUEUED_LABEL}</span>
                  <span className="agent-waiting-q" title={q.label}>
                    {q.label}
                  </span>
                  <button
                    type="button"
                    className="genie-toggle agent-x"
                    data-chip={`agent-stop:${turnId}`}
                    aria-label={`Remove from the queue: ${q.label}`}
                    onClick={() =>
                      void emitView(mogId, 'chip_click', { chipId: `agent-stop:${turnId}` }).catch(
                        () => undefined,
                      )
                    }
                  >
                    ×
                  </button>
                </li>
              );
            })}
          </ul>
        </>
      )}
      <div className="agent-card-foot">
        <TurnFooter turn={turn} actions={chipButtons} />
      </div>
    </div>
  );
}
