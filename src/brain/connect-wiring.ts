/**
 * The brain side of "Connect a workspace" (#162; W-07a, W-07b, W-07c, W-07f, W-07g, W-07h,
 * W-07i, W-15, ARC-2).
 *
 * The thin hub view sends clicks (id `connect`); the brain owns every rule and sends the model
 * to the hub only on `hub:connect_workspace`:
 * - W-07a, W-07b, W-07g: the URL step keeps only `https://<host>` of the pasted URL and refuses a
 *   host outside the 4.6 Databricks domain list. The host checks the same rules again
 *   (`mog_core::profile_login`). The app never guesses a URL.
 * - W-07g: it proposes `bottle-<workspace>`, not a name that exists; the user can change it.
 * - W-07i: a PAT or service-principal profile for the host gives a notice; Bottle makes a new
 *   `bottle-*` OAuth profile and never changes the old one.
 * - W-07c, W-15: T7 `login` calls `profile_login`, which blocks until `databricks auth login`
 *   (5 min limit) and the W-06a check end. A custom OAuth client ID is optional.
 * - W-07h: Cancel calls `profile_login_cancel`; "Open the browser again" cancels and signs in
 *   again. A failure shows one fixed Table W-ERR text and one action, never upstream text.
 * - W-07f: when the hub is ready and no usable (`databricks-cli`) profile exists, the form opens
 *   at the URL step (`profiles_list`, routed by #555). With a usable profile, it sends the
 *   explicit `closed` step instead (the hub's setup region then renders nothing, review of
 *   #602/the "Connect a workspace" panel bug). If the list call fails, the form opens only from
 *   its button (Settings, "Connect another workspace…").
 */
import { emitTo } from '@tauri-apps/api/event';
import {
  CONNECT_EVENT,
  CONNECT_VIEW_ID,
  PROFILE_NAME_INVALID_TEXT,
  PROFILE_NAME_TAKEN_TEXT,
  type ConnectModel,
} from '../shared/connect-view';
import type { Profile, ProfileService } from '../shared/interfaces';
import { onViewInput, type BrainBus } from './bus';

/**
 * The Databricks domain list of 4.6 rule 2 (`mog_core::http_transport::host_policy`).
 * `staging.databricks.com` and `dev.databricks.com` cover internal staging/dev workspaces (e.g.
 * `dogfood.staging.databricks.com`), owner review 2026-11-24: Connect refused those hosts before
 * this list named them.
 */
export const DATABRICKS_DOMAINS = [
  'cloud.databricks.com',
  'gcp.databricks.com',
  'azuredatabricks.net',
  'cloud.databricks.us',
  'databricks.azure.us',
  'staging.databricks.com',
  'dev.databricks.com',
] as const;

/** The fixed texts of the form. Upstream text never shows (W-14). */
export const CONNECT_TEXT = {
  host_refused:
    'Use the URL of a Databricks workspace, for example https://<name>.cloud.databricks.com.',
  profile_exists: PROFILE_NAME_TAKEN_TEXT,
  invalid_args: PROFILE_NAME_INVALID_TEXT,
  otherAuthNotice:
    'A personal access token or service principal profile exists for this workspace. Bottle does not change it. Bottle makes a new OAuth profile.',
  timeout: 'The sign-in did not finish in 5 minutes.',
  cli_failed: 'The Databricks CLI could not finish the sign-in.',
  auth_expired: 'Bottle could not use the new sign-in for this workspace.',
  no_grant: 'Your account has no access to this workspace. Ask your workspace admin for access.',
  non_json_reply:
    'A network page came back instead of Databricks. An IP access list, a VPN or a proxy can cause this.',
  offline: 'Bottle cannot reach this workspace. Check your network.',
  failed: 'Bottle could not connect this workspace.',
} as const;

const PREFIX = 'bottle-';
const U2M = 'databricks-cli';

function isDnsLabel(l: string): boolean {
  return /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(l);
}

/** True for a workspace host of 4.6 rule 2: the domain list, DNS labels, not `accounts`. */
function isWorkspaceHost(host: string): boolean {
  const labels = host.split('.');
  const first = labels[0] ?? '';
  return (
    DATABRICKS_DOMAINS.some((d) => host.length > d.length + 1 && host.endsWith(`.${d}`)) &&
    labels.every(isDnsLabel) &&
    first !== 'accounts' &&
    !first.startsWith('accounts-')
  );
}

