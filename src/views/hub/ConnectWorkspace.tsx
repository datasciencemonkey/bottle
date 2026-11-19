/**
 * "Connect a workspace" in the hub (thin; #162, W-07a, W-07e, W-07f, W-07g, W-07h, W-07i, W-15,
 * ARC-2).
 *
 * The view shows the model that the brain sends on `hub:connect_workspace` and sends each click
 * with `view_emit` (id `connect`). The brain owns every rule: the URL check, the proposed name,
 * the sign-in, Cancel, "Open the browser again" and the Table W-ERR texts.
 *
 * Two hosts show it, by the `inline` flag of the model. The setup region (`placement="setup"`)
 * shows the first-run flow (W-07f). Settings (`placement="settings"`, under the "Connect another
 * workspace…" row) shows a flow that the user opened from that row (W-07e), so the result is
 * next to the click, and the first field gets focus. With no model, or the brain's `closed`
 * step, both render nothing (the setup region rule, `Hub.tsx`). A flow that Settings opened can
 * be closed (Cancel, Done, or the connected step times out in the brain). The first-run flow has
 * no Cancel: W-07f must always give a way to connect.
 *
 * On mount the setup host sends `view_emit` `ready` (id `connect`), so the brain sends the
 * current step again if the first send came before this view listened.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useRef, useState } from 'react';
import {
  asConnectModel,
  CONNECT_EVENT,
  CONNECT_VIEW_ID,
  type ConnectModel,
  type ConnectTarget,
} from '../../shared/connect-view';
import { emitView, sendViewReady } from '../view-ipc';

function send(target: ConnectTarget, fields: Record<string, string> = {}) {
  void emitView(CONNECT_VIEW_ID, 'click', { target, ...fields }).catch(() => undefined);
}

/** Focus the first field of an inline step, so the click on the Settings row visibly did something. */
function useFocusFirst(inline: boolean) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!inline) return;
    ref.current?.scrollIntoView?.({ block: 'nearest' });
    ref.current?.focus({ preventScroll: true });
  }, [inline]);
  return ref;
}

function CancelButton() {
  return (
    <button type="button" data-connect-close onClick={() => send('connect_close')}>
      Cancel
    </button>
  );
}

type Open = Exclude<ConnectModel, { step: 'closed' }>;

function UrlStep({ model }: { model: Extract<Open, { step: 'url' }> }) {
  const [url, setUrl] = useState('');
  const first = useFocusFirst(model.inline === true);
  return (
    <form
      data-connect-step="url"
      onSubmit={(e) => {
        e.preventDefault();
        send('connect_url', { url });
      }}
    >
      {model.firstRun && <p>Connect Bottle to a Databricks workspace to start.</p>}
      <label>
        Workspace URL
        <input
          ref={first}
          name="workspace-url"
          type="url"
          value={url}
          placeholder="Paste the URL from your browser"
          onChange={(e) => setUrl(e.target.value)}
        />
      </label>
      {model.error !== undefined && <p role="alert">{model.error}</p>}
      <button type="submit" data-connect-next>
        Next
      </button>
      {model.inline === true && <CancelButton />}
    </form>
  );
}

function NameStep({ model }: { model: Extract<Open, { step: 'name' }> }) {
  const [name, setName] = useState(model.proposedName);
  const first = useFocusFirst(model.inline === true);
  const [clientId, setClientId] = useState('');
  return (
    <form
      data-connect-step="name"
      onSubmit={(e) => {
        e.preventDefault();
        send(
          'connect_start',
          clientId.trim() === '' ? { profile: name } : { profile: name, clientId },
        );
      }}
    >
      <p>
        Workspace: <span data-connect-host>{model.host}</span>
      </p>
      {model.otherAuthNotice && (
        <p data-connect-other-auth>
          A personal access token or service principal profile exists for this workspace. Bottle
          does not change it. Bottle makes a new OAuth profile.
        </p>
      )}
      <label>
        Profile name
        <input
          ref={first}
          name="profile-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </label>
      <details>
        <summary>Use a custom OAuth app (optional)</summary>
        <label>
          OAuth client ID
          <input name="client-id" value={clientId} onChange={(e) => setClientId(e.target.value)} />
        </label>
      </details>
      {model.error !== undefined && <p role="alert">{model.error}</p>}
      <button type="submit" data-connect-start>
        Sign in
      </button>
      {model.inline === true && <CancelButton />}
    </form>
  );
}

function Step({ model }: { model: Open }) {
  switch (model.step) {
    case 'url':
      return <UrlStep model={model} />;
    case 'name':
      // `key`: a new proposal (or a refused name) resets the field to the brain's value.
      return (
        <NameStep key={`${model.host} ${model.proposedName} ${model.error ?? ''}`} model={model} />
      );
    case 'waiting':
      return (
        <div data-connect-step="waiting">
          <p>Finish the sign-in in your browser. Bottle waits up to 5 minutes.</p>
          <button type="button" data-connect-cancel onClick={() => send('connect_cancel')}>
            Cancel
          </button>
          <button type="button" data-connect-reopen onClick={() => send('connect_reopen')}>
            Open the browser again
          </button>
        </div>
      );
    case 'connected':
      return (
        <p data-connect-step="connected">
          Connected {model.host} as profile <span data-connect-profile>{model.profile}</span>.{' '}
          <button type="button" data-connect-done onClick={() => send('connect_close')}>
            Done
          </button>
        </p>
      );
    case 'failed':
      return (
        <div data-connect-step="failed">
          <p role="alert">{model.text}</p>
          <button type="button" onClick={() => send(model.action.target)}>
            {model.action.label}
          </button>
        </div>
      );
  }
}

export function ConnectWorkspace({ placement = 'setup' }: { placement?: 'setup' | 'settings' }) {
  const [model, setModel] = useState<ConnectModel | undefined>(undefined);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    const target = getCurrentWindow().label;
    void listen<unknown>(
      CONNECT_EVENT,
      (e) => {
        // A bad model is ignored: the last good one stays.
        const m = asConnectModel(e.payload);
        if (m) setModel(m);
      },
      { target },
    ).then((u) => {
      if (gone) u();
      else {
        off = u;
        // One host asks: the brain sends the current step again to every listener.
        if (placement === 'setup') sendViewReady(CONNECT_VIEW_ID);
      }
    });
    return () => {
      gone = true;
      off?.();
    };
  }, [placement]);

  // No model yet, the brain's `closed` step, or a flow that the other host shows: render
  // nothing, the way every other setup step does once it clears (`Hub.tsx`'s `SetupRegion`).
  if (model === undefined || model.step === 'closed') return null;
  if ((model.inline === true) !== (placement === 'settings')) return null;

  const inline = placement === 'settings';
  return (
    <section
      data-view="connect-workspace"
      data-placement={placement}
      aria-labelledby={inline ? 'connect-workspace-title-inline' : 'connect-workspace-title-setup'}
      className={inline ? 'mt-[var(--mog-space-4)]' : 'hub-section'}
    >
      {inline ? (
        <h3 id="connect-workspace-title-inline">Connect a workspace</h3>
      ) : (
        <h2 id="connect-workspace-title-setup">Connect a workspace</h2>
      )}
      <Step model={model} />
      {inline && model.step === 'failed' && <CancelButton />}
    </section>
  );
}
