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
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  asHubAgentView,
  HUB_AGENT_EVENT,
  HUB_SCOPE,
  type AgentAction,
  type HubAgentView,
} from '../../shared/hub-agent';
import type { AgentTurnView } from '../../shared/ipc';
import { LINK_OPEN } from '../../shared/link-trust';
import { TurnBody, WAITING_COPY, tokensText, useNow } from '../card/AgentTurnCard';
import { Button } from '../ui/button';
import { askBrain, emitView } from '../view-ipc';
import { genieCodeLine } from './AgentSettings';

function send(action: AgentAction): void {
  void emitView(HUB_SCOPE, 'click', action).catch(() => undefined);
}

export const NO_SESSION_COPY =
  'No session. Ask with ⌥G. Questions that name another workspace come here.';

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

function headerText(v: HubAgentView): string {
  const t = v.live ?? v.waiting[0] ?? v.earlier[0];
  const ws = t?.agentWorkspace.name ?? v.session.agentProfile ?? 'your workspace';
  const parts = [`Genie Code in ${ws}`];
  if (v.session.startedAt !== undefined)
    parts.push(
      `started ${new Date(v.session.startedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`,
    );
  parts.push(plural(v.session.turns, 'question', 'questions'));
  if (v.session.tokens > 0) parts.push(tokensText(v.session.tokens));
  return parts.join(' · ');
}

function WaitingRow({ turn, onRemoved }: { turn: AgentTurnView; onRemoved: () => void }) {
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
        onClick={() => {
          send({ action: 'agent.remove_queued', turnId: turn.turnId });
          onRemoved();
        }}
      >
        <XIcon className="size-4" aria-hidden="true" />
      </Button>
    </li>
  );
}

export function AgentPanel({ view, active }: { view: HubAgentView | undefined; active: boolean }) {
  const heading = useRef<HTMLHeadingElement>(null);
  const counting = view?.live?.calls.some((c) => c.state === 'asking') ?? false;
  const now = useNow(active && counting);
  // A removed or stopped row takes its button with it: the focus goes to the heading.
  const toHeading = useCallback(() => requestAnimationFrame(() => heading.current?.focus()), []);
  const notice = view?.notice;
  const g = view
    ? genieCodeLine(view.genieCode, view.session.agentProfile ?? 'your workspace')
    : undefined;
  const ready = view?.genieCode.binary === 'ready' && view.genieCode.agentProfileState === 'ready';
  const hasSession = view !== undefined && view.session.state !== 'none';
  const hasContent =
    view !== undefined &&
    (view.live !== undefined || view.waiting.length > 0 || view.earlier.length > 0);
  return (
    <section
      data-hub-view="agent"
      aria-labelledby="agent-title"
      hidden={!active}
      className="hub-section"
    >
      <div className="flex items-center justify-between gap-[var(--mog-space-3)]">
        <h2 id="agent-title" ref={heading} tabIndex={-1} data-agent-header className="m-0">
          {view === undefined ? 'Agent' : hasSession || hasContent ? headerText(view) : 'Agent'}
        </h2>
        <Button
          type="button"
          variant="outline"
          size="sm"
          data-agent-new-session
          disabled={!hasSession}
          onClick={() => {
            send({ action: 'agent.new_session' });
            toHeading();
          }}
        >
          New session
        </Button>
      </div>
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
        <div data-agent-empty className="hub-muted">
          <p className="m-0">{ready ? NO_SESSION_COPY : (g?.text ?? '')}</p>
          {!ready && (
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
            actions={
              <Button
                type="button"
                size="sm"
                variant="outline"
                data-agent-stop={view.live.turnId}
                aria-label={`Stop: ${view.live.question}`}
                onClick={() => {
                  send({ action: 'agent.stop', turnId: (view.live as AgentTurnView).turnId });
                  toHeading();
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
              <WaitingRow key={t.turnId} turn={t} onRemoved={toHeading} />
            ))}
          </ul>
        </section>
      )}
      {view !== undefined && view.earlier.length > 0 && (
        <section aria-label="Earlier questions" className="agent-panel-block">
          <h3 className="agent-panel-label">Earlier</h3>
          <ul className="agent-earlier" aria-label="Earlier questions">
            {view.earlier.map((t) => (
              <li key={t.turnId} data-agent-earlier={t.turnId} className="agent-earlier-row">
                <h4 className="agent-panel-q" title={t.question}>
                  {t.question}
                </h4>
                <TurnBody turn={t} now={now} showQuestion={false} onOpenLink={openLink} />
              </li>
            ))}
          </ul>
        </section>
      )}
    </section>
  );
}
