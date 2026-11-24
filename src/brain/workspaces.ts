/**
 * The brain side of the one MOG with workspaces (ADR 0117): the client over `workspace_list`,
 * `workspace_set_available`, `workspace_make_home`, `workspace_settings_write` and
 * `profile_reauth`, and the hub Settings → Workspace actions.
 *
 * - It keeps the last `WorkspaceList` (the Rust store is the truth) and maps it to the
 *   `HubWorkspaceList` that goes to the hub only (ARC-5). The name and the colour of a row come
 *   from the config of `mogIds[0]`. A row with no config shows the profile name and the next
 *   free palette colour.
 * - A row with no config gets one before it is switched on or made home: a new uuid, the name
 *   and `binding.home` of the profile, and the next free palette colour.
 * - Bottle never picks a home. With no home and two or more candidates the hub shows the pick
 *   dialog; `suggested` (the profile with most exchanges in the last 30 days) is a suggestion
 *   only. The first Connect with no home makes that profile home (the user's own connect).
 *   A later connection stays off.
 * - Every refusal shows a fixed text. No upstream text and no token reach the hub.
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  asWorkspaceAction,
  HUB_SCOPE,
  HUB_WORKSPACES_EVENT,
  type HubWorkspaceList,
  type HubWorkspaceRow,
  type WorkspaceAction,
} from '../shared/hub-workspaces';
import type { MogId, ProfileName } from '../shared/interfaces';
import { call, type GenieCodeStatus, type WorkspaceList, type WorkspaceRow } from '../shared/ipc';
import { MOG_PALETTE, nextVariant } from '../shared/mog-palette';
import { SIGN_IN_NOT_BROWSER, SIGN_IN_WAITING, signInCommand } from '../shared/sign-in-text';
import { onViewInput, type BrainBus } from './bus';
import { MOG_CONFIG_DEFAULTS, type MogConfig } from './components/mog-config';
import {
  ipcMogConfigClient,
  type MogConfigClient,
  type MogManager,
} from './components/mog-manager';
import { IpcLocalMemoryStore, type LocalMemoryStore } from './components/memory-store';
import { configIdentity, type MogIdentity } from './talk-target';

/** The exchanges that count for the suggested home: the last 30 days. */
export const SUGGEST_WINDOW_MS = 30 * 24 * 60 * 60 * 1000;
/** The newest exchanges read from each MOG for the suggestion. */
const SUGGEST_READ_MAX = 200;

/** The fixed texts of a refused action. */
export const WORKSPACE_TEXT = {
  failed: 'Bottle could not do that. Try again.',
  not_listed: 'Bottle cannot find this workspace. Reload the list.',
  home_always_available: 'Your home workspace is always on.',
  needs_available: 'Turn this workspace on first.',
  no_home: 'Choose a home workspace first.',
  choose_home: 'Choose a home workspace in Settings → Workspace.',
  storage_failed: 'Bottle could not read your workspaces. Try again.',
  settings: 'Your workspaces are in Settings → Workspace.',
  signed_in: 'Signed in again.',
  waiting: SIGN_IN_WAITING,
  login_cancelled: 'Sign-in cancelled.',
  timeout: 'The sign-in did not finish in 5 minutes.',
  cli_failed: 'The Databricks CLI could not finish the sign-in.',
  not_browser: SIGN_IN_NOT_BROWSER,
  auth_expired: 'Bottle could not use the new sign-in for this workspace.',
  no_grant: 'Your account has no access to this workspace. Ask your workspace admin for access.',
  offline: 'Bottle cannot reach this workspace. Check your network.',
  install_failed: 'Bottle could not install Genie Code. Try again later.',
  checksum_mismatch:
    'The Genie Code download did not match its checksum. Bottle did not install it.',
} as const;

/** The text of a sign-in that the app does not run itself. `profile` is a name, never a token. */
export const signInByHand = (profile: string): string =>
  `To sign in again, run "${signInCommand(profile)}" in Terminal, then reopen Settings.`;

