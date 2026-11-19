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
  CLI_SETTINGS_EVENT,
  CLI_SETTINGS_TARGET,
  CLI_VIEW_ID,
  cliSettingsModel,
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
