// @vitest-environment jsdom
/** Slice 2a: Settings → Workspace → "MOG agent". All values are fake. */
import { act } from 'react';
import {
  HUB_SCOPE,
  type HubWorkspaceList,
  type HubWorkspaceRow,
} from '../../shared/hub-workspaces';
import type { GenieCodeStatus } from '../../shared/ipc';
import { inputEmits, render, tauri } from '../test-tauri';
import {
  AgentSettings,
  APPLIES_AFTER,
  BUDGET_HELP,
  COMBINE_APPLIES,
  COMBINE_LOCKED,
} from './AgentSettings';

vi.mock('@tauri-apps/api/core', async () => (await import('../test-tauri')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('../test-tauri')).eventModule);
vi.mock('@tauri-apps/api/window', async () => (await import('../test-tauri')).windowModule);

const row = (o: Partial<HubWorkspaceRow> & { profile: string }): HubWorkspaceRow => ({
  name: o.profile,
  host: `${o.profile}.fake.test`,
  colorVariant: 'coral',
  signIn: 'signed_in',
  available: true,
  isHome: false,
  ...o,
});
const READY: GenieCodeStatus = {
  binary: 'ready',
  version: '0.5.0',
  pinned: '0.5.0',
  agentProfile: 'sales',
  agentProfileState: 'ready',
};
const list = (
  g: GenieCodeStatus | undefined = READY,
  runProfile: string | null = null,
): HubWorkspaceList => ({
  rows: [
    row({ profile: 'sales', name: 'Sales', isHome: true }),
    row({ profile: 'fin', name: 'Finance' }),
    row({ profile: 'pat', name: 'Token only', signIn: 'unsupported' }),
  ],
  home: 'sales',
  agent: {
    crossWorkspaceData: false,
    crossWorkspaceLocked: false,
    genieCallBudget: 10,
    runProfile,
    ...(g ? { genieCode: g } : {}),
  },
});
const clicks = () => inputEmits().map((e) => (e as { payload: unknown }).payload);
const act2 = (fn: () => void) => act(async () => fn());
const click = (el: Element | null | undefined) => act2(() => (el as HTMLElement).click());
const openMenu = (el: HTMLElement) =>
  act2(() => {
    el.querySelector('[data-agent-profile]')?.dispatchEvent(
      new MouseEvent('pointerdown', { bubbles: true, button: 0 }),
    );
  });

beforeEach(() => tauri.reset());
afterEach(() => {
  document.body.innerHTML = '';
});

