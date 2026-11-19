/**
 * The brain side of the hub Workspaces area (#560, #486, #163).
 *
 * The brain sends the list to the hub only (ARC-5). The hub sends `workspaces.ready` when it
 * opens and `workspaces.refresh` on the Refresh click. "Connect a workspace" sends no action of
 * this module: the view reuses the existing ConnectWorkspace trigger (`connect_open`) directly.
 * After each MOG change (a create, an edit, a home change, a delete, ...) the list goes again,
 * so "Home of" stays correct.
 * TODO(#407): the event name and the actions move into the contract.
 */
import { emitTo } from '@tauri-apps/api/event';
import { call } from '../shared/ipc';
import {
  asWorkspaceAction,
  HUB_SCOPE,
  HUB_WORKSPACES_EVENT,
  type HubCliInfo,
  type HubWorkspaceList,
  type HubWorkspaceRow,
} from '../shared/hub-workspaces';
import type { Profile, ProfileName } from '../shared/interfaces';
import { U2M } from './connect-wiring';
import { onViewInput, type BrainBus } from './bus';
import type { MogManager } from './components/mog-manager';
import type { SignInState } from './sign-in-state';

/** W-03b, W-10a: why Bottle cannot use an unsupported profile. Never upstream text. */
function reasonFor(p: Profile): string {
  return p.authType === U2M
    ? "This profile's token is not stored securely. Sign in again after you fix the storage."
    : 'This profile does not use Databricks sign-in in the browser. Sign in with a bottle profile.';
}

export interface HubWorkspacesDeps {
  /** T1, for the MOGs whose home is each profile, and to refresh after a MOG change. */
  manager: Pick<MogManager, 'list' | 'onChange'>;
  /** `profiles_list` (T7, R5). */
  listProfiles(opts: { refresh?: boolean }): Promise<Profile[]>;
  /** `cli_status` (R5, W-10a). */
  cliStatus(): Promise<HubCliInfo>;
  /** The sign-in state of a supported profile this session. TODO(#486): `profile_status`. */
  signInState(profile: ProfileName): SignInState;
}

async function snapshot(deps: HubWorkspacesDeps, refresh: boolean): Promise<HubWorkspaceList> {
  let cli: HubCliInfo;
  try {
    cli = await deps.cliStatus();
  } catch {
    cli = { path: '', version: '', found: false, plaintextStorage: false };
  }
  try {
    const [profiles, listing] = await Promise.all([
      deps.listProfiles({ refresh }),
      deps.manager.list(),
    ]);
    const homeOf = new Map<string, string[]>();
    for (const m of listing.mogs) {
      homeOf.set(m.binding.home, [...(homeOf.get(m.binding.home) ?? []), m.name]);
    }
    const rows: HubWorkspaceRow[] = profiles.map((p) => ({
      name: p.name,
      host: p.host,
      authType: p.authType,
      supported: p.supported,
      ...(p.supported ? { signIn: deps.signInState(p.name) } : { reason: reasonFor(p) }),
      homeOf: homeOf.get(p.name) ?? [],
    }));
    return { profiles: rows, cli };
  } catch {
    return {
      profiles: [],
      cli,
      notice: { kind: 'error', text: 'Bottle cannot list your Databricks CLI profiles.' },
    };
  }
}

/** Send the Workspaces list to the hub window only. */
export function sendHubWorkspaces(list: HubWorkspaceList): Promise<void> {
  return emitTo('hub', HUB_WORKSPACES_EVENT, list);
}

/**
 * Wire the hub Workspaces actions to `deps`, send the list once now, and send it again after
 * each MOG change so "Home of" stays correct.
 */
export function wireHubWorkspaces(
  bus: BrainBus,
  deps: HubWorkspacesDeps,
  send: (list: HubWorkspaceList) => Promise<unknown> = sendHubWorkspaces,
): () => void {
  let live = true;
  const publish = async (refresh: boolean) => {
    const list = await snapshot(deps, refresh);
    if (!live) return;
    try {
      await send(list);
    } catch {
      // No hub is open. The hub asks again with workspaces.ready when it opens.
    }
  };
  void publish(false);
  const offMogs = deps.manager.onChange(() => void publish(false));
  const offInput = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== HUB_SCOPE || input.kind !== 'click') return;
    // HUB_SCOPE is shared with the hub MOG list (#26) and the hub notice (#27); any payload that
    // is not exactly one of ours is ignored.
    const action = asWorkspaceAction(input.payload);
    if (!action) return;
    if (action.action === 'workspaces.ready') void publish(false);
    else void publish(true);
  });
  return () => {
    live = false;
    offMogs();
    offInput();
  };
}

/** `profiles_list` and `cli_status` over `call` (T7, R5). */
export function ipcHubWorkspacesIo(): Pick<HubWorkspacesDeps, 'listProfiles' | 'cliStatus'> {
  return {
    listProfiles: (opts) => call('profiles_list', opts),
    cliStatus: () => call('cli_status', {}),
  };
}
