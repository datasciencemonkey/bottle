/**
 * The hub "Agent" area (slice 2a, plan Amendment 2 A2, "Agent panel"). Thin view (ARC-2, ARC-4):
 * it draws the `HubAgentView` that the brain sends on `hub:agent` and sends one `AgentAction` for
 * each click with `view_emit`. The content lives in the brain's memory only; a new session, quit
 * or Wipe clears it.
 *
 * - Header: "Genie Code in sales · started 10:42 · 3 questions · 124k tokens" and [New session].
 *   With no session: "No session. Ask with ⌥G. Questions that name another workspace come here."
 * - The live turn: the same body as the card, with [Stop].
 * - Waiting: up to 3 rows, "Sends when the answer above lands" and [×].
 * - Earlier turns, newest first.
 * Each row has the shape of `AgentTurnView`, so the later Activity list can reuse it.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { XIcon } from 'lucide-react';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import {
  asHubAgentView,
  HUB_AGENT_EVENT,
  HUB_SCOPE,
  type AgentAction,
  type HubAgentView,
} from '../../shared/hub-agent';
import type { AgentTurnView } from '../../shared/ipc';
import { LINK_OPEN } from '../../shared/link-trust';
import { TurnBody, WAITING_COPY, tokensText, turnHeadline, useNow } from '../card/AgentTurnCard';
import { Button } from '../ui/button';
import { askBrain, emitView } from '../view-ipc';

function send(action: AgentAction): void {
  void emitView(HUB_SCOPE, 'click', action).catch(() => undefined);
}

export const NO_SESSION_COPY = 'Ask with ⌥G. Questions that name another workspace come here.';

/**
 * The panel the brain sends to this window, or undefined before the brain answers. Sends
 * `agent.ready` once the listener is on (ARC-5: only events sent to this window).
 */