describe('AgentSettings', () => {
  it('shows home as the default agent workspace and the disclosure line', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    const trigger = el.querySelector('[data-agent-profile]');
    expect(trigger?.textContent).toContain('Sales');
    expect(trigger?.textContent).toContain('(home)');
    expect(trigger?.getAttribute('aria-label')).toBe('Genie Code runs on: Sales');
    expect(el.querySelector('[data-agent-disclosure]')?.textContent).toContain(
      'sends it to Genie Code in Sales.',
    );
    expect(el.querySelector('[data-agent-applies]')).toBeNull();
  });

  it('sends the chosen profile, null for home, and disables a profile that cannot run it', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    await openMenu(el);
    const items = [...document.querySelectorAll<HTMLElement>('[role="menuitemradio"]')];
    expect(items.map((i) => i.getAttribute('data-agent-profile-item'))).toEqual([
      'sales',
      'fin',
      'pat',
    ]);
    expect(items[2]?.getAttribute('data-disabled')).not.toBeNull();
    expect(items[2]?.textContent).toContain('Bottle cannot use this profile type.');
    expect(items[0]?.getAttribute('aria-checked')).toBe('true');
    await click(items[1]);
    expect(clicks()).toEqual([{ action: 'workspace.agent_profile', profile: 'fin' }]);
  });

  it('choosing home sends null', async () => {
    const { el } = await render(<AgentSettings list={list(READY, 'fin')} />);
    expect(el.querySelector('[data-agent-profile]')?.textContent).toContain('Finance');
    await openMenu(el);
    await click(document.querySelector('[data-agent-profile-item="sales"]'));
    expect(clicks()).toEqual([{ action: 'workspace.agent_profile', profile: null }]);
  });

  it('says "Applies after the current question" while a turn runs', async () => {
    const { el } = await render(<AgentSettings list={list()} busy />);
    expect(el.querySelector('[data-agent-applies]')?.textContent).toBe(APPLIES_AFTER);
  });

  const withAgent = (a: Partial<HubWorkspaceList['agent']>): HubWorkspaceList => {
    const l = list();
    return { ...l, agent: { ...l.agent, ...a } };
  };
  const combine = (el: Element) => el.querySelector<HTMLElement>('[data-agent-combine]');

  it('[D1] "Combine across workspaces" is a labelled switch, off by default, with its help text', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    const sw = combine(el);
    expect(sw?.getAttribute('role')).toBe('switch');
    expect(sw?.getAttribute('aria-checked')).toBe('false');
    expect(sw?.hasAttribute('disabled')).toBe(false);
    const label = document.getElementById(sw?.getAttribute('aria-labelledby') ?? '');
    expect(label?.textContent).toBe('Combine across workspaces');
    const help = document.getElementById(sw?.getAttribute('aria-describedby') ?? '');
    // The help starts with the plan sentence.
    expect(
      help?.textContent?.startsWith(
        "Answers from your other workspaces go to Genie Code in Sales, which runs on that workspace's AI model.",
      ),
    ).toBe(true);
    expect(help?.textContent).toContain('Genie Code can see them and combine them.');
    expect(help?.textContent).toContain(
      'With the switch on, Genie Code can also put an answer from one workspace into a question to another workspace.',
    );
    expect(help?.textContent).toContain(
      "When off, each workspace's answer is shown to you, and the agent sees only a receipt.",
    );
    expect(el.querySelector('[data-agent-combine-locked]')).toBeNull();
    // The live region is there, and empty.
    expect(el.querySelector('[data-agent-combine-applies]')?.textContent).toBe('');
    // The label toggles the switch only when it is not locked.
    expect(el.querySelector('label[for]')).not.toBeNull();
  });

  it('[D1] the disclosure line says that other answers stay out unless the switch is on', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    expect(el.querySelector('[data-agent-disclosure]')?.textContent).toContain(
      'Answers from your other workspaces stay out of it unless you turn on Combine across workspaces.',
    );
  });

  it('[D1] the switch writes the setting through workspace.cross, on and off', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    await click(combine(el));
    expect(clicks()).toEqual([{ action: 'workspace.cross', crossWorkspaceData: true }]);
    tauri.reset();
    const on = await render(<AgentSettings list={withAgent({ crossWorkspaceData: true })} />);
    expect(combine(on.el)?.getAttribute('aria-checked')).toBe('true');
    await click(combine(on.el));
    expect(clicks()).toEqual([{ action: 'workspace.cross', crossWorkspaceData: false }]);
  });

  it('[AC-2b.5] a locked switch is off, disabled and says "Set by your admin"', async () => {
    const { el } = await render(
      <AgentSettings list={withAgent({ crossWorkspaceData: true, crossWorkspaceLocked: true })} />,
    );
    const sw = combine(el);
    expect(sw?.getAttribute('aria-checked')).toBe('false');
    expect(sw?.hasAttribute('disabled')).toBe(true);
    const locked = el.querySelector('[data-agent-combine-locked]');
    expect(locked?.textContent).toBe(COMBINE_LOCKED);
    expect(locked?.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    // A locked row has no label link, so no pointer cursor and no click-through.
    expect(el.querySelector('label[for]')).toBeNull();
    expect(sw?.getAttribute('aria-describedby')).toContain(locked?.id ?? 'none');
    await click(sw);
    expect(clicks()).toEqual([]);
  });

  it('[AC-2b.4] says "Applies to your next question." while a turn runs, and not when locked', async () => {
    const { el } = await render(<AgentSettings list={list()} busy />);
    expect(el.querySelector('[data-agent-combine-applies]')?.textContent).toBe(COMBINE_APPLIES);
    const l = await render(<AgentSettings list={withAgent({ crossWorkspaceLocked: true })} busy />);
    expect(l.el.querySelector('[data-agent-combine-applies]')?.textContent).toBe('');
  });

  it('[D2] the budget is a labelled radio group; a click sends the value', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    const group = el.querySelector('[data-agent-budget]');
    expect(group?.getAttribute('role')).toBe('radiogroup');
    expect(group?.getAttribute('aria-labelledby')).toBeTruthy();
    expect(el.textContent).toContain(BUDGET_HELP);
    expect(el.querySelector('[data-budget="10"]')?.getAttribute('aria-checked')).toBe('true');
    await click(el.querySelector('[data-budget="20"]'));
    expect(clicks()).toEqual([{ action: 'workspace.budget', genieCallBudget: 20 }]);
  });

  it('shows "Installed 0.5.0" with no action when ready', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    expect(el.querySelector('[data-agent-genie-status]')?.textContent).toBe('Installed 0.5.0');
    expect(el.querySelector('[data-agent-install], [data-agent-recheck]')).toBeNull();
  });

  it('Install… asks first; only the consent click installs, and Cancel sends nothing', async () => {
    const { el } = await render(
      <AgentSettings list={list({ ...READY, binary: 'not_installed' })} />,
    );
    expect(el.querySelector('[data-agent-genie-status]')?.textContent).toBe('Not installed');
    await click(el.querySelector('[data-agent-install]'));
    const confirm = el.querySelector('[data-agent-install-confirm]');
    expect(confirm?.getAttribute('role')).toBe('group');
    expect(clicks()).toEqual([]);
    // The safe choice has the focus; Escape closes and sends nothing.
    expect(document.activeElement?.textContent).toBe('Cancel');
    await act2(() => {
      confirm?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(el.querySelector('[data-agent-install-confirm]')).toBeNull();
    await click(el.querySelector('[data-agent-install]'));
    await click(el.querySelector('[data-confirm-install]'));
    expect(clicks()).toEqual([{ action: 'genie_code.install', consent: true }]);
  });

  it("a wrong version offers Bottle's copy", async () => {
    const { el } = await render(
      <AgentSettings list={list({ ...READY, binary: 'wrong_version', version: '0.6.1' })} />,
    );
    expect(el.querySelector('[data-agent-genie-status]')?.textContent).toBe(
      'Bottle needs 0.5.0 (found 0.6.1)',
    );
    expect(el.querySelector('[data-agent-install]')?.textContent).toBe("Install Bottle's copy…");
  });

  it('shows the busy state while it installs, with no Install button', async () => {
    const { el } = await render(<AgentSettings list={list({ ...READY, binary: 'installing' })} />);
    expect(el.querySelector('[data-agent-busy]')).not.toBeNull();
    expect(el.querySelector('[data-agent-genie-status]')?.getAttribute('aria-busy')).toBe('true');
    expect(el.querySelector('[data-agent-install]')).toBeNull();
  });

  it('Check again sends one recheck and shows busy until the next list', async () => {
    const l = list({ ...READY, agentProfileState: 'not_eligible' });
    const { el, root } = await render(<AgentSettings list={l} />);
    expect(el.querySelector('[data-agent-genie-status]')?.textContent).toBe(
      'Not available in this workspace',
    );
    await click(el.querySelector('[data-agent-recheck]'));
    expect(clicks()).toEqual([{ action: 'genie_code.recheck' }]);
    expect(el.querySelector('[data-agent-busy]')).not.toBeNull();
    expect(el.querySelector<HTMLButtonElement>('[data-agent-recheck]')?.disabled).toBe(true);
    await act(async () => root.render(<AgentSettings list={{ ...l }} />));
    expect(el.querySelector('[data-agent-busy]')).toBeNull();
  });

  it('every click uses the hub scope (ARC-4)', async () => {
    const { el } = await render(<AgentSettings list={list()} />);
    await click(el.querySelector('[data-budget="5"]'));
    expect(inputEmits()[0]).toMatchObject({ mogId: HUB_SCOPE, kind: 'click' });
  });
});
