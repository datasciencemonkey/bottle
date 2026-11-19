/**
 * thread-focus (#145 item 4): the hub's "focused on a thread" signal, and the view input that
 * feeds it and the composer's Send (`ThreadComposer.tsx`'s `thread.active` / `thread.inactive` /
 * `thread.send`, all under the Conversations view id).
 */
import { BrainBus } from './bus';
import type { FollowupInput, GenieFollowup } from './genie-followup';
import { createThreadFocus, wireThreadFollowup } from './thread-focus';

let bus: BrainBus;
beforeEach(() => {
  bus = new BrainBus();
});

function input(payload: unknown, from = 'hub', mogId = 'conversations', kind = 'click'): void {
  bus.dispatch({ event: 'view:input', payload: { from, input: { mogId, kind, payload } } });
}

function fakeFollowup() {
  const sends: FollowupInput[] = [];
  const followup: GenieFollowup = {
    send: async (x) => void sends.push(x),
  };
  return { followup, sends };
}

describe('createThreadFocus (#145)', () => {
  it('[#145] holds no thread until set, then holds exactly what was set', () => {
    const focus = createThreadFocus();
    expect(focus.active()).toBeUndefined();
    focus.set({ conversationId: 'c1', profile: 'fake-west' });
    expect(focus.active()).toEqual({ conversationId: 'c1', profile: 'fake-west' });
    focus.set(undefined);
    expect(focus.active()).toBeUndefined();
  });
});

describe('wireThreadFollowup (#145)', () => {
  it('[#145] thread.active sets the focus; thread.inactive clears it', () => {
    const focus = createThreadFocus();
    wireThreadFollowup(bus, { focus, followup: fakeFollowup().followup });
    input({ action: 'thread.active', conversationId: 'c1', profile: 'fake-west' });
    expect(focus.active()).toEqual({ conversationId: 'c1', profile: 'fake-west' });
    input({ action: 'thread.inactive' });
    expect(focus.active()).toBeUndefined();
  });

  it('[#145] thread.send calls the follow-up sender with the exact question', async () => {
    const focus = createThreadFocus();
    const f = fakeFollowup();
    wireThreadFollowup(bus, { focus, followup: f.followup });
    input({
      action: 'thread.send',
      conversationId: 'c1',
      profile: 'fake-west',
      question: 'How many are there?',
    });
    await Promise.resolve();
    expect(f.sends).toEqual([
      { conversationId: 'c1', profile: 'fake-west', question: 'How many are there?' },
    ]);
  });

  it('[#145] [ARC-5] ignores input from another window, another view id or another kind', () => {
    const focus = createThreadFocus();
    wireThreadFollowup(bus, { focus, followup: fakeFollowup().followup });
    input({ action: 'thread.active', conversationId: 'c1', profile: 'fake-west' }, 'mog-x');
    input({ action: 'thread.active', conversationId: 'c1', profile: 'fake-west' }, 'hub', 'home');
    input(
      { action: 'thread.active', conversationId: 'c1', profile: 'fake-west' },
      'hub',
      'conversations',
      'ready',
    );
    expect(focus.active()).toBeUndefined();
  });

  it('[#145] a malformed payload changes nothing', () => {
    const focus = createThreadFocus();
    const f = fakeFollowup();
    wireThreadFollowup(bus, { focus, followup: f.followup });
    input({ action: 'thread.active', conversationId: '', profile: 'fake-west' });
    input({ action: 'thread.send', conversationId: 'c1', profile: '', question: 'x' });
    input({ action: 'thread.send', conversationId: 'c1', profile: 'fake-west', question: '   ' });
    expect(focus.active()).toBeUndefined();
    expect(f.sends).toEqual([]);
  });

  it('[#145] dispose stops the hub input', () => {
    const focus = createThreadFocus();
    const w = wireThreadFollowup(bus, { focus, followup: fakeFollowup().followup });
    w.dispose();
    input({ action: 'thread.active', conversationId: 'c1', profile: 'fake-west' });
    expect(focus.active()).toBeUndefined();
  });
});