export function useHubAgent(): HubAgentView | undefined {
  const [view, setView] = useState<HubAgentView | undefined>(undefined);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      HUB_AGENT_EVENT,
      (e) => {
        const v = asHubAgentView(e.payload);
        if (v) setView(v);
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else {
        off = u;
        askBrain(HUB_SCOPE, 'click', { action: 'agent.ready' } satisfies AgentAction);
      }
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return view;
}

const openLink = (url: string) =>
  void emitView(HUB_SCOPE, 'click', { target: LINK_OPEN, url }).catch(() => undefined);

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "Genie Code in sales · started 10:42 · 3 questions · 124k tokens". */
function sessionText(v: HubAgentView): string {
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

/** One line of the answer of a turn, with no markdown marks. */
function oneLine(t: AgentTurnView): string {
  const md =
    t.summary?.md ??
    t.parts
      .flatMap((p) => p.sections)
      .find((s): s is Extract<typeof s, { id: 'answer' }> => s.id === 'answer')?.md ??
    '';
  return (md.split('\n').find((l) => l.trim() !== '') ?? '').replace(/[*_`#>|]/g, '').trim();
}

function WaitingRow({ turn }: { turn: AgentTurnView }) {
  return (
    <li className="agent-waiting" data-agent-waiting={turn.turnId}>
      <div className="agent-waiting-text">
        <span className="agent-waiting-q" title={turn.question}>
          {turn.question}
        </span>
        <span className="agent-sub">{WAITING_COPY}</span>
      </div>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="size-7 shrink-0"
        data-agent-remove={turn.turnId}
        aria-label={`Remove from the queue: ${turn.question}`}
        onClick={(e) => {
          // The next row's button takes the focus, so the queue clears in a row from the keyboard.
          const li = e.currentTarget.closest('li');
          const near = li?.nextElementSibling ?? li?.previousElementSibling;
          const target = near?.querySelector<HTMLElement>('[data-agent-remove]');
          send({ action: 'agent.remove_queued', turnId: turn.turnId });
          if (target) target.focus();
          else (document.querySelector('[data-agent-session]') as HTMLElement | null)?.focus();
        }}
      >
        <XIcon className="size-4" aria-hidden="true" />
      </Button>
    </li>
  );
}

/** An earlier turn in the shape of an Activity row: the question, one line of the answer, the meta. */
function EarlierRow({ turn, now }: { turn: AgentTurnView; now: number }) {
  // A turn that needs the user stays open, with its action.
  const [open, setOpen] = useState(turn.state === 'needs_you');
  const bodyId = `agent-earlier-${turn.turnId}`;
  // While the row is open its body has the headline: the meta keeps the tokens only.
  const meta = [
    open ? '' : turnHeadline(turn),
    turn.tokens !== undefined ? tokensText(turn.tokens) : '',
  ]
    .filter((m) => m !== '')
    .join(' · ');
  const line = oneLine(turn) || (turn.notice ?? '');
  return (
    <li data-agent-earlier={turn.turnId} data-state={turn.state} className="agent-earlier-row">
      <button
        type="button"
        className="agent-earlier-head"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="agent-panel-q" title={turn.question}>
          {turn.question}
        </span>
        {line !== '' && <span className="agent-earlier-line">{line}</span>}
        <span className="agent-sub">{meta}</span>
      </button>
      <div id={bodyId} hidden={!open}>
        {open && (
          <TurnBody
            turn={turn}
            now={now}
            showQuestion={false}
            onOpenLink={openLink}
            actions={
              turn.state === 'needs_you' ? (
                <Button
                  type="button"
                  size="sm"
                  data-agent-open-settings
                  onClick={() => send({ action: 'agent.open_settings' })}
                >
                  Open Settings
                </Button>
              ) : undefined
            }
          />
        )}
      </div>
    </li>
  );
}

export function AgentPanel({ view, active }: { view: HubAgentView | undefined; active: boolean }) {
  const sessionLine = useRef<HTMLParagraphElement>(null);
  const [confirm, setConfirm] = useState(false);
  const counting = view?.live?.calls.some((c) => c.state === 'asking') ?? false;
  const now = useNow(active && counting);
  const toSession = useCallback(
    () => requestAnimationFrame(() => sessionLine.current?.focus()),
    [],
  );
  const notice = view?.notice;
  const ready = view?.genieCode.binary === 'ready' && view.genieCode.agentProfileState === 'ready';
  const hasSession = view !== undefined && view.session.state !== 'none';
  const hasContent =
    view !== undefined &&
    (view.live !== undefined || view.waiting.length > 0 || view.earlier.length > 0);
  // New session stops a live question and clears the list: it asks first.
  const needsConfirm = view?.live !== undefined || (view?.waiting.length ?? 0) > 0;
  const newSession = () => {
    send({ action: 'agent.new_session' });
    setConfirm(false);
    toSession();
  };
  return (
    <section data-hub-view="agent" aria-label="Agent" hidden={!active} className="hub-section">
      {view !== undefined && (hasSession || hasContent) && (
        <div className="agent-session-row">
          <p ref={sessionLine} tabIndex={-1} data-agent-session className="agent-session">
            {hasSession || hasContent ? sessionText(view) : ''}
          </p>
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
        </div>
      )}
      {confirm && (
        <NewSessionConfirm
          live={view?.live !== undefined}
          onConfirm={newSession}
          onCancel={() => setConfirm(false)}
        />
      )}
      <p
        role="status"
        aria-live="polite"
        data-agent-notice={notice?.kind ?? 'none'}
        className="hub-muted"
      >
        {notice?.kind === 'info' ? notice.text : ''}
      </p>
      {notice?.kind === 'error' && (
        <p role="alert" className="hub-error">
          {notice.text}
        </p>
      )}
      {view === undefined && (
        <p data-agent-loading className="hub-muted">
          Checking your session…
        </p>
      )}
      {view !== undefined && !hasSession && !hasContent && (
        <div data-agent-empty className="agent-empty">
          {ready ? (
            <>
              <p className="agent-empty-title">No session yet</p>
              <p className="hub-muted m-0">{NO_SESSION_COPY}</p>
            </>
          ) : (
            <>
              <p className="hub-muted m-0">{notReadyText(view.genieCode)}</p>
              {view.genieCode.pinned !== '' && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="mt-[var(--mog-space-2)]"
                  data-agent-open-settings
                  onClick={() => send({ action: 'agent.open_settings' })}
                >
                  Open Settings
                </Button>
              )}
            </>
          )}
        </div>
      )}
      {view?.live && (
        <section aria-label="Live question" data-agent-live className="agent-panel-block">
          <h3 className="agent-panel-q" title={view.live.question}>
            {view.live.question}
          </h3>
          <TurnBody
            turn={view.live}
            now={now}
            showQuestion={false}
            onOpenLink={openLink}
            live
            actions={
              <Button
                type="button"
                size="sm"
                variant="outline"
                data-agent-stop={view.live.turnId}
                aria-label={`Stop: ${view.live.question}`}
                onClick={() => {
                  send({ action: 'agent.stop', turnId: (view.live as AgentTurnView).turnId });
                  toSession();
                }}
              >
                Stop
              </Button>
            }
          />
        </section>
      )}
      {view !== undefined && view.waiting.length > 0 && (
        <section aria-label="Waiting" className="agent-panel-block">
          <h3 className="agent-panel-label">Waiting</h3>
          <ul className="agent-waiting-list" aria-label="Waiting questions">
            {view.waiting.map((t) => (
              <WaitingRow key={t.turnId} turn={t} />
            ))}
          </ul>
        </section>
      )}
      {view !== undefined && view.earlier.length > 0 && (
        <section aria-label="Earlier questions" className="agent-panel-block">
          <h3 className="agent-panel-label">Earlier</h3>
          <ul className="agent-earlier" aria-label="Earlier questions">
            {view.earlier.map((t) => (
              <EarlierRow key={t.turnId} turn={t} now={now} />
            ))}
          </ul>
        </section>
      )}
    </section>
  );
}

function NewSessionConfirm({
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
          ? 'Start a new session? The live question stops and this list clears.'
          : 'Start a new session? The waiting questions are removed and this list clears.'}
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
