// @vitest-environment jsdom
/**
 * #162: the thin hub view of "Connect a workspace" (W-07a, W-07f, W-07g, W-07h, W-07i, W-15).
 * The brain owns every rule; the view shows the model and sends clicks with `view_emit`.
 * Fake hosts and fake profiles only.
 */
import { act } from 'react';
import { CONNECT_EVENT, CONNECT_VIEW_ID } from '../../shared/connect-view';
import { inputEmits, render, tauri, viewEmits } from '../test-tauri';
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

  it('with no model, the view renders nothing and sends nothing on mount', async () => {
    const { el } = await render(<ConnectWorkspace />);
    expect(el.querySelector('[data-view="connect-workspace"]')).toBeNull();
    expect(el.textContent).toBe('');
    expect(inputEmits()).toEqual([]);
  });

  it('[W-07f] with the brain\'s explicit "closed" step (a usable profile already exists), the view renders nothing', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
    expect(el.querySelector('[data-view="connect-workspace"]')).not.toBeNull();
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'closed' });
    expect(el.querySelector('[data-view="connect-workspace"]')).toBeNull();
    expect(el.textContent).toBe('');
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

  it('[W-07f] the hub mounts the Connect a workspace view, open once the brain opens it', async () => {
    const { el } = await render(<Hub />);
    // No model yet: the setup region shows nothing for this step (review of #602, the panel
    // bug) — not even with a profile, since the brain never had to say so.
    expect(el.querySelector('[data-view="connect-workspace"]')).toBeNull();
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
    expect(el.querySelector('[data-view="connect-workspace"]')).not.toBeNull();
  });

  it('[W-07f] the setup panel sends ready (id connect) once it listens, so the brain sends the step again', async () => {
    await render(<ConnectWorkspace />);
    expect(viewEmits()).toContainEqual({ mogId: CONNECT_VIEW_ID, kind: 'ready' });
    expect(inputEmits()).toEqual([]);
    tauri.reset();
    await render(<ConnectWorkspace placement="settings" />);
    expect(viewEmits()).toEqual([]);
  });

  it('[W-07f] the first-run steps have no Cancel (W-07f needs a way to connect)', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
    expect(el.querySelector('[data-connect-close]')).toBeNull();
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'name',
      host: HOST,
      proposedName: 'bottle-x',
      otherAuthNotice: false,
    });
    expect(el.querySelector('[data-connect-close]')).toBeNull();
  });

  it('[W-07h] connected has a Done button that sends connect_close', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'connected',
      host: HOST,
      profile: 'bottle-x',
    });
    await click(el.querySelector('[data-connect-done]'));
    expect(inputEmits()).toEqual([
      { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_close' } },
    ]);
    // The brain answers `closed`: the panel is gone.
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'closed' });
    expect(el.querySelector('[data-view="connect-workspace"]')).toBeNull();
  });
});

