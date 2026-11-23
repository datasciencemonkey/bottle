/**
 * The hub workspace list and the hub workspace actions (ADR 0117, one MOG with workspaces).
 *
 * The brain (`src/brain/workspaces.ts`) and the hub view (Settings → Workspace) both read this
 * file. It holds names and shape checks only, no state. It is not a contract file, but it is
 * frozen for the slice: change it only through the orchestrator.
 */

import { asGenieCodeStatus } from './hub-agent';
import type { GenieCodeStatus } from './ipc';

/** The brain-to-view event that carries the workspace list to the hub. */
export const HUB_WORKSPACES_EVENT = 'hub:workspaces';

/** The `mogId` of a hub-level `view_emit`, shared with the hub notice. */
export { HUB_SCOPE } from './hub-notice';

export type HubSignIn = 'signed_in' | 'expired' | 'unknown' | 'unsupported' | 'missing';
export type HubGenieCallBudget = 5 | 10 | 20;

/** One row of Settings → Workspace. */
export interface HubWorkspaceRow {
  profile: string;
  /** The workspace name (the old MOG name, or the profile name for a new connection). */
  name: string;
  /** The workspace host, for the muted line. "" when the profile is missing. */
  host: string;
  /** `mog.ui.variant` of the workspace (`src/shared/mog-palette.ts`). Never a raw hex. */
  colorVariant: string;
  signIn: HubSignIn;
  /** "Available to your MOG". Always true for home. */
  available: boolean;
  isHome: boolean;
}

/** What Settings → Workspace shows. */
export interface HubWorkspaceList {
  /** Home first, then by profile. */
  rows: HubWorkspaceRow[];
  /** The home profile. null until the user picks one. */
  home: string | null;
  /** The one-time "Choose your home workspace" dialog. Present only when the user must pick. */
  homePick?: { suggested: string; candidates: string[] };
  agent: {
    /** D1 "Combine across workspaces". */
    crossWorkspaceData: boolean;
    /** SEC-28b: an admin forces D1 off. */
    crossWorkspaceLocked: boolean;
    /** D2 "Genie calls per question". */
    genieCallBudget: HubGenieCallBudget;
    /** Slice 2a: "Agent runs on". null means home. Absent until the brain sends the agent state. */
    runProfile?: string | null;
    /** Slice 2a: the Genie Code binary and the agent profile state. Absent until the brain sends it. */
    genieCode?: GenieCodeStatus;
  };
  /** A row with a sign-in or a switch change in progress: the view shows a spinner on it. */
  busyProfile?: string;
  notice?: { kind: 'error' | 'info'; text: string };
}

/** The hub workspace actions, sent as `view_emit` kind `click` with mogId `HUB_SCOPE`. */
export type WorkspaceAction =
  | { action: 'workspaces.ready' }
  | { action: 'workspace.set_available'; profile: string; available: boolean }
  | { action: 'workspace.make_home'; profile: string; confirmed: true }
  | { action: 'workspace.pick_home'; profile: string }
  | { action: 'workspace.sign_in'; profile: string }
  | { action: 'workspace.edit'; profile: string; name?: string; colorVariant?: string }
  /** Slice 1 (the agent loop) ships the control. */
  | { action: 'workspace.budget'; genieCallBudget: HubGenieCallBudget }
  /** Slice 2 (combine across workspaces) ships the control. */
  | { action: 'workspace.cross'; crossWorkspaceData: boolean }
  /** Slice 2a: "Agent runs on". null means home. */
  | { action: 'workspace.agent_profile'; profile: string | null }
  /** Slice 2a: [Install…] of Genie Code, after the user's consent. */
  | { action: 'genie_code.install'; consent: true }
  /** Slice 2a: "Check again" clears the not-eligible cache. */
  | { action: 'genie_code.recheck' };

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
const isStr = (v: unknown): v is string => typeof v === 'string';
const isBool = (v: unknown): v is boolean => typeof v === 'boolean';
const isBudget = (v: unknown): v is HubGenieCallBudget => v === 5 || v === 10 || v === 20;
const SIGN_IN: readonly HubSignIn[] = ['signed_in', 'expired', 'unknown', 'unsupported', 'missing'];
const isSignIn = (v: unknown): v is HubSignIn => SIGN_IN.includes(v as HubSignIn);
/** A profile name: not empty, at most 128 characters. */
const isProfile = (v: unknown): v is string => isStr(v) && v.length > 0 && v.length <= 128;

