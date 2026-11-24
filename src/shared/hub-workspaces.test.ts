import { asHubWorkspaceList, asWorkspaceAction } from './hub-workspaces';

const row = {
  profile: 'sales',
  name: 'Sales',
  host: 'adb-1.azuredatabricks.net',
  colorVariant: 'teal',
  signIn: 'signed_in',
  available: true,
  isHome: true,
};
const list = {
  rows: [row],
  home: 'sales',
  agent: {
    crossWorkspaceData: false,
    crossWorkspaceLocked: false,
    genieCallBudget: 10,
    runProfile: null,
  },
};

describe('hub-workspaces shape checks (ADR 0117)', () => {
  it('accepts each documented action and drops unknown fields', () => {
    expect(asWorkspaceAction({ action: 'workspaces.ready', x: 1 })).toEqual({
      action: 'workspaces.ready',
    });
    expect(
      asWorkspaceAction({ action: 'workspace.set_available', profile: 'p', available: false }),
    ).toEqual({
      action: 'workspace.set_available',
      profile: 'p',
      available: false,
    });
    expect(
      asWorkspaceAction({ action: 'workspace.make_home', profile: 'p', confirmed: true }),
    ).toEqual({
      action: 'workspace.make_home',
      profile: 'p',
      confirmed: true,
    });
    expect(asWorkspaceAction({ action: 'workspace.pick_home', profile: 'p' })).toBeDefined();
    expect(asWorkspaceAction({ action: 'workspace.sign_in', profile: 'p' })).toBeDefined();
    expect(asWorkspaceAction({ action: 'workspace.sign_in_cancel', profile: 'p', x: 1 })).toEqual({
      action: 'workspace.sign_in_cancel',
      profile: 'p',
    });
    expect(asWorkspaceAction({ action: 'workspace.edit', profile: 'p', name: 'Fin' })).toEqual({
      action: 'workspace.edit',
      profile: 'p',
      name: 'Fin',
    });
    expect(asWorkspaceAction({ action: 'workspace.budget', genieCallBudget: 20 })).toBeDefined();
    expect(
      asWorkspaceAction({ action: 'workspace.cross', crossWorkspaceData: true }),
    ).toBeDefined();
  });

  it('accepts the slice 2a agent actions', () => {
    expect(asWorkspaceAction({ action: 'workspace.agent_profile', profile: 'fin', x: 1 })).toEqual({
      action: 'workspace.agent_profile',
      profile: 'fin',
    });
    expect(asWorkspaceAction({ action: 'workspace.agent_profile', profile: null })).toEqual({
      action: 'workspace.agent_profile',
      profile: null,
    });
  });

  it('refuses a bad agent action', () => {
    expect(asWorkspaceAction({ action: 'workspace.agent_profile' })).toBeUndefined();
    expect(asWorkspaceAction({ action: 'workspace.agent_profile', profile: '' })).toBeUndefined();
  });

  it('refuses bad shapes', () => {
    expect(asWorkspaceAction({ action: 'workspace.make_home', profile: 'p' })).toBeUndefined();
    expect(asWorkspaceAction({ action: 'workspace.budget', genieCallBudget: 7 })).toBeUndefined();
    expect(
      asWorkspaceAction({ action: 'workspace.set_available', profile: '', available: true }),
    ).toBeUndefined();
    expect(asWorkspaceAction({ action: 'workspace.edit', profile: 'p' })).toBeUndefined();
    expect(
      asWorkspaceAction({ action: 'workspace.edit', profile: 'p', name: '  ' }),
    ).toBeUndefined();
    expect(asWorkspaceAction({ action: 'workspace.sign_in_cancel' })).toBeUndefined();
    expect(asWorkspaceAction({ action: 'workspace.sign_in_cancel', profile: '' })).toBeUndefined();
    expect(asWorkspaceAction({ action: 'mog.create' })).toBeUndefined();
    expect(asWorkspaceAction(null)).toBeUndefined();
  });

  it('parses a list and the optional home pick', () => {
    expect(asHubWorkspaceList(list)).toEqual(list);
    const pick = {
      ...list,
      home: null,
      homePick: { suggested: 'sales', candidates: ['sales', 'fin'] },
    };
    expect(asHubWorkspaceList(pick)?.homePick).toEqual(pick.homePick);
  });

  it('accepts a list from before the agent state: runProfile is optional', () => {
    const old = {
      crossWorkspaceData: false,
      crossWorkspaceLocked: false,
      genieCallBudget: 10,
    };
    const got = asHubWorkspaceList({ ...list, agent: old });
    expect(got?.agent).toEqual(old);
    expect(got?.agent).not.toHaveProperty('runProfile');
  });

  it('drops unknown fields at every level', () => {
    const extra = {
      ...list,
      x: 1,
      rows: [{ ...row, secret: 'no' }],
      agent: { ...list.agent, extra: true },
    };
    expect(asHubWorkspaceList(extra)).toEqual(list);
  });

  it('refuses a bad home pick', () => {
    const base = { ...list, home: null };
    expect(
      asHubWorkspaceList({ ...base, homePick: { suggested: 'x', candidates: ['a', 'b'] } }),
    ).toBeUndefined();
    expect(
      asHubWorkspaceList({ ...base, homePick: { suggested: 'a', candidates: ['a'] } }),
    ).toBeUndefined();
    expect(
      asHubWorkspaceList({ ...base, homePick: { suggested: 'a', candidates: ['a', ''] } }),
    ).toBeUndefined();
  });

  it('refuses a list with a bad row, budget or sign-in state', () => {
    expect(asHubWorkspaceList({ ...list, rows: [{ ...row, profile: '' }] })).toBeUndefined();
    expect(asHubWorkspaceList({ ...list, rows: [{ ...row, signIn: 'maybe' }] })).toBeUndefined();
    expect(
      asHubWorkspaceList({ ...list, agent: { ...list.agent, genieCallBudget: 3 } }),
    ).toBeUndefined();
    expect(asHubWorkspaceList({ ...list, home: 5 })).toBeUndefined();
    expect(
      asHubWorkspaceList({ ...list, agent: { ...list.agent, runProfile: 5 } }),
    ).toBeUndefined();
  });
});