const SIGN_IN_CODES: ReadonlySet<string> = new Set([
  'login_cancelled',
  'timeout',
  'cli_failed',
  'auth_expired',
  'no_grant',
  'offline',
]);

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const codeOf = (e: unknown): string =>
  isObj(e) && typeof e['error'] === 'string' ? e['error'] : '';
const PALETTE_IDS: ReadonlySet<string> = new Set(MOG_PALETTE.map((c) => c.id));

export interface WorkspacesDeps {
  /** `workspace_list`. */
  list(refresh?: boolean): Promise<WorkspaceList>;
  /** `workspace_set_available`. */
  setAvailable(profile: ProfileName, available: boolean, mogId?: MogId): Promise<WorkspaceList>;
  /** `workspace_make_home`. */
  makeHome(profile: ProfileName, mogId?: MogId): Promise<WorkspaceList>;
  /** `workspace_settings_write`. */
  settingsWrite(args: {
    crossWorkspaceData?: boolean;
    genieCallBudget?: 5 | 10 | 20;
    /** Slice 2a: the profile the agent runs on. `null` means home. */
    runProfile?: ProfileName | null;
  }): Promise<WorkspaceList>;
  /** Slice 2a: `genie_code_status`. `recheck` clears the not-eligible cache. */
  genieCodeStatus?(recheck?: boolean): Promise<GenieCodeStatus>;
  /** Slice 2a: `genie_code_install` after the user's consent. */
  genieCodeInstall?(): Promise<GenieCodeStatus>;
  /** `profile_reauth`. */
  reauth(profile: ProfileName): Promise<unknown>;
  /** `profile_login_cancel`: stops the running sign-in of the profile. */
  cancelLogin(profile: ProfileName): Promise<unknown>;
  configs: MogConfigClient;
  /** The local store, for the `mem.exchange.recent` statement of the home suggestion. */
  store: Pick<LocalMemoryStore, 'exec'>;
  /** Send the list to the hub window only. */
  send(list: HubWorkspaceList): Promise<unknown>;
  newId(): MogId;
  /** The lowest config id (`config_list`): the lamp when the workspace list cannot be read. */
  fallbackLampMogId?(): Promise<MogId | undefined>;
  now(): number;
  /** The lamp's identity changed or the home moved: push the identity to the lamp. */
  onChanged?(): void;
}

export interface Workspaces {
  /** The last list that the Rust store gave, if any. */
  last(): WorkspaceList | undefined;
  /** Slice 2a: the last known Genie Code status, or undefined before the first read. */
  genieCode(): GenieCodeStatus | undefined;
  /** Slice 2a: read the Genie Code status again and send the list. Never rejects. */
  refreshGenieCode(recheck?: boolean): Promise<void>;
  /** Slice 2a: the available workspaces with their names, for the routing rules. */
  catalog(): { profile: string; name: string; isHome: boolean; colorVariant: string }[];
  /**
   * "Change workspace…": send the list with no notice, then with the plain settings notice, so
   * the hub switches to Settings → Workspace each time.
   */
  showSettings(): Promise<void>;
  /** Read the list from the store, keep it and send it to the hub. */
  refresh(opts?: { refresh?: boolean; notice?: HubWorkspaceList['notice'] }): Promise<void>;
  /** The home profile, or undefined with no home. A failed read gives the last list's home. */
  home(): Promise<ProfileName | undefined>;
  /** The data MOG id (`mogIds[0]`) of a profile, or undefined when it has no config. */
  dataMogId(profile: ProfileName): Promise<MogId | undefined>;
  /** The lamp's name, colour and home: those of the home workspace (ADR 0117). */
  homeIdentity(lampMogId: MogId): Promise<MogIdentity | undefined>;
  /** The id of the one lamp MOG, or undefined. */
  lampMogId(): Promise<MogId | undefined>;
  /** The user picked a home (the ⌥G pick card or the hub dialog). */
  pickHome(profile: ProfileName): Promise<void>;
  /** The user connected a profile (`profile_login` ended). The first one with no home is home. */
  connected(profile: ProfileName): Promise<void>;
  /** One hub action. It never rejects: a refusal goes to the hub as a notice. */
  act(action: WorkspaceAction): Promise<void>;
}

