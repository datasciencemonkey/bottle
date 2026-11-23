/** The router input of the one MOG from the home workspace and the profile list (#163, ADR 0117). */
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
    readMog: async () => ({ name: 'Finance' }),
    home: async () => 'fake-home',
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
  it('[RTR-01b] [W-07d] the router input uses the home workspace', async () => {
    const h = setup();
    const input = await h.routerInput(A);
    expect(input.profile).toBe('fake-home');
    expect(input.currentTarget).toBe('fake-home');
    expect(input.activeMogs).toEqual([{ id: A, name: 'Finance' }]);
    // Only profiles Bottle can use go to the router (W-03a); never a token.
    expect(input.profiles.map((p) => p.name)).toEqual(['fake-home', 'fake-other']);
    expect(JSON.stringify(input)).not.toMatch(/token/i);
    expect(h.home(A)).toBe('fake-home');
  });

  it('[RTR-15c] [RTR-01b] a MOG with no readable config still gets a router input, with its home', async () => {
    const h = setup({ readMog: async () => Promise.reject({ error: 'config_not_found' }) });
    const input = await h.routerInput(A);
    expect(input.profile).toBe('fake-home');
    expect(input.activeMogs).toEqual([{ id: A, name: '' }]);
  });

  it('[AC-1.10] [ADR 0117] with no home the router input is NO_HOME, even with several listed profiles', async () => {
    const h = setup({ home: async () => undefined });
    const input = await h.routerInput(A);
    expect(input.profile).toBe(NO_HOME);
    expect(h.home(A)).toBeUndefined();
  });

  it('[AC-1.10] a failed home read is no home: Bottle picks none', async () => {
    const h = setup({ home: async () => Promise.reject({ error: 'host_failed' }) });
    expect((await h.routerInput(A)).profile).toBe(NO_HOME);
  });

  it('[RTR-15c] [W-07d] a home profile that the CLI does not list is no home; Bottle picks no other', async () => {
    const h = setup({ home: async () => 'fake-gone' });
    const input = await h.routerInput(A);
    expect(input.profile).toBe(NO_HOME);
    expect(input.currentTarget).toBe(NO_HOME);
    expect(h.home(A)).toBeUndefined();
  });

  it('[W-07d] one listed profile is never picked for the user', async () => {
    const h = setup({
      home: async () => undefined,
      listProfiles: async () => [fake('fake-only')],
    });
    expect((await h.routerInput(A)).profile).toBe(NO_HOME);
  });

  it('[RTR-01b] when the profile list fails, the home workspace stays in use', async () => {
    // The host does not serve `profiles_list` yet (TODO(#17)): `not_implemented`.
    const h = setup({ listProfiles: async () => Promise.reject({ error: 'not_implemented' }) });
    const input = await h.routerInput(A);
    expect(input.profile).toBe('fake-home');
    expect(input.profiles).toEqual([]);
  });
});
