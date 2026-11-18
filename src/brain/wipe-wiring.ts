/**
 * The brain side of Settings → Wipe (#124; MEM-33a, SEC-15c, MB-15, G-45b, U9, ARC-5; ADR 0087).
 *
 * - The hub panel sends `wipe_settings` when it mounts. The brain answers the 9.2 retention rows
 *   with the admin locks, to the hub only.
 * - The hub Wipe button sends `wipe_open` with the option and the Genie choice. The brain builds
 *   the confirm model and sends it to the hub only.
 * - `wipe_confirm` runs the Wipe that the brain opened. The brain keeps the option; the view
 *   sends only the option, and one that does not match the open dialog does nothing.
 * - `genie_one_delete` opens a link of the done dialog. The view sends no URL.
 * - `wipe_cancel` closes the dialog.
 * Every Wipe click comes from the hub window with the fixed id `WIPE_VIEW_ID`. The Wipe runs only
 * on the confirm click (U9). There is no timer.
 * TODO(#525): the event names and the click targets move into the contract.
 */
import { emitTo } from '@tauri-apps/api/event';
import { GENIE_ONE_DELETE_TARGET } from '../shared/genie-one-delete-view';
import {
  isWipeScope,
  WIPE_CANCEL_TARGET,
  WIPE_CONFIRM_TARGET,
  WIPE_DIALOG_EVENT,
  WIPE_OPEN_TARGET,
  WIPE_SETTINGS_EVENT,
  WIPE_SETTINGS_TARGET,
  WIPE_VIEW_ID,
  type WipeDialogModel,
  type WipeScope,
  type WipeSettingsModel,
} from '../shared/wipe-view';
import { onViewInput, type BrainBus, type ViewInput } from './bus';
import type { WipeFlow } from './components/wipe-flow';

/** Send the dialog model (or `null` to close it) to the hub window only (ARC-5). */
export function sendWipeDialog(model: WipeDialogModel | null): Promise<void> {
  return emitTo('hub', WIPE_DIALOG_EVENT, model);
}

/** Send the retention rows to the hub window only (ARC-5). */
export function sendWipeSettings(model: WipeSettingsModel): Promise<void> {
  return emitTo('hub', WIPE_SETTINGS_EVENT, model);
}

/** The default link opener. TODO(#87): the G-19a safe link opener. The URL shows as text. */
const noOpener = async (_url: string): Promise<void> => undefined;

export interface WipeDialogDeps {
  flow: Pick<WipeFlow, 'dialog' | 'confirm'>;
  /** The retention rows with the admin policy applied (SEC-15c). */
  settings: () => Promise<WipeSettingsModel>;
  /** Default {@link sendWipeDialog}. */
  send?: (model: WipeDialogModel | null) => Promise<unknown>;
  /** Default {@link sendWipeSettings}. */
  sendSettings?: (model: WipeSettingsModel) => Promise<unknown>;
  /** Opens a checked https link. Default: TODO(#87). */
  openLink?: (url: string) => Promise<void>;
}

const FAILED = 'Bottle could not wipe your data. Nothing on this computer changed. Try again.';

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

export class WipeDialogController {
  private readonly flow: WipeDialogDeps['flow'];
  private readonly settings: () => Promise<WipeSettingsModel>;
  private readonly send: (model: WipeDialogModel | null) => Promise<unknown>;
  private readonly sendSettings: (model: WipeSettingsModel) => Promise<unknown>;
  private readonly openLink: (url: string) => Promise<void>;
  private current: WipeDialogModel | undefined;
  private busy = false;

  constructor(deps: WipeDialogDeps) {
    this.flow = deps.flow;
    this.settings = deps.settings;
    this.send = deps.send ?? sendWipeDialog;
    this.sendSettings = deps.sendSettings ?? sendWipeSettings;
    this.openLink = deps.openLink ?? noOpener;
  }

  /** The dialog that is open now, if any. */
  get model(): WipeDialogModel | undefined {
    return this.current;
  }

  /** Handle one hub click. Anything that is not a Wipe click is ignored. */
  async onInput(input: ViewInput): Promise<void> {
    if (input.from !== 'hub' || input.kind !== 'click' || !isObj(input.payload)) return;
    const p = input.payload;
    const target = p['target'];
    if (target === GENIE_ONE_DELETE_TARGET) {
      const link = this.current?.result?.notices
        .find((n) => n.mogId === input.mogId)
        ?.links.find((l) => l.conversationId === p['conversationId']);
      if (link !== undefined) await this.openUrl(link.url);
      return;
    }
    if (input.mogId !== WIPE_VIEW_ID) return;
    if (target === WIPE_SETTINGS_TARGET) {
      await this.publishSettings();
    } else if (target === WIPE_OPEN_TARGET) {
      if (this.busy || !isWipeScope(p['scope']) || typeof p['alsoGenie'] !== 'boolean') return;
      await this.open(p['scope'], p['alsoGenie']);
    } else if (target === WIPE_CANCEL_TARGET) {
      if (this.busy || this.current === undefined) return;
      this.current = undefined;
      await this.publish(null);
    } else if (target === WIPE_CONFIRM_TARGET) {
      const cur = this.current;
      if (this.busy || cur === undefined || cur.phase !== 'confirm' || p['scope'] !== cur.scope) {
        return;
      }
      await this.confirm(cur);
    }
  }

  private async open(scope: WipeScope, alsoGenie: boolean): Promise<void> {
    let model: WipeDialogModel;
    try {
      model = this.flow.dialog(scope, alsoGenie);
    } catch {
      return;
    }
    this.current = model;
    await this.publish(model);
  }

  private async confirm(cur: WipeDialogModel): Promise<void> {
    this.busy = true;
    try {
      const done = await this.flow.confirm(cur.scope, cur.alsoGenie);
      this.current = done;
      await this.publish(done);
    } catch {
      const model = { ...cur, text: FAILED };
      this.current = model;
      await this.publish(model);
    } finally {
      this.busy = false;
    }
  }

  private async openUrl(url: string): Promise<void> {
    try {
      await this.openLink(url);
    } catch {
      // The URL also shows as text in the dialog.
    }
  }

  private async publish(model: WipeDialogModel | null): Promise<void> {
    try {
      await this.send(model);
    } catch {
      // No hub is open. The user opens the dialog again from Settings.
    }
  }

  private async publishSettings(): Promise<void> {
    try {
      await this.sendSettings(await this.settings());
    } catch {
      // No hub is open, or no policy: the panel asks again when it mounts.
    }
  }
}

/** Wire the hub Wipe clicks to a controller. Returns the controller and the unsubscribe. */
export function wireWipe(
  bus: BrainBus,
  deps: WipeDialogDeps,
): { controller: WipeDialogController; off: () => void } {
  const controller = new WipeDialogController(deps);
  const off = onViewInput(bus, (input) => void controller.onInput(input));
  return { controller, off };
}