const supportedRow = (r: WorkspaceRow) => r.signIn !== 'unsupported' && r.signIn !== 'missing';

export function createWorkspaces(deps: WorkspacesDeps): Workspaces {
  let last: WorkspaceList | undefined;
  /** Slice 2a: the last Genie Code status and the last rows with their names. */
  let genie: GenieCodeStatus | undefined;
  let namedRows: HubWorkspaceRow[] = [];
  /** Serialises the actions, so two clicks never write at once. */
  let queue: Promise<unknown> = Promise.resolve();
  const serial = <T>(fn: () => Promise<T>): Promise<T> => {
    const run = queue.then(fn, fn);
    queue = run.catch(() => undefined);
    return run;
  };

  const fetchList = async (refresh?: boolean): Promise<WorkspaceList> => {
    const l = await deps.list(refresh);
    if (!isObj(l) || !Array.isArray(l.workspaces)) throw { error: 'bad_reply' };
    last = l;
    return l;
  };
  const keep = (l: WorkspaceList): WorkspaceList => (last = l);

  /** The name and colour of each row, from the config of `mogIds[0]`. */
  async function rowsOf(l: WorkspaceList): Promise<HubWorkspaceRow[]> {
    const configs = await Promise.all(
      l.workspaces.map((r) =>
        r.mogIds[0] === undefined
          ? Promise.resolve(undefined)
          : deps.configs.read(r.mogIds[0]).catch(() => undefined),
      ),
    );
    const used = configs.flatMap((c) => (c?.ui?.variant ? [c.ui.variant] : []));
    return l.workspaces.map((r, i) => {
      const c = configs[i];
      let colorVariant = c?.ui?.variant;
      if (colorVariant === undefined) {
        colorVariant = nextVariant(used, used.length);
        used.push(colorVariant);
      }
      return {
        profile: r.profile,
        name: c?.name && c.name.trim() !== '' ? c.name : r.profile,
        host: r.host,
        colorVariant,
        signIn: r.signIn,
        available: r.available || r.isHome,
        isHome: r.isHome,
      };
    });
  }

  /** The profile with the most exchanges in the last 30 days among the candidates. */
  async function suggest(candidates: WorkspaceRow[]): Promise<string> {
    const since = deps.now() - SUGGEST_WINDOW_MS;
    const counts = new Map<string, number>(candidates.map((r) => [r.profile, 0]));
    await Promise.all(
      candidates.flatMap((r) =>
        r.mogIds.map(async (mogId) => {
          try {
            const { rows } = await deps.store.exec('mem.exchange.recent', {
              mogId,
              limit: SUGGEST_READ_MAX,
            });
            for (const x of rows) {
              if (!isObj(x) || typeof x['askedAt'] !== 'number' || x['askedAt'] < since) continue;
              const p = typeof x['profileName'] === 'string' ? x['profileName'] : r.profile;
              if (counts.has(p)) counts.set(p, (counts.get(p) ?? 0) + 1);
            }
          } catch {
            // No store (locked or off): the count stays 0.
          }
        }),
      ),
    );
    let best = candidates[0]?.profile ?? '';
    let bestN = -1;
    for (const r of candidates) {
      const n = counts.get(r.profile) ?? 0;
      if (n > bestN) {
        best = r.profile;
        bestN = n;
      }
    }
    return best;
  }

  async function toHub(
    l: WorkspaceList,
    extra: { busyProfile?: string; notice?: HubWorkspaceList['notice'] } = {},
  ): Promise<HubWorkspaceList> {
    const rows = await rowsOf(l);
    namedRows = rows;
    const out: HubWorkspaceList = {
      rows,
      home: l.home,
      agent: {
        crossWorkspaceData: l.agent.crossWorkspaceData,
        crossWorkspaceLocked: l.agent.crossWorkspaceLocked,
        genieCallBudget: l.agent.genieCallBudget,
        // Slice 2a: "Genie Code runs on", and the Genie Code status.
        ...(l.agent.runProfile !== undefined ? { runProfile: l.agent.runProfile } : {}),
        ...(genie ? { genieCode: genie } : {}),
      },
    };
    if (l.home === null) {
      // An old home that is missing or unsupported still counts as a pending decision, but only
      // supported rows can be the home.
      const old = l.workspaces.filter((r) => r.mogIds.length > 0);
      const candidates = old.filter(supportedRow);
      if (old.length >= 2) {
        if (candidates.length >= 2) {
          out.homePick = {
            suggested: await suggest(candidates),
            candidates: candidates.map((r) => r.profile),
          };
        } else if (!extra.notice) {
          out.notice = { kind: 'info', text: WORKSPACE_TEXT.choose_home };
        }
      }
    }
    if (extra.busyProfile !== undefined) out.busyProfile = extra.busyProfile;
    if (extra.notice) out.notice = extra.notice;
    return out;
  }

  /** Read the Genie Code status. A failed read keeps the last one. */
  async function readGenie(recheck?: boolean): Promise<void> {
    if (!deps.genieCodeStatus) return;
    try {
      genie = await deps.genieCodeStatus(recheck);
    } catch {
      // The status stays as it was.
    }
  }

  const publish = async (
    l: WorkspaceList | undefined,
    extra?: { busyProfile?: string; notice?: HubWorkspaceList['notice'] },
  ): Promise<void> => {
    if (!l) return;
    try {
      await deps.send(await toHub(l, extra));
    } catch {
      // No hub is open. The hub asks again with workspaces.ready when it opens.
    }
  };

  /** A new config for a profile that has none. Returns the new id. */
  async function newConfig(profile: ProfileName): Promise<MogId> {
    const used = (
      await Promise.all(
        (last?.workspaces ?? []).map((r) =>
          r.mogIds[0] === undefined
            ? Promise.resolve(undefined)
            : deps.configs.read(r.mogIds[0]).catch(() => undefined),
        ),
      )
    ).flatMap((c) => (c?.ui?.variant ? [c.ui.variant] : []));
    const config: MogConfig = {
      ...structuredClone(MOG_CONFIG_DEFAULTS),
      name: profile.length > 64 ? [...profile].slice(0, 64).join('') : profile,
      ui: { ...MOG_CONFIG_DEFAULTS.ui, variant: nextVariant(used, used.length) },
      binding: {
        home: profile,
        backend: { kind: 'genie_one_mcp', profile },
        genieCompanion: true,
        profileAliases: {},
      },
    };
    const id = deps.newId();
    await deps.configs.write(id, config);
    return id;
  }

  /**
   * Run a store change on a profile. A row with no config gets one first; if the change fails,
   * that new config is removed again, so no orphan stays.
   */
  async function withConfig(
    row: WorkspaceRow,
    change: (mogId: MogId | undefined) => Promise<WorkspaceList>,
  ): Promise<WorkspaceList> {
    if (row.mogIds.length > 0) return change(undefined);
    const id = await newConfig(row.profile);
    try {
      return await change(id);
    } catch (e) {
      await deps.configs.remove(id).catch(() => false);
      throw e;
    }
  }

  const rowOf = (profile: string): WorkspaceRow | undefined =>
    last?.workspaces.find((r) => r.profile === profile);

  function noticeOf(e: unknown): NonNullable<HubWorkspaceList['notice']> {
    const code = codeOf(e);
    if (code === 'home_always_available')
      return { kind: 'info', text: WORKSPACE_TEXT.home_always_available };
    return { kind: 'error', text: WORKSPACE_TEXT.failed };
  }

  async function makeHomeOf(profile: ProfileName): Promise<WorkspaceList> {
    const row = rowOf(profile) ?? (await fetchList()).workspaces.find((r) => r.profile === profile);
    if (!row) throw { error: 'profile_invalid' };
    return keep(await withConfig(row, (mogId) => deps.makeHome(profile, mogId)));
  }

  async function edit(a: Extract<WorkspaceAction, { action: 'workspace.edit' }>): Promise<void> {
    const row = rowOf(a.profile);
    if (!row) return publish(last, { notice: { kind: 'error', text: WORKSPACE_TEXT.not_listed } });
    const mogId = row.mogIds[0];
    if (mogId === undefined)
      return publish(last, { notice: { kind: 'info', text: WORKSPACE_TEXT.needs_available } });
    if (a.colorVariant !== undefined && !PALETTE_IDS.has(a.colorVariant))
      return publish(last, { notice: { kind: 'error', text: WORKSPACE_TEXT.failed } });
    const c = await deps.configs.read(mogId);
    if (a.name !== undefined) c.name = a.name.trim();
    if (a.colorVariant !== undefined) c.ui = { ...c.ui, variant: a.colorVariant };
    await deps.configs.write(mogId, c);
    deps.onChanged?.();
    await publish(last);
  }

  async function run(a: WorkspaceAction): Promise<void> {
    switch (a.action) {
      case 'workspaces.ready':
        // The hub opens: read again, and let the store probe the sign-in states.
        await readGenie();
        return publish(await fetchList(true));
      case 'workspace.set_available': {
        const row = rowOf(a.profile);
        if (!row)
          return publish(last, { notice: { kind: 'error', text: WORKSPACE_TEXT.not_listed } });
        if (row.isHome && !a.available)
          return publish(last, {
            notice: { kind: 'info', text: WORKSPACE_TEXT.home_always_available },
          });
        await publish(last, { busyProfile: a.profile });
        const l = a.available
          ? await withConfig(row, (mogId) => deps.setAvailable(a.profile, true, mogId))
          : await deps.setAvailable(a.profile, false);
        keep(l);
        return publish(l);
      }
      case 'workspace.make_home':
      case 'workspace.pick_home': {
        if (a.action === 'workspace.pick_home' && last?.home) {
          // The dialog shows only with no home. A pick after a home exists changes nothing.
          return publish(last);
        }
        await publish(last, { busyProfile: a.profile });
        const l = await makeHomeOf(a.profile);
        deps.onChanged?.();
        return publish(l);
      }
      case 'workspace.sign_in': {
        const row = rowOf(a.profile);
        if (!row)
          return publish(last, { notice: { kind: 'error', text: WORKSPACE_TEXT.not_listed } });
        // A click queued behind another sign-in of this row: that one may have signed it in.
        if (row.signIn !== 'expired') return publish(last);
        // `profile_reauth` blocks until the CLI ends (as `profile_login` does): say what it waits for.
        await publish(last, {
          busyProfile: a.profile,
          notice: { kind: 'info', text: WORKSPACE_TEXT.waiting },
        });
        try {
          await deps.reauth(a.profile);
        } catch (e) {
          const code = codeOf(e);
          if (code === 'not_implemented') {
            // The host has no re-sign-in (yet): show the CLI command as a visible error, not a
            // quiet note. The hub row also has a Copy button. The next `workspaces.ready`
            // reads the new state.
            return publish(last, { notice: { kind: 'error', text: signInByHand(a.profile) } });
          }
          if (code === 'login_cancelled') {
            const l = await fetchList().catch(() => last);
            return publish(l, { notice: { kind: 'info', text: WORKSPACE_TEXT.login_cancelled } });
          }
          if (code === 'profile_invalid') {
            // A PAT or service-principal profile: a CLI login would change it. The row has Copy.
            return publish(last, { notice: { kind: 'error', text: WORKSPACE_TEXT.not_browser } });
          }
          const text = SIGN_IN_CODES.has(code)
            ? WORKSPACE_TEXT[code as keyof typeof WORKSPACE_TEXT]
            : WORKSPACE_TEXT.failed;
          const l = await fetchList().catch(() => last);
          return publish(l, { notice: { kind: 'error', text } });
        }
        const l = await fetchList(true);
        return publish(l, { notice: { kind: 'info', text: WORKSPACE_TEXT.signed_in } });
      }
      case 'workspace.sign_in_cancel':
        return; // `act` runs it outside the queue.
      case 'workspace.edit':
        return edit(a);
      case 'workspace.budget':
        return publish(keep(await deps.settingsWrite({ genieCallBudget: a.genieCallBudget })));
      case 'workspace.cross':
        return publish(
          keep(await deps.settingsWrite({ crossWorkspaceData: a.crossWorkspaceData })),
        );
      case 'workspace.agent_profile': {
        // Slice 2a: Rust ends the session after the live turn. The agent profile state changes.
        const l = keep(await deps.settingsWrite({ runProfile: a.profile }));
        await readGenie();
        return publish(l);
      }
      case 'genie_code.recheck':
        await readGenie(true);
        return publish(last);
      case 'genie_code.install': {
        // The busy state shows while the install runs.
        if (genie) genie = { ...genie, binary: 'installing' };
        await publish(last);
        try {
          genie = await (deps.genieCodeInstall?.() ?? Promise.reject({ error: 'not_implemented' }));
        } catch (e) {
          await readGenie();
          const code = codeOf(e);
          const text =
            code === 'checksum_mismatch'
              ? WORKSPACE_TEXT.checksum_mismatch
              : WORKSPACE_TEXT.install_failed;
          return publish(last, { notice: { kind: 'error', text } });
        }
        return publish(last);
      }
    }
  }

  return {
    last: () => last,
    genieCode: () => genie,
    async refreshGenieCode(recheck) {
      await readGenie(recheck);
      await publish(last);
    },
    catalog: () =>
      namedRows
        .filter(
          (r) => (r.available || r.isHome) && r.signIn !== 'unsupported' && r.signIn !== 'missing',
        )
        .map((r) => ({
          profile: r.profile,
          name: r.name,
          isHome: r.isHome,
          colorVariant: r.colorVariant,
        })),
    async refresh(opts) {
      try {
        await readGenie();
        const l = await fetchList(opts?.refresh);
        await publish(l, opts?.notice ? { notice: opts.notice } : undefined);
      } catch (e) {
        // A list that cannot be read shows its own text; an action keeps the generic one.
        const notice: NonNullable<HubWorkspaceList['notice']> =
          codeOf(e) === 'workspace_storage_failed'
            ? { kind: 'error', text: WORKSPACE_TEXT.storage_failed }
            : noticeOf(e);
        if (last) await publish(last, { notice });
        else
          // Nothing was ever read: the hub still gets an empty list with the reason.
          await deps
            .send({
              rows: [],
              home: null,
              agent: {
                crossWorkspaceData: false,
                crossWorkspaceLocked: false,
                genieCallBudget: 10,
              },
              notice,
            })
            .catch(() => undefined);
      }
    },
    async showSettings() {
      // Two pushes: the hub resets its notice key on a push with no notice, so the second push
      // is always a new notice.
      const l = last ?? (await fetchList().catch(() => undefined));
      await publish(l);
      await publish(l, { notice: { kind: 'info', text: WORKSPACE_TEXT.settings } });
    },
    async home() {
      // The cache is read again after a write and on `workspaces.ready` only.
      const l = last ?? (await fetchList().catch(() => undefined));
      return l?.home ?? undefined;
    },
    async dataMogId(profile) {
      const l = last ?? (await fetchList().catch(() => undefined));
      return l?.workspaces.find((r) => r.profile === profile)?.mogIds[0];
    },
    async lampMogId() {
      const l = last ?? (await fetchList().catch(() => undefined));
      // A failed list must not kill the lamp: fall back to the lowest config id.
      return l?.lampMogId ?? (await deps.fallbackLampMogId?.().catch(() => undefined));
    },
    async homeIdentity(lampMogId) {
      const l = last ?? (await fetchList().catch(() => undefined));
      const home = l?.workspaces.find((r) => r.isHome);
      const homeMog = home?.mogIds[0];
      if (home && homeMog === undefined) return { name: home.profile, home: home.profile };
      const id = await configIdentity(homeMog ?? lampMogId);
      if (!home && id) {
        // With no home the lamp names no workspace.
        const rest = { ...id };
        delete rest.home;
        return rest;
      }
      return id;
    },
    pickHome: (profile) =>
      serial(async () => {
        keep(await makeHomeOf(profile));
        deps.onChanged?.();
        await publish(last);
      }),
    connected: (profile) =>
      serial(async () => {
        try {
          const l = await fetchList(true);
          const row = l.workspaces.find((r) => r.profile === profile);
          // Any other row with a MOG is an old home, even one that is missing or unsupported.
          const oldHomes = l.workspaces.filter((r) => r.profile !== profile && r.mogIds.length > 0);
          // The first Connect with no home is home. A home that waits for the user's pick
          // stays open: Bottle never overrides it. A later connection stays off.
          if (row && supportedRow(row) && l.home === null && oldHomes.length === 0) {
            keep(await withConfig(row, (mogId) => deps.makeHome(profile, mogId)));
            deps.onChanged?.();
          }
        } catch {
          // The list shows the real state; the hub can make the home by hand.
        }
        await publish(last);
      }),
    act: (action) => {
      if (action.action === 'workspace.sign_in_cancel') {
        if (!rowOf(action.profile)) {
          return publish(last, { notice: { kind: 'error', text: WORKSPACE_TEXT.not_listed } });
        }
        // The sign-in holds the queue until it ends: the cancel must not wait behind it. The
        // waiting `workspace.sign_in` then ends with `login_cancelled`, which shows the notice.
        return deps.cancelLogin(action.profile).then(
          () => undefined,
          () => publish(last, { notice: { kind: 'error', text: WORKSPACE_TEXT.failed } }),
        );
      }
      return serial(async () => {
        try {
          await run(action);
        } catch (e) {
          await publish(last ?? (await fetchList().catch(() => undefined)), {
            notice: noticeOf(e),
          });
        }
      });
    },
  };
}

