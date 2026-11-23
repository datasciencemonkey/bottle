// @vitest-environment jsdom
/**
 * Settings → Workspace (ADR 0117, S1-C). Every value is fake. The view only listens to
 * `hub:workspaces` and sends `view_emit` clicks with the hub scope (ARC-4).
 */
import { act } from 'react';
import type { Root } from 'react-dom/client';
import {
  HUB_SCOPE,
  HUB_WORKSPACES_EVENT,
  type HubWorkspaceList,
  type HubWorkspaceRow,
} from '../../shared/hub-workspaces';
import { inputEmits, render, tauri, viewEmits } from '../test-tauri';
import { Hub } from './Hub';
import { useHubWorkspaces, WorkspaceSettings } from './WorkspaceSettings';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const AGENT = {
  crossWorkspaceData: false,
  crossWorkspaceLocked: false,
  genieCallBudget: 10,
} as const;
const row = (o: Partial<HubWorkspaceRow> & { profile: string }): HubWorkspaceRow => ({
  name: o.profile,
  host: `${o.profile}.cloud.databricks.com`,
  colorVariant: 'coral',
  signIn: 'signed_in',
  available: false,
  isHome: false,
  ...o,
});
const ROWS: HubWorkspaceRow[] = [
  row({ profile: 'sales', name: 'Sales', isHome: true, available: true }),
  row({ profile: 'ops', name: 'Ops', available: true, colorVariant: 'teal' }),
  row({ profile: 'old', name: 'Old', signIn: 'expired' }),
];
const LIST: HubWorkspaceList = { rows: ROWS, home: 'sales', agent: AGENT };

const act2 = (fn: () => void) => act(async () => fn());
const click = (el: Element | null | undefined) => act2(() => (el as HTMLElement).click());
const key = (el: Element | null, k: string) =>
  act2(() => {
    (el ?? document.body).dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
  });
const li = (el: HTMLElement, p: string) =>
  el.querySelector<HTMLElement>(`[data-workspace-row="${p}"]`) as HTMLElement;

/** Radix opens a menu on pointerdown (button 0). */
const openMenu = (row: HTMLElement) =>
  act2(() => {
    row
      .querySelector('[data-row-menu]')
      ?.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, button: 0 }));
  });
const menuItem = (text: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
    (i) => i.textContent === text,
  );
const settle = () => act(async () => new Promise((r) => requestAnimationFrame(() => r(undefined))));
const typeInto = (input: HTMLInputElement, v: string) =>
  act2(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, v);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });

function Host() {
  return <WorkspaceSettings list={useHubWorkspaces()} />;
}
const roots: Root[] = [];
async function mount(list: HubWorkspaceList = LIST) {
  const r = await render(<Host />);
  roots.push(r.root);
  await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, list);
  return r;
}
/** User input only: the one `workspaces.ready` ping a listening view sends is not input. */
const emits = () =>
  inputEmits().filter(
    (e) => (e as { payload?: { action?: string } }).payload?.action !== 'workspaces.ready',
  );
const hubClicks = () => emits().map((e) => (e as { payload: unknown }).payload);

beforeEach(() => tauri.reset());
// A row's focus return runs on the next frame: unmount, then let a pending frame pass, so a
// test never sees the focus move of the one before it.
afterEach(async () => {
  await act(async () => roots.splice(0).forEach((r) => r.unmount()));
  await act(async () => new Promise((r) => requestAnimationFrame(() => r(undefined))));
  document.body.innerHTML = '';
});

