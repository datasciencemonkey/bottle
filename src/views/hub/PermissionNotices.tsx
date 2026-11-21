/**
 * The hub notice of a missing OS permission (thin; CAP-36, CAP-37, #552, ADR 0101).
 *
 * The hub asks the brain with `view_emit` when it opens (ARC-2). The brain answers on
 * `hub:permissions` with the permissions that are missing. Each one shows its text, the System
 * Settings path and one control. A click only tells the brain; the brain asks the OS. The hub
 * asks again each time its window gets focus.
 * TODO(#43): the wizard step 7 mounts this notice too.
 */
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import type { PermissionKind } from '../../shared/ipc';
import {
  asPermissionModel,
  PERMISSION_ALLOW_TARGET,
  PERMISSION_CHECK_TARGET,
  PERMISSION_EVENT,
  PERMISSION_LABEL,
  PERMISSION_TEXT,
  PERMISSION_VIEW_ID,
  type PermissionNeed,
} from '../../shared/permission-view';
import { askBrain, emitView } from '../view-ipc';

const ask = (payload: { target: string; permission?: PermissionKind }) =>
  void emitView(PERMISSION_VIEW_ID, 'click', payload).catch(() => undefined);

export function PermissionNotices() {
  const [needs, setNeeds] = useState<PermissionNeed[]>([]);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void listen<unknown>(
      PERMISSION_EVENT,
      (e) => {
        const m = asPermissionModel(e.payload);
        if (m) setNeeds(m.needs);
      },
      { target: getCurrentWindow().label },
    ).then((u) => {
      if (gone) u();
      else off = u;
    });
    askBrain(PERMISSION_VIEW_ID, 'click', { target: PERMISSION_CHECK_TARGET });
    // Back from System Settings (or from the launch prompt): ask again.
    const onFocus = () => ask({ target: PERMISSION_CHECK_TARGET });
    window.addEventListener('focus', onFocus);
    return () => {
      gone = true;
      off?.();
      window.removeEventListener('focus', onFocus);
    };
  }, []);

  if (needs.length === 0) return null;
  return (
    <section
      data-view="permissions"
      aria-label="Permissions"
      className="hub-section hub-permissions"
    >
      {needs.map((n) => (
        <div key={n.permission} data-permission={n.permission} role="status">
          <p className="hub-permission-need">{PERMISSION_TEXT[n.permission].need}.</p>
          <p className="hub-muted">{PERMISSION_TEXT[n.permission].hub}</p>
          <div className="hub-actions">
            <button
              type="button"
              className="hub-primary"
              onClick={() => ask({ target: PERMISSION_ALLOW_TARGET, permission: n.permission })}
            >
              {PERMISSION_LABEL[n.action]}
            </button>
          </div>
        </div>
      ))}
    </section>
  );
}
