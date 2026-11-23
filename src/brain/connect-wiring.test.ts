/**
 * #162: the brain side of "Connect a workspace" (W-07a, W-07b, W-07c, W-07f, W-07g, W-07h,
 * W-07i, W-15). Fake hosts and fake profiles only; no CLI runs and no token exists here.
 */
import { CONNECT_EVENT, CONNECT_VIEW_ID, type ConnectModel } from '../shared/connect-view';
import { BrainBus } from './bus';
import {
  CONNECT_TEXT,
  CONNECTED_CLOSE_MS,
  proposedProfileName,
  wireConnectWorkspace,
  workspaceHost,
  type ConnectDeps,
} from './connect-wiring';
import { core } from './test-core';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const HOST = 'https://acme-prod.cloud.databricks.com';
const flush = () => new Promise((r) => setTimeout(r, 0));

function click(bus: BrainBus, payload: Record<string, unknown>, from = 'hub') {
  bus.dispatch({
    event: 'view:input',
    payload: { from, input: { mogId: CONNECT_VIEW_ID, kind: 'click', payload } },
  });
}

function ready(bus: BrainBus) {
  bus.dispatch({
    event: 'view:input',
    payload: { from: 'hub', input: { mogId: 'hub', kind: 'ready' } },
  });
}

/** A deferred `profile_login`: the test ends it. */
function deferred() {
  let resolve!: (v: { loginId: string }) => void;
  let reject!: (e: unknown) => void;
  const p = new Promise<{ loginId: string }>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { p, resolve, reject };
}

function setup(over: Partial<ConnectDeps> = {}) {
  const sent: ConnectModel[] = [];
  const deps: ConnectDeps = {
    login: vi.fn(async () => ({ loginId: 'login-1' })),
    cancel: vi.fn(async () => undefined),
    profiles: vi.fn(async () => []),
    send: async (m) => {
      sent.push(m);
    },
    ...over,
  };
  const bus = new BrainBus();
  wireConnectWorkspace(bus, deps);
  return { bus, deps, sent, last: (): ConnectModel | undefined => sent.at(-1) };
}

describe('Connect a workspace: URL step (W-07a, W-07b, W-07g)', () => {
  it('[W-07g] a pasted browser URL keeps only https://<host>', () => {
    expect(workspaceHost('https://acme-prod.cloud.databricks.com/browse?o=123')).toBe(HOST);
    expect(workspaceHost('  acme-prod.cloud.databricks.com/sql#x ')).toBe(HOST);
    expect(workspaceHost('https://ACME-prod.cloud.databricks.com:443/')).toBe(HOST);
    expect(workspaceHost('https://adb-1234567890123456.7.azuredatabricks.net/?o=1')).toBe(
      'https://adb-1234567890123456.7.azuredatabricks.net',
    );
  });

  it('[W-07b] a non-Databricks domain is refused', () => {
    for (const bad of [
      'https://example.com/?o=1',
      'https://cloud.databricks.com',
      'https://acme.cloud.databricks.com.evil.example',
      'https://accounts.cloud.databricks.com',
      'http://acme-prod.cloud.databricks.com',
      'https://u@acme-prod.cloud.databricks.com',
      'https://acme-prod.cloud.databricks.com:8443',
      '',
    ]) {
      expect(workspaceHost(bad), bad).toBeUndefined();
    }
  });

  it('[W-07a] [W-07g] proposes bottle-<workspace> for the host', () => {
    expect(proposedProfileName(HOST)).toBe('bottle-acme-prod');
  });

  it('[W-07a] [W-07g] the URL step answers the host and the proposed name; the user URL is never guessed', async () => {
    const { bus, last, deps } = setup();
    click(bus, { target: 'connect_open' });
    await flush();
    expect(last()).toEqual({ step: 'url', firstRun: false, inline: true });
    click(bus, {
      target: 'connect_url',
      url: 'https://acme-prod.cloud.databricks.com/browse?o=123',
    });
    await flush();
    expect(last()).toEqual({
      step: 'name',
      host: HOST,
      proposedName: 'bottle-acme-prod',
      otherAuthNotice: false,
      inline: true,
    });
    expect(deps.login).not.toHaveBeenCalled();
  });

  it('[W-07b] another domain stays on the URL step with the refusal text', async () => {
    const { bus, last } = setup();
    click(bus, { target: 'connect_url', url: 'https://example.com' });
    await flush();
    expect(last()).toEqual({ step: 'url', firstRun: false, error: CONNECT_TEXT.host_refused });
  });

  it('[W-07g] the proposed name is not one that exists', async () => {
    const { bus, last } = setup({
      profiles: async () => [
        { name: 'bottle-acme-prod', host: HOST, authType: 'databricks-cli', supported: true },
      ],
    });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    expect(last()).toMatchObject({ step: 'name', proposedName: 'bottle-acme-prod-2' });
  });
});

