/**
 * Settings → Databricks CLI in the hub (thin; #16, W-02a, W-02c, W-02d, ARC-2, ARC-5).
 *
 * "Show the Databricks CLI" asks the brain with `view_emit` (ARC-2), as Settings → Wipe does, so
 * the hub sends nothing on open. The brain answers on
 * `hub:cli_settings`, to the hub only: the path and version of the CLI that R5 verified at
 * launch, the "1.25 or later is recommended" text, and the launch warnings of a changed CLI.
 * TODO(#77): the full Settings view. TODO(#570): the launch warnings reach the brain, and the
 * brain pushes them at launch.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ChevronRightIcon } from 'lucide-react';
import { useEffect, useState } from 'react';
import {
  asCliSettingsModel,
  CLI_RECOMMENDED_TEXT,
  CLI_SETTINGS_EVENT,
  CLI_SETTINGS_TARGET,
  CLI_VIEW_ID,
  type CliSettingsModel,
} from '../../shared/cli-view';
import { emitView } from '../view-ipc';

export function CliSettings() {
  const [model, setModel] = useState<CliSettingsModel | undefined>(undefined);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    const target = getCurrentWindow().label;
    void listen<unknown>(
      CLI_SETTINGS_EVENT,
      (e) => {
        // A bad model reads as "could not read", never as a stale good one.
        setModel(asCliSettingsModel(e.payload) ?? { unknown: true });
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

  return (
    // ui-1 (ADR 0106): the path and version move to a Workspaces area in a later PR; for now
    // this stays a quiet row in Settings, not a standalone outline button (the ui-audit
    // "content as buttons" finding). One consistent "show more" style (review of #602): the
    // chevron and text-sm/muted-foreground label match every disclosure trigger in Settings,
    // not a plain text link. The chevron does not rotate (unlike a Collapsible's): asking the
    // brain is one-way, nothing here folds back closed.
    <section data-view="cli-settings" aria-labelledby="cli-settings-title" className="hub-section">
      <h2 id="cli-settings-title">Databricks CLI</h2>
      <button
        type="button"
        className="group mt-[var(--mog-space-1)] flex h-auto min-h-0 items-center gap-[var(--mog-space-2)] border-0 bg-transparent p-0 text-[var(--mog-text-sm)] text-muted-foreground hover:text-foreground"
        data-cli-settings
        onClick={() =>
          void emitView(CLI_VIEW_ID, 'click', { target: CLI_SETTINGS_TARGET }).catch(
            () => undefined,
          )
        }
      >
        <ChevronRightIcon className="size-3.5" />
        Show the Databricks CLI
      </button>
      {model === undefined ? null : 'unknown' in model ? (
        <p>Bottle could not read the Databricks CLI status.</p>
      ) : !model.found ? (
        <p>Bottle did not find the Databricks CLI.</p>
      ) : (
        <>
          {model.warnings.map((w) => (
            <p key={w} role="alert">
              {w}
            </p>
          ))}
          <dl>
            <dt>Path</dt>
            <dd data-cli-path>{model.path}</dd>
            <dt>Version</dt>
            <dd data-cli-version>{model.version}</dd>
          </dl>
          {model.belowRecommended && <p>{CLI_RECOMMENDED_TEXT}</p>}
        </>
      )}
    </section>
  );
}
