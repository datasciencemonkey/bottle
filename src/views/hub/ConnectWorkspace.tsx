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
import { useEffect, useId, useRef, useState } from 'react';
import {
  asConnectModel,
  CONNECT_EVENT,
  CONNECT_VIEW_ID,
  isValidProfileName,
  PROFILE_NAME_INVALID_TEXT,
  PROFILE_NAME_TAKEN_TEXT,
  sameProfileName,
  type ConnectModel,
  type ConnectTarget,
} from '../../shared/connect-view';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { emitView, sendViewReady } from '../view-ipc';

function send(target: ConnectTarget, fields: Record<string, string> = {}) {
  void emitView(CONNECT_VIEW_ID, 'click', { target, ...fields }).catch(() => undefined);
}

/**
 * The button of a step that closes the flow. `secondary`: the step's main action stays the one
 * primary button. `data-autofocus` marks the element that gets focus when the step changes.
 */
function CloseButton({ label }: { label: string }) {
  return (
    <Button
      type="button"
      variant="secondary"
      size="sm"
      data-connect-close
      onClick={() => send('connect_close')}
    >
      {label}
    </Button>
  );
}

type Open = Exclude<ConnectModel, { step: 'closed' }>;

function UrlStep({ model }: { model: Extract<Open, { step: 'url' }> }) {
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
        <Input
          data-autofocus
          name="workspace-url"
          type="url"
          value={url}
          placeholder="Paste the URL from your browser"
          onChange={(e) => setUrl(e.target.value)}
        />
      </label>
      {model.error !== undefined && <p role="alert">{model.error}</p>}
      <Button type="submit" size="sm" data-connect-next>
        Next
      </Button>
      {model.inline === true && <CloseButton label="Cancel" />}
    </form>
  );
}

function NameStep({ model }: { model: Extract<Open, { step: 'name' }> }) {
  const [name, setName] = useState(model.proposedName);
  const [clientId, setClientId] = useState('');
  const errorId = useId();
  const helpId = useId();
  const trimmed = name.trim();
  const taken = model.taken ?? [];
  const duplicate = trimmed !== '' && taken.some((t) => sameProfileName(t, trimmed));
  const valid = isValidProfileName(trimmed) && !duplicate;
  // An empty field is only incomplete, not "wrong": it gets no error line, and the help line
  // below falls back to the brain's own proposal instead of echoing nothing (or, while the
  // field holds a bad or taken name, instead of echoing a name that would not be saved).
  const showError = trimmed !== '' && !valid;
  const helpName = valid ? trimmed : model.proposedName;
  return (
    <form
      data-connect-step="name"
      onSubmit={(e) => {
        e.preventDefault();
        if (!valid) return;
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
        <Input
          data-autofocus
          name="profile-name"
          value={name}
          aria-invalid={showError}
          aria-describedby={showError ? `${errorId} ${helpId}` : helpId}
          onChange={(e) => setName(e.target.value)}
        />
      </label>
      {showError && (
        <p role="alert" id={errorId} data-profile-name-error>
          {duplicate ? PROFILE_NAME_TAKEN_TEXT : PROFILE_NAME_INVALID_TEXT}
        </p>
      )}
      <p className="hub-muted" id={helpId} data-profile-name-help>
        {`Saved as a Databricks CLI profile in ~/.databrickscfg. You can use it from the terminal with --profile ${helpName}.`}
      </p>
      <details>
        <summary>Use a custom OAuth app (optional)</summary>
        <label>
          OAuth client ID
          <Input name="client-id" value={clientId} onChange={(e) => setClientId(e.target.value)} />
        </label>
      </details>
      {model.error !== undefined && <p role="alert">{model.error}</p>}
      <Button type="submit" size="sm" data-connect-start disabled={!valid}>
        Sign in
      </Button>
      {model.inline === true && <CloseButton label="Cancel" />}
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
          <Button
            type="button"
            size="sm"
            variant="secondary"
            data-autofocus
            data-connect-cancel
            onClick={() => send('connect_cancel')}
          >
            Cancel
          </Button>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            data-connect-reopen
            onClick={() => send('connect_reopen')}
          >
            Open the browser again
          </Button>
        </div>
      );
    case 'connected':
      return (
        <p data-connect-step="connected">
          <span>
            Connected {model.host} as profile <span data-connect-profile>{model.profile}</span>.
          </span>
          <Button
            type="button"
            size="sm"
            data-autofocus
            data-connect-done
            onClick={() => send('connect_close')}
          >
            Done
          </Button>
        </p>
      );
    case 'failed':
      return (
        <div data-connect-step="failed">
          <p role="alert">{model.text}</p>
          <Button type="button" size="sm" data-autofocus onClick={() => send(model.action.target)}>
            {model.action.label}
          </Button>
          {model.inline === true && <CloseButton label="Close" />}
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
  const shown =
    model !== undefined &&
    model.step !== 'closed' &&
    (model.inline === true) === (placement === 'settings');
  const inline = placement === 'settings';
  // A change of step (or a refused name) moves focus to the new step's first field or button,
  // so the element that had focus does not drop it when it leaves the page. A flow that Settings
  // opens also takes focus when it appears and scrolls into view. The first-run panel does not
  // take focus when it first appears: nothing in the page asked for it.
  const section = useRef<HTMLElement>(null);
  const seen = useRef(false);
  const stepKey = shown ? `${model.step} ${'error' in model ? (model.error ?? '') : ''}` : '';
  useEffect(() => {
    if (!shown) {
      seen.current = false;
      return;
    }
    const had = seen.current;
    seen.current = true;
    if (!had && !inline) return;
    const el = section.current;
    el?.scrollIntoView?.({ block: 'nearest' });
    el?.querySelector<HTMLElement>('[data-autofocus]')?.focus({ preventScroll: true });
  }, [shown, stepKey, inline]);

  if (!shown) return null;

  return (
    <section
      ref={section}
      data-view="connect-workspace"
      data-placement={placement}
      aria-labelledby={inline ? 'connect-workspace-title-inline' : 'connect-workspace-title-setup'}
      className={inline ? 'mt-[var(--mog-space-4)]' : 'hub-section'}
    >
      {inline ? (
        <h3 id="connect-workspace-title-inline" className="mb-[var(--mog-space-2)]">
          Connect a workspace
        </h3>
      ) : (
        <h2 id="connect-workspace-title-setup">Connect a workspace</h2>
      )}
      <Step model={model} />
    </section>
  );
}
