/**
 * Slice 2a: the view of one agent turn (plan Amendment 2 A2, "What the user sees"). The card
 * shows it in compact form with Stop and "Open in Agent"; the hub Agent panel shows the same body
 * for the live turn and for each earlier turn (`AgentPanel.tsx`). Thin view (ARC-2): the brain
 * sends the `AgentTurnView`; this file draws it and sends clicks with `view_emit`.
 *
 * - Header: "Planning with Genie Code in sales…", then "Asking 2 workspaces · 3 of 10 Genie
 *   calls", "Still working…" after 4 min, and a fixed line for each end.
 * - One row for each Genie call: colour stripe, workspace chip, the question on one line (the full
 *   text in `title`), and the state (Queued, "Asking finance… 0:12", Ready, No answer, Needs you,
 *   Failed, Stopped). The same names as the direct path and the UX proposal.
 * - "Note from your MOG": the model's note, muted. Then the parts, each with its chip. A part that
 *   no source cites folds into "1 more answer". A part of an earlier turn says so.
 * - A queued turn shows the pill "Queued" and "Sends when the answer above lands".
 * Colours, sizes and curves come from `tokens.css` (`agent-run.css`).
 */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { CARD_DONE } from '../../shared/genie-status';
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
export const OPEN_IN_DATABRICKS = 'Open in Databricks';
export const CLOSE_LABEL = 'Close';
export const FROM_EARLIER_COPY = 'From an earlier answer';
export const NOT_SHOWN_COPY = 'Not shown to the agent';
/** A finished card fades after this time with no pointer or focus on it. */
export const DONE_SHOW_MS = 20_000;

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

/** The header line of a turn. */
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
    case 'done':
      return asked === 0
        ? `Answered by Genie Code in ${t.agentWorkspace.name}`
        : `Asked ${plural(asked, 'workspace', 'workspaces')} · ${plural(t.used, 'Genie call', 'Genie calls')}`;
    case 'stopped':
      return 'Stopped';
    case 'timed_out':
      return 'The agent timed out.';
    case 'offline':
      return 'Offline.';
    case 'failed':
      return 'The agent could not finish.';
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

const stripe = (chip: AgentWsChip): CSSProperties =>
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

function Part({
  part,
  onOpenLink,
}: {
  part: AgentPartView;
  onOpenLink?: ((url: string) => void) | undefined;
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
        <WsChip chip={part.workspace} />
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
}: {
  turn: AgentTurnView;
  onOpenLink?: ((url: string) => void) | undefined;
}) {
  const [open, setOpen] = useState(false);
  const shown = turn.parts.filter((p) => p.folded !== true);
  const folded = turn.parts.filter((p) => p.folded === true);
  const foldId = `agent-fold-${turn.turnId}`;
  if (turn.parts.length === 0) return null;
  return (
    <>
      {shown.length > 0 && (
        <ul className="agent-parts" aria-label="Answers">
          {shown.map((p) => (
            <Part key={p.callId} part={p} onOpenLink={onOpenLink} />
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
            {open ? 'Hide' : plural(folded.length, 'more answer', 'more answers')}
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
}

/** The body of a turn, the same in the card and in the hub panel. */
export function TurnBody({ turn, now, onOpenLink, showQuestion = true, actions }: TurnBodyProps) {
  const queued = turn.state === 'queued';
  const footer = footerText(turn);
  return (
    <div className="agent-turn" data-agent-turn={turn.turnId} data-state={turn.state}>
      {showQuestion && (
        <p className="genie-question agent-question" data-genie-question title={turn.question}>
          {turn.question}
        </p>
      )}
      <p className="agent-headline" data-agent-headline role="status" aria-live="polite">
        {queued && <span className="agent-pill">{QUEUED_LABEL}</span>}
        {queued ? <span className="agent-sub">{WAITING_COPY}</span> : turnHeadline(turn)}
        {turn.state === 'slow' && <span className="agent-slow">{SLOW_COPY}</span>}
      </p>
      {turn.notice !== undefined && (
        <p className="agent-notice" data-agent-notice>
          {turn.notice}
        </p>
      )}
      <CallRows calls={turn.calls} now={now} />
      {turn.summary && (
        <div className="agent-note" data-agent-note>
          <p className="agent-note-label">{NOTE_LABEL}</p>
          <div className="agent-note-body">
            <SafeMarkdown
              text={turn.summary.md}
              trust={{ profileHost: '', allowedAppUrls: [] }}
              {...(onOpenLink ? { onOpenLink } : {})}
            />
          </div>
        </div>
      )}
      <TurnParts turn={turn} onOpenLink={onOpenLink} />
      {(footer !== undefined || actions !== undefined) && (
        <footer className="agent-foot">
          {footer !== undefined && (
            <span className="agent-foot-text" data-agent-footer>
              {footer}
            </span>
          )}
          {actions}
        </footer>
      )}
    </div>
  );
}

export interface AgentTurnCardProps {
  mogId: MogId;
  turn: AgentTurnView;
  /** The chips of the compact card: Stop (or Remove), Open in Agent. */
  chips: { id: string; label: string }[];
  /** The final height of the card with its frame, so the window can fit it (`fit-window.ts`). */
  onSize?: ((px: number) => Promise<void> | void) | undefined;
  identity?: { name: string; colorVariant?: string } | undefined;
}

/** The card of an agent turn. It never takes focus (ADR 0102). */
export function AgentTurnCard({ mogId, turn, chips, onSize, identity }: AgentTurnCardProps) {
  const ref = useRef<HTMLDivElement>(null);
  const open = isOpenState(turn.state);
  const counting = turn.calls.some((c) => c.state === 'asking');
  const now = useNow(counting);
  const accent = identity && paletteColorOf(identity.colorVariant, mogId).hex;

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

  // A finished card fades away after a while, unless the pointer or the keyboard is on it.
  const held = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const arm = useCallback(() => {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      if (!held.current) done();
    }, DONE_SHOW_MS);
  }, [done]);
  useEffect(() => {
    clearTimeout(timer.current);
    if (!open) arm();
    return () => clearTimeout(timer.current);
  }, [open, arm, turn.turnId, turn.state]);
  const hold = (on: boolean) => {
    held.current = on;
    if (!on && !open) arm();
  };

  return (
    <div
      ref={ref}
      className="genie genie-answer agent-card"
      data-genie="agent"
      data-agent-card={turn.state}
      style={accent ? ({ '--mog-id-color': accent } as CSSProperties) : undefined}
      onMouseEnter={() => hold(true)}
      onMouseLeave={() => hold(false)}
      onFocus={() => hold(true)}
      onBlur={() => hold(false)}
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
        <TurnBody
          turn={turn}
          now={now}
          onOpenLink={openLink}
          actions={
            chips.length > 0 && (
              <span className="agent-actions">
                {chips.map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className={c.id.startsWith('agent-stop:') ? 'popup-action' : 'genie-toggle'}
                    data-chip={c.id}
                    onClick={() =>
                      void emitView(mogId, 'chip_click', { chipId: c.id }).catch(() => undefined)
                    }
                  >
                    {c.label}
                  </button>
                ))}
              </span>
            )
          }
        />
      </div>
    </div>
  );
}
