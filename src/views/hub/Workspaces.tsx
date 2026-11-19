/**
 * The hub Workspaces area (#560, #486, #163). Thin view (ARC-2): it renders the list the brain
 * sends on `hub:workspaces` and sends one action per user click with `view_emit`. "Connect a
 * workspace" reuses the existing ConnectWorkspace trigger (`connect_open`) directly; it is not a
 * Workspaces action.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useId, useState } from 'react';
import { CONNECT_VIEW_ID } from '../../shared/connect-view';
import {
  asHubWorkspaceList,
  HUB_SCOPE,
  HUB_WORKSPACES_EVENT,
  type HubWorkspaceList,
  type HubWorkspaceRow,
  type WorkspaceAction,
} from '../../shared/hub-workspaces';
import { emitView } from '../view-ipc';

function send(action: WorkspaceAction): void {
  void emitView(HUB_SCOPE, 'click', action).catch(() => undefined);
}

/** Opens "Connect a workspace" at its URL step (`src/views/hub/ConnectWorkspace.tsx`). */
export function openConnectWorkspace(): void {
  void emitView(CONNECT_VIEW_ID, 'click', { target: 'connect_open' }).catch(() => undefined);
}

const SIGN_IN_LABEL: Record<'signed_in' | 'needs_sign_in' | 'unknown', string> = {
  signed_in: 'Signed in',
  needs_sign_in: 'Sign-in needed',
  unknown: 'Not checked yet',
};

/** The list the brain sends to this window, or null before the first answer. */
export function useHubWorkspaces(): { list: HubWorkspaceList | null; seq: number } {
  const [state, setState] = useState<{ list: HubWorkspaceList | null; seq: number }>({
    list: null,
    seq: 0,
  });
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    // ARC-5: only events sent to this window.
    const target = getCurrentWindow().label;
    void listen<unknown>(
      HUB_WORKSPACES_EVENT,
      (e) => {
        const l = asHubWorkspaceList(e.payload);
        if (l) setState((s) => ({ list: l, seq: l.notice ? s.seq + 1 : s.seq }));
      },
      { target },
    ).then((u) => {
      if (gone) u();
      else {
        off = u;
        send({ action: 'workspaces.ready' });
      }
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return state;
}

function ProfileRow({ row }: { row: HubWorkspaceRow }) {
  return (
    <li
      data-profile={row.name}
      data-signed-in={row.signIn === undefined ? undefined : String(row.signIn === 'signed_in')}
      className="hub-row ws-row"
    >
      <span className="hub-row-name">{row.name}</span>{' '}
      <span className="hub-muted">
        {row.host} · {row.authType}
      </span>{' '}
      {row.supported ? (
        <span className="ws-state">{SIGN_IN_LABEL[row.signIn ?? 'unknown']}</span>
      ) : (
        <span className="ws-state">Not supported</span>
      )}
      {!row.supported && row.reason && <p className="ws-reason">{row.reason}</p>}
      {row.homeOf.length > 0 && <p className="ws-home-of">{`Home of: ${row.homeOf.join(', ')}`}</p>}
    </li>
  );
}

/** The hub section that lists the Databricks CLI profiles and their sign-in state. */
export function Workspaces() {
  const { list, seq } = useHubWorkspaces();
  const id = useId();
  const notice = list?.notice;
  return (
    <section aria-labelledby={`${id}-h`} className="hub-section ws-section">
      <h2 id={`${id}-h`}>Workspaces</h2>
      {list === null ? (
        <p className="hub-muted">Loading your Databricks workspaces…</p>
      ) : notice?.kind === 'error' ? (
        <div>
          <p role="alert" key={seq}>
            {notice.text}
          </p>
          <button type="button" onClick={() => send({ action: 'workspaces.refresh' })}>
            Retry
          </button>
        </div>
      ) : list.profiles.length === 0 ? (
        <div>
          <p>No Databricks workspace is connected yet. Connect one so a MOG can ask Genie in it.</p>
          <button type="button" data-ws-connect onClick={openConnectWorkspace}>
            Connect a workspace
          </button>
        </div>
      ) : (
        <>
          <p className="ws-cli">
            Databricks CLI: {list.cli.found ? list.cli.path : 'not found'}
            {list.cli.found && ` · version ${list.cli.version}`}
          </p>
          {list.cli.plaintextStorage && (
            <p role="alert" className="hub-error">
              Bottle found a Databricks CLI profile with less secure (plaintext) token storage. Sign
              in again to fix it.
            </p>
          )}
          <ul aria-label="Databricks workspaces" className="hub-rows ws-rows">
            {list.profiles.map((p) => (
              <ProfileRow key={p.name} row={p} />
            ))}
          </ul>
          <p className="hub-actions">
            <button type="button" data-ws-connect onClick={openConnectWorkspace}>
              Connect a workspace
            </button>
            <button type="button" onClick={() => send({ action: 'workspaces.refresh' })}>
              Refresh
            </button>
          </p>
        </>
      )}
    </section>
  );
}
