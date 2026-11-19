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
 *
 * ui-1 (ADR 0104): rebuilt on shadcn/ui's RadioGroup, Checkbox, Button and Collapsible instead of
 * a raw `<fieldset>`/`<legend>` and bare `<input>`s (the ui-audit "Settings baseline" findings).
 * "Delete data" is a RadioGroup with helper text per option; the Wipe button is a real destructive
 * style with helper text while it is disabled; "Show how long Bottle keeps data" is a Collapsible
 * disclosure row instead of a standalone outline button. The RadioGroup and Checkbox render as
 * accessible buttons (`role="radio"`/`role="checkbox"`), not native `<input>`s, so
 * WipeSettings.test.tsx asserts on role and `aria-checked`, not `input[type=...]` (documented
 * there).
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ChevronDownIcon } from 'lucide-react';
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
import { Checkbox } from '../ui/checkbox';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../ui/collapsible';
import { RadioGroup, RadioGroupItem } from '../ui/radio-group';
import { Button } from '../ui/button';
import { emitView } from '../view-ipc';
import { WipeDialog } from '../WipeDialog';

/** A short, calm reason for each scope (ui-1): the radio list reads at a glance, not a guess. */
const WIPE_SCOPE_HELP: Readonly<Record<WipeScope, string>> = {
  older_than_30d: 'Keeps the last 30 days. Deletes everything older.',
  older_than_90d: 'Keeps the last 90 days. Deletes everything older.',
  everything: 'Deletes every saved transcript, answer and memory item.',
};

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
    <section
      data-view="wipe-settings"
      aria-labelledby="wipe-settings-title"
      className="hub-section"
    >
      <h2 id="wipe-settings-title">Wipe</h2>
      <p>Bottle keeps your data on this computer until you wipe it.</p>
      <div className="mt-[var(--mog-space-4)]">
        <span id="wipe-scope-label" className="text-[var(--mog-text-sm)] font-medium">
          Delete data
        </span>
        <RadioGroup
          aria-labelledby="wipe-scope-label"
          className="mt-[var(--mog-space-2)] gap-[var(--mog-space-2)]"
          value={scope ?? ''}
          onValueChange={(v) => setScope(v as WipeScope)}
        >
          {WIPE_SCOPES.map((s) => (
            <label
              key={s}
              htmlFor={`wipe-${s}`}
              className="flex items-start gap-[var(--mog-space-3)] rounded-[var(--radius-md)] border border-border/60 p-[var(--mog-space-3)] cursor-pointer hover:bg-accent/40"
            >
              <RadioGroupItem id={`wipe-${s}`} value={s} className="mt-0.5" />
              <span className="flex flex-col gap-[var(--mog-space-1)]">
                <span className="text-[var(--mog-text-md)] font-medium">
                  {WIPE_OPTION_LABEL[s]}
                </span>
                <span className="text-[var(--mog-text-xs)] text-muted-foreground">
                  {WIPE_SCOPE_HELP[s]}
                </span>
              </span>
            </label>
          ))}
        </RadioGroup>
      </div>
      <label
        htmlFor="wipe-also-genie"
        className="mt-[var(--mog-space-4)] flex items-center gap-[var(--mog-space-3)] cursor-pointer"
      >
        <Checkbox
          id="wipe-also-genie"
          checked={alsoGenie}
          onCheckedChange={(c) => setAlsoGenie(c === true)}
        />
        <span className="text-[var(--mog-text-md)]">{ALSO_DELETE_GENIE}</span>
      </label>
      <div className="mt-[var(--mog-space-4)] flex flex-col gap-[var(--mog-space-2)]">
        <Button
          type="button"
          variant="destructive"
          className="w-fit"
          data-wipe-open
          disabled={scope === undefined}
          onClick={() => {
            if (scope === undefined) return;
            void emitView(WIPE_VIEW_ID, 'click', { target: WIPE_OPEN_TARGET, scope, alsoGenie });
          }}
        >
          Wipe…
        </Button>
        {scope === undefined && (
          <p className="text-[var(--mog-text-xs)] text-muted-foreground">
            Pick what to delete first.
          </p>
        )}
      </div>
      <Collapsible className="mt-[var(--mog-space-5)]">
        <CollapsibleTrigger
          data-wipe-settings
          className="group flex h-auto min-h-0 items-center gap-[var(--mog-space-2)] border-0 bg-transparent p-0 text-[var(--mog-text-sm)] text-muted-foreground hover:text-foreground"
          onClick={() =>
            void emitView(WIPE_VIEW_ID, 'click', { target: WIPE_SETTINGS_TARGET }).catch(
              () => undefined,
            )
          }
        >
          <ChevronDownIcon className="size-3.5 transition-transform group-data-[state=open]:rotate-180" />
          Show how long Bottle keeps data
        </CollapsibleTrigger>
        <CollapsibleContent className="mt-[var(--mog-space-2)]">
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
        </CollapsibleContent>
      </Collapsible>
      {dialog !== null && <WipeDialog model={dialog} />}
    </section>
  );
}
