/**
 * The brain side of the OS permission notice (CAP-36, CAP-37, #552, ADR 0101).
 *
 * - At launch, and each time the hub opens, the brain reads Input Monitoring and Accessibility
 *   with `permission_status`. It never shows a prompt. The hub gets the permissions that are
 *   missing (`hub:permissions`). A failed read shows nothing.
 * - The first MOG view that opens in this run shows the first missing permission in its pop-up,
 *   with one control. The other MOGs show nothing.
 * - Only a click on that control, or on the hub control of a permission, sends
 *   `permission_request` for that permission (views call only `view_*`, MOG-08a). The host
 *   shows the OS prompt on the first click and opens the System Settings pane on a later one.
 *   After the click the brain reads again, so the texts and the control follow the state.
 */
import { emitTo } from '@tauri-apps/api/event';
import type { MogId } from '../shared/interfaces';
import type { PermissionKind, PermissionState } from '../shared/ipc';
import { isViewReady } from '../shared/mog-target';
import {
  isPermissionKind,
  PERMISSION_ALLOW_TARGET,
  PERMISSION_CHECK_TARGET,
  PERMISSION_CHIP_PREFIX,
  PERMISSION_EVENT,
  PERMISSION_KINDS,
  PERMISSION_LABEL,
  PERMISSION_TEXT,
  PERMISSION_VIEW_ID,
  type PermissionModel,
  type PermissionNeed,
} from '../shared/permission-view';
import { onViewInput, type BrainBus } from './bus';
import type { CardStack } from './card-stack';

export interface PermissionWiringDeps {
  /** `permission_status`: no prompt. */
  status(permission: PermissionKind): Promise<PermissionState>;
  /** `permission_request`: only for a user click on a control of `permission`. */
  request(permission: PermissionKind): Promise<PermissionState>;
  /** Send the model to the hub only. */
  sendHub?(model: PermissionModel): Promise<unknown>;
  stackFor(mogId: MogId): Pick<CardStack, 'add' | 'setControls' | 'clearControls'>;
  showCard(mogId: MogId): Promise<unknown>;
  hideCard(mogId: MogId): Promise<unknown>;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const chipId = (p: PermissionKind) => `${PERMISSION_CHIP_PREFIX}${p}`;

export function wirePermissions(bus: BrainBus, deps: PermissionWiringDeps): () => void {
  const sendHub = deps.sendHub ?? ((m: PermissionModel) => emitTo('hub', PERMISSION_EVENT, m));
  /** The permissions whose prompt showed in this run: the next click opens Settings. */
  const asked = new Set<PermissionKind>();
  const busy = new Set<PermissionKind>();
  let needs: PermissionNeed[] = [];
  /** The MOG of the pop-up notice, and the permission that it shows now. */
  let popup: { mogId: MogId; shows?: PermissionKind } | undefined;

  const read = async () => {
    const next: PermissionNeed[] = [];
    for (const permission of PERMISSION_KINDS) {
      let s: PermissionState;
      try {
        s = await deps.status(permission);
      } catch {
        continue; // No host answer: show nothing for it.
      }
      if (s === 'granted') continue;
      const action = s === 'denied' || asked.has(permission) ? 'settings' : 'allow';
      next.push({ permission, action });
    }
    needs = next;
  };

  const renderPopup = () => {
    if (!popup) return;
    const { mogId, shows } = popup;
    const stack = deps.stackFor(mogId);
    const need = needs[0];
    if (shows !== undefined && shows !== need?.permission) stack.clearControls(chipId(shows));
    if (!need) {
      if (shows !== undefined) void deps.hideCard(mogId).catch(() => undefined);
      popup = { mogId };
      return;
    }
    const id = chipId(need.permission);
    const text = PERMISSION_TEXT[need.permission];
    stack.add({
      id,
      text: need.action === 'settings' ? text.settings : text.need,
      completed: true,
    });
    stack.setControls(id, [{ id, label: PERMISSION_LABEL[need.action] }], () =>
      allow(need.permission),
    );
    popup = { mogId, shows: need.permission };
  };

  const publish = async () => {
    renderPopup();
    try {
      await sendHub({ needs: needs.map((n) => ({ ...n })) });
    } catch {
      // No hub is open: it asks again when it opens.
    }
  };

  /** Reads run one after the other, so an old reply never wins. */
  let last: Promise<void> = Promise.resolve();
  const refresh = () => (last = last.then(read).then(publish, publish));

  const allow = (permission: PermissionKind) => {
    if (busy.has(permission)) return;
    busy.add(permission);
    void (async () => {
      try {
        await deps.request(permission);
      } catch {
        // The host did not answer: the next read shows the state.
      } finally {
        asked.add(permission);
        busy.delete(permission);
      }
      await refresh();
    })();
  };

  void refresh();

  return onViewInput(bus, (input) => {
    if (input.from === `mog-${input.mogId}` && input.kind === 'click') {
      if (!isViewReady(input.payload) || popup) return;
      const mogId = input.mogId;
      popup = { mogId };
      void last.then(() => {
        renderPopup();
        if (popup?.shows !== undefined) void deps.showCard(mogId).catch(() => undefined);
      });
      return;
    }
    if (input.from !== 'hub' || input.kind !== 'click' || input.mogId !== PERMISSION_VIEW_ID)
      return;
    if (!isObj(input.payload)) return;
    const target = input.payload['target'];
    if (target === PERMISSION_CHECK_TARGET) void refresh();
    else if (target === PERMISSION_ALLOW_TARGET && isPermissionKind(input.payload['permission']))
      allow(input.payload['permission']);
  });
}
