/**
 * The hub Workspaces area: the Databricks CLI profiles, their sign-in state and which MOGs use
 * them as home (#560, #486, #163).
 *
 * The brain (T7 ProfileService, T1 MogManager) and the hub view both read this file. It holds
 * names and shape checks only, no state. It is not a contract file.
 * TODO(#407): move `HUB_WORKSPACES_EVENT`, `HubWorkspaceList` and the workspace actions into
 * `src/shared/ipc.ts` (`EVENTS['hub.workspaces']` and a hub-level view input), together with the
 * `hub-mogs.ts` items under the same TODO.
 */
import type { HubNoticeLine } from './hub-mogs';

/** The brain-to-view event that carries the workspace list to the hub. TODO(#407): `EVENTS`. */
export const HUB_WORKSPACES_EVENT = 'hub:workspaces';

/**
 * The `mogId` of a hub-level `view_emit`, shared with #27 and #26. It cannot be a MOG ID, because
 * MOG IDs are UUIDs. TODO(#407): a hub-level view input with no `mogId`.
 */
export { HUB_SCOPE } from './hub-notice';

/**
 * The sign-in state of a supported profile, derived from the outcome of the last Genie ask on it
 * this session (there is no host route for `profile_status` yet, TODO(#486)). `unknown` means
 * Bottle has not sent anything to this profile yet this session.
 */
export type ProfileSignInState = 'signed_in' | 'needs_sign_in' | 'unknown';

/** One row of the hub Workspaces list: one Databricks CLI profile. */
export interface HubWorkspaceRow {
  name: string;
  host: string;
  authType: string;
  supported: boolean;
  /** Why Bottle cannot use this profile. Undefined when `supported` is true. */
  reason?: string;
  /** Undefined for an unsupported profile: it never shows a sign-in state. */
  signIn?: ProfileSignInState;
  /** The names of the MOGs whose `binding.home` is this profile's name. */
  homeOf: string[];
}

/** The Databricks CLI that `cli_status` reports (#16, W-10a). */
export interface HubCliInfo {
  path: string;
  version: string;
  found: boolean;
  /** W-10a: at least one `databricks-cli` profile stores its token in plaintext. */
  plaintextStorage: boolean;
}

/** What the hub Workspaces area shows. */
export interface HubWorkspaceList {
  profiles: HubWorkspaceRow[];
  cli: HubCliInfo;
  notice?: HubNoticeLine;
}

/** The hub Workspaces actions, sent as `view_emit` kind `click` with mogId `HUB_SCOPE`. */
export type WorkspaceAction = { action: 'workspaces.ready' } | { action: 'workspaces.refresh' };

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isStr = (v: unknown): v is string => typeof v === 'string';
const isStrArray = (v: unknown): v is string[] => Array.isArray(v) && v.every(isStr);
const SIGN_IN_STATES = ['signed_in', 'needs_sign_in', 'unknown'] as const;

/** Shape check of a hub Workspaces action. */
export function asWorkspaceAction(p: unknown): WorkspaceAction | undefined {
  if (!isObj(p)) return undefined;
  const { action } = p;
  if (action === 'workspaces.ready' || action === 'workspaces.refresh') return { action };
  return undefined;
}

function isRow(v: unknown): v is HubWorkspaceRow {
  if (
    !isObj(v) ||
    !isStr(v['name']) ||
    !isStr(v['host']) ||
    !isStr(v['authType']) ||
    typeof v['supported'] !== 'boolean' ||
    !isStrArray(v['homeOf'])
  )
    return false;
  if (v['reason'] !== undefined && !isStr(v['reason'])) return false;
  if (
    v['signIn'] !== undefined &&
    !(SIGN_IN_STATES as readonly string[]).includes(v['signIn'] as string)
  )
    return false;
  return true;
}

function isCliInfo(v: unknown): v is HubCliInfo {
  return (
    isObj(v) &&
    isStr(v['path']) &&
    isStr(v['version']) &&
    typeof v['found'] === 'boolean' &&
    typeof v['plaintextStorage'] === 'boolean'
  );
}

/** Shape check of a `hub:workspaces` payload. */
export function asHubWorkspaceList(p: unknown): HubWorkspaceList | undefined {
  if (!isObj(p)) return undefined;
  const { profiles, cli, notice } = p;
  if (!Array.isArray(profiles) || !profiles.every(isRow)) return undefined;
  if (!isCliInfo(cli)) return undefined;
  if (
    notice !== undefined &&
    !(
      isObj(notice) &&
      (notice['kind'] === 'error' || notice['kind'] === 'info') &&
      isStr(notice['text'])
    )
  )
    return undefined;
  return p as unknown as HubWorkspaceList;
}
