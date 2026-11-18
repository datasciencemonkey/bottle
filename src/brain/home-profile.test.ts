/** The router input of a MOG from its home profile and the profile list (#163). */
import type { Profile } from '../shared/interfaces';
import { createHomeProfiles, type HomeProfileDeps } from './home-profile';
import { NO_HOME } from './offline-intent';

const A = '00000000-0000-4000-8000-0000000000a1';
const fake = (name: string, over: Partial<Profile> = {}): Profile => ({
  name,
  host: `https://${name}.example.test`,
  authType: 'databricks-cli',
  supported: true,
  ...over,
});

function setup(over: Partial<HomeProfileDeps> = {}) {
  const deps: HomeProfileDeps = {
    readMog: async () => ({ name: 'Finance', home: 'fake-home' }),
    listProfiles: async () => [
      fake('fake-home'),
      fake('fake-other'),
      fake('fake-pat', { supported: false }),
    ],
    ...over,
  };
  return createHomeProfiles(deps);
}

describe('home profile router input (#163)', () => {
  it('[RTR-01b] [W-07d] the router input uses the home profile of the MOG', async () => {
    const h = setup();
    const input = await h.routerInput(A);
    expect(input.profile).toBe('fake-home');
    expect(input.currentTarget).toBe('fake-home');
    expect(input.activeMogs).toEqual([{ id: A, name: 'Finance' }]);
    // Only profiles Bottle can use go to the router (W-03a); never a token.
    expect(input.profiles.map((p) => p.name)).toEqual(['fake-home', 'fake-other']);
    expect(JSON.stringify(input)).not.toMatch(/token/i);
    expect(h.hasHome(A)).toBe(true);
  });

  it('[RTR-15c] [RTR-01b] a MOG with no readable config still gets a router input, with no home', async () => {
    const h = setup({ readMog: async () => Promise.reject({ error: 'config_not_found' }) });
    const input = await h.routerInput(A);
    expect(input.profile).toBe(NO_HOME);
    expect(h.hasHome(A)).toBe(false);
  });

  it('[RTR-15c] [W-07d] a home profile that the CLI does not list is no home; Bottle picks no other', async () => {
    const h = setup({ readMog: async () => ({ name: 'Finance', home: 'fake-gone' }) });
    const input = await h.routerInput(A);
    expect(input.profile).toBe(NO_HOME);
    expect(input.currentTarget).toBe(NO_HOME);
    expect(h.hasHome(A)).toBe(false);
  });

  it('[W-07d] one listed profile is never picked for the user', async () => {
    const h = setup({
      readMog: async () => ({ name: 'Finance', home: '' }),
      listProfiles: async () => [fake('fake-only')],
    });
    expect((await h.routerInput(A)).profile).toBe(NO_HOME);
  });

  it('[RTR-01b] when the profile list fails, the home profile the user typed stays in use', async () => {
    // The host does not serve `profiles_list` yet (TODO(#17)): `not_implemented`.
    const h = setup({ listProfiles: async () => Promise.reject({ error: 'not_implemented' }) });
    const input = await h.routerInput(A);
    expect(input.profile).toBe('fake-home');
    expect(input.profiles).toEqual([]);
  });
});