describe('Connect a workspace: sign-in (W-07c, W-07g, W-07h, W-15)', () => {
  it('[W-07c] sign-in calls profile_login with the host only and shows the waiting step, then connected', async () => {
    const d = deferred();
    const { bus, last, deps } = setup({ login: vi.fn(() => d.p) });
    click(bus, { target: 'connect_url', url: `${HOST}/browse?o=1` });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-acme-prod' });
    await flush();
    expect(deps.login).toHaveBeenCalledWith(HOST, 'bottle-acme-prod', undefined);
    expect(last()).toEqual({ step: 'waiting', host: HOST, profile: 'bottle-acme-prod' });
    d.resolve({ loginId: 'login-1' });
    await flush();
    expect(last()).toEqual({ step: 'connected', host: HOST, profile: 'bottle-acme-prod' });
  });

  it('[ADR 0117] a finished sign-in tells onConnected the profile once; a failed one does not', async () => {
    const onConnected = vi.fn();
    const ok = setup({ onConnected });
    click(ok.bus, { target: 'connect_url', url: `${HOST}/browse?o=1` });
    await flush();
    click(ok.bus, { target: 'connect_start', profile: 'bottle-acme-prod' });
    await flush();
    expect(onConnected).toHaveBeenCalledTimes(1);
    expect(onConnected).toHaveBeenCalledWith('bottle-acme-prod');
    const failedCb = vi.fn();
    const bad = setup({
      onConnected: failedCb,
      login: vi.fn(async () => Promise.reject({ error: 'timeout' })),
    });
    click(bad.bus, { target: 'connect_url', url: `${HOST}/browse?o=1` });
    await flush();
    click(bad.bus, { target: 'connect_start', profile: 'bottle-acme-prod' });
    await flush();
    expect(failedCb).not.toHaveBeenCalled();
  });

  it('[ADR 0117] a throwing onConnected never changes the connected step', async () => {
    const { bus, last } = setup({
      onConnected: () => {
        throw new Error('x');
      },
    });
    click(bus, { target: 'connect_url', url: `${HOST}/browse?o=1` });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-acme-prod' });
    await flush();
    expect(last()).toMatchObject({ step: 'connected' });
  });

  it('[W-15] a custom OAuth client ID goes to profile_login', async () => {
    const { bus, deps } = setup();
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-custom', clientId: ' custom-app-id ' });
    await flush();
    expect(deps.login).toHaveBeenCalledWith(HOST, 'bottle-custom', { clientId: 'custom-app-id' });
  });

  it('[W-07g] an existing name is refused and the name step shows why', async () => {
    const { bus, last } = setup({
      login: vi.fn(async () => {
        throw { error: 'profile_exists' };
      }),
    });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'acme' });
    await flush();
    expect(last()).toEqual({
      step: 'name',
      host: HOST,
      proposedName: 'acme',
      otherAuthNotice: false,
      error: CONNECT_TEXT.profile_exists,
    });
  });

  it('[W-07h] Cancel stops the CLI child and closes the flow (not the name step)', async () => {
    const d = deferred();
    const cancel = vi.fn(async () => d.reject({ error: 'login_cancelled' }));
    const { bus, last, deps } = setup({ login: vi.fn(() => d.p), cancel });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-acme-prod' });
    await flush();
    click(bus, { target: 'connect_cancel' });
    await flush();
    expect(cancel).toHaveBeenCalledWith('bottle-acme-prod');
    expect(last()).toEqual({ step: 'closed' });
    expect(deps.login).toHaveBeenCalledTimes(1);
  });

  it('[W-07h] "Open the browser again" cancels and starts the same sign-in again', async () => {
    const first = deferred();
    const second = deferred();
    const login = vi.fn().mockReturnValueOnce(first.p).mockReturnValueOnce(second.p);
    const cancel = vi.fn(async () => first.reject({ error: 'login_cancelled' }));
    const { bus, last } = setup({ login, cancel });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-x', clientId: 'custom-app-id' });
    await flush();
    click(bus, { target: 'connect_reopen' });
    await flush();
    expect(cancel).toHaveBeenCalledWith('bottle-x');
    expect(login).toHaveBeenCalledTimes(2);
    expect(login).toHaveBeenLastCalledWith(HOST, 'bottle-x', { clientId: 'custom-app-id' });
    expect(last()).toEqual({ step: 'waiting', host: HOST, profile: 'bottle-x' });
    second.resolve({ loginId: 'login-2' });
    await flush();
    expect(last()).toMatchObject({ step: 'connected' });
  });

  it('[W-07h] a failed check shows the Table W-ERR text and exactly one action, never upstream text', async () => {
    for (const code of [
      'timeout',
      'cli_failed',
      'auth_expired',
      'no_grant',
      'non_json_reply',
      'offline',
      'host_failed',
    ]) {
      const { bus, last } = setup({
        login: vi.fn(async () => {
          throw { error: code, detail: 'upstream-detail-that-must-not-show' };
        }),
      });
      click(bus, { target: 'connect_url', url: HOST });
      await flush();
      click(bus, { target: 'connect_start', profile: 'bottle-x' });
      await flush();
      const m = last();
      expect(m?.step, code).toBe('failed');
      if (m?.step !== 'failed') continue;
      expect(m.text.length, code).toBeGreaterThan(0);
      expect(JSON.stringify(m)).not.toContain('upstream-detail');
      expect(typeof m.action.label).toBe('string');
      expect(['connect_retry', 'connect_open']).toContain(m.action.target);
    }
  });

  it('[W-07c] the 5 min timeout text names the limit, and Try again signs in again', async () => {
    const login = vi
      .fn()
      .mockRejectedValueOnce({ error: 'timeout' })
      .mockResolvedValueOnce({ loginId: 'login-2' });
    const { bus, last } = setup({ login });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-x' });
    await flush();
    expect(last()).toMatchObject({ step: 'failed', text: CONNECT_TEXT.timeout });
    expect(CONNECT_TEXT.timeout).toContain('5 minutes');
    click(bus, { target: 'connect_retry' });
    await flush();
    expect(login).toHaveBeenCalledTimes(2);
    expect(last()).toMatchObject({ step: 'connected', profile: 'bottle-x' });
  });

  it('[W-07h] a click from a MOG window does nothing', async () => {
    const { bus, sent } = setup();
    click(bus, { target: 'connect_open' }, 'mog-0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01');
    await flush();
    expect(sent).toEqual([]);
  });
});