/** The action of one hub click, or undefined. Unknown fields are dropped. */
export function asWorkspaceAction(p: unknown): WorkspaceAction | undefined {
  if (!isObj(p)) return undefined;
  const { action, profile } = p;
  switch (action) {
    case 'workspaces.ready':
      return { action };
    case 'workspace.set_available':
      return isProfile(profile) && isBool(p['available'])
        ? { action, profile, available: p['available'] }
        : undefined;
    case 'workspace.make_home':
      return isProfile(profile) && p['confirmed'] === true
        ? { action, profile, confirmed: true }
        : undefined;
    case 'workspace.pick_home':
    case 'workspace.sign_in':
      return isProfile(profile) ? { action, profile } : undefined;
    case 'workspace.edit': {
      if (!isProfile(profile)) return undefined;
      const name = p['name'];
      const colorVariant = p['colorVariant'];
      if (name !== undefined && !(isStr(name) && name.trim().length > 0 && name.length <= 64)) {
        return undefined;
      }
      if (colorVariant !== undefined && !isStr(colorVariant)) return undefined;
      if (name === undefined && colorVariant === undefined) return undefined;
      return {
        action,
        profile,
        ...(name !== undefined ? { name } : {}),
        ...(colorVariant !== undefined ? { colorVariant } : {}),
      };
    }
    case 'workspace.budget':
      return isBudget(p['genieCallBudget'])
        ? { action, genieCallBudget: p['genieCallBudget'] }
        : undefined;
    case 'workspace.cross':
      return isBool(p['crossWorkspaceData'])
        ? { action, crossWorkspaceData: p['crossWorkspaceData'] }
        : undefined;
    case 'workspace.agent_profile':
      return profile === null || isProfile(profile) ? { action, profile } : undefined;
    case 'genie_code.install':
      return p['consent'] === true ? { action, consent: true } : undefined;
    case 'genie_code.recheck':
      return { action };
    default:
      return undefined;
  }
}

function asRow(v: unknown): HubWorkspaceRow | undefined {
  if (!isObj(v)) return undefined;
  const { profile, name, host, colorVariant, signIn, available, isHome } = v;
  if (!isProfile(profile) || !isStr(name) || !isStr(host) || !isStr(colorVariant)) {
    return undefined;
  }
  if (!isSignIn(signIn) || !isBool(available) || !isBool(isHome)) return undefined;
  return { profile, name, host, colorVariant, signIn, available, isHome };
}

/** The workspace list from one `hub:workspaces` event, or undefined. Unknown fields are dropped. */
export function asHubWorkspaceList(p: unknown): HubWorkspaceList | undefined {
  if (!isObj(p) || !Array.isArray(p['rows']) || !isObj(p['agent'])) return undefined;
  const rows: HubWorkspaceRow[] = [];
  for (const r of p['rows']) {
    const row = asRow(r);
    if (!row) return undefined;
    rows.push(row);
  }
  const home = p['home'];
  if (home !== null && !isProfile(home)) return undefined;
  const a = p['agent'];
  if (!isBool(a['crossWorkspaceData']) || !isBool(a['crossWorkspaceLocked'])) return undefined;
  if (!isBudget(a['genieCallBudget'])) return undefined;
  const runProfile = a['runProfile'];
  if (runProfile !== undefined && runProfile !== null && !isProfile(runProfile)) return undefined;
  const genieCode = a['genieCode'] === undefined ? undefined : asGenieCodeStatus(a['genieCode']);
  if (a['genieCode'] !== undefined && !genieCode) return undefined;
  const out: HubWorkspaceList = {
    rows,
    home,
    agent: {
      crossWorkspaceData: a['crossWorkspaceData'],
      crossWorkspaceLocked: a['crossWorkspaceLocked'],
      genieCallBudget: a['genieCallBudget'],
      ...(runProfile !== undefined ? { runProfile } : {}),
      ...(genieCode ? { genieCode } : {}),
    },
  };
  const pick = p['homePick'];
  if (pick !== undefined) {
    if (!isObj(pick) || !isProfile(pick['suggested']) || !Array.isArray(pick['candidates'])) {
      return undefined;
    }
    const candidates = pick['candidates'];
    // The dialog shows only when the user must pick from 2 or more, and it preselects one of them.
    if (!candidates.every(isProfile) || candidates.length < 2) return undefined;
    if (!candidates.includes(pick['suggested'])) return undefined;
    out.homePick = { suggested: pick['suggested'], candidates: candidates as string[] };
  }
  // `busyProfile` and `notice` are hints for the view: a bad value is dropped, not fatal.
  if (isProfile(p['busyProfile'])) out.busyProfile = p['busyProfile'];
  const n = p['notice'];
  if (isObj(n) && (n['kind'] === 'error' || n['kind'] === 'info') && isStr(n['text'])) {
    out.notice = { kind: n['kind'], text: n['text'] };
  }
  return out;
}
