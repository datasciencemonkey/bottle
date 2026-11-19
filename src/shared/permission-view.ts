/**
 * The OS permission notice (CAP-36, CAP-37, #552, ADR 0101). At launch the host asks once for
 * each permission that was never asked (ADR 0101 decision 7). Without Input Monitoring, macOS
 * gives Bottle no talk-key events, so a talk-key press does nothing and shows nothing. Without
 * Accessibility, Bottle cannot paste. The brain reads both with `permission_status` (no prompt)
 * at launch and sends the hub the permissions that are missing. The first MOG that opens shows
 * the first one in its pop-up. Each surface has one control: a click asks the brain, and the
 * brain relays it with `permission_request` (MOG-08a). The first click shows the OS prompt; a
 * later click opens the System Settings pane.
 *
 * The brain and the hub view both read this file. It holds static data and shape checks only.
 * It is not a contract file. TODO(#525): the event name moves into the contract with the other
 * hub events.
 */
import type { PermissionKind } from './ipc';

/** The brain-to-hub event of the model. */
export const PERMISSION_EVENT = 'hub:permissions';
/** The `view_emit` id of the hub's permission notice. */
export const PERMISSION_VIEW_ID = 'permissions';
/** The hub asks for the model when it opens. */
export const PERMISSION_CHECK_TARGET = 'permissions.check';
/** A click on the control of one permission: `{ target, permission }`. */
export const PERMISSION_ALLOW_TARGET = 'permissions.allow';
/** The blurb, control and chip ID prefix of a pop-up notice. */
export const PERMISSION_CHIP_PREFIX = 'perm:';

/**
 * What the control of a missing permission does: `allow` shows the OS prompt, `settings` opens
 * the System Settings pane (the permission is denied, or the prompt showed in this run).
 */
export type PermissionAction = 'allow' | 'settings';

export interface PermissionNeed {
  permission: PermissionKind;
  action: PermissionAction;
}

/** The missing permissions, Input Monitoring first. Empty: nothing to show. */
export interface PermissionModel {
  needs: PermissionNeed[];
}

/** The control labels. */
export const PERMISSION_LABEL: Record<PermissionAction, string> = {
  allow: 'Allow',
  settings: 'Open Settings',
};

/**
 * The texts. `need` is the first notice. `settings` is the short pop-up line after the prompt
 * showed or for a denied permission. `hub` is the hub help line. macOS starts the talk key only
 * after Bottle reopens, so the Input Monitoring texts say so.
 */
export const PERMISSION_TEXT: Record<
  PermissionKind,
  { need: string; settings: string; hub: string }
> = {
  input_monitoring: {
    need: 'Bottle needs Input Monitoring to hear the talk key',
    settings: 'Turn on Bottle in Settings > Input Monitoring, then reopen Bottle',
    hub: 'Turn on Bottle in System Settings > Privacy & Security > Input Monitoring. If Bottle is on the list already, remove it and add it again. Then quit and reopen Bottle.',
  },
  accessibility: {
    need: 'Bottle needs Accessibility to paste',
    settings: 'Turn on Bottle in Settings > Accessibility',
    hub: 'Turn on Bottle in System Settings > Privacy & Security > Accessibility. If Bottle is on the list already, remove it and add it again.',
  },
};

export const PERMISSION_KINDS: readonly PermissionKind[] = ['input_monitoring', 'accessibility'];

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

export const isPermissionKind = (v: unknown): v is PermissionKind =>
  PERMISSION_KINDS.includes(v as PermissionKind);

/** Shape check of a `hub:permissions` payload. */
export function asPermissionModel(p: unknown): PermissionModel | undefined {
  if (!isObj(p) || !Array.isArray(p['needs'])) return undefined;
  const needs: PermissionNeed[] = [];
  for (const n of p['needs']) {
    if (!isObj(n) || !isPermissionKind(n['permission'])) return undefined;
    if (n['action'] !== 'allow' && n['action'] !== 'settings') return undefined;
    needs.push({ permission: n['permission'], action: n['action'] });
  }
  return { needs };
}
