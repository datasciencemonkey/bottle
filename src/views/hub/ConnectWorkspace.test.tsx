// @vitest-environment jsdom
/**
 * #162: the thin hub view of "Connect a workspace" (W-07a, W-07f, W-07g, W-07h, W-07i, W-15).
 * The brain owns every rule; the view shows the model and sends clicks with `view_emit`.
 * Fake hosts and fake profiles only.
 */
import { act } from 'react';
import { CONNECT_EVENT, CONNECT_VIEW_ID } from '../../shared/connect-view';
import { inputEmits, render, tauri } from '../test-tauri';
import { ConnectWorkspace } from './ConnectWorkspace';
import { Hub } from './Hub';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const HOST = 'https://acme-prod.cloud.databricks.com';

function setValue(input: HTMLInputElement | null, value: string) {
  if (!input) throw new Error('no input');
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  set?.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

const click = (el: Element | null) => act(async () => (el as HTMLElement).click());

describe('Connect a workspace (hub view)', () => {
  beforeEach(() => tauri.reset());

  it('[W-07a] the button asks the brain for the URL step; the view sends nothing on mount', async () => {
    const { el } = await render(<ConnectWorkspace />);
    expect(inputEmits()).toEqual([]);
    await click(el.querySelector('[data-connect-open]'));
    expect(inputEmits()).toEqual([
      { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_open' } },
    ]);
  });

  it('[W-07a] [W-07f] the URL step asks for the URL and sends what the user pasted', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
    expect(el.textContent).toContain('Connect a workspace');
    const url = el.querySelector<HTMLInputElement>('input[name="workspace-url"]');
    expect(url?.value).toBe('');
    await act(async () => setValue(url, `${HOST}/browse?o=123`));
    await click(el.querySelector('[data-connect-next]'));
    expect(inputEmits()).toContainEqual({
      mogId: CONNECT_VIEW_ID,
      kind: 'click',
      payload: { target: 'connect_url', url: `${HOST}/browse?o=123` },
    });
  });

  it('[W-07g] [W-15] the name step shows the host and an editable proposed name, and an optional client ID', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'name',
      host: HOST,
      proposedName: 'bottle-acme-prod',
      otherAuthNotice: false,
    });
    expect(el.textContent).toContain(HOST);
    const name = el.querySelector<HTMLInputElement>('input[name="profile-name"]');
    expect(name?.value).toBe('bottle-acme-prod');
    await act(async () => setValue(name, 'bottle-mine'));
    await act(async () =>
      setValue(el.querySelector<HTMLInputElement>('input[name="client-id"]'), 'custom-app-id'),
    );
    await click(el.querySelector('[data-connect-start]'));
    expect(inputEmits()).toContainEqual({
      mogId: CONNECT_VIEW_ID,
      kind: 'click',
      payload: { target: 'connect_start', profile: 'bottle-mine', clientId: 'custom-app-id' },
    });
  });

  it('[W-07g] [W-07i] the name step shows the brain error and the PAT/SP notice', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'name',
      host: HOST,
      proposedName: 'acme',
      otherAuthNotice: true,
      error: 'A profile with this name exists.',
    });
    expect(el.querySelector('[role="alert"]')?.textContent).toBe(
      'A profile with this name exists.',
    );
    expect(el.querySelector('[data-connect-other-auth]')).not.toBeNull();
  });

  it('[W-07h] the waiting step says to finish in the browser, with Cancel and Open the browser again', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'waiting', host: HOST, profile: 'bottle-x' });
    expect(el.textContent).toContain('Finish the sign-in in your browser');
    await click(el.querySelector('[data-connect-cancel]'));
    await click(el.querySelector('[data-connect-reopen]'));
    expect(inputEmits()).toEqual([
      { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_cancel' } },
      { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_reopen' } },
    ]);
    expect(el.querySelector('[data-connect-cancel]')?.textContent).toBe('Cancel');
    expect(el.querySelector('[data-connect-reopen]')?.textContent).toBe('Open the browser again');
  });

  it('[W-07h] a failed check shows the text and exactly one action', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'failed',
      host: HOST,
      profile: 'bottle-x',
      text: 'Sign-in did not finish in 5 minutes.',
      action: { label: 'Try again', target: 'connect_retry' },
    });
    expect(el.querySelector('[role="alert"]')?.textContent).toBe(
      'Sign-in did not finish in 5 minutes.',
    );
    const buttons = el.querySelectorAll(
      '[data-view="connect-workspace"] [data-connect-step] button',
    );
    expect(buttons).toHaveLength(1);
    await click(buttons[0] ?? null);
    expect(inputEmits()).toEqual([
      { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_retry' } },
    ]);
  });

  it('[W-07h] connected shows the profile; a bad model is ignored', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'connected',
      host: HOST,
      profile: 'bottle-x',
    });
    expect(el.textContent).toContain('bottle-x');
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'nope' });
    expect(el.textContent).toContain('bottle-x');
  });

  it('[W-07f] the hub shows the Connect a workspace view', async () => {
    const { el } = await render(<Hub />);
    expect(el.querySelector('[data-view="connect-workspace"]')).not.toBeNull();
  });
});
