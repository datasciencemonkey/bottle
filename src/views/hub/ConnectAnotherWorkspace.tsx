/**
 * Settings → "Connect another workspace…" (#162 follow-up, W-07e; review of #602: the "Connect
 * a workspace" panel showing on every hub page even with a working profile).
 *
 * `ConnectWorkspace.tsx` now renders nothing until the brain opens it (no model, or the brain's
 * explicit `closed` step). That leaves no way to add a second workspace once a usable profile
 * already exists, so this row is the one quiet, deliberate way back in: it sends the same
 * `connect_open` click the view's old always-visible button used to send (`view_emit`, id
 * `connect`, ARC-2, MOG-08a). The brain (`connect-wiring.ts`) then opens the flow with
 * `inline: true`, and it shows right under this row (a second `ConnectWorkspace`, with Cancel
 * and Done), with focus on its first field. There is exactly one entry point for `connect_open`
 * in the hub: this row. W-07e also asks for disconnecting a profile; that is not done here.
 */
import { CONNECT_VIEW_ID } from '../../shared/connect-view';
import { Button } from '../ui/button';
import { emitView } from '../view-ipc';
import { ConnectWorkspace } from './ConnectWorkspace';
import { SettingsGroup, SettingsRow } from './SettingsGroup';

export function ConnectAnotherWorkspace() {
  return (
    <section
      data-view="connect-another-workspace"
      aria-labelledby="connect-another-workspace-title"
      className="hub-section"
    >
      <h2 id="connect-another-workspace-title">Workspace</h2>
      <SettingsGroup className="mt-[var(--mog-space-3)]">
        <SettingsRow
          label="Connect another workspace…"
          description="Add a profile for another Databricks workspace."
          control={
            <Button
              type="button"
              variant="outline"
              size="sm"
              data-connect-another-open
              onClick={() =>
                void emitView(CONNECT_VIEW_ID, 'click', { target: 'connect_open' }).catch(
                  () => undefined,
                )
              }
            >
              Connect…
            </Button>
          }
        />
      </SettingsGroup>
      {/* The flow shows here, under the row that opened it (the brain sets `inline`), not in the
          setup region at the top of the page, which is out of sight from this row. */}
      <ConnectWorkspace placement="settings" />
    </section>
  );
}
