/**
 * The brain side of the hub CLI settings (#16; W-02a, W-02c, W-02d, ARC-5).
 *
 * The hub sends `cli_settings` (id `cli`) when it shows Settings. The brain asks T7 `cliInfo()`
 * (`cli_status`, R5) and sends the model to the hub only. A refused call (for example
 * `not_implemented` until the host routes `cli_status`, TODO(#570)) sends `{unknown: true}`:
 * no error text goes to the view. TODO(#570): pass the launch warning codes of the reply.
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  CLI_INSTALL_EVENT,
  CLI_INSTALL_SOURCE,
  CLI_INSTALL_TARGET,
  CLI_INSTALL_VERSION,
  CLI_SETTINGS_EVENT,
  CLI_SETTINGS_TARGET,
  CLI_SETUP_TARGET,
  CLI_VIEW_ID,
  cliInstallFolder,
  cliSettingsModel,
  type CliInstallModel,
  type CliPlatform,
  type CliSettingsModel,
} from '../shared/cli-view';
import type { ProfileService } from '../shared/interfaces';
import { onViewInput, type BrainBus } from './bus';

export interface CliSettingsDeps {
  cliInfo: ProfileService['cliInfo'];
  /** Default: `emitTo('hub', CLI_SETTINGS_EVENT, model)`. */
  send?: (model: CliSettingsModel) => Promise<unknown>;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/** Wire the hub CLI settings ask to T7. Returns the unsubscribe. */
export function wireCliSettings(bus: BrainBus, deps: CliSettingsDeps): () => void {
  const send = deps.send ?? ((m: CliSettingsModel) => emitTo('hub', CLI_SETTINGS_EVENT, m));
  return onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.kind !== 'click' || input.mogId !== CLI_VIEW_ID) return;
    if (!isObj(input.payload) || input.payload['target'] !== CLI_SETTINGS_TARGET) return;
    void (async () => {
      let model: CliSettingsModel;
      try {
        model = cliSettingsModel(await deps.cliInfo());
      } catch {
        model = { unknown: true };
      }
      try {
        await send(model);
      } catch {
        // No hub is open: the hub asks again when it shows Settings.
      }
    })();
  });
}

export interface CliInstallDeps {
  cliInfo: ProfileService['cliInfo'];
  /** `cli_install {consent:true}` (R5). Called only after the user's Install click (W-17b). */
  install: () => Promise<{ path: string; version: string }>;
  platform: CliPlatform;
  /** Default: `emitTo('hub', CLI_INSTALL_EVENT, model)`. */
  send?: (model: CliInstallModel) => Promise<unknown>;
}

/**
 * #161 (W-17b, W-17c, W-17f, U16): the first-run step "Install the Databricks CLI". The hub asks
 * (`cli_setup`) when it opens. With no CLI that R5 verified (none, or older than 1.17) the brain
 * sends the step with the source, version and install folder. Only the user's Install click
 * (`cli_install`) calls `cli_install {consent:true}`. A refused install sends `failed`, with no
 * error text. Returns the unsubscribe.
 */
export function wireCliInstall(bus: BrainBus, deps: CliInstallDeps): () => void {
  const send = deps.send ?? ((m: CliInstallModel) => emitTo('hub', CLI_INSTALL_EVENT, m));
  const offer = {
    source: CLI_INSTALL_SOURCE,
    version: CLI_INSTALL_VERSION,
    folder: cliInstallFolder(deps.platform),
  };
  let installing = false;
  const sendSafe = async (m: CliInstallModel) => {
    try {
      await send(m);
    } catch {
      // No hub is open: the hub asks again when it opens.
    }
  };
  return onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.kind !== 'click' || input.mogId !== CLI_VIEW_ID) return;
    if (!isObj(input.payload)) return;
    const target = input.payload['target'];
    if (target === CLI_SETUP_TARGET) {
      void (async () => {
        let info;
        try {
          info = await deps.cliInfo();
        } catch {
          // R5 could not answer: show no step (Settings shows the status).
          return;
        }
        await sendSafe(
          info.path !== ''
            ? { step: 'none', path: info.path, version: info.version }
            : installing
              ? { step: 'installing', ...offer }
              : { step: 'offer', ...offer },
        );
      })();
    } else if (target === CLI_INSTALL_TARGET && !installing) {
      installing = true;
      void (async () => {
        await sendSafe({ step: 'installing', ...offer });
        let model: CliInstallModel;
        try {
          const { path, version } = await deps.install();
          model = { step: 'installed', path, version };
        } catch {
          model = { step: 'failed', ...offer };
        } finally {
          installing = false;
        }
        await sendSafe(model);
      })();
    }
  });
}
