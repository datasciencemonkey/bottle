// @vitest-environment jsdom
import { act } from 'react';
import { accessibleText } from '../../test-support/a11y';
import { render, tauri, viewEmits } from '../test-tauri';
import { Workspaces } from './Workspaces';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const CLI = {
  path: '/usr/local/bin/databricks',
  version: '1.25.0',
  found: true,
  plaintextStorage: false,
};
const list = {
  profiles: [
    {
      name: 'DEFAULT',
      host: 'ws-a.cloud.databricks.com',
      authType: 'databricks-cli',
      supported: true,
      signIn: 'signed_in' as const,
      homeOf: ['M1'],
    },
    {
      name: 'other',
      host: 'ws-b.cloud.databricks.com',
      authType: 'databricks-cli',
      supported: true,
      signIn: 'needs_sign_in' as const,
      homeOf: [],
    },
    {
      name: 'legacy-pat',
      host: 'ws-c.cloud.databricks.com',
      authType: 'basic',
      supported: false,
      reason:
        'This profile does not use Databricks sign-in in the browser. Sign in with a bottle profile.',
      homeOf: [],
    },
  ],
  cli: CLI,
};

const byLabel = (el: HTMLElement, label: string) => {
  const hit = [...el.querySelectorAll('button')].find((b) => accessibleText(b) === label);
  if (!hit) throw new Error(`no control "${label}"`);
  return hit as HTMLElement;
};

async function click(c: HTMLElement): Promise<void> {
  await act(async () => c.click());
}

const actions = () => viewEmits().map((e) => (e as { payload?: unknown }).payload);

async function shown(): Promise<HTMLElement> {
  const { el } = await render(<Workspaces />);
  await tauri.emitTo('hub', 'hub:workspaces', list);
  return el;
}

describe('hub Workspaces area (#560)', () => {
  beforeEach(() => tauri.reset());

  it('[#560] [ARC-5] listens on hub:workspaces for its own window, then asks with workspaces.ready', async () => {
    await render(<Workspaces />);
    expect(tauri.listeners('hub:workspaces')).toBe(1);
    expect(tauri.listenOptions.get('hub:workspaces')).toEqual([{ target: 'hub' }]);
    expect(viewEmits()).toEqual([
      { mogId: 'hub', kind: 'click', payload: { action: 'workspaces.ready' } },
    ]);
  });

  it('[#560] shows a loading state before the brain answers', async () => {
    const { el } = await render(<Workspaces />);
    expect(el.textContent).toContain('Loading your Databricks workspaces');
  });

  it('[#560] lists each profile with its host, auth type, sign-in state and Home of', async () => {
    const el = await shown();
    expect(el.textContent).toContain('ws-a.cloud.databricks.com');
    expect(el.textContent).toContain('databricks-cli');
    expect(el.textContent).toContain('Signed in');
    expect(el.textContent).toContain('Sign-in needed');
    expect(el.textContent).toContain('Home of: M1');
  });

  it('[#560] [#486] an unsupported profile shows "Not supported" and its reason, never a sign-in state', async () => {
    const el = await shown();
    const row = el.querySelector('li[data-profile="legacy-pat"]') as HTMLElement;
    expect(row.textContent).toContain('Not supported');
    expect(row.textContent).toContain('This profile does not use Databricks sign-in');
    expect(row.textContent).not.toContain('Signed in');
    expect(row.textContent).not.toContain('Sign-in needed');
  });

  it('[#560] shows the CLI path and version, and the plaintext-storage warning when cli_status says so', async () => {
    const el = await shown();
    expect(el.textContent).toContain('/usr/local/bin/databricks');
    expect(el.textContent).toContain('1.25.0');
    expect(el.querySelector('[role="alert"]')).toBeNull();
    await tauri.emitTo('hub', 'hub:workspaces', {
      ...list,
      cli: { ...CLI, plaintextStorage: true },
    });
    expect(el.querySelector('[role="alert"]')?.textContent).toContain('plaintext');
  });

  it('[#560] the empty state offers Connect a workspace, with no Refresh', async () => {
    const { el } = await render(<Workspaces />);
    await tauri.emitTo('hub', 'hub:workspaces', { profiles: [], cli: CLI });
    expect(el.textContent).toContain('No Databricks workspace is connected yet.');
    expect(el.querySelector('[data-ws-connect]')).not.toBeNull();
    expect(() => byLabel(el, 'Refresh')).toThrow();
  });

  it('[#560] the error state shows a calm message with Retry, which sends workspaces.refresh', async () => {
    const { el } = await render(<Workspaces />);
    await tauri.emitTo('hub', 'hub:workspaces', {
      profiles: [],
      cli: CLI,
      notice: { kind: 'error', text: 'Bottle cannot list your Databricks CLI profiles.' },
    });
    expect(el.querySelector('[role="alert"]')?.textContent).toBe(
      'Bottle cannot list your Databricks CLI profiles.',
    );
    await click(byLabel(el, 'Retry'));
    expect(actions().at(-1)).toEqual({ action: 'workspaces.refresh' });
  });

  it('[#560] Refresh sends workspaces.refresh', async () => {
    const el = await shown();
    await click(byLabel(el, 'Refresh'));
    expect(actions().at(-1)).toEqual({ action: 'workspaces.refresh' });
  });

  it('[#560] Connect a workspace reuses the ConnectWorkspace trigger (connect_open), not a Workspaces action', async () => {
    const el = await shown();
    await click(byLabel(el, 'Connect a workspace'));
    expect(actions().at(-1)).toEqual({ target: 'connect_open' });
    expect(viewEmits().at(-1)).toMatchObject({ mogId: 'connect' });
  });
});
