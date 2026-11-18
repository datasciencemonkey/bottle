/**
 * Settings → Wipe in the hub (thin; MEM-33a, SEC-15a, SEC-15c, MB-15, U9, ARC-2, ARC-5; ADR 0087).
 *
 * U9: Bottle deletes no user content on its own. The user picks one option (older than 30 days,
 * older than 90 days, or everything) and, if they want, "Also delete Genie conversations" (9.2).
 * The Wipe button only asks the brain for the confirm dialog; the brain runs the Wipe after the
 * confirm click. The panel calls only `view_emit` (ARC-2).
 *
 * "Show how long Bottle keeps data" asks the brain for the retention rows. They come back on
 * `hub:wipe_settings`, with the admin lock and "Set by
 * your admin" for each MDM value (SEC-15c, MB-15). Both events are sent to the hub only (ARC-5).
 * TODO(#77): the full Settings view. TODO(#525): the event names move into the contract.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import {
  ALSO_DELETE_GENIE,
  asWipeDialogModel,
  asWipeSettingsModel,
  SET_BY_ADMIN,
  WIPE_DIALOG_EVENT,
  WIPE_OPEN_TARGET,
  WIPE_OPTION_LABEL,
  WIPE_SCOPES,
  WIPE_SETTINGS_EVENT,
  WIPE_SETTINGS_TARGET,
  WIPE_VIEW_ID,
  type WipeDialogModel,
  type WipeScope,
  type WipeSettingsModel,
} from '../../shared/wipe-view';
import { emitView } from '../view-ipc';
import { WipeDialog } from '../WipeDialog';

/** Listen to one brain-to-hub event while mounted (ARC-5: only events sent to this window). */
function useHubEvent(name: string, onPayload: (p: unknown) => void): void {
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    const target = getCurrentWindow().label;
    void listen<unknown>(name, (e) => onPayload(e.payload), { target }).then((u) => {
      if (gone) u();
      else off = u;
    });
    return () => {
      gone = true;
      off?.();
    };
  }, [name, onPayload]);
}

/** A padlock, drawn inline so the view loads no asset. */
function Lock() {
  return (
    <svg data-admin-lock role="img" aria-label="Locked" width="12" height="12" viewBox="0 0 12 12">
      <path
        d="M3.5 5V3.5a2.5 2.5 0 0 1 5 0V5M2.5 5h7v6h-7z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
      />
    </svg>
  );
}

export function WipeSettings() {
  const [scope, setScope] = useState<WipeScope | undefined>(undefined);
  const [alsoGenie, setAlsoGenie] = useState(false);
  const [settings, setSettings] = useState<WipeSettingsModel | undefined>(undefined);
  const [dialog, setDialog] = useState<WipeDialogModel | null>(null);

  const [onSettings] = useState(() => (p: unknown) => {
    const m = asWipeSettingsModel(p);
    if (m !== undefined) setSettings(m);
  });
  const [onDialog] = useState(() => (p: unknown) => {
    const m = asWipeDialogModel(p);
    if (m !== undefined) setDialog(m);
  });
  useHubEvent(WIPE_SETTINGS_EVENT, onSettings);
  useHubEvent(WIPE_DIALOG_EVENT, onDialog);

  return (
    <section data-view="wipe-settings" aria-labelledby="wipe-settings-title">
      <h2 id="wipe-settings-title">Wipe</h2>
      <p>Bottle keeps your data on this computer until you wipe it.</p>
      <fieldset>
        <legend>Delete data</legend>
        {WIPE_SCOPES.map((s) => (
          <div key={s}>
            <input
              id={`wipe-${s}`}
              type="radio"
              name="wipeScope"
              value={s}
              checked={scope === s}
              onChange={() => setScope(s)}
            />
            <label htmlFor={`wipe-${s}`}>{WIPE_OPTION_LABEL[s]}</label>
          </div>
        ))}
      </fieldset>
      <input
        id="wipe-also-genie"
        type="checkbox"
        name="alsoGenie"
        checked={alsoGenie}
        onChange={(e) => setAlsoGenie(e.currentTarget.checked)}
      />
      <label htmlFor="wipe-also-genie">{ALSO_DELETE_GENIE}</label>
      <p>
        <button
          type="button"
          data-wipe-open
          disabled={scope === undefined}
          onClick={() => {
            if (scope === undefined) return;
            void emitView(WIPE_VIEW_ID, 'click', { target: WIPE_OPEN_TARGET, scope, alsoGenie });
          }}
        >
          Wipe…
        </button>
      </p>
      <p>
        <button
          type="button"
          data-wipe-settings
          onClick={() =>
            void emitView(WIPE_VIEW_ID, 'click', { target: WIPE_SETTINGS_TARGET }).catch(
              () => undefined,
            )
          }
        >
          Show how long Bottle keeps data
        </button>
      </p>
      {settings !== undefined && (
        <table aria-label="How long Bottle keeps data">
          <tbody>
            {settings.rows.map((r) => (
              <tr key={r.id} data-retention-row={r.id}>
                <th scope="row">{r.label}</th>
                <td>
                  {r.value}
                  {r.locked && (
                    <>
                      {' '}
                      <Lock /> {SET_BY_ADMIN}
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {dialog !== null && <WipeDialog model={dialog} />}
    </section>
  );
}
