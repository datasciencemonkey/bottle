import type { BackendBinding } from '../shared/interfaces';
import { core } from './test-core';
import { createRequestOwner, sendOwnedCard } from './request-owner';
import { createTalkTarget } from './talk-target';

vi.mock('@tauri-apps/api/core', async () => (await import('./test-core')).coreModule);
vi.mock('@tauri-apps/api/event', async () => (await import('./test-core')).eventModule);

const A = '00000000-0000-4000-8000-00000000000a';
const B = '00000000-0000-4000-8000-00000000000b';
const binding: BackendBinding = { kind: 'genie_one_mcp', profile: 'test-profile' };

/** The utterance `begin` gave, or a test failure. */
function must<T>(v: T | undefined): T {
  if (v === undefined) throw new Error('expected a value');
  return v;
}

function setup() {
  const target = createTalkTarget({
    send: () => undefined,
    setActive: async () => undefined,
    after: () => () => undefined,
  });
  let n = 0;
  const cards: { mogId: string; update: unknown }[] = [];
  const owner = createRequestOwner({
    isOpen: (id) => target.isOpen(id),
    newId: () => `u${++n}`,
    sendCard: async (mogId, update) => void cards.push({ mogId, update }),
  });
  target.viewReady(A);
  target.viewReady(B);
  return { target, owner, cards };
}

describe('the MOG that captures an utterance owns the request (#73)', () => {
  beforeEach(() => core.reset());

  it('[MOG-13a] the request carries the mogId of the capturing MOG, also after a target change', () => {
    const { target, owner } = setup();
    target.interacted(A);
    const u = owner.begin(A);
    expect(u).toEqual({ utteranceId: 'u1', mogId: A });
    // The user clicks MOG B while A transcribes.
    target.interacted(B);
    const req = owner.stampRequest('u1', { opId: 'op-1', binding, question: 'q' });
    expect(req?.mogId).toBe(A);
    expect(req?.tags).toEqual({ app: 'bottle', mog: A, op: 'genie' });
  });

  it('[MOG-13a] no utterance starts for a MOG that is not open; a cancel drops the owner', () => {
    const { target, owner } = setup();
    expect(owner.begin('00000000-0000-4000-8000-00000000000f')).toBeUndefined();
    target.closed(B);
    expect(owner.begin(B)).toBeUndefined();
    const u = must(owner.begin(A));
    owner.cancel(u.utteranceId);
    expect(owner.ownerOf(u.utteranceId)).toBeUndefined();
    expect(owner.stampRequest(u.utteranceId, { opId: 'op', binding, question: 'q' })).toBe(
      undefined,
    );
  });

  it('[MOG-13b] the answer appears on the capturing MOG in 20 of 20 trials after a focus change', async () => {
    const { target, owner, cards } = setup();
    for (let i = 0; i < 20; i++) {
      const [from, to] = i % 2 === 0 ? [A, B] : [B, A];
      // A focus change: the card of the other MOG gets focus (Rust sees it too).
      target.interacted(from, 'focus');
      const u = must(owner.begin(from));
      target.interacted(to, 'focus');
      expect(target.current()).toBe(to);
      expect(await owner.deliverAnswer(u.utteranceId, { blurbs: [], chips: [] })).toBe(true);
      expect(cards.at(-1)).toEqual({
        mogId: from,
        update: { mogId: from, blurbs: [], chips: [] },
      });
      owner.release(u.utteranceId);
    }
    expect(cards).toHaveLength(20);
  });

  it('[MOG-13b] an answer for an unknown or released utterance goes nowhere', async () => {
    const { owner, cards } = setup();
    expect(await owner.deliverAnswer('nope', { blurbs: [], chips: [] })).toBe(false);
    const u = must(owner.begin(A));
    owner.release(u.utteranceId);
    expect(await owner.deliverAnswer(u.utteranceId, { blurbs: [], chips: [] })).toBe(false);
    expect(cards).toEqual([]);
  });

  it('[MOG-13b] [ARC-5] the production send goes to the card window of the owner only', async () => {
    await sendOwnedCard(A, { mogId: A, blurbs: [], chips: [] });
    expect(core.emitTo).toHaveBeenCalledWith(`card-${A}`, 'card:update', {
      mogId: A,
      blurbs: [],
      chips: [],
    });
    expect(core.emit).not.toHaveBeenCalled();
  });
});
