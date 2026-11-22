/**
 * First-run step "Install the Databricks CLI" in the hub (thin; #161, W-17b, W-17c, W-17f, U16).
 *
 * The hub asks the brain with `view_emit` when it opens (ARC-2). With no CLI that R5 verified
 * (none, or older than 1.17), the brain answers on `hub:cli_install` with the source, version and
 * install folder, and the step shows before the other setup steps. Only "Install the Databricks
 * CLI" asks for the install: that click is the consent (W-17b). "Not now" hides the step and the
 * hub stays in the state "No workspace connected". TODO(#43): the wizard frame mounts this step.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import {
  asCliInstallModel,
  CLI_INSTALL_EVENT,
  CLI_INSTALL_TARGET,
  CLI_SETUP_TARGET,
  CLI_VIEW_ID,
  type CliInstallModel,
} from '../../shared/cli-view';
import { askBrain, emitView } from '../view-ipc';

const ask = (target: string) =>
  void emitView(CLI_VIEW_ID, 'click', { target }).catch(() => undefined);

export function CliInstallStep() {
  const [model, setModel] = useState<CliInstallModel | undefined>(undefined);
  const [later, setLater] = useState(false);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      CLI_INSTALL_EVENT,
      (e) => {
        const m = asCliInstallModel(e.payload);
        if (m) setModel(m);
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else off = u;
    });
    askBrain(CLI_VIEW_ID, 'click', { target: CLI_SETUP_TARGET });
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  if (later) return <p className="hub-status">No workspace connected.</p>;
  if (model === undefined || model.step === 'none') return null;

  return (
    <section data-view="cli-install" aria-labelledby="cli-install-title" className="hub-section">
      <h2 id="cli-install-title">Install the Databricks CLI</h2>
      {model.step === 'installed' ? (
        <p data-cli-installed>
          The Databricks CLI {model.version} is installed. Bottle uses the CLI at {model.path}.
        </p>
      ) : (
        <>
          <p>
            Bottle uses the Databricks CLI to connect to a workspace. Bottle downloads the official
            CLI only after you agree. It checks the SHA-256 checksum before it unpacks the CLI. It
            needs no admin rights and does not change your PATH or an existing CLI.
          </p>
          <dl>
            <dt>Source</dt>
            <dd>{model.source}</dd>
            <dt>Version</dt>
            <dd>{model.version}</dd>
            <dt>Install folder</dt>
            <dd>{model.folder}</dd>
          </dl>
          {model.step === 'failed' && (
            <p role="alert" className="hub-error">
              The install stopped and Bottle deleted the download. Check your network connection,
              then select Install again.
            </p>
          )}
          {model.step === 'installing' ? (
            <p className="hub-status">Installing the Databricks CLI…</p>
          ) : (
            <div className="hub-actions">
              <button type="button" className="hub-primary" onClick={() => ask(CLI_INSTALL_TARGET)}>
                Install the Databricks CLI
              </button>
              <button type="button" onClick={() => setLater(true)}>
                Not now
              </button>
            </div>
          )}
        </>
      )}
    </section>
  );
}
