/**
 * "Connect a workspace" in the hub (thin; #162, W-07a, W-07f, W-07g, W-07h, W-07i, W-15, ARC-2).
 *
 * The view shows the model that the brain sends on `hub:connect_workspace` and sends each click
 * with `view_emit` (id `connect`). The brain owns every rule: the URL check, the proposed name,
 * the sign-in, Cancel, "Open the browser again" and the Table W-ERR texts. The view sends
 * nothing on mount; at first run the brain opens the URL step when the hub is ready (W-07f).
 *
 * The setup region's rule ("each step renders nothing once it is cleared", `Hub.tsx`) holds for
 * this step too (fix for the "Connect a workspace" panel showing on every hub page even with a
 * working profile): with no model yet, or the brain's explicit `closed` step (W-07f: a usable
 * profile already exists), this renders nothing — no heading, no button. There is exactly one
 * way left to open the form on purpose: the "Connect another workspace…" row in Settings
 * (`ConnectAnotherWorkspace.tsx`), which sends the same `connect_open` click this view used to
 * send from its own always-visible button.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import {
  asConnectModel,
  CONNECT_EVENT,
  CONNECT_VIEW_ID,
  type ConnectModel,
  type ConnectTarget,
} from '../../shared/connect-view';
import { emitView } from '../view-ipc';

function send(target: ConnectTarget, fields: Record<string, string> = {}) {
  void emitView(CONNECT_VIEW_ID, 'click', { target, ...fields }).catch(() => undefined);
}

function UrlStep({ model }: { model: Extract<ConnectModel, { step: 'url' }> }) {
  const [url, setUrl] = useState('');
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
    </form>
  );
}

function NameStep({ model }: { model: Extract<ConnectModel, { step: 'name' }> }) {
  const [name, setName] = useState(model.proposedName);
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
        <input name="profile-name" value={name} onChange={(e) => setName(e.target.value)} />
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
    </form>
  );
}

function Step({ model }: { model: Exclude<ConnectModel, { step: 'closed' }> }) {
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
          Connected {model.host} as profile <span data-connect-profile>{model.profile}</span>.
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

export function ConnectWorkspace() {
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
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  // No model yet, or the brain's explicit `closed` step (W-07f: a usable profile already
  // exists): render nothing, the way every other setup step does once it clears (`Hub.tsx`'s
  // `SetupRegion`). Settings' "Connect another workspace…" row is the one way left to open it.
  if (model === undefined || model.step === 'closed') return null;

  return (
    <section
      data-view="connect-workspace"
      aria-labelledby="connect-workspace-title"
      className="hub-section"
    >
      <h2 id="connect-workspace-title">Connect a workspace</h2>
      <Step model={model} />
    </section>
  );
}