/** W-07g: `https://<host>` of a pasted URL, or `undefined` if it is not a workspace (W-07b). */
export function workspaceHost(pasted: string): string | undefined {
  const text = pasted.trim();
  if (text === '' || /\s/.test(text)) return undefined;
  let url: URL;
  try {
    // A scheme only at the start: a `://` in a query does not count.
    url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `https://${text}`);
  } catch {
    return undefined;
  }
  if (url.protocol !== 'https:' || url.username !== '' || url.password !== '') return undefined;
  if (url.port !== '' && url.port !== '443') return undefined;
  const host = url.hostname.toLowerCase();
  return isWorkspaceHost(host) ? `https://${host}` : undefined;
}

/** W-07g: `bottle-<workspace>`, from the first label of the host. */
export function proposedProfileName(host: string): string {
  const label = host.replace(/^https:\/\//, '').split('.')[0] ?? '';
  return `${PREFIX}${label}`;
}

/** The proposal, or `<proposal>-2`, `-3`, ... when that name exists (any case). */
function freeName(base: string, taken: readonly string[]): string {
  const lower = new Set(taken.map((n) => n.toLowerCase()));
  if (!lower.has(base.toLowerCase())) return base;
  for (let i = 2; ; i += 1) {
    const name = `${base}-${i}`;
    if (!lower.has(name.toLowerCase())) return name;
  }
}

export interface ConnectDeps {
  /** T7 `login` (`profile_login`). */
  login: ProfileService['login'];
  /** `profile_login_cancel`. */
  cancel: (profile: string) => Promise<unknown>;
  /** `profiles_list`: names, hosts and auth types only. */
  profiles: () => Promise<Profile[]>;
  /**
   * ADR 0117: the user's sign-in of `profile` ended well. The first Connect with no home makes
   * that profile home; a later connection stays off. A fault never changes the connected step.
   */
  onConnected?: ((profile: string) => unknown) | undefined;
  /** Default: `emitTo('hub', CONNECT_EVENT, model)`. */
  send?: (model: ConnectModel) => Promise<unknown>;
  /** Runs `fn` after `ms`; returns the cancel. Default: `setTimeout`. Tests inject their own. */
  after?: (ms: number, fn: () => void) => () => void;
}

/** How long the connected step shows before the brain closes the flow by itself. */
export const CONNECTED_CLOSE_MS = 4000;

/** Where the open flow came from: first run (W-07f, setup region) or Settings (W-07e, inline). */
type Origin = 'first_run' | 'settings';

/** The model without the `inline` flag, so `show` sets it again from the current origin. */
function stripInline(m: ConnectModel): ConnectModel {
  if (m.step === 'closed') return m;
  const rest: Record<string, unknown> = { ...m };
  delete rest['inline'];
  return rest as ConnectModel;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/** The fixed code of a refused call (`{error}`), or `''`. */
function codeOf(e: unknown): string {
  return isObj(e) && typeof e['error'] === 'string' ? e['error'] : '';
}

/** Wire the hub form to T7. Returns the unsubscribe. */
export function wireConnectWorkspace(bus: BrainBus, deps: ConnectDeps): () => void {
  const send = deps.send ?? ((m: ConnectModel) => emitTo('hub', CONNECT_EVENT, m));
  const after =
    deps.after ??
    ((ms: number, fn: () => void) => {
      const t = setTimeout(fn, ms);
      return () => clearTimeout(t);
    });
  // The flow was opened from Settings (W-07e): the model carries `inline`, so Settings shows it
  // under its row and the setup region does not. The last model is kept so a view that mounts
  // late (or a reloaded hub) gets the current step again.
  let origin: Origin | undefined;
  let last: ConnectModel | undefined;
  let cancelClose: (() => void) | undefined;
  const show = (m: ConnectModel) => {
    const out: ConnectModel =
      m.step !== 'closed' && origin === 'settings' ? { ...m, inline: true } : m;
    last = out;
    void send(out).catch(() => undefined);
  };
  /** End the flow: both hosts render nothing for `closed`. */
  const close = () => {
    cancelClose?.();
    cancelClose = undefined;
    origin = undefined;
    show({ step: 'closed' });
  };
  const profiles = () => deps.profiles().catch((): Profile[] => []);

  let host: string | undefined;
  let profile = '';
  let clientId: string | undefined;
  let otherAuthNotice = false;
  let running = false;
  let reopen = false;
  /** The other profile names of the last `profiles_list` read (W-07g): the view refuses a
   * duplicate at once, without a round trip. */
  let taken: string[] = [];

  const nameStep = (error?: string): ConnectModel => ({
    step: 'name',
    host: host ?? '',
    proposedName: profile,
    otherAuthNotice,
    taken,
    ...(error === undefined ? {} : { error }),
  });

  async function signIn(): Promise<void> {
    if (host === undefined) return;
    const h = host;
    running = true;
    show({ step: 'waiting', host: h, profile });
    try {
      await deps.login(h, profile, clientId === undefined ? undefined : { clientId });
      running = false;
      if (deps.onConnected) {
        try {
          void Promise.resolve(deps.onConnected(profile)).catch(() => undefined);
        } catch {
          // The workspace list shows the real state.
        }
      }
      show({ step: 'connected', host: h, profile });
      // The confirmation clears by itself; Done closes it sooner.
      cancelClose?.();
      cancelClose = after(CONNECTED_CLOSE_MS, () => {
        if (last?.step === 'connected') close();
      });
    } catch (e) {
      running = false;
      const code = codeOf(e);
      if (code === 'login_cancelled') {
        if (reopen) {
          reopen = false;
          await signIn();
          return;
        }
        // Cancel ends the flow. First run keeps the URL step: W-07f needs a way to connect.
        if (origin === 'first_run') show({ step: 'url', firstRun: true });
        else close();
      } else if (code === 'profile_exists' || code === 'invalid_args') {
        show(nameStep(CONNECT_TEXT[code]));
      } else if (code === 'host_refused') {
        show({ step: 'url', firstRun: false, error: CONNECT_TEXT.host_refused });
      } else if (code === 'no_grant') {
        show({
          step: 'failed',
          host: h,
          profile,
          text: CONNECT_TEXT.no_grant,
          action: { label: 'Use another workspace', target: 'connect_open' },
        });
      } else {
        const text =
          code === 'timeout' ||
          code === 'cli_failed' ||
          code === 'auth_expired' ||
          code === 'non_json_reply' ||
          code === 'offline'
            ? CONNECT_TEXT[code]
            : CONNECT_TEXT.failed;
        show({
          step: 'failed',
          host: h,
          profile,
          text,
          action: { label: 'Try again', target: 'connect_retry' },
        });
      }
    }
  }

  async function onUrl(url: string): Promise<void> {
    const h = workspaceHost(url);
    if (h === undefined) {
      show({ step: 'url', firstRun: false, error: CONNECT_TEXT.host_refused });
      return;
    }
    const list = await profiles();
    host = h;
    otherAuthNotice = list.some((p) => p.host === h && p.authType !== U2M);
    taken = list.map((p) => p.name);
    profile = freeName(proposedProfileName(h), taken);
    show(nameStep());
  }

  /** Show the current step again, or run the W-07f first-run check when nothing is open. */
  const refresh = () => {
    if (last !== undefined && last.step !== 'closed') {
      show(stripInline(last));
      return;
    }
    // W-07f: no usable profile (none, or only PAT or service-principal profiles) opens the URL
    // step. A usable profile sends the explicit `closed` state. A failed `profiles_list` call
    // sends nothing: the form opens only from the Settings row.
    void deps
      .profiles()
      .then((list) => {
        if (list.some((p) => p.supported)) show({ step: 'closed' });
        else {
          origin = 'first_run';
          show({ step: 'url', firstRun: true });
        }
      })
      .catch(() => undefined);
  };

  const offReady = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.kind !== 'ready') return;
    // The hub is ready, or the connect panel (`mogId` `connect`) mounted and listens.
    if (input.mogId === 'hub' || input.mogId === CONNECT_VIEW_ID) refresh();
  });

  const offClick = onViewInput(bus, (input) => {
    if (input.from !== 'hub' || input.kind !== 'click' || input.mogId !== CONNECT_VIEW_ID) return;
    const p = input.payload;
    if (!isObj(p)) return;
    switch (p['target']) {
      case 'connect_open':
        if (running) break;
        cancelClose?.();
        cancelClose = undefined;
        origin ??= 'settings';
        show({ step: 'url', firstRun: origin === 'first_run' });
        break;
      case 'connect_close':
        if (!running) close();
        break;
      case 'connect_url':
        if (!running && typeof p['url'] === 'string') void onUrl(p['url']);
        break;
      case 'connect_start': {
        if (running || host === undefined || typeof p['profile'] !== 'string') return;
        profile = p['profile'].trim();
        const id = typeof p['clientId'] === 'string' ? p['clientId'].trim() : '';
        clientId = id === '' ? undefined : id;
        void signIn();
        break;
      }
      case 'connect_cancel':
        if (running) void deps.cancel(profile).catch(() => undefined);
        break;
      case 'connect_reopen':
        if (running) {
          reopen = true;
          void deps.cancel(profile).catch(() => undefined);
        }
        break;
      case 'connect_retry':
        if (!running && host !== undefined && profile !== '') void signIn();
        break;
      default:
        break;
    }
  });

  return () => {
    cancelClose?.();
    offReady();
    offClick();
  };
}