/** The production deps: `call`, the config client, the local store and `emitTo('hub', …)`. */
export function ipcWorkspacesDeps(onChanged?: () => void): WorkspacesDeps {
  const configs = ipcMogConfigClient();
  return {
    list: (refresh) => call('workspace_list', refresh === undefined ? {} : { refresh }),
    setAvailable: (profile, available, mogId) =>
      call('workspace_set_available', {
        profile,
        available,
        ...(mogId !== undefined ? { mogId } : {}),
      }),
    makeHome: (profile, mogId) =>
      call('workspace_make_home', { profile, ...(mogId !== undefined ? { mogId } : {}) }),
    settingsWrite: (args) => call('workspace_settings_write', args),
    genieCodeStatus: (recheck) => call('genie_code_status', recheck ? { recheck } : {}),
    genieCodeInstall: () => call('genie_code_install', { consent: true }),
    reauth: (profile) => call('profile_reauth', { profile }),
    cancelLogin: (profile) => call('profile_login_cancel', { profile }),
    configs,
    store: new IpcLocalMemoryStore(),
    send: (list) => emitTo('hub', HUB_WORKSPACES_EVENT, list),
    newId: () => crypto.randomUUID(),
    fallbackLampMogId: async () => {
      const ids = (await configs.list()).items.map((i) => i.id).sort();
      return ids[0];
    },
    now: () => Date.now(),
    ...(onChanged ? { onChanged } : {}),
  };
}

/** Wire the hub workspace actions. Only the hub window may run one. Returns the unsubscribe. */
export function wireWorkspaces(bus: BrainBus, ws: Workspaces): () => void {
  return onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.mogId !== HUB_SCOPE || input.kind !== 'click') return;
    const action = asWorkspaceAction(input.payload);
    if (action) void ws.act(action);
  });
}

/**
 * ADR 0117: the talk key with no open MOG starts the lamp and no other MOG. The old MOGs stay
 * defined as the data folders of their workspaces, so this view of T1 lists the lamp only.
 */
export function lampOnlyManager(
  manager: Pick<MogManager, 'list' | 'start' | 'runState'>,
  lampMogId: () => Promise<MogId | undefined>,
): Pick<MogManager, 'list' | 'start' | 'runState'> {
  return {
    list: async () => {
      const [l, lamp] = await Promise.all([manager.list(), lampMogId()]);
      return { ...l, mogs: l.mogs.filter((m) => m.mogId === lamp) };
    },
    start: (mogId) => manager.start(mogId),
    runState: (mogId) => manager.runState(mogId),
  };
}
