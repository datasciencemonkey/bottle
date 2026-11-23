/**
 * The router input of the one MOG from the home profile and the profile list (#163, ADR 0092,
 * ADR 0117).
 *
 * - The home profile is `WorkspaceList.home` (`workspaces.ts`), which the user chose: clicked in
 *   the hub dialog, on the pick card (`profile-pick.ts`), or made by their first Connect. With
 *   no home the input has `NO_HOME`, which gives the pick card. Bottle never picks one.
 * - W-03a: only the `profiles_list` profiles that Bottle supports go to the router.
 * - A home that the CLI does not list is no home: the router model cannot run on it, so the
 *   RTR-15c heuristic decides and a question shows the pick card.
 * - When the list itself fails (the host does not serve `profiles_list` yet, TODO(#17)), the
 *   home of the workspace list stays in use.
 * - It never returns undefined, so every utterance is routed (RTR-01b).
 * It holds profile names and hosts only, never a token.
 */
import type { MogId, Profile, ProfileName } from '../shared/interfaces';
import { call } from '../shared/ipc';
import type { RouterInput } from './components/router-step';
import { ipcMogConfigClient } from './components/mog-manager';
import { NO_HOME } from './offline-intent';

export interface HomeProfileDeps {
  /** The name of a MOG (`config_read`). */
  readMog(mogId: MogId): Promise<{ name: string }>;
  /** The home profile (`WorkspaceList.home`), or undefined with no home. */
  home(): Promise<ProfileName | undefined>;
  /** `profiles_list` (T7, R5). */
  listProfiles(): Promise<Profile[]>;
}

export interface HomeProfiles {
  routerInput(mogId: MogId): Promise<RouterInput>;
  /** The usable home profile of the MOG's last router input, or undefined if it had none. */
  home(mogId: MogId): ProfileName | undefined;
}

const isProfileList = (v: unknown): v is Profile[] =>
  Array.isArray(v) &&
  v.every(
    (p) =>
      typeof p === 'object' &&
      p !== null &&
      typeof (p as Profile).name === 'string' &&
      typeof (p as Profile).host === 'string' &&
      typeof (p as Profile).supported === 'boolean',
  );

/** `profiles_list` over `call`. A malformed reply is a failure. */
export async function ipcListProfiles(): Promise<Profile[]> {
  const r: unknown = await call('profiles_list', {});
  if (!isProfileList(r)) throw { error: 'bad_reply' };
  return r;
}

export function ipcHomeProfileDeps(home: () => Promise<ProfileName | undefined>): HomeProfileDeps {
  const configs = ipcMogConfigClient();
  return {
    readMog: async (mogId) => ({ name: (await configs.read(mogId)).name }),
    home,
    listProfiles: ipcListProfiles,
  };
}

export function createHomeProfiles(deps: HomeProfileDeps): HomeProfiles {
  const homes = new Map<MogId, ProfileName>();
  return {
    async routerInput(mogId) {
      const [mog, typed, listed] = await Promise.all([
        deps.readMog(mogId).catch(() => undefined),
        deps.home().then(
          (h) => h?.trim() ?? '',
          () => '',
        ),
        deps.listProfiles().catch(() => undefined),
      ]);
      const supported = (listed ?? []).filter((p) => p.supported);
      const home =
        typed !== '' && (listed === undefined || supported.some((p) => p.name === typed))
          ? typed
          : NO_HOME;
      if (home === NO_HOME) homes.delete(mogId);
      else homes.set(mogId, home);
      return {
        profile: home,
        speakerTrack: 'you',
        captureMode: 'ptt',
        pendingClarification: '',
        recentTopics: [],
        // TODO(#112): the other open MOGs, for `directed_mog_id` (D63).
        activeMogs: [{ id: mogId, name: mog?.name ?? '' }],
        profiles: supported.map((p) => ({ name: p.name, host: p.host, aliases: [] })),
        currentTarget: home,
      };
    },
    home: (mogId) => homes.get(mogId),
  };
}
