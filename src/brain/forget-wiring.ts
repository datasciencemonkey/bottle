/**
 * The brain side of the Forget dialog (#123; MEM-32a, MEM-32c, MEM-32d, G-45b, U9, ARC-5; ADR 0081).
 *
 * - The hub "Forget memory" button sends `forget_open` with the MOG id. The brain builds the
 *   confirm model and sends it to the hub only.
 * - `forget_confirm` runs the Forget that the brain opened. The brain keeps the scope; the view
 *   sends only the scope kind, and a kind that does not match the open dialog does nothing.
 * - `genie_one_memory_review` and `genie_one_delete` open the link of the open dialog. The view
 *   sends no URL.
 * - `forget_cancel` closes the dialog.
 * Forget runs only on these user clicks (U9). Other callers (a session list, "Stop and delete")
 * call `open(scope)` to show the dialog for a session or for items.
 * TODO(#521): the event name and the click targets move into the contract.
 */
import { emitTo } from '@tauri-apps/api/event';
import { GENIE_ONE_DELETE_TARGET } from '../shared/genie-one-delete-view';
import {
  FORGET_CANCEL_TARGET,
  FORGET_CONFIRM_TARGET,
  FORGET_DIALOG_EVENT,
  FORGET_OPEN_TARGET,
  GENIE_ONE_MEMORY_REVIEW_TARGET,
  type ForgetDialogModel,
} from '../shared/forget-view';
import { onViewInput, type BrainBus, type ViewInput } from './bus';
import type { ForgetFlow } from './components/forget-flow';
import type { ForgetScope } from './components/memory-service';

/** Send the dialog model (or `null` to close it) to the hub window only (ARC-5). */
export function sendForgetDialog(model: ForgetDialogModel | null): Promise<void> {
  return emitTo('hub', FORGET_DIALOG_EVENT, model);
}

/**
 * The default link opener. TODO(#87): open the link with the G-19a safe link opener in the
 * system browser. Until then the dialog shows each URL as text, and this opens nothing.
 */
const noOpener = async (_url: string): Promise<void> => undefined;

export interface ForgetDialogDeps {
  flow: Pick<ForgetFlow, 'dialog' | 'confirm'>;
  /** Default {@link sendForgetDialog}. */
  send?: (model: ForgetDialogModel | null) => Promise<unknown>;
  /** Opens a checked https link. Default: TODO(#87). */
  openLink?: (url: string) => Promise<void>;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const FAILED = 'Bottle could not forget this. Nothing on this computer changed. Try again.';

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

export class ForgetDialogController {
  private readonly flow: ForgetDialogDeps['flow'];
  private readonly send: (model: ForgetDialogModel | null) => Promise<unknown>;
  private readonly openLink: (url: string) => Promise<void>;
  private current: { scope: ForgetScope; model: ForgetDialogModel } | undefined;
  private busy = false;

  constructor(deps: ForgetDialogDeps) {
    this.flow = deps.flow;
    this.send = deps.send ?? sendForgetDialog;
    this.openLink = deps.openLink ?? noOpener;
  }

  /** The dialog that is open now, if any. */
  get model(): ForgetDialogModel | undefined {
    return this.current?.model;
  }

  /** Show the confirm dialog for a scope. A bad scope shows nothing (MEM-01). */
  async open(scope: ForgetScope): Promise<boolean> {
    if (this.busy) return false;
    let model: ForgetDialogModel;
    try {
      model = this.flow.dialog(scope);
    } catch {
      return false;
    }
    this.current = { scope, model };
    await this.publish(model);
    return true;
  }

  /** Handle one hub click. Anything that is not a Forget click is ignored. */
  async onInput(input: ViewInput): Promise<void> {
    if (input.from !== 'hub' || input.kind !== 'click' || !isObj(input.payload)) return;
    const p = input.payload;
    const target = p['target'];
    if (target === FORGET_OPEN_TARGET) {
      if (UUID.test(input.mogId)) await this.open({ mogId: input.mogId });
      return;
    }
    const cur = this.current;
    if (cur === undefined || cur.model.mogId !== input.mogId) return;
    if (target === FORGET_CANCEL_TARGET) {
      if (this.busy) return;
      this.current = undefined;
      await this.publish(null);
    } else if (target === FORGET_CONFIRM_TARGET) {
      if (this.busy || cur.model.phase !== 'confirm' || p['scope'] !== cur.model.scope) return;
      await this.confirm(cur.scope);
    } else if (target === GENIE_ONE_MEMORY_REVIEW_TARGET) {
      const url = cur.model.memoryReview.url;
      if (url !== undefined) await this.openUrl(url);
    } else if (target === GENIE_ONE_DELETE_TARGET) {
      const link = cur.model.result?.notice?.links.find(
        (l) => l.conversationId === p['conversationId'],
      );
      if (link !== undefined) await this.openUrl(link.url);
    }
  }

  private async confirm(scope: ForgetScope): Promise<void> {
    this.busy = true;
    try {
      const done = await this.flow.confirm(scope);
      this.current = { scope, model: done };
      await this.publish(done);
    } catch {
      const cur = this.current;
      if (cur !== undefined) {
        const model = { ...cur.model, text: FAILED };
        this.current = { scope, model };
        await this.publish(model);
      }
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

  private async publish(model: ForgetDialogModel | null): Promise<void> {
    try {
      await this.send(model);
    } catch {
      // No hub is open. The user opens the dialog again from the hub.
    }
  }
}

/** Wire the hub Forget clicks to a controller. Returns the controller and the unsubscribe. */
export function wireForget(
  bus: BrainBus,
  deps: ForgetDialogDeps,
): { controller: ForgetDialogController; off: () => void } {
  const controller = new ForgetDialogController(deps);
  const off = onViewInput(bus, (input) => void controller.onInput(input));
  return { controller, off };
}
