/**
 * The "Pick a profile to ask Genie" card of a question with no home profile (#163, RTR-15c).
 * It never sends, never picks a profile for the user, and never loses the spoken text.
 */
import type { Profile } from '../shared/interfaces';
import { routerDecision } from '../test-support/model-invoke';
import { createCardStack, type CardStackUpdate } from './card-stack';
import { createProfilePick, PICK_TEXT, type ProfilePickDeps } from './profile-pick';
import type { AskRoute } from './utterance-wiring';

const A = '00000000-0000-4000-8000-0000000000a1';
const MISREAD = 'What I mean is the deck goes out on Friday.';
const owned = { utteranceId: 'utt-1', mogId: A };
const chip = (text: string): AskRoute => ({
  route: 'chip',
  decision: routerDecision({
    intent: 'question',
    cleaned_text: text,
    genie_question: text,
    confidence: 0.5,
  }),
  warning: undefined,
});
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
  const inserted: string[] = [];
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
    insertionOn: () => true,
    insertText: async (a) => {
      log.push('insert_text');
      inserted.push(a.text);
      return { result: 'inserted' };
    },
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
  return { pick, log, saved, inserted, texts, chips, click };
}

describe('the no-profile pick card (#163)', () => {
  it('[RTR-15c] [B1] a question with no home profile shows the text and the pick state, and sends nothing', async () => {
    const s = setup();
    await s.pick.showNoProfile(chip('What was revenue?'), owned);
    expect(s.log[0]).toBe('show');
    expect(s.texts().join('\n')).toContain('What was revenue?');
    expect(s.texts().join('\n')).toContain(PICK_TEXT);
    expect(s.chips().map((c) => c.label)).toEqual(['Pick a profile', 'Insert as text', 'Copy']);
    expect(s.inserted).toEqual([]);
    expect(s.saved).toEqual([]);
  });

  it('[RTR-15c] [RTR-01b] a wh-word dictation on the no-profile card can be inserted as text', async () => {
    const s = setup();
    await s.pick.showNoProfile(chip(MISREAD), owned);
    await s.click('Insert as text');
    expect(s.inserted).toEqual([MISREAD]);
    expect(s.saved).toEqual([]);
  });

  it('[RTR-15c] [RTR-01b] with insertion off, or a failed insert, the text stays on the card with Copy', async () => {
    for (const over of [
      { insertionOn: () => false },
      { insertText: async () => ({ result: 'no_focus' as const }) },
      { insertText: async () => Promise.reject({ error: 'host_failed' }) },
    ]) {
      const s = setup(over);
      await s.pick.showNoProfile(chip(MISREAD), owned);
      await s.click('Insert as text');
      expect(s.texts().join('\n')).toContain(MISREAD);
      expect(s.chips().map((c) => c.label)).toContain('Copy');
    }
  });

  it('[RTR-15c] a preview route (Shift held) with no home profile also shows the card, not a send', async () => {
    const s = setup();
    const r: AskRoute = {
      route: 'preview',
      question: 'Revenue by region?',
      forced: true,
      previewMs: 1500,
      decision: routerDecision(),
    };
    await s.pick.showNoProfile(r, owned);
    expect(s.texts().join('\n')).toContain('Revenue by region?');
    expect(s.chips().map((c) => c.label)).toContain('Pick a profile');
  });

  it('[W-07d] Pick a profile lists the profiles; the user clicks one and only its name is saved as home', async () => {
    const s = setup();
    await s.pick.showNoProfile(chip('What was revenue?'), owned);
    await s.click('Pick a profile');
    expect(s.chips().map((c) => c.label)).toEqual(expect.arrayContaining(['fake-a', 'fake-b']));
    // Nothing is saved before the click on a profile.
    expect(s.saved).toEqual([]);
    await s.click('fake-b');
    expect(s.saved).toEqual([[A, 'fake-b']]);
    expect(s.texts().join('\n')).toContain('fake-b');
  });

  it('[W-07d] one profile is listed, never picked for the user', async () => {
    const s = setup({ listProfiles: async () => [fake('fake-only')] });
    await s.pick.showNoProfile(chip('What was revenue?'), owned);
    await s.click('Pick a profile');
    expect(s.chips().map((c) => c.label)).toContain('fake-only');
    expect(s.saved).toEqual([]);
  });

  it('[W-07d] unsupported profiles are not offered', async () => {
    const s = setup({
      listProfiles: async () => [
        fake('fake-a'),
        { ...fake('fake-pat'), authType: 'pat', supported: false },
      ],
    });
    await s.pick.showNoProfile(chip('What was revenue?'), owned);
    await s.click('Pick a profile');
    expect(s.chips().map((c) => c.label)).not.toContain('fake-pat');
  });

  it('[W-07d] when the profile list fails, the card says so and keeps the text', async () => {
    const s = setup({ listProfiles: async () => Promise.reject({ error: 'not_implemented' }) });
    await s.pick.showNoProfile(chip(MISREAD), owned);
    await s.click('Pick a profile');
    expect(s.texts().join('\n')).toContain(MISREAD);
    expect(s.texts().join('\n')).toMatch(/cannot list/i);
    expect(s.saved).toEqual([]);
  });

  it('[RTR-01c] with insertion off, Insert as text makes no insert_text call', async () => {
    const s = setup({ insertionOn: () => false });
    await s.pick.showNoProfile(chip(MISREAD), owned);
    await s.click('Insert as text');
    expect(s.log).not.toContain('insert_text');
    expect(s.texts().join('\n')).toContain(MISREAD);
  });

  it('[W-07d] a second click on Pick a profile does not list the profiles twice', async () => {
    const s = setup();
    await s.pick.showNoProfile(chip('What was revenue?'), owned);
    await s.click('Pick a profile');
    await s.click('Pick a profile');
    expect(s.chips().filter((c) => c.label === 'fake-a')).toHaveLength(1);
    expect(s.texts().filter((t) => t.startsWith('Which workspace?'))).toHaveLength(1);
  });
});