describe('Connect a workspace: the flow opened from Settings (review of #605)', () => {
  beforeEach(() => tauri.reset());

  it('[W-07e] each host shows only its own flow, by the inline flag', async () => {
    const setup = await render(<ConnectWorkspace />);
    const settings = await render(<ConnectWorkspace placement="settings" />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: false, inline: true });
    expect(setup.el.querySelector('[data-view="connect-workspace"]')).toBeNull();
    expect(settings.el.querySelector('[data-view="connect-workspace"]')).not.toBeNull();
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
    expect(setup.el.querySelector('[data-view="connect-workspace"]')).not.toBeNull();
    expect(settings.el.querySelector('[data-view="connect-workspace"]')).toBeNull();
  });

  it('[W-07e] the first field gets focus: the URL field, then the name field', async () => {
    const { el } = await render(<ConnectWorkspace placement="settings" />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: false, inline: true });
    expect(document.activeElement).toBe(el.querySelector('input[name="workspace-url"]'));
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'name',
      host: HOST,
      proposedName: 'bottle-x',
      otherAuthNotice: false,
      inline: true,
    });
    expect(document.activeElement).toBe(el.querySelector('input[name="profile-name"]'));
  });

  it('[W-07e] the URL and name steps have a Cancel that sends connect_close', async () => {
    const { el } = await render(<ConnectWorkspace placement="settings" />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: false, inline: true });
    await click(el.querySelector('[data-connect-close]'));
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'name',
      host: HOST,
      proposedName: 'bottle-x',
      otherAuthNotice: false,
      inline: true,
    });
    await click(el.querySelector('[data-connect-close]'));
    const close = { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_close' } };
    expect(inputEmits()).toEqual([close, close]);
  });

  it('[W-07h] the waiting step has Cancel and no close; connected has Done; failed can be closed', async () => {
    const { el } = await render(<ConnectWorkspace placement="settings" />);
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'waiting',
      host: HOST,
      profile: 'bottle-x',
      inline: true,
    });
    expect(el.querySelector('[data-connect-close]')).toBeNull();
    expect(el.querySelector('[data-connect-cancel]')).not.toBeNull();
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'failed',
      host: HOST,
      profile: 'bottle-x',
      text: 'Sign-in did not finish in 5 minutes.',
      action: { label: 'Try again', target: 'connect_retry' },
      inline: true,
    });
    // The one action plus a secondary Close, in the same row of the step.
    const failed = el.querySelector('[data-connect-step="failed"]');
    expect(failed?.querySelectorAll('button')).toHaveLength(2);
    expect(failed?.querySelector('[data-connect-close]')?.textContent).toBe('Close');
    await click(el.querySelector('[data-connect-close]'));
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'connected',
      host: HOST,
      profile: 'bottle-x',
      inline: true,
    });
    await click(el.querySelector('[data-connect-done]'));
    expect(inputEmits()).toEqual([
      { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_close' } },
      { mogId: CONNECT_VIEW_ID, kind: 'click', payload: { target: 'connect_close' } },
    ]);
  });

  it('[W-07e] a bad inline flag is ignored', async () => {
    const { el } = await render(<ConnectWorkspace placement="settings" />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: false, inline: 'yes' });
    expect(el.querySelector('[data-view="connect-workspace"]')).toBeNull();
  });

  it('[W-07a] the connect fields are the shadcn Input, so a type=url field has a box', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
    const url = el.querySelector('input[name="workspace-url"]');
    expect(url?.getAttribute('data-slot')).toBe('input');
    expect(url?.getAttribute('type')).toBe('url');
    expect(url?.className).toMatch(/\bborder\b/);
    expect(url?.className).toMatch(/\bh-9\b/);
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'name',
      host: HOST,
      proposedName: 'bottle-x',
      otherAuthNotice: false,
    });
    for (const n of ['profile-name', 'client-id']) {
      expect(el.querySelector(`input[name="${n}"]`)?.getAttribute('data-slot')).toBe('input');
    }
  });

  it('[W-07h] the connected sentence is one element, so no flex gap lands between its words', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'connected',
      host: HOST,
      profile: 'bottle-x',
    });
    const step = el.querySelector('[data-connect-step="connected"]');
    expect(step?.children).toHaveLength(2);
    expect(step?.children[0]?.textContent).toBe(`Connected ${HOST} as profile bottle-x.`);
    expect(step?.children[1]?.textContent).toBe('Done');
  });

  it('[W-07f] a change of step moves focus to the new step, and the first appearance steals none', async () => {
    const { el } = await render(<ConnectWorkspace />);
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'url', firstRun: true });
    // (Earlier tests leave their DOM in the document: compare with this panel's own field.)
    expect(document.activeElement).not.toBe(el.querySelector('input[name="workspace-url"]'));
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'name',
      host: HOST,
      proposedName: 'bottle-x',
      otherAuthNotice: false,
    });
    expect(document.activeElement).toBe(el.querySelector('input[name="profile-name"]'));
    await tauri.emitTo('hub', CONNECT_EVENT, { step: 'waiting', host: HOST, profile: 'bottle-x' });
    expect(document.activeElement).toBe(el.querySelector('[data-connect-cancel]'));
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'failed',
      host: HOST,
      profile: 'bottle-x',
      text: 'x',
      action: { label: 'Try again', target: 'connect_retry' },
    });
    expect(document.activeElement?.textContent).toBe('Try again');
    await tauri.emitTo('hub', CONNECT_EVENT, {
      step: 'connected',
      host: HOST,
      profile: 'bottle-x',
    });
    expect(document.activeElement).toBe(el.querySelector('[data-connect-done]'));
  });
});