describe('Settings → Workspace', () => {
  it('listens on the hub window only and sends workspaces.ready once', async () => {
    await mount();
    expect(tauri.listenOptions.get(HUB_WORKSPACES_EVENT)).toEqual([{ target: 'hub' }]);
    expect(
      viewEmits().filter(
        (e) => (e as { payload?: { action?: string } }).payload?.action === 'workspaces.ready',
      ),
    ).toEqual([{ mogId: HUB_SCOPE, kind: 'click', payload: { action: 'workspaces.ready' } }]);
  });

  it('renders one row for each of 3 profiles, home first, with name, host and status', async () => {
    const { el } = await mount();
    const rows = [...el.querySelectorAll('[data-workspace-row]')];
    expect(rows.map((r) => r.getAttribute('data-workspace-row'))).toEqual(['sales', 'ops', 'old']);
    expect(li(el, 'ops').querySelector('[data-workspace-name]')?.textContent).toBe('Ops');
    expect(li(el, 'ops').querySelector('[data-workspace-host]')?.textContent).toBe(
      'ops.cloud.databricks.com',
    );
    expect(li(el, 'ops').querySelector('[data-color]')?.getAttribute('data-color')).toBe('teal');
    expect(li(el, 'ops').querySelector('[data-workspace-status]')?.textContent).toContain(
      'Signed in',
    );
    expect(li(el, 'old').querySelector('[data-workspace-status]')?.textContent).toContain(
      'Expired',
    );
  });

  it('shows the four status texts', async () => {
    const { el } = await mount({
      ...LIST,
      rows: [
        ...ROWS,
        row({ profile: 'm', signIn: 'unsupported' }),
        row({ profile: 'x', signIn: 'missing', host: '' }),
      ],
    });
    expect(li(el, 'm').textContent).toContain('Not supported');
    expect(li(el, 'x').textContent).toContain('Not in your CLI config');
    // Neither can be turned on or made home.
    expect(li(el, 'm').querySelector<HTMLButtonElement>('[role="switch"]')?.disabled).toBe(true);
    expect(li(el, 'x').querySelector<HTMLButtonElement>('[data-make-home]')?.disabled).toBe(true);
    // A missing profile has no host line.
    expect(li(el, 'x').querySelector('[data-workspace-host]')).toBeNull();
  });

  it('shows "Not checked yet" for an unknown status and the reasons for unusable rows', async () => {
    const { el } = await mount({
      ...LIST,
      rows: [
        ...ROWS,
        row({ profile: 'u', signIn: 'unknown' }),
        row({ profile: 'm', signIn: 'unsupported' }),
        row({ profile: 'x', signIn: 'missing', host: '' }),
      ],
    });
    expect(li(el, 'u').querySelector('[data-workspace-status]')?.textContent).toBe(
      'Not checked yet',
    );
    expect(li(el, 'm').querySelector('[data-workspace-reason]')?.textContent).toBe(
      'Bottle cannot use this profile type.',
    );
    expect(li(el, 'x').querySelector('[data-workspace-reason]')?.textContent).toBe(
      'This profile is not in ~/.databrickscfg.',
    );
    expect(li(el, 'm').querySelector('[data-workspace-name]')?.className).toContain(
      'text-muted-foreground',
    );
    expect(li(el, 'ops').querySelector('[data-workspace-reason]')).toBeNull();
  });

  it('a missing row that is still available can be switched off', async () => {
    const { el } = await mount({
      ...LIST,
      rows: [...ROWS, row({ profile: 'x', signIn: 'missing', host: '', available: true })],
    });
    const sw = li(el, 'x').querySelector<HTMLButtonElement>('[role="switch"]');
    expect(sw?.disabled).toBe(false);
    await click(sw);
    expect(hubClicks()).toEqual([
      { action: 'workspace.set_available', profile: 'x', available: false },
    ]);
  });

  it('an expired home row has Sign in… and still no switch', async () => {
    const { el } = await mount({
      ...LIST,
      rows: [{ ...(ROWS[0] as HubWorkspaceRow), signIn: 'expired' }, ...ROWS.slice(1)],
    });
    const home = li(el, 'sales');
    expect(home.querySelector('[role="switch"]')).toBeNull();
    await click(home.querySelector('[data-sign-in]'));
    expect(hubClicks()).toEqual([{ action: 'workspace.sign_in', profile: 'sales' }]);
  });

  it('shows "Available to your MOG" beside each switch and "Always on" on home', async () => {
    const { el } = await mount();
    expect(li(el, 'ops').querySelector('[data-availability]')?.textContent).toContain(
      'Available to your MOG',
    );
    expect(li(el, 'sales').querySelector('[data-availability]')?.textContent).toBe('Always on');
  });

  it('the home row has a Home badge and no switch and no Make home; others have both', async () => {
    const { el } = await mount();
    const home = li(el, 'sales');
    expect(home.querySelector('[data-home-badge]')?.textContent).toBe('Home');
    expect(home.querySelector('[role="switch"]')).toBeNull();
    expect(home.querySelector('[data-make-home]')).toBeNull();
    const ops = li(el, 'ops');
    expect(ops.querySelector('[data-home-badge]')).toBeNull();
    expect(ops.querySelector('[data-make-home]')?.textContent).toBe('Make home');
    const sw = ops.querySelector('[role="switch"]');
    expect(sw?.getAttribute('aria-label')).toContain('Ops');
    expect(sw?.getAttribute('aria-label')).toContain('Available to your MOG');
    expect(sw?.getAttribute('aria-checked')).toBe('true');
    expect(li(el, 'old').querySelector('[role="switch"]')?.getAttribute('aria-checked')).toBe(
      'false',
    );
  });

  it('the switch emits workspace.set_available both ways', async () => {
    const { el } = await mount();
    await click(li(el, 'old').querySelector('[role="switch"]'));
    await click(li(el, 'ops').querySelector('[role="switch"]'));
    expect(emits()).toEqual([
      {
        mogId: HUB_SCOPE,
        kind: 'click',
        payload: { action: 'workspace.set_available', profile: 'old', available: true },
      },
      {
        mogId: HUB_SCOPE,
        kind: 'click',
        payload: { action: 'workspace.set_available', profile: 'ops', available: false },
      },
    ]);
  });

  it('an expired row has Sign in… that emits workspace.sign_in; a signed-in row has none', async () => {
    const { el } = await mount();
    expect(li(el, 'ops').querySelector('[data-sign-in]')).toBeNull();
    const btn = li(el, 'old').querySelector('[data-sign-in]');
    expect(btn?.textContent).toBe('Sign in…');
    await click(btn);
    expect(hubClicks()).toEqual([{ action: 'workspace.sign_in', profile: 'old' }]);
  });

  it('busyProfile shows a spinner on that row only and disables its controls', async () => {
    const { el } = await mount({ ...LIST, busyProfile: 'old' });
    expect(el.querySelectorAll('[data-busy-spinner]')).toHaveLength(1);
    expect(li(el, 'old').querySelector('[data-busy-spinner]')).not.toBeNull();
    expect(li(el, 'old').getAttribute('aria-busy')).toBe('true');
    expect(li(el, 'old').querySelector<HTMLButtonElement>('[role="switch"]')?.disabled).toBe(true);
    expect(li(el, 'old').querySelector<HTMLButtonElement>('[data-sign-in]')?.disabled).toBe(true);
    expect(li(el, 'ops').querySelector('[data-busy-spinner]')).toBeNull();
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, LIST);
    expect(el.querySelector('[data-busy-spinner]')).toBeNull();
  });

  it('Make home asks first: Cancel sends nothing, Make home confirms with confirmed:true', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    await click(ops.querySelector('[data-make-home]'));
    expect(ops.textContent).toContain('Make Ops your home? Plain questions go there first.');
    expect(emits()).toEqual([]);
    // Focus lands on the safe choice.
    expect(document.activeElement).toBe(ops.querySelector('[data-cancel-home]'));
    await click(ops.querySelector('[data-cancel-home]'));
    expect(ops.textContent).not.toContain('your home?');
    expect(emits()).toEqual([]);
    await settle();
    await click(ops.querySelector('[data-make-home]'));
    await click(ops.querySelector('[data-confirm-home]'));
    expect(hubClicks()).toEqual([
      { action: 'workspace.make_home', profile: 'ops', confirmed: true },
    ]);
    expect(ops.textContent).not.toContain('your home?');
  });

  it('Escape closes the Make home confirm and sends nothing', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    await click(ops.querySelector('[data-make-home]'));
    await key(ops.querySelector('[data-cancel-home]'), 'Escape');
    expect(ops.querySelector('[data-row-panel]')).toBeNull();
    expect(emits()).toEqual([]);
  });

  it('the row menu is a portal menu with Rename and Colour (not clipped by the group)', async () => {
    const { el } = await mount({ ...LIST, rows: [ROWS[0] as HubWorkspaceRow] });
    const home = li(el, 'sales');
    const btn = home.querySelector<HTMLButtonElement>('[data-row-menu]') as HTMLButtonElement;
    expect(btn.getAttribute('aria-label')).toBe('Options for Sales');
    expect(btn.getAttribute('aria-expanded')).toBe('false');
    await openMenu(home);
    expect(btn.getAttribute('aria-expanded')).toBe('true');
    const menu = document.querySelector('[role="menu"]');
    expect(menu).not.toBeNull();
    // Rendered outside the overflow-hidden settings group.
    expect(el.contains(menu)).toBe(false);
    expect([...document.querySelectorAll('[role="menuitem"]')].map((i) => i.textContent)).toEqual([
      'Rename',
      'Colour',
    ]);
  });

  it('the menu opens from the keyboard and Escape returns focus to its button', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    const btn = ops.querySelector<HTMLButtonElement>('[data-row-menu]') as HTMLButtonElement;
    btn.focus();
    await key(btn, 'ArrowDown');
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    expect(document.activeElement).toBe(menuItem('Rename'));
    await key(document.activeElement, 'ArrowDown');
    // Radix moves roving focus on a timer.
    await act(async () => new Promise((r) => setTimeout(r, 20)));
    expect(document.activeElement).toBe(menuItem('Colour'));
    await key(document.activeElement, 'Escape');
    await settle();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(btn);
  });

  it('Rename edits inline (max 64), Enter sends workspace.edit with the trimmed name', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    await openMenu(ops);
    await click(menuItem('Rename'));
    const input = ops.querySelector<HTMLInputElement>('form input') as HTMLInputElement;
    expect(input.maxLength).toBe(64);
    expect(input.value).toBe('Ops');
    await settle();
    expect(document.activeElement).toBe(input);
    await typeInto(input, '  Operations  ');
    await act2(() => (ops.querySelector('form') as HTMLFormElement).requestSubmit());
    expect(hubClicks()).toEqual([{ action: 'workspace.edit', profile: 'ops', name: 'Operations' }]);
    expect(ops.querySelector('form')).toBeNull();
    await settle();
    expect(document.activeElement).toBe(ops.querySelector('[data-row-menu]'));
  });

  it('Rename with an empty name cannot save; Escape cancels and sends nothing', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    await openMenu(ops);
    await click(menuItem('Rename'));
    const input = ops.querySelector<HTMLInputElement>('form input') as HTMLInputElement;
    await typeInto(input, '   ');
    expect(ops.querySelector<HTMLButtonElement>('form button[type="submit"]')?.disabled).toBe(true);
    await key(input, 'Escape');
    expect(ops.querySelector('form')).toBeNull();
    expect(emits()).toEqual([]);
  });

  it('Colour focuses the pressed swatch; a pick emits workspace.edit and returns focus', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    await openMenu(ops);
    await click(menuItem('Colour'));
    const group = ops.querySelector('[role="group"]');
    expect(group?.getAttribute('aria-label')).toBe('Colour for Ops');
    await settle();
    const pressed = group?.querySelector('[aria-pressed="true"]');
    expect(pressed?.getAttribute('data-color')).toBe('teal');
    expect(document.activeElement).toBe(pressed);
    await click(group?.querySelector('[data-color="violet"]'));
    expect(hubClicks()).toEqual([
      { action: 'workspace.edit', profile: 'ops', colorVariant: 'violet' },
    ]);
    expect(ops.querySelector('[data-row-panel]')).toBeNull();
    await settle();
    expect(document.activeElement).toBe(ops.querySelector('[data-row-menu]'));
  });

  it('the Colour panel has Done and closes on a pointer press outside the row', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    await openMenu(ops);
    await click(menuItem('Colour'));
    await click(ops.querySelector('[data-colour-done]'));
    expect(ops.querySelector('[data-row-panel]')).toBeNull();
    await settle();
    expect(document.activeElement).toBe(ops.querySelector('[data-row-menu]'));
    await openMenu(ops);
    await click(menuItem('Colour'));
    expect(ops.querySelector('[data-row-panel="colour"]')).not.toBeNull();
    await act2(() => {
      document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    });
    expect(ops.querySelector('[data-row-panel]')).toBeNull();
    expect(emits()).toEqual([]);
  });

  it('Make home: Cancel returns focus to Make home, which stays but is disabled while open', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    const mh = ops.querySelector<HTMLButtonElement>('[data-make-home]') as HTMLButtonElement;
    await click(mh);
    expect(mh.disabled).toBe(true);
    expect(mh.getAttribute('aria-label')).toBe('Make home: Ops');
    const grp = ops.querySelector('[data-row-panel="confirm"] [role="group"]');
    expect(grp?.getAttribute('aria-labelledby')).toBeTruthy();
    expect(document.getElementById(grp?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      'Make Ops your home? Plain questions go there first.',
    );
    await click(ops.querySelector('[data-cancel-home]'));
    await settle();
    expect(mh.disabled).toBe(false);
    expect(document.activeElement).toBe(mh);
  });

  it('after the Make home confirm, focus returns to the row menu button', async () => {
    const { el } = await mount();
    const ops = li(el, 'ops');
    await click(ops.querySelector('[data-make-home]'));
    await click(ops.querySelector('[data-confirm-home]'));
    await settle();
    expect(document.activeElement).toBe(ops.querySelector('[data-row-menu]'));
  });

  it('an open panel closes when the row turns busy', async () => {
    const { el } = await mount();
    await click(li(el, 'ops').querySelector('[data-make-home]'));
    expect(li(el, 'ops').querySelector('[data-row-panel]')).not.toBeNull();
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, { ...LIST, busyProfile: 'ops' });
    expect(li(el, 'ops').querySelector('[data-row-panel]')).toBeNull();
  });

  it('keeps the Connect another workspace… row at the bottom, with the hint line', async () => {
    const { el } = await mount();
    const section = el.querySelector('[data-view="workspace-settings"]') as HTMLElement;
    expect(section.querySelectorAll('h2')).toHaveLength(1);
    expect(section.querySelector('h2')?.textContent).toBe('Workspace');
    const connect = section.querySelector('[data-view="connect-another-workspace"]');
    expect(connect?.textContent).toContain('Connect another workspace…');
    expect(connect?.querySelector('h2')).toBeNull();
    expect(
      section.compareDocumentPosition(connect as Node) & Node.DOCUMENT_POSITION_CONTAINED_BY,
    ).toBeTruthy();
    expect(
      (section.querySelector('ul') as Node).compareDocumentPosition(connect as Node) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(section.textContent).toContain(
      'New workspaces start off. Turn one on to let your MOG use it.',
    );
  });

  it('an info notice shows in a polite live region; an error in a new alert each time', async () => {
    const { el } = await mount();
    const live = el.querySelector('[role="status"][aria-live="polite"][data-workspace-notice]');
    expect(live?.textContent).toBe('');
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, {
      ...LIST,
      notice: { kind: 'info', text: 'Sales is now your home.' },
    });
    expect(el.querySelector('[role="status"][aria-live="polite"][data-workspace-notice]')).toBe(
      live,
    );
    expect(live?.textContent).toBe('Sales is now your home.');
    expect(el.querySelector('[role="alert"]')).toBeNull();
    const err = { ...LIST, notice: { kind: 'error', text: 'Could not sign in to Old.' } };
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, err);
    const first = el.querySelector('[role="alert"]');
    expect(first?.textContent).toBe('Could not sign in to Old.');
    // The same notice sent again is the same alert (not read twice).
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, { ...err, notice: { ...err.notice } });
    expect(el.querySelector('[role="alert"]')).toBe(first);
    // Once it clears, the same text is a new alert.
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, LIST);
    expect(el.querySelector('[role="alert"]')).toBeNull();
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, err);
    const second = el.querySelector('[role="alert"]');
    expect(second?.textContent).toBe('Could not sign in to Old.');
    expect(second).not.toBe(first);
  });

  it('shows a loading line, then the empty line, then the rows', async () => {
    const { el } = await render(<Host />);
    expect(el.querySelector('[data-workspaces-loading]')?.textContent).toBe(
      'Checking your workspaces…',
    );
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, { rows: [], home: null, agent: AGENT });
    expect(el.querySelector('[data-workspaces-loading]')).toBeNull();
    expect(el.querySelector('[data-workspaces-empty]')?.textContent).toBe(
      'No workspaces yet. Connect one below.',
    );
    expect(el.querySelector('[data-no-home]')).toBeNull();
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, LIST);
    expect(el.querySelector('[data-workspaces-empty]')).toBeNull();
  });

  it('without a home it shows "Choose a home workspace" with Choose… only when a pick exists', async () => {
    const onChoose = vi.fn();
    const { root, el } = await render(
      <WorkspaceSettings
        onChoose={onChoose}
        list={{
          ...LIST,
          home: null,
          rows: ROWS.map((r) => ({ ...r, isHome: false })),
          homePick: { suggested: 'ops', candidates: ['sales', 'ops'] },
        }}
      />,
    );
    roots.push(root);
    expect(el.querySelector('[data-no-home]')?.textContent).toContain('Choose a home workspace');
    await click(el.querySelector('[data-choose-home]'));
    expect(onChoose).toHaveBeenCalledTimes(1);
    await act2(() =>
      root.render(
        <WorkspaceSettings
          onChoose={onChoose}
          list={{ ...LIST, home: null, rows: ROWS.map((r) => ({ ...r, isHome: false })) }}
        />,
      ),
    );
    expect(el.querySelector('[data-no-home]')).not.toBeNull();
    expect(el.querySelector('[data-choose-home]')).toBeNull();
  });

  it('ignores a payload that fails the shape check', async () => {
    const { el } = await mount();
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, { rows: 'x' });
    expect(el.querySelectorAll('[data-workspace-row]')).toHaveLength(3);
  });

  it('is part of Settings in the hub, not a separate area', async () => {
    const { el } = await render(<Hub />);
    await tauri.emitTo('hub', HUB_WORKSPACES_EVENT, LIST);
    expect(
      el.querySelector('[data-hub-view="settings"] [data-view="workspace-settings"]'),
    ).not.toBeNull();
    expect(el.querySelectorAll('[data-connect-another-open]')).toHaveLength(1);
  });
});
