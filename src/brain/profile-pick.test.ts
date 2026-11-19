/**
 * The profile picker of an Ask Genie press with no home profile (#163, #557, ADR 0096).
 * It never sends by itself and never picks a profile for the user.
 */
import type { Profile } from '../shared/interfaces';
import { createCardStack, type CardStackUpdate } from './card-stack';
import { createProfilePick, type ProfilePickDeps } from './profile-pick';

const A = '00000000-0000-4000-8000-0000000000a1';
const fake = (name: string): Profile => ({
  name,
  host: `https://${name}.example.test`,
  authType: 'databricks-cli',
  supported: true,
});

function setup(over: Partial<ProfilePickDeps> = {}) {
  const sent: CardStackUpdate[] = [];
  const log: string[] = [];
  const saved: [string, string][] = [];
  const stacks = new Map<string, ReturnType<typeof createCardStack>>();
  const ref: { pick?: ReturnType<typeof createProfilePick> } = {};
  const deps: ProfilePickDeps = {
    stackFor: (mogId) => {
      let s = stacks.get(mogId);
      if (!s) {
        s = createCardStack(mogId, {
          clock: { now: () => 0 },
          schedule: () => () => undefined,
          send: (u) => void sent.push(u),
          openHistory: () => undefined,
          onChip: (a) => void ref.pick?.chip(mogId, a),
        });
        stacks.set(mogId, s);
      }
      return s;
    },
    showCard: async () => void log.push('show'),
    listProfiles: async () => [fake('fake-a'), fake('fake-b')],
    saveHome: async (mogId, name) => void saved.push([mogId, name]),
    ...over,
  };
  const pick = createProfilePick(deps);
  ref.pick = pick;
  const last = () => sent.at(-1);
  const texts = () => last()?.blurbs.map((b) => b.text) ?? [];
  const chips = () => [...(last()?.chips ?? []), ...(last()?.overflowChips ?? [])];
  const click = async (label: string) => {
    const c = chips().find((x) => x.label === label);
    if (!c) throw new Error(`no chip ${label}`);
    stacks.get(A)?.input('chip_click', { chipId: c.id });
    await new Promise((r) => setTimeout(r, 0));
  };
  return { pick, log, saved, texts, chips, click };
}

describe('the profile picker of an Ask Genie press (#163, #557)', () => {
  it('[#557] [W-07d] it lists the profiles; the user clicks one, only its name is saved, then the press goes on', async () => {
    const s = setup();
    const picked: string[] = [];
    await s.pick.choose(A, () => void picked.push('go'));
    expect(s.log).toContain('show');
    expect(s.chips().map((c) => c.label)).toEqual(expect.arrayContaining(['fake-a', 'fake-b']));
    // Nothing is saved and nothing goes on before the click on a profile.
    expect(s.saved).toEqual([]);
    expect(picked).toEqual([]);
    await s.click('fake-b');
    expect(s.saved).toEqual([[A, 'fake-b']]);
    expect(picked).toEqual(['go']);
    expect(s.chips().map((c) => c.label)).not.toContain('fake-a');
  });

  it('[W-07d] one profile is listed, never picked for the user', async () => {
    const s = setup({ listProfiles: async () => [fake('fake-only')] });
    const picked: string[] = [];
    await s.pick.choose(A, () => void picked.push('go'));
    expect(s.chips().map((c) => c.label)).toContain('fake-only');
    expect(s.saved).toEqual([]);
    expect(picked).toEqual([]);
  });

  it('[W-07d] unsupported profiles are not offered', async () => {
    const s = setup({
      listProfiles: async () => [
        fake('fake-a'),
        { ...fake('fake-pat'), authType: 'pat', supported: false },
      ],
    });
    await s.pick.choose(A, () => undefined);
    expect(s.chips().map((c) => c.label)).not.toContain('fake-pat');
  });

  it('[W-07d] when the profile list fails, the card says so and nothing goes on', async () => {
    const s = setup({ listProfiles: async () => Promise.reject({ error: 'not_implemented' }) });
    const picked: string[] = [];
    await s.pick.choose(A, () => void picked.push('go'));
    expect(s.texts().join('\n')).toMatch(/cannot list/i);
    expect(s.saved).toEqual([]);
    expect(picked).toEqual([]);
  });

  it('[W-07d] a failed save says so and nothing goes on', async () => {
    const s = setup({ saveHome: async () => Promise.reject({ error: 'io' }) });
    const picked: string[] = [];
    await s.pick.choose(A, () => void picked.push('go'));
    await s.click('fake-a');
    expect(s.texts().join('\n')).toMatch(/could not save/i);
    expect(picked).toEqual([]);
  });

  it('[W-07d] a second press while the list is open does not list the profiles twice', async () => {
    const s = setup();
    await s.pick.choose(A, () => undefined);
    await s.pick.choose(A, () => undefined);
    expect(s.chips().filter((c) => c.label === 'fake-a')).toHaveLength(1);
    expect(s.texts().filter((t) => t.startsWith('Which workspace?'))).toHaveLength(1);
  });

  it('[#557] a second press while the list is open is kept: one pick sends both presses', async () => {
    const s = setup();
    const picked: string[] = [];
    await s.pick.choose(A, () => void picked.push('first'));
    await s.pick.choose(A, () => void picked.push('second'));
    await s.click('fake-a');
    expect(picked).toEqual(['first', 'second']);
  });
});