describe('Connect a workspace: first run and other profiles (W-07f, W-07i)', () => {
  beforeEach(() => core.reset());

  it('[W-07f] with no databricks-cli profile, the hub opens the URL step at once', async () => {
    const { bus, last } = setup({
      profiles: async () => [{ name: 'acme', host: HOST, authType: 'pat', supported: false }],
    });
    ready(bus);
    await flush();
    expect(last()).toEqual({ step: 'url', firstRun: true });
  });

  it('[W-07f] with no config file at all (an empty list), the hub opens the URL step at once', async () => {
    const { bus, last } = setup({ profiles: async () => [] });
    ready(bus);
    await flush();
    expect(last()).toEqual({ step: 'url', firstRun: true });
  });

  it('[W-07f] with a usable profile, the brain sends the explicit "closed" state', async () => {
    const { bus, last, sent } = setup({
      profiles: async () => [
        { name: 'bottle-acme-prod', host: HOST, authType: 'databricks-cli', supported: true },
      ],
    });
    ready(bus);
    await flush();
    expect(last()).toEqual({ step: 'closed' });
    expect(sent).toEqual([{ step: 'closed' }]);
  });

  it('[W-07f] with an unreadable profile list, the form stays closed and nothing is sent', async () => {
    const { bus, sent } = setup({
      profiles: async () => {
        throw { error: 'not_implemented' };
      },
    });
    ready(bus);
    await flush();
    expect(sent).toEqual([]);
  });

  it('[W-07i] a PAT or service-principal profile for the host gives the new-profile notice', async () => {
    const { bus, last } = setup({
      profiles: async () => [
        { name: 'acme', host: HOST, authType: 'pat', supported: false },
        { name: 'acme-sp', host: HOST, authType: 'oauth-m2m', supported: false },
      ],
    });
    click(bus, { target: 'connect_url', url: `${HOST}/browse` });
    await flush();
    expect(last()).toEqual({
      step: 'name',
      host: HOST,
      proposedName: 'bottle-acme-prod',
      otherAuthNotice: true,
    });
    expect(CONNECT_TEXT.otherAuthNotice).toContain('does not change');
  });

  it('[W-07f] the model goes to the hub only, on the connect event', async () => {
    const bus = new BrainBus();
    wireConnectWorkspace(bus, {
      login: async () => ({ loginId: 'x' }),
      cancel: async () => undefined,
      profiles: async () => [],
    });
    ready(bus);
    await flush();
    expect(core.emitTo).toHaveBeenCalledWith('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
  });
});

describe('Connect a workspace: pasted URLs (review #583)', () => {
  it('[W-07g] a scheme-less paste with a URL in its query keeps its host', () => {
    expect(workspaceHost('acme-prod.cloud.databricks.com/?next=https://x.example')).toBe(HOST);
  });
});

/** A fake timer: the test runs the scheduled function itself. */
function fakeAfter() {
  const timers: { ms: number; fn: () => void; cancelled: boolean }[] = [];
  return {
    timers,
    after: (ms: number, fn: () => void) => {
      const t = { ms, fn, cancelled: false };
      timers.push(t);
      return () => {
        t.cancelled = true;
      };
    },
    fire: () => timers.filter((t) => !t.cancelled).forEach((t) => t.fn()),
  };
}

describe('Connect a workspace: where the flow shows and how it ends (review of #605)', () => {
  it('[W-07e] a flow opened from Settings is inline; the first-run flow is not', async () => {
    const first = setup({ profiles: async () => [] });
    ready(first.bus);
    await flush();
    expect(first.last()).toEqual({ step: 'url', firstRun: true });
    const settings = setup();
    click(settings.bus, { target: 'connect_open' });
    await flush();
    expect(settings.last()).toEqual({ step: 'url', firstRun: false, inline: true });
    click(settings.bus, { target: 'connect_url', url: HOST });
    await flush();
    expect(settings.last()).toMatchObject({ step: 'name', inline: true });
  });

  it('[W-07e] connect_close ends a Settings flow at the URL and name steps', async () => {
    const { bus, last } = setup();
    click(bus, { target: 'connect_open' });
    await flush();
    click(bus, { target: 'connect_close' });
    await flush();
    expect(last()).toEqual({ step: 'closed' });
    click(bus, { target: 'connect_open' });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_close' });
    await flush();
    expect(last()).toEqual({ step: 'closed' });
    // A new flow from Settings is inline again after a close.
    click(bus, { target: 'connect_open' });
    await flush();
    expect(last()).toEqual({ step: 'url', firstRun: false, inline: true });
  });

  it('[W-07h] connect_close does nothing while the sign-in runs', async () => {
    const d = deferred();
    const { bus, last } = setup({ login: vi.fn(() => d.p) });
    click(bus, { target: 'connect_open' });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-x' });
    await flush();
    click(bus, { target: 'connect_close' });
    await flush();
    expect(last()).toMatchObject({ step: 'waiting', inline: true });
  });

  it('[W-07h] Cancel in the wait closes a Settings flow', async () => {
    const d = deferred();
    const cancel = vi.fn(async () => d.reject({ error: 'login_cancelled' }));
    const { bus, last } = setup({ login: vi.fn(() => d.p), cancel });
    click(bus, { target: 'connect_open' });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-x' });
    await flush();
    click(bus, { target: 'connect_cancel' });
    await flush();
    expect(last()).toEqual({ step: 'closed' });
  });

  it('[W-07f] Cancel in the wait at first run goes back to the first-run URL step', async () => {
    const d = deferred();
    const cancel = vi.fn(async () => d.reject({ error: 'login_cancelled' }));
    const { bus, last } = setup({ login: vi.fn(() => d.p), cancel, profiles: async () => [] });
    ready(bus);
    await flush();
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-x' });
    await flush();
    click(bus, { target: 'connect_cancel' });
    await flush();
    expect(last()).toEqual({ step: 'url', firstRun: true });
  });

  it('[W-07h] connected closes by itself after a few seconds', async () => {
    const t = fakeAfter();
    const { bus, last } = setup({ after: t.after });
    click(bus, { target: 'connect_open' });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-x' });
    await flush();
    expect(last()).toMatchObject({ step: 'connected', inline: true });
    expect(t.timers).toHaveLength(1);
    expect(t.timers[0]?.ms).toBe(CONNECTED_CLOSE_MS);
    t.fire();
    expect(last()).toEqual({ step: 'closed' });
  });

  it('[W-07h] Done (connect_close) closes the connected step at once, and the timer does not fire later', async () => {
    const t = fakeAfter();
    const { bus, sent, last } = setup({ after: t.after });
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-x' });
    await flush();
    click(bus, { target: 'connect_close' });
    await flush();
    expect(last()).toEqual({ step: 'closed' });
    expect(t.timers[0]?.cancelled).toBe(true);
    const n = sent.length;
    t.fire();
    expect(sent).toHaveLength(n);
  });

  it('[W-07f] the first-run flow closes after it connects', async () => {
    const t = fakeAfter();
    const { bus, last } = setup({ after: t.after, profiles: async () => [] });
    ready(bus);
    await flush();
    click(bus, { target: 'connect_url', url: HOST });
    await flush();
    click(bus, { target: 'connect_start', profile: 'bottle-x' });
    await flush();
    expect(last()).toEqual({ step: 'connected', host: HOST, profile: 'bottle-x' });
    t.fire();
    expect(last()).toEqual({ step: 'closed' });
  });

  it('[W-07f] the panel-ready signal sends the open step again, or runs the first-run check', async () => {
    const first = setup({ profiles: async () => [] });
    click(first.bus, { target: 'connect_url', url: HOST }); // an open flow, before any view listens
    await flush();
    const before = first.sent.length;
    first.bus.dispatch({
      event: 'view:input',
      payload: { from: 'hub', input: { mogId: CONNECT_VIEW_ID, kind: 'ready' } },
    });
    await flush();
    expect(first.sent).toHaveLength(before + 1);
    expect(first.last()).toMatchObject({ step: 'name' });
    // Nothing open: the panel-ready signal runs the W-07f check, so a lost first send is made up.
    const fresh = setup({ profiles: async () => [] });
    fresh.bus.dispatch({
      event: 'view:input',
      payload: { from: 'hub', input: { mogId: CONNECT_VIEW_ID, kind: 'ready' } },
    });
    await flush();
    expect(fresh.last()).toEqual({ step: 'url', firstRun: true });
  });
});
